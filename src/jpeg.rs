//! Full-chroma JPEG delivery. Native libjpeg-turbo is selected at build time
//! when available; the portable encoder keeps the CLI usable without it.
use crate::Error;
use image::RgbaImage;

pub fn backend() -> &'static str {
    if cfg!(rusty_tiles_native_jpeg) {
        "libjpeg-turbo"
    } else {
        "portable JPEG"
    }
}

#[cfg(not(rusty_tiles_native_jpeg))]
pub fn encode(image: &RgbaImage) -> Result<Vec<u8>, Error> {
    let rgb = image::DynamicImage::ImageRgba8(image.clone()).to_rgb8();
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 95).encode(
        rgb.as_raw(),
        rgb.width(),
        rgb.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(bytes)
}

#[cfg(rusty_tiles_native_jpeg)]
pub fn encode(image: &RgbaImage) -> Result<Vec<u8>, Error> {
    use std::ffi::{c_char, c_int, c_ulong, c_void, CStr};
    extern "C" {
        fn tjInitCompress() -> *mut c_void;
        fn tjCompress2(
            handle: *mut c_void,
            src: *const u8,
            width: c_int,
            pitch: c_int,
            height: c_int,
            format: c_int,
            output: *mut *mut u8,
            size: *mut c_ulong,
            subsampling: c_int,
            quality: c_int,
            flags: c_int,
        ) -> c_int;
        fn tjGetErrorStr2(handle: *mut c_void) -> *const c_char;
        fn tjDestroy(handle: *mut c_void) -> c_int;
        fn tjFree(buffer: *mut u8);
    }
    let width = c_int::try_from(image.width())
        .map_err(|_| Error::msg("JPEG width exceeds native limit"))?;
    let height = c_int::try_from(image.height())
        .map_err(|_| Error::msg("JPEG height exceeds native limit"))?;
    // Each call owns its compressor and output. No handle crosses threads.
    // RGBA is tightly packed; TurboJPEG ignores alpha (the caller handles it).
    unsafe {
        let handle = tjInitCompress();
        if handle.is_null() {
            return Err(Error::msg("could not allocate JPEG compressor"));
        }
        let mut output = std::ptr::null_mut();
        let mut size = 0;
        // TJPF_RGBA=7, TJSAMP_444=0, TJFLAG_ACCURATEDCT=4096 (stable v2 ABI).
        let status = tjCompress2(
            handle,
            image.as_ptr(),
            width,
            0,
            height,
            7,
            &mut output,
            &mut size,
            0,
            95,
            4096,
        );
        let result = if status == 0 && !output.is_null() && size > 0 {
            Ok(std::slice::from_raw_parts(output, size as usize).to_vec())
        } else {
            let error = tjGetErrorStr2(handle);
            let message = if error.is_null() {
                "JPEG encoding failed".into()
            } else {
                CStr::from_ptr(error).to_string_lossy().into_owned()
            };
            Err(Error::msg(message))
        };
        if !output.is_null() {
            tjFree(output);
        }
        tjDestroy(handle);
        result
    }
}

pub fn decode(bytes: &[u8]) -> Result<RgbaImage, Error> {
    #[cfg(rusty_tiles_native_jpeg)]
    if bytes.starts_with(&[255, 216]) {
        return decode_native(bytes);
    }
    Ok(image::load_from_memory(bytes)?.to_rgba8())
}

#[cfg(rusty_tiles_native_jpeg)]
fn decode_native(bytes: &[u8]) -> Result<RgbaImage, Error> {
    use std::ffi::{c_char, c_int, c_ulong, c_void, CStr};
    extern "C" {
        fn tjInitDecompress() -> *mut c_void;
        fn tjDecompressHeader3(
            handle: *mut c_void,
            src: *const u8,
            len: c_ulong,
            width: *mut c_int,
            height: *mut c_int,
            subsampling: *mut c_int,
            colour: *mut c_int,
        ) -> c_int;
        fn tjDecompress2(
            handle: *mut c_void,
            src: *const u8,
            len: c_ulong,
            dst: *mut u8,
            width: c_int,
            pitch: c_int,
            height: c_int,
            format: c_int,
            flags: c_int,
        ) -> c_int;
        fn tjGetErrorStr2(handle: *mut c_void) -> *const c_char;
        fn tjDestroy(handle: *mut c_void) -> c_int;
    }
    let len = c_ulong::try_from(bytes.len())
        .map_err(|_| Error::msg("JPEG exceeds native input limit"))?;
    unsafe {
        let handle = tjInitDecompress();
        if handle.is_null() {
            return Err(Error::msg("could not allocate JPEG decoder"));
        }
        let result = (|| {
            let (mut width, mut height, mut subsampling, mut colour) = (0, 0, 0, 0);
            let fail = || {
                let p = tjGetErrorStr2(handle);
                Error::msg(if p.is_null() {
                    "JPEG decoding failed".into()
                } else {
                    CStr::from_ptr(p).to_string_lossy().into_owned()
                })
            };
            if tjDecompressHeader3(
                handle,
                bytes.as_ptr(),
                len,
                &mut width,
                &mut height,
                &mut subsampling,
                &mut colour,
            ) != 0
            {
                return Err(fail());
            }
            if width <= 0 || height <= 0 {
                return Err(Error::msg("invalid JPEG dimensions"));
            }
            // The portable decoder also supports CMYK JPEGs.
            if colour == 3 || colour == 4 {
                return Ok(image::load_from_memory(bytes)?.to_rgba8());
            }
            let size = (width as usize)
                .checked_mul(height as usize)
                .and_then(|v| v.checked_mul(4))
                .ok_or_else(|| Error::msg("JPEG dimensions overflow"))?;
            let mut pixels = vec![0u8; size];
            if tjDecompress2(
                handle,
                bytes.as_ptr(),
                len,
                pixels.as_mut_ptr(),
                width,
                0,
                height,
                7,
                4096,
            ) != 0
            {
                return Err(fail());
            }
            Ok(RgbaImage::from_raw(width as u32, height as u32, pixels).unwrap())
        })();
        tjDestroy(handle);
        result
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn retains_full_chroma_and_fine_colour_detail() {
        let image = image::RgbaImage::from_fn(64, 64, |x, y| {
            image::Rgba([(x * 3) as u8, (y * 3) as u8, ((x + y) * 2) as u8, 255])
        });
        let jpeg = super::encode(&image).unwrap();
        let decoded = image::load_from_memory(&jpeg).unwrap().to_rgb8();
        assert_eq!(decoded.dimensions(), image.dimensions());
        let direct = super::decode(&jpeg).unwrap();
        assert_eq!(direct.dimensions(), image.dimensions());
        assert!(direct.pixels().all(|p| p[3] == 255));
        assert!(super::decode(&[255, 216, 0, 0]).is_err());
        let error: f64 = image
            .pixels()
            .zip(decoded.pixels())
            .flat_map(|(a, b)| (0..3).map(move |i| (a[i] as f64 - b[i] as f64).powi(2)))
            .sum();
        assert!(
            error / (64. * 64. * 3.) < 4.,
            "fine colour gradient changed excessively"
        );
        let mut offset = 2;
        loop {
            assert_eq!(jpeg[offset], 255);
            let marker = jpeg[offset + 1];
            let len = u16::from_be_bytes([jpeg[offset + 2], jpeg[offset + 3]]) as usize;
            if marker == 0xc0 {
                assert_eq!(jpeg[offset + 9], 3);
                for component in 0..3 {
                    assert_eq!(
                        jpeg[offset + 11 + component * 3],
                        0x11,
                        "chroma was subsampled"
                    );
                }
                break;
            }
            assert_ne!(marker, 0xda, "missing JPEG frame header");
            offset += 2 + len;
        }
    }
}
