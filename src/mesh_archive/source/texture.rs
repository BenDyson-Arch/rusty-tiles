//! Embedded core textures admitted from bounded bytes; no URI resolution or reencoding.
use super::{
    field, invalid, list, object, offset, reference, uint, unsupported, Image, Result,
    MAX_SOURCE_BYTES,
};
use image::{ColorType, DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};
use serde_json::Value;
use std::io::Cursor;

const MAX_IMAGES: usize = 32;
const MAX_DIMENSION: u32 = 4096;
const MAX_IMAGE_PIXELS: u64 = 4 * 1024 * 1024;
const MAX_TOTAL_PIXELS: u64 = 16 * 1024 * 1024;

// Framing complements the image decoder, which decodes the first PNG frame
// without necessarily consuming the terminal chunks. This is deliberately not
// another PNG raster/schema decoder.
fn png_framing(bytes: &[u8], check: &mut impl FnMut() -> Result<()>) -> Result<()> {
    let mut cursor = 8usize;
    let mut saw_header = false;
    let mut saw_data = false;
    while cursor < bytes.len() {
        check()?;
        let header_end = cursor
            .checked_add(8)
            .ok_or_else(|| invalid("PNG chunk header overflow"))?;
        let header = bytes
            .get(cursor..header_end)
            .ok_or_else(|| invalid("truncated PNG chunk header"))?;
        let length = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
        let kind = &header[4..];
        if !kind.iter().all(u8::is_ascii_alphabetic) {
            return Err(invalid("invalid PNG chunk type"));
        }
        let data_end = header_end
            .checked_add(length)
            .ok_or_else(|| invalid("PNG chunk length overflow"))?;
        let end = data_end
            .checked_add(4)
            .ok_or_else(|| invalid("PNG chunk CRC offset overflow"))?;
        let crc_bytes = bytes
            .get(data_end..end)
            .ok_or_else(|| invalid("truncated PNG chunk data/CRC"))?;
        let mut hash = crc32fast::Hasher::new();
        for portion in bytes[cursor + 4..data_end].chunks(64 * 1024) {
            check()?;
            hash.update(portion);
        }
        if hash.finalize() != u32::from_be_bytes(crc_bytes.try_into().unwrap()) {
            return Err(invalid("PNG chunk CRC mismatch"));
        }
        if !saw_header && kind != b"IHDR" {
            return Err(invalid("PNG first chunk must be IHDR"));
        }
        match kind {
            b"IHDR" => {
                if saw_header || length != 13 {
                    return Err(invalid("PNG requires one 13-byte IHDR"));
                }
                saw_header = true;
            }
            b"IDAT" => saw_data = true,
            b"IEND" => {
                if length != 0 || end != bytes.len() || !saw_data {
                    return Err(invalid("PNG requires final zero-length IEND after IDAT"));
                }
                return Ok(());
            }
            b"acTL" | b"fcTL" | b"fdAT" => {
                return Err(unsupported("animated PNG outside static image profile"));
            }
            b"PLTE" => {}
            _ if kind[0].is_ascii_uppercase() => {
                return Err(unsupported("unknown critical PNG chunk"));
            }
            _ => {}
        }
        cursor = end;
    }
    Err(invalid("PNG missing terminal IEND"))
}

fn image_error(error: image::ImageError) -> crate::JobError {
    match error {
        image::ImageError::Limits(_) => {
            unsupported(format!("image decoder admission limit: {error}"))
        }
        _ => invalid(format!("invalid embedded image: {error}")),
    }
}

fn labels_removed(values: &[Value]) -> Vec<Value> {
    values
        .iter()
        .cloned()
        .map(|mut value| {
            value
                .as_object_mut()
                .expect("validated object")
                .remove("name");
            value
        })
        .collect()
}

pub(super) fn decode(
    doc: &Value,
    bin: &[u8],
    check: &mut impl FnMut() -> Result<()>,
) -> Result<(Vec<Image>, Vec<Value>, Vec<Value>)> {
    let source_images = list(doc, "images")?;
    let textures = list(doc, "textures")?;
    let samplers = list(doc, "samplers")?;
    if [source_images.len(), textures.len(), samplers.len()]
        .into_iter()
        .any(|n| n > MAX_IMAGES)
    {
        return Err(unsupported("image/texture/sampler count exceeds 32"));
    }
    for sampler in samplers {
        check()?;
        object(
            sampler,
            &["name", "magFilter", "minFilter", "wrapS", "wrapT"],
        )?;
        for (key, allowed) in [
            ("magFilter", &[9728, 9729][..]),
            ("minFilter", &[9728, 9729, 9984, 9985, 9986, 9987][..]),
            ("wrapS", &[33071, 33648, 10497][..]),
            ("wrapT", &[33071, 33648, 10497][..]),
        ] {
            if sampler
                .get(key)
                .map(uint)
                .transpose()?
                .is_some_and(|n| !allowed.contains(&n))
            {
                return Err(invalid(format!("invalid sampler {key}")));
            }
        }
    }
    for texture in textures {
        check()?;
        object(texture, &["name", "source", "sampler"])?;
        let source = texture
            .get("source")
            .ok_or_else(|| unsupported("texture without core image source"))?;
        reference(source_images, source)?;
        if let Some(index) = texture.get("sampler") {
            reference(samplers, index)?;
        }
    }
    let views = list(doc, "bufferViews")?;
    let accessors = list(doc, "accessors")?;
    let mut accessor_views = std::collections::HashSet::new();
    let mut accessor_ranges = Vec::new();
    for (index, accessor) in accessors.iter().enumerate() {
        if index.is_multiple_of(1024) {
            check()?;
        }
        let view_index = field(accessor, "bufferView")?;
        if accessor_views.insert(view_index) {
            let view = reference(views, &accessor["bufferView"])?;
            let start = offset(view, "byteOffset")?;
            let end = start
                .checked_add(field(view, "byteLength")?)
                .ok_or_else(|| invalid("accessor view range overflow"))?;
            accessor_ranges.push((start, end));
        }
    }
    let mut images = Vec::with_capacity(source_images.len());
    let mut total_pixels = 0u64;
    let mut total_bytes = 0usize;
    for source in source_images {
        check()?;
        // In particular, URI, extras, and extensions remain outside this profile.
        object(source, &["name", "bufferView", "mimeType"])?;
        let view = reference(views, &source["bufferView"])?;
        if view.get("byteStride").is_some() || view.get("target").is_some() {
            return Err(invalid(
                "image bufferView must not have byteStride or target",
            ));
        }
        let start = offset(view, "byteOffset")?;
        let length = field(view, "byteLength")?;
        let end = start
            .checked_add(length)
            .ok_or_else(|| invalid("image view range overflow"))?;
        for (index, &(other_start, other_end)) in accessor_ranges.iter().enumerate() {
            if index.is_multiple_of(1024) {
                check()?;
            }
            if start < other_end && other_start < end {
                return Err(invalid("image bufferView overlaps accessor bufferView"));
            }
        }
        let encoded = bin
            .get(start..end)
            .ok_or_else(|| invalid("image bufferView outside actual BIN"))?;
        total_bytes = total_bytes
            .checked_add(length)
            .ok_or_else(|| unsupported("image byte sum overflow"))?;
        if total_bytes > MAX_SOURCE_BYTES {
            return Err(unsupported("owned image bytes exceed 32 MiB"));
        }
        let (mime_type, extension, format) = match source.get("mimeType").and_then(Value::as_str) {
            Some("image/png") => ("image/png", "png", ImageFormat::Png),
            Some("image/jpeg") => ("image/jpeg", "jpg", ImageFormat::Jpeg),
            Some(_) => return Err(unsupported("only PNG/JPEG embedded images are supported")),
            None => return Err(invalid("embedded image requires string mimeType")),
        };
        let detected = image::guess_format(encoded)
            .map_err(|_| invalid("embedded image signature is invalid"))?;
        if detected != format {
            return Err(invalid("image MIME type does not match encoded bytes"));
        }
        match format {
            ImageFormat::Png => png_framing(encoded, check)?,
            // A terminal-marker requirement supplements pixel decoding; it
            // does not claim a separate complete JPEG stream schema check.
            ImageFormat::Jpeg if !encoded.ends_with(&[0xff, 0xd9]) => {
                return Err(invalid("JPEG requires terminal EOI marker"));
            }
            _ => {}
        }
        let mut limits = Limits::default();
        limits.max_image_width = Some(MAX_DIMENSION);
        limits.max_image_height = Some(MAX_DIMENSION);
        // Decoded pixels are discarded serially. This allocation ceiling is a
        // decoder guard, not a claim that image's internal limits bound RSS.
        limits.max_alloc = Some(128 * 1024 * 1024);
        let mut reader = ImageReader::with_format(Cursor::new(encoded), format);
        reader.limits(limits);
        let decoder = reader.into_decoder().map_err(image_error)?;
        if !matches!(
            decoder.color_type(),
            ColorType::L8 | ColorType::La8 | ColorType::Rgb8 | ColorType::Rgba8
        ) {
            return Err(unsupported(
                "only images with 8-bit decoded channels are supported",
            ));
        }
        let (width, height) = decoder.dimensions();
        let pixels = u64::from(width) * u64::from(height);
        if width == 0 || height == 0 {
            return Err(invalid("image dimensions must be nonzero"));
        }
        if width > MAX_DIMENSION || height > MAX_DIMENSION || pixels > MAX_IMAGE_PIXELS {
            return Err(unsupported(
                "image exceeds 4096 edge or 4 Mi pixel admission ceiling",
            ));
        }
        total_pixels = total_pixels
            .checked_add(pixels)
            .ok_or_else(|| unsupported("image pixel sum overflow"))?;
        if total_pixels > MAX_TOTAL_PIXELS {
            return Err(unsupported("images exceed aggregate 16 Mi pixel ceiling"));
        }
        check()?;
        let decoded = DynamicImage::from_decoder(decoder).map_err(image_error)?;
        drop(decoded);
        check()?;
        images.push(Image {
            bytes: encoded.to_vec(),
            mime_type,
            extension,
            width,
            height,
        });
    }
    Ok((images, labels_removed(textures), labels_removed(samplers)))
}
