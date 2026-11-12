//! Core textures admitted from bounded borrowed snapshots; no URI resolution or reencoding.
use super::{
    field, invalid, list, object, offset, reference, uint, unsupported, Image, ImageSource,
    ResourceRequest, Result, MAX_SOURCE_BYTES,
};
use image::{ColorType, DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};
use serde_json::Value;
use std::io::Cursor;

const MAX_DIMENSION: u32 = 4096;
const MAX_IMAGE_PIXELS: u64 = 4 * 1024 * 1024;
const MAX_TOTAL_PIXELS: u64 = 16 * 1024 * 1024;

/// Per-image decoder admission: encoded-length, edge, texel and decoder-allocation
/// ceilings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DecodeAdmission {
    /// Encoded bytes of one image; a whole number of MiB (it is named in refusals).
    pub max_encoded: u64,
    pub max_edge: u32,
    pub max_pixels: u64,
    pub max_alloc: u64,
}
impl DecodeAdmission {
    /// The retained finite-source consumer: 32 MiB encoded, 4096 edge, 4 Mi texels.
    pub(crate) const FINITE: Self = Self {
        max_encoded: MAX_SOURCE_BYTES as u64,
        max_edge: MAX_DIMENSION,
        max_pixels: MAX_IMAGE_PIXELS,
        max_alloc: 128 * 1024 * 1024,
    };
    /// Streamed large meshes: the 16384 maximum texture edge that WebGL2/WebGPU
    /// devices and the KTX2 encoder accept, so any source a GPU could sample
    /// directly is admitted. The decoder may allocate at most eight bytes per
    /// admitted texel (component planes plus RGB(A) output). Whether the
    /// decoded RGBA8 owner fits is a separate, memory-derived refusal of the
    /// consuming stage, which charges every decode against available memory.
    /// The encoded length is likewise charged at its actual size by each
    /// consumer; 1 GiB is only a sanity ceiling, above any legitimate 16384²
    /// RGBA8 PNG or JPEG (at most about 4 bytes per texel plus framing).
    pub(crate) const LARGE_MESH: Self = Self {
        max_encoded: 1 << 30,
        max_edge: 16_384,
        max_pixels: 16_384 * 16_384,
        max_alloc: 8 * 16_384 * 16_384,
    };
}

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

pub(super) fn metadata(
    doc: &Value,
    resources: &mut Vec<ResourceRequest>,
    max_images: usize,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<(Vec<ImageSource>, Vec<Value>, Vec<Value>)> {
    let source_images = list(doc, "images")?;
    let textures = list(doc, "textures")?;
    let samplers = list(doc, "samplers")?;
    if [source_images.len(), textures.len(), samplers.len()]
        .into_iter()
        .any(|n| n > max_images)
    {
        return Err(unsupported(format!(
            "image/texture/sampler count exceeds {max_images}"
        )));
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
            accessor_ranges.push((field(view, "buffer")?, start, end));
        }
    }
    let mut sources = Vec::with_capacity(source_images.len());
    for source in source_images {
        check()?;
        object(source, &["name", "bufferView", "mimeType", "uri"])?;
        if let Some(mime) = source.get("mimeType") {
            match mime.as_str() {
                Some("image/png" | "image/jpeg") => {}
                Some(_) => return Err(unsupported("only PNG/JPEG images are supported")),
                None => return Err(invalid("image mimeType must be a string")),
            }
        }
        match (source.get("uri"), source.get("bufferView")) {
            (Some(uri), None) => {
                let uri = uri
                    .as_str()
                    .ok_or_else(|| invalid("image URI must be a string"))?;
                sources.push(ImageSource::External(resources.len()));
                resources.push(ResourceRequest { uri: uri.into() });
            }
            (None, Some(view_index)) => {
                if source.get("mimeType").is_none() {
                    return Err(invalid("bufferView image requires mimeType"));
                }
                let view = reference(views, view_index)?;
                if view.get("byteStride").is_some() || view.get("target").is_some() {
                    return Err(invalid(
                        "image bufferView must not have byteStride or target",
                    ));
                }
                let buffer = field(view, "buffer")?;
                let start = offset(view, "byteOffset")?;
                let end = start
                    .checked_add(field(view, "byteLength")?)
                    .ok_or_else(|| invalid("image view range overflow"))?;
                for (i, &(other_buffer, other_start, other_end)) in
                    accessor_ranges.iter().enumerate()
                {
                    if i.is_multiple_of(1024) {
                        check()?;
                    }
                    if buffer == other_buffer && start < other_end && other_start < end {
                        return Err(invalid("image bufferView overlaps accessor bufferView"));
                    }
                }
                sources.push(ImageSource::BufferView(uint(view_index)?));
            }
            _ => return Err(invalid("image requires exactly one of URI or bufferView")),
        }
    }
    Ok((sources, labels_removed(textures), labels_removed(samplers)))
}

/// One bounded decoder shared by the retained finite-source consumer and streamed images.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ImageInfo {
    pub mime_type: &'static str,
    pub extension: &'static str,
    pub width: u32,
    pub height: u32,
    /// Length of the admitted encoded image, which decoding holds beside its output.
    pub encoded_bytes: u64,
}
/// The declared MIME type, or the signature when none is declared, resolved to PNG or JPEG.
/// `prefix` is the encoded image or at least its first 64 bytes, which every signature fits.
fn resolve_format(
    prefix: &[u8],
    mime: Option<&str>,
) -> Result<(&'static str, &'static str, ImageFormat)> {
    let detected =
        image::guess_format(prefix).map_err(|_| invalid("image signature is invalid"))?;
    let resolved = match mime {
        Some("image/png") => ("image/png", "png", ImageFormat::Png),
        Some("image/jpeg") => ("image/jpeg", "jpg", ImageFormat::Jpeg),
        Some(_) => return Err(unsupported("only PNG/JPEG images are supported")),
        None => match detected {
            ImageFormat::Png => ("image/png", "png", ImageFormat::Png),
            ImageFormat::Jpeg => ("image/jpeg", "jpg", ImageFormat::Jpeg),
            _ => return Err(unsupported("only PNG/JPEG image signatures are supported")),
        },
    };
    if detected != resolved.2 {
        return Err(invalid("image MIME type does not match encoded bytes"));
    }
    Ok(resolved)
}

/// Admits declared dimensions under `admission` and returns the texel count.
fn admit_dimensions(width: u32, height: u32, admission: DecodeAdmission) -> Result<u64> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 {
        return Err(invalid("image dimensions must be nonzero"));
    }
    if width > admission.max_edge || height > admission.max_edge || pixels > admission.max_pixels {
        return Err(unsupported(format!(
            "image exceeds {} edge or {} pixel admission ceiling",
            admission.max_edge, admission.max_pixels
        )));
    }
    Ok(pixels)
}

/// One window of an image read on demand: at most 64 KiB resident, refilled forward.
struct HeaderWindow<'a> {
    read: &'a mut dyn FnMut(u64, &mut [u8]) -> Result<()>,
    length: u64,
    start: u64,
    bytes: Vec<u8>,
}
impl HeaderWindow<'_> {
    const BYTES: u64 = 64 * 1024;
    /// `n` bytes at `offset`, or `None` when the image ends first.
    fn get(
        &mut self,
        offset: u64,
        n: usize,
        check: &mut dyn FnMut() -> Result<()>,
    ) -> Result<Option<&[u8]>> {
        let end = match offset.checked_add(n as u64) {
            Some(end) if end <= self.length => end,
            _ => return Ok(None),
        };
        if offset < self.start || end > self.start + self.bytes.len() as u64 {
            check()?;
            let take = (self.length - offset).min(Self::BYTES.max(n as u64));
            self.bytes.resize(take as usize, 0);
            (self.read)(offset, &mut self.bytes)?;
            self.start = offset;
        }
        let at = (offset - self.start) as usize;
        Ok(Some(&self.bytes[at..at + n]))
    }
}

/// An image's declared dimensions read from its leading header alone, so a consumer can
/// admit and charge its decode before holding the encoded image. `read(offset, out)` fills
/// `out` from the encoded image of `length` bytes. Format resolution and the encoded,
/// edge and texel ceilings are those of [`admitted_decoder`]; PNG's IHDR (with its CRC
/// and bit depth) or JPEG's first frame header, found by walking marker segments, gives
/// the dimensions. At most 64 KiB is resident. This is an early refusal only: the
/// decoder still admits the whole image, and its consumer checks that it reports these
/// dimensions.
pub(crate) fn probe_header_dimensions(
    length: u64,
    mime: Option<&str>,
    admission: DecodeAdmission,
    read: &mut dyn FnMut(u64, &mut [u8]) -> Result<()>,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<(u32, u32)> {
    encoded_admission(length, admission)?;
    let mut window = HeaderWindow {
        read,
        length,
        start: 0,
        bytes: Vec::new(),
    };
    let prefix = window
        .get(0, length.min(64) as usize, check)?
        .unwrap_or_default();
    let (_, _, format) = resolve_format(prefix, mime)?;
    let be16 = |b: &[u8], at: usize| u16::from_be_bytes([b[at], b[at + 1]]);
    let be32 = |b: &[u8], at: usize| u32::from_be_bytes(b[at..at + 4].try_into().unwrap());
    let (width, height) = if format == ImageFormat::Png {
        let ihdr = window
            .get(8, 25, check)?
            .ok_or_else(|| invalid("truncated PNG chunk header"))?;
        if &ihdr[4..8] != b"IHDR" {
            return Err(invalid("PNG first chunk must be IHDR"));
        }
        if be32(ihdr, 0) != 13 {
            return Err(invalid("PNG requires one 13-byte IHDR"));
        }
        if crc32fast::hash(&ihdr[4..21]) != be32(ihdr, 21) {
            return Err(invalid("PNG chunk CRC mismatch"));
        }
        if ihdr[16] == 16 {
            return Err(unsupported(
                "only images with 8-bit decoded channels are supported",
            ));
        }
        (be32(ihdr, 8), be32(ihdr, 12))
    } else {
        // JPEG: segments after SOI until the first SOFn (not DHT C4, JPG C8 or DAC CC).
        // Like the decoder, stray bytes before a marker and 0xFF fill or 0x00 stuffing
        // after one are skipped rather than refused.
        let mut offset = 2u64;
        loop {
            let marker = window
                .get(offset, 2, check)?
                .ok_or_else(|| invalid("JPEG ends before its frame header"))?;
            if marker[0] != 0xff {
                offset += 1;
                continue;
            }
            match marker[1] {
                0xff => offset += 1,
                0x00 | 0x01 | 0xd0..=0xd7 => offset += 2,
                0xc0..=0xcf if !matches!(marker[1], 0xc4 | 0xc8 | 0xcc) => {
                    let frame = window
                        .get(offset + 2, 7, check)?
                        .ok_or_else(|| invalid("JPEG ends inside its frame header"))?;
                    break (u32::from(be16(frame, 5)), u32::from(be16(frame, 3)));
                }
                0xd8..=0xda => {
                    return Err(invalid("JPEG frame header missing before scan"));
                }
                _ => {
                    let size = window
                        .get(offset + 2, 2, check)?
                        .ok_or_else(|| invalid("JPEG ends inside a segment length"))?;
                    let size = be16(size, 0);
                    if size < 2 {
                        return Err(invalid("JPEG segment length is invalid"));
                    }
                    offset += 2 + u64::from(size);
                }
            }
        }
    };
    admit_dimensions(width, height, admission)?;
    Ok((width, height))
}

fn admitted_decoder<'a>(
    encoded: &'a [u8],
    mime: Option<&str>,
    admission: DecodeAdmission,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<(impl ImageDecoder + 'a, ImageInfo)> {
    encoded_admission(encoded.len() as u64, admission)?;
    let (mime_type, extension, format) = resolve_format(encoded, mime)?;
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
    limits.max_image_width = Some(admission.max_edge);
    limits.max_image_height = Some(admission.max_edge);
    // This allocation ceiling is a decoder guard. Callers also charge their
    // encoded/RGBA owners; image's internal limits alone do not bound RSS.
    limits.max_alloc = Some(admission.max_alloc);
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
    admit_dimensions(width, height, admission)?;
    Ok((
        decoder,
        ImageInfo {
            mime_type,
            extension,
            width,
            height,
            encoded_bytes: encoded.len() as u64,
        },
    ))
}

/// Decodes one captured image, with per-image guards and no aggregate payload ceiling.
/// The caller charges encoded bytes, decoder workspace and the owning RGBA8 allocation.
#[cfg(test)]
pub(crate) fn decode_image(
    encoded: &[u8],
    mime: Option<&str>,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<(ImageInfo, Vec<u8>)> {
    decode_image_admitted(encoded, mime, DecodeAdmission::FINITE, check)
}

fn encoded_admission(length: u64, admission: DecodeAdmission) -> Result<()> {
    if length > admission.max_encoded {
        return Err(unsupported(format!(
            "image exceeds {} MiB encoded-byte admission ceiling",
            admission.max_encoded >> 20
        )));
    }
    Ok(())
}

/// Admits one image's header under `admission` and returns its dimensions
/// without decoding pixels, so callers can charge the decode before it runs.
pub(crate) fn probe_image_admitted(
    encoded: &[u8],
    mime: Option<&str>,
    admission: DecodeAdmission,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<ImageInfo> {
    admitted_decoder(encoded, mime, admission, check).map(|(_, info)| info)
}

/// [`decode_image`] under an explicit per-image admission.
pub(crate) fn decode_image_admitted(
    encoded: &[u8],
    mime: Option<&str>,
    admission: DecodeAdmission,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<(ImageInfo, Vec<u8>)> {
    let (decoder, info) = admitted_decoder(encoded, mime, admission, check)?;
    check()?;
    let rgba = DynamicImage::from_decoder(decoder)
        .map_err(image_error)?
        .into_rgba8()
        .into_raw();
    check()?;
    Ok((info, rgba))
}

pub(super) fn decode(
    doc: &Value,
    buffers: &[&[u8]],
    sources: &[ImageSource],
    external_images: &[Option<&[u8]>],
    check: &mut impl FnMut() -> Result<()>,
) -> Result<Vec<Image>> {
    let source_images = list(doc, "images")?;
    if sources.len() != source_images.len() || external_images.len() != source_images.len() {
        return Err(invalid(
            "image snapshot count disagrees with parsed document",
        ));
    }
    let views = list(doc, "bufferViews")?;
    let mut images = Vec::with_capacity(source_images.len());
    let mut total_pixels = 0u64;
    let mut total_bytes = 0usize;
    for (index, source) in source_images.iter().enumerate() {
        check()?;
        let encoded = match &sources[index] {
            ImageSource::BufferView(view_index) => {
                if external_images[index].is_some() {
                    return Err(invalid("bufferView image has an external snapshot"));
                }
                let view = views
                    .get(*view_index)
                    .ok_or_else(|| invalid("image view index out of range"))?;
                let start = offset(view, "byteOffset")?;
                let end = start
                    .checked_add(field(view, "byteLength")?)
                    .ok_or_else(|| invalid("image view range overflow"))?;
                buffers
                    .get(field(view, "buffer")?)
                    .and_then(|bytes| bytes.get(start..end))
                    .ok_or_else(|| invalid("image bufferView outside actual buffer"))?
            }
            ImageSource::External(_) => external_images[index]
                .ok_or_else(|| invalid("URI image requires external snapshot"))?,
        };
        let length = encoded.len();
        total_bytes = total_bytes
            .checked_add(length)
            .ok_or_else(|| unsupported("image byte sum overflow"))?;
        if total_bytes > MAX_SOURCE_BYTES {
            return Err(unsupported("owned image bytes exceed 32 MiB"));
        }
        let (decoder, info) = admitted_decoder(
            encoded,
            source.get("mimeType").and_then(Value::as_str),
            DecodeAdmission::FINITE,
            check,
        )?;
        let pixels = u64::from(info.width) * u64::from(info.height);
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
            mime_type: info.mime_type,
            extension: info.extension,
            width: info.width,
            height: info.height,
        });
    }
    Ok(images)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::JobErrorKind;

    /// `png` with one private ancillary chunk of zeros inserted before IEND, so the
    /// result is exactly `length` bytes and decodes to the same pixels.
    fn padded(png: &[u8], length: usize) -> Vec<u8> {
        let (body, end) = png.split_at(png.len() - 12);
        let data = length - png.len() - 12;
        let mut out = Vec::with_capacity(length);
        out.extend_from_slice(body);
        out.extend_from_slice(&u32::try_from(data).unwrap().to_be_bytes());
        let start = out.len();
        out.extend_from_slice(b"ruTi");
        out.resize(out.len() + data, 0);
        let crc = crc32fast::hash(&out[start..]);
        out.extend_from_slice(&crc.to_be_bytes());
        out.extend_from_slice(end);
        assert_eq!(out.len(), length);
        out
    }

    #[test]
    fn encoded_ceiling_belongs_to_the_admission_profile() {
        let mut png = Vec::new();
        image::RgbaImage::from_fn(3, 2, |x, y| {
            image::Rgba([x as u8 * 80, y as u8 * 90, 7, 255])
        })
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .unwrap();
        assert_eq!(DecodeAdmission::FINITE.max_encoded, 32 << 20);
        assert_eq!(DecodeAdmission::LARGE_MESH.max_encoded, 1 << 30);
        let over = padded(&png, (32 << 20) + 1);
        let finite =
            probe_image_admitted(&over, None, DecodeAdmission::FINITE, &mut || Ok(())).unwrap_err();
        assert_eq!(finite.kind(), JobErrorKind::Unsupported);
        assert!(finite
            .to_string()
            .contains("image exceeds 32 MiB encoded-byte admission ceiling"));
        let at = padded(&png, 32 << 20);
        let info =
            probe_image_admitted(&at, None, DecodeAdmission::FINITE, &mut || Ok(())).unwrap();
        assert_eq!(info.encoded_bytes, 32 << 20);
        let (_, plain) =
            decode_image_admitted(&png, None, DecodeAdmission::LARGE_MESH, &mut || Ok(())).unwrap();
        let (info, rgba) =
            decode_image_admitted(&over, None, DecodeAdmission::LARGE_MESH, &mut || Ok(()))
                .unwrap();
        assert_eq!((info.width, info.height), (3, 2));
        assert_eq!(info.encoded_bytes, (32 << 20) + 1);
        assert_eq!(rgba, plain);
    }

    /// Probes `encoded` as a consumer would, through reads at offsets, returning the
    /// dimensions and the bytes read.
    fn probe(
        encoded: &[u8],
        mime: Option<&str>,
        admission: DecodeAdmission,
    ) -> (Result<(u32, u32)>, u64) {
        let mut read = 0u64;
        let result = probe_header_dimensions(
            encoded.len() as u64,
            mime,
            admission,
            &mut |offset, out| {
                read += out.len() as u64;
                let at = offset as usize;
                out.copy_from_slice(&encoded[at..at + out.len()]);
                Ok(())
            },
            &mut || Ok(()),
        );
        (result, read)
    }

    #[test]
    fn header_probe_matches_the_decoder_and_reads_only_the_header() {
        let mut png = Vec::new();
        image::RgbImage::from_fn(300, 17, |x, y| image::Rgb([x as u8, y as u8, 3]))
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        let mut jpeg = Vec::new();
        image::RgbImage::from_fn(41, 260, |x, y| image::Rgb([x as u8 * 5, y as u8, 90]))
            .write_to(&mut Cursor::new(&mut jpeg), ImageFormat::Jpeg)
            .unwrap();
        // A large APP15 segment, stray bytes and 0xFF fill before the next marker, all of
        // which the decoder tolerates, so the walk must cross them too.
        let mut walked = jpeg[..2].to_vec();
        walked.extend([0xff, 0xef, 0xff, 0xff]);
        walked.extend(vec![0x5a; 0xfffd]);
        walked.extend([0x12, 0x34, 0xff, 0xff]);
        walked.extend(&jpeg[2..]);
        let padded = padded(&png, 3 << 20);
        for (name, encoded, mime) in [
            ("png", &png, Some("image/png")),
            ("png-padded", &padded, None),
            ("jpeg", &jpeg, Some("image/jpeg")),
            ("jpeg-walked", &walked, None),
        ] {
            let (_, info) =
                admitted_decoder(encoded, mime, DecodeAdmission::LARGE_MESH, &mut || Ok(()))
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
            let (dimensions, read) = probe(encoded, mime, DecodeAdmission::LARGE_MESH);
            assert_eq!(dimensions.unwrap(), (info.width, info.height), "{name}");
            // One 64 KiB window, or two when the walk crosses the APP15 segment.
            assert!(read <= 2 * HeaderWindow::BYTES, "{name}: read {read}");
        }
        assert_eq!(
            probe(&png, None, DecodeAdmission::LARGE_MESH).0.unwrap(),
            (300, 17)
        );
        assert_eq!(
            probe(&walked, None, DecodeAdmission::LARGE_MESH).0.unwrap(),
            (41, 260)
        );
    }

    #[test]
    fn header_probe_refusals_are_the_decoders() {
        let mut png = Vec::new();
        image::RgbaImage::from_fn(5, 4, |_, _| image::Rgba([1, 2, 3, 4]))
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        let large = DecodeAdmission::LARGE_MESH;
        let kind = |encoded: &[u8], mime: Option<&str>, admission: DecodeAdmission| {
            probe(encoded, mime, admission).0.unwrap_err().kind()
        };
        // Declared length over the encoded ceiling, before any read.
        let mut reads = 0;
        let error = probe_header_dimensions(
            large.max_encoded + 1,
            None,
            large,
            &mut |_, _| {
                reads += 1;
                Ok(())
            },
            &mut || Ok(()),
        )
        .unwrap_err();
        assert_eq!((error.kind(), reads), (JobErrorKind::Unsupported, 0));
        // Format resolution: declared non-PNG/JPEG MIME, and MIME disagreeing with bytes.
        assert_eq!(
            kind(&png, Some("image/webp"), large),
            JobErrorKind::Unsupported
        );
        assert_eq!(
            kind(&png, Some("image/jpeg"), large),
            JobErrorKind::InvalidInput
        );
        assert_eq!(
            kind(b"not an image at all", None, large),
            JobErrorKind::InvalidInput
        );
        // A corrupt IHDR CRC is invalid; a 16-bit IHDR (CRC fixed) is unsupported.
        let mut crc = png.clone();
        crc[29] ^= 1;
        assert_eq!(kind(&crc, None, large), JobErrorKind::InvalidInput);
        let mut deep = png.clone();
        deep[24] = 16;
        let fixed = crc32fast::hash(&deep[12..29]).to_be_bytes();
        deep[29..33].copy_from_slice(&fixed);
        assert_eq!(kind(&deep, None, large), JobErrorKind::Unsupported);
        // Edges over the admission are unsupported; the finite admission's 4096 edge is
        // the cheap witness of the same shared check.
        let mut wide = png.clone();
        wide[16..20].copy_from_slice(&4097u32.to_be_bytes());
        let fixed = crc32fast::hash(&wide[12..29]).to_be_bytes();
        wide[29..33].copy_from_slice(&fixed);
        assert_eq!(probe(&wide, None, large).0.unwrap(), (4097, 4));
        assert_eq!(
            kind(&wide, None, DecodeAdmission::FINITE),
            JobErrorKind::Unsupported
        );
        // A JPEG whose scan starts before any frame header, or that ends first.
        assert_eq!(
            kind(&[0xff, 0xd8, 0xff, 0xda, 0, 2, 0xff, 0xd9], None, large),
            JobErrorKind::InvalidInput
        );
        assert_eq!(
            kind(&[0xff, 0xd8, 0xff, 0xe0, 0, 16], None, large),
            JobErrorKind::InvalidInput
        );
    }
}
