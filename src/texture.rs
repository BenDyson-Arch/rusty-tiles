//! Decode source images (JPEG at IDCT 1/8 when huge); crop + resize + JPEG per leaf.

use std::io::Cursor;

use image::imageops::{self, FilterType};
use image::{DynamicImage, RgbaImage};

use crate::error::Error;
use crate::mesh::EncodedImage;

const UV_PAD_PX: u32 = 2;
const JPEG_QUALITY: u8 = 85;

/// Decode, downsampling so the long edge is at most `4 * tile_size` (JPEG uses IDCT 1/8/4/2).
pub fn decode_rgba(encoded: &EncodedImage, tile_size: u32) -> Result<RgbaImage, Error> {
    let bytes = encoded.load()?;
    if let Ok(img) = decode_jpeg_scaled(&bytes, tile_size) {
        return Ok(img);
    }
    let dynimg = image::load_from_memory(&bytes)?;
    let mut rgba = dynimg.to_rgba8();
    let cap = decode_cap(tile_size);
    let long = rgba.width().max(rgba.height());
    if long > cap {
        let s = cap as f32 / long as f32;
        let nw = ((rgba.width() as f32) * s).round().max(1.0) as u32;
        let nh = ((rgba.height() as f32) * s).round().max(1.0) as u32;
        rgba = imageops::resize(&rgba, nw, nh, FilterType::Triangle);
    }
    Ok(rgba)
}

fn decode_cap(tile_size: u32) -> u32 {
    tile_size.max(1).saturating_mul(4).max(256)
}

fn decode_jpeg_scaled(bytes: &[u8], tile_size: u32) -> Result<RgbaImage, Error> {
    let mut dec = jpeg_decoder::Decoder::new(Cursor::new(bytes));
    dec.read_info()
        .map_err(|e| Error::msg(format!("jpeg: {e}")))?;
    let cap = decode_cap(tile_size).min(u16::MAX as u32) as u16;
    let (w, h) = dec
        .scale(cap, cap)
        .map_err(|e| Error::msg(format!("jpeg scale: {e}")))?;
    let pixels = dec
        .decode()
        .map_err(|e| Error::msg(format!("jpeg decode: {e}")))?;
    let info = dec.info().ok_or_else(|| Error::msg("jpeg lost header"))?;
    let (w, h) = (w as u32, h as u32);
    match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => {
            let rgb = image::RgbImage::from_raw(w, h, pixels)
                .ok_or_else(|| Error::msg("jpeg RGB buffer size"))?;
            Ok(DynamicImage::ImageRgb8(rgb).to_rgba8())
        }
        jpeg_decoder::PixelFormat::L8 => {
            let gray = image::GrayImage::from_raw(w, h, pixels)
                .ok_or_else(|| Error::msg("jpeg L buffer size"))?;
            Ok(DynamicImage::ImageLuma8(gray).to_rgba8())
        }
        _ => Err(Error::msg("jpeg pixel format not RGB/L")),
    }
}

/// Crop the UV AABB (plus padding), fit to `tile_size`, encode JPEG, remap UVs to 0–1.
pub fn crop_leaf(
    image: &RgbaImage,
    uvs: &[[f32; 2]],
    tile_size: u32,
) -> Result<(Vec<u8>, Vec<[f32; 2]>), Error> {
    let (w, h) = (image.width().max(1), image.height().max(1));
    let (u0, v0, u1, v1) = uv_bounds(uvs);
    let mut x0 = ((u0 * w as f32).floor() as i32) - UV_PAD_PX as i32;
    let mut y0 = ((v0 * h as f32).floor() as i32) - UV_PAD_PX as i32;
    let mut x1 = ((u1 * w as f32).ceil() as i32) + UV_PAD_PX as i32;
    let mut y1 = ((v1 * h as f32).ceil() as i32) + UV_PAD_PX as i32;
    x0 = x0.clamp(0, w as i32 - 1);
    y0 = y0.clamp(0, h as i32 - 1);
    x1 = x1.clamp(x0 + 1, w as i32);
    y1 = y1.clamp(y0 + 1, h as i32);

    let crop_w = (x1 - x0) as u32;
    let crop_h = (y1 - y0) as u32;
    let cropped = imageops::crop_imm(image, x0 as u32, y0 as u32, crop_w, crop_h).to_image();

    let (rw, rh) = fit(crop_w, crop_h, tile_size.max(1));
    let resized = if rw == crop_w && rh == crop_h {
        cropped
    } else {
        imageops::resize(&cropped, rw, rh, FilterType::Triangle)
    };

    let jpeg = encode_jpeg(&resized)?;

    let u_span = (x1 - x0) as f32 / w as f32;
    let v_span = (y1 - y0) as f32 / h as f32;
    let u_org = x0 as f32 / w as f32;
    let v_org = y0 as f32 / h as f32;
    let remapped = uvs
        .iter()
        .map(|uv| {
            let u = if u_span > 1e-8 {
                ((uv[0] - u_org) / u_span).clamp(0.0, 1.0)
            } else {
                0.5
            };
            let v = if v_span > 1e-8 {
                ((uv[1] - v_org) / v_span).clamp(0.0, 1.0)
            } else {
                0.5
            };
            [u, v]
        })
        .collect();

    Ok((jpeg, remapped))
}

fn uv_bounds(uvs: &[[f32; 2]]) -> (f32, f32, f32, f32) {
    let mut u0 = f32::INFINITY;
    let mut v0 = f32::INFINITY;
    let mut u1 = f32::NEG_INFINITY;
    let mut v1 = f32::NEG_INFINITY;
    for uv in uvs {
        let u = uv[0].clamp(0.0, 1.0);
        let v = uv[1].clamp(0.0, 1.0);
        u0 = u0.min(u);
        v0 = v0.min(v);
        u1 = u1.max(u);
        v1 = v1.max(v);
    }
    if !u0.is_finite() {
        (0.0, 0.0, 1.0, 1.0)
    } else {
        (u0, v0, u1.max(u0 + 1e-6), v1.max(v0 + 1e-6))
    }
}

fn fit(w: u32, h: u32, tile_size: u32) -> (u32, u32) {
    if w <= tile_size && h <= tile_size {
        return (w.max(1), h.max(1));
    }
    let long = w.max(h) as f32;
    let s = tile_size as f32 / long;
    (
        ((w as f32) * s).round().max(1.0) as u32,
        ((h as f32) * s).round().max(1.0) as u32,
    )
}

fn encode_jpeg(img: &RgbaImage) -> Result<Vec<u8>, Error> {
    let rgb = DynamicImage::ImageRgba8(img.clone()).to_rgb8();
    let mut buf = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, JPEG_QUALITY);
    enc.encode(
        rgb.as_raw(),
        rgb.width(),
        rgb.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(buf)
}
