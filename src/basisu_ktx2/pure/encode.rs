//! The pure-Rust encoder: ktx-rs's port of `basis_compressor` produces a `.basis` file, and
//! this module rewrites its slices and BasisLZ codebooks as the KTX2 file that the vendored
//! `basis_compressor::create_ktx2_file` (Basis Universal v2.50) writes. Parameters are derived
//! exactly as `basis_compress2` derives them from the shim's arguments.

use ktx::basisu::basis_file::{
    BasisFileHeader, BasisSliceDesc, BASIS_FILE_HEADER_SIZE, BASIS_SLICE_DESC_SIZE,
    C_SLICE_DESC_FLAGS_HAS_ALPHA,
};
use ktx::basisu::comp::{BasisCompressor, BasisCompressorErrorCode, BasisCompressorParams, Image};

use super::super::{MipLevel, Mode, Transfer};
use super::{SUPERCOMPRESSION_BASIS_LZ, SUPERCOMPRESSION_NONE, SUPERCOMPRESSION_ZSTD};

/// `basis_compressor_params::m_ktx2_zstd_supercompression_level`'s default.
const ZSTD_LEVEL: i32 = 6;
/// `BASISU_MAX_ETC1S_COMPRESSION_LEVEL`.
const MAX_ETC1S_COMPRESSION_LEVEL: f32 = 6.0;
/// `cPackUASTCLevelFastest` and `cPackUASTCLevelVerySlow`.
const UASTC_LEVEL_FASTEST: f32 = 0.0;
const UASTC_LEVEL_VERY_SLOW: f32 = 4.0;
/// `BASISU_LIB_VERSION_STRING` of the vendored release, written as the KTXwriter value.
const KTX_WRITER: &str = "Basis Universal 2.50";

const KTX2_IDENTIFIER: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];
const HEADER_LEN: usize = 80;
const LEVEL_ENTRY_LEN: usize = 24;

/// `g_ktx2_etc1s_nonalpha_dfd`, `g_ktx2_etc1s_alpha_dfd`, `g_ktx2_uastc_ldr_4x4_nonalpha_dfd`
/// and `g_ktx2_uastc_ldr_4x4_alpha_dfd` (basisu_comp.cpp), before `get_dfd`'s fix-ups.
const DFD_ETC1S: [u8; 44] = [
    0x2C, 0, 0, 0, 0, 0, 0, 0, 0x2, 0, 0x28, 0, 0xA3, 0x1, 0x2, 0, 0x3, 0x3, 0, 0, 0x8, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0x3F, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF,
];
const DFD_ETC1S_ALPHA: [u8; 60] = [
    0x3C, 0, 0, 0, 0, 0, 0, 0, 0x2, 0, 0x38, 0, 0xA3, 0x1, 0x2, 0, 0x3, 0x3, 0, 0, 0x8, 0x8, 0, 0,
    0, 0, 0, 0, 0, 0, 0x3F, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0x40, 0, 0x3F, 0xF,
    0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF,
];
const DFD_UASTC: [u8; 44] = [
    0x2C, 0, 0, 0, 0, 0, 0, 0, 0x2, 0, 0x28, 0, 0xA6, 0x1, 0x2, 0, 0x3, 0x3, 0, 0, 0x10, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0x7F, 0x4, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF,
];
const DFD_UASTC_ALPHA: [u8; 44] = [
    0x2C, 0, 0, 0, 0, 0, 0, 0, 0x2, 0, 0x28, 0, 0xA6, 0x1, 0x2, 0, 0x3, 0x3, 0, 0, 0x10, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0x7F, 0x3, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF,
];
/// `KTX2_DF_CHANNEL_UASTC_RGB` and `KTX2_DF_CHANNEL_UASTC_RGBA`.
const CHANNEL_UASTC_RGB: u32 = 0;
const CHANNEL_UASTC_RGBA: u32 = 3;

/// Encoder settings resolved from a request, exactly as `basis_compress2` resolves them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Settings {
    pub etc1s: bool,
    /// `m_quality_level` (ETC1S, 1..=255).
    pub quality_level: i32,
    /// `m_etc1s_compression_level` (ETC1S, 0..=6).
    pub etc1s_compression_level: i32,
    /// `m_pack_uastc_ldr_4x4_flags` (UASTC): the pack level plus any extra pack flags.
    pub uastc_pack_flags: u32,
    pub zstd: bool,
    pub srgb: bool,
}

/// `(int)std::round(lerp<float>(a, b, s))` in single precision, as basisu computes it.
fn lerp_round(a: f32, b: f32, s: f32) -> i32 {
    (a + (b - a) * s).round() as i32
}

/// `clamp<float>((float)value / 100.0f or 10.0f, 0.0f, 1.0f)`.
fn fraction(value: i32, scale: f32) -> f32 {
    (value as f32 / scale).clamp(0.0, 1.0)
}

impl Settings {
    /// `set_format_mode_and_quality_effort(mode, quality, effort)` for the shim's arguments
    /// (quality 1..=100 for ETC1S, 100 for UASTC; effort 0..=10), plus `extra_pack_flags`
    /// OR'd into the UASTC pack flags after the effort sets the level.
    pub(crate) fn resolve(
        mode: Mode,
        effort: u8,
        transfer: Transfer,
        extra_pack_flags: u32,
    ) -> Self {
        let effort = fraction(i32::from(effort), 10.0);
        let srgb = transfer == Transfer::Srgb;
        match mode {
            Mode::Etc1s { quality } => Settings {
                etc1s: true,
                quality_level: lerp_round(0.0, 255.0, fraction(i32::from(quality), 100.0)),
                etc1s_compression_level: lerp_round(0.0, MAX_ETC1S_COMPRESSION_LEVEL, effort),
                uastc_pack_flags: 0,
                zstd: false,
                srgb,
            },
            Mode::Uastc { zstd } => Settings {
                etc1s: false,
                quality_level: 0,
                etc1s_compression_level: 0,
                uastc_pack_flags: lerp_round(UASTC_LEVEL_FASTEST, UASTC_LEVEL_VERY_SLOW, effort)
                    as u32
                    | extra_pack_flags,
                zstd,
                srgb,
            },
        }
    }
}

fn image(mip: &MipLevel<'_>) -> Image {
    let mut image = Image::with_size(mip.width, mip.height);
    for (texel, rgba) in image
        .get_pixels_mut()
        .iter_mut()
        .zip(mip.rgba.as_chunks::<4>().0)
    {
        *texel = *rgba;
    }
    image
}

/// The compressor parameters `basis_compress_internal` builds for a plain 2D LDR request
/// without statistics, threading, debug output or mip generation.
fn params(mips: &[MipLevel<'_>], settings: Settings) -> BasisCompressorParams {
    let mut p = BasisCompressorParams {
        uastc: !settings.etc1s,
        status_output: false,
        // ktx-rs has no `m_source_mipmap_images`: caller levels are supplied as further
        // images, which yields the same slices in the same order (one source chain).
        source_images: mips.iter().map(image).collect(),
        multithreading: false,
        rdo_uastc_ldr_4x4_multithreading: false,
        job_pool_threads: 1,
        ..BasisCompressorParams::default()
    };
    p.set_srgb_options(settings.srgb);
    if settings.etc1s {
        p.quality_level = settings.quality_level;
        p.etc1s_compression_level
            .set(settings.etc1s_compression_level);
    } else {
        p.pack_uastc_ldr_4x4_flags = settings.uastc_pack_flags;
        p.rdo_uastc_ldr_4x4 = false;
        p.rdo_uastc_ldr_4x4_quality_scalar.set(1.0);
    }
    p
}

fn u32_le(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn u64_le(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// The parts of a `.basis` file the KTX2 writer needs.
struct BasisOutput<'a> {
    /// Per level, the RGB (or UASTC) slice and for ETC1S with alpha the alpha slice.
    levels: Vec<(&'a [u8], Option<&'a [u8]>)>,
    endpoints: (u32, &'a [u8]),
    selectors: (u32, &'a [u8]),
    tables: &'a [u8],
}

fn range(file: &[u8], offset: u32, length: u32) -> Result<&[u8], String> {
    let start = offset as usize;
    file.get(start..start + length as usize).ok_or_else(|| {
        format!(
            "encoder output range {offset}+{length} exceeds its {} bytes",
            file.len()
        )
    })
}

fn read_basis<'a>(
    file: &'a [u8],
    mips: &[MipLevel<'_>],
    settings: Settings,
    has_alpha: bool,
) -> Result<BasisOutput<'a>, String> {
    let header = BasisFileHeader::from_bytes(file).ok_or("encoder output has no .basis header")?;
    let alpha_slices = settings.etc1s && has_alpha;
    let per_level = if alpha_slices { 2 } else { 1 };
    if header.total_slices as usize != mips.len() * per_level {
        return Err(format!(
            "encoder output has {} slices for {} levels",
            header.total_slices,
            mips.len()
        ));
    }
    let mut levels = Vec::with_capacity(mips.len());
    for (index, mip) in mips.iter().enumerate() {
        let slice = |which: usize| -> Result<(BasisSliceDesc, &'a [u8]), String> {
            let at = header.slice_desc_file_ofs as usize
                + (index * per_level + which) * BASIS_SLICE_DESC_SIZE as usize;
            let desc = file
                .get(at..)
                .and_then(BasisSliceDesc::from_bytes)
                .ok_or("encoder output slice descriptor is truncated")?;
            let alpha = desc.flags & C_SLICE_DESC_FLAGS_HAS_ALPHA != 0;
            // Levels were supplied as images: image i, level 0.
            if desc.image_index != index as u32
                || desc.level_index != 0
                || (desc.orig_width, desc.orig_height) != (mip.width, mip.height)
                || (settings.etc1s && alpha != (which == 1))
            {
                return Err(format!(
                    "encoder output slice {index}/{which} is not the expected level"
                ));
            }
            Ok((desc, range(file, desc.file_ofs, desc.file_size)?))
        };
        let (_, rgb) = slice(0)?;
        let alpha = if alpha_slices {
            Some(slice(1)?.1)
        } else {
            None
        };
        levels.push((rgb, alpha));
    }
    let (endpoints, selectors, tables) = if settings.etc1s {
        (
            range(
                file,
                header.endpoint_cb_file_ofs,
                header.endpoint_cb_file_size,
            )?,
            range(
                file,
                header.selector_cb_file_ofs,
                header.selector_cb_file_size,
            )?,
            range(file, header.tables_file_ofs, header.tables_file_size)?,
        )
    } else {
        (&[][..], &[][..], &[][..])
    };
    debug_assert!(file.len() >= BASIS_FILE_HEADER_SIZE as usize);
    Ok(BasisOutput {
        levels,
        endpoints: (header.total_endpoints, endpoints),
        selectors: (header.total_selectors, selectors),
        tables,
    })
}

/// `basis_compressor::get_dfd` for ETC1S and UASTC LDR 4x4 without REC2020.
fn dfd(settings: Settings, has_alpha: bool) -> Vec<u8> {
    let mut dfd = match (settings.etc1s, has_alpha) {
        (true, false) => DFD_ETC1S.to_vec(),
        (true, true) => DFD_ETC1S_ALPHA.to_vec(),
        (false, false) => DFD_UASTC.to_vec(),
        (false, true) => DFD_UASTC_ALPHA.to_vec(),
    };
    let dword = |dfd: &[u8], index: usize| {
        u32::from_le_bytes([
            dfd[index * 4],
            dfd[index * 4 + 1],
            dfd[index * 4 + 2],
            dfd[index * 4 + 3],
        ])
    };
    let transfer = if settings.srgb { 2 } else { 1 };
    let bits = (dword(&dfd, 3) & !(0xFF << 16)) | (transfer << 16);
    dfd[12..16].copy_from_slice(&bits.to_le_bytes());
    if !settings.etc1s {
        let channel = if has_alpha {
            CHANNEL_UASTC_RGBA
        } else {
            CHANNEL_UASTC_RGB
        };
        let chan0 = (dword(&dfd, 7) & !(0xF << 24)) | (channel << 24);
        dfd[28..32].copy_from_slice(&chan0.to_le_bytes());
    }
    dfd
}

/// One key/value entry: `(key with its NUL, value)`.
type KeyValue = (Vec<u8>, Vec<u8>);

fn key_value_data(entries: &[KeyValue]) -> Vec<u8> {
    let mut out = Vec::new();
    for (key, value) in entries {
        u32_le(&mut out, (key.len() + value.len()) as u32);
        out.extend_from_slice(key);
        out.extend_from_slice(value);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    out
}

/// One-shot `ZSTD_compress` at the encoder's level, as `create_ktx2_file` calls it.
pub(crate) fn zstd_compress(level_data: &[u8], level: i32) -> Result<Vec<u8>, String> {
    use zstd_pure_rs::prelude as zstd;
    let mut out = vec![0u8; zstd::ZSTD_compressBound(level_data.len())];
    let written = zstd::ZSTD_compress(&mut out, level_data, level);
    if zstd::ZSTD_isError(written) {
        return Err(format!("Zstandard compression failed (code {written})"));
    }
    out.truncate(written);
    Ok(out)
}

/// `basis_compressor::create_ktx2_file` for one 2D ETC1S or UASTC LDR 4x4 image chain.
fn write_ktx2(
    basis: &BasisOutput<'_>,
    mips: &[MipLevel<'_>],
    settings: Settings,
    has_alpha: bool,
) -> Result<Vec<u8>, String> {
    let level_count = mips.len();
    let supercompression = if settings.etc1s {
        SUPERCOMPRESSION_BASIS_LZ
    } else if settings.zstd {
        SUPERCOMPRESSION_ZSTD
    } else {
        SUPERCOMPRESSION_NONE
    };
    // Level data and each slice's offset within its level.
    let mut level_data = Vec::with_capacity(level_count);
    let mut image_descs = Vec::with_capacity(level_count * 20);
    for &(rgb, alpha) in &basis.levels {
        let mut data = rgb.to_vec();
        u32_le(&mut image_descs, 0);
        u32_le(&mut image_descs, 0);
        u32_le(&mut image_descs, rgb.len() as u32);
        match alpha {
            Some(alpha) => {
                u32_le(&mut image_descs, data.len() as u32);
                u32_le(&mut image_descs, alpha.len() as u32);
                data.extend_from_slice(alpha);
            }
            None => {
                u32_le(&mut image_descs, 0);
                u32_le(&mut image_descs, 0);
            }
        }
        level_data.push(data);
    }
    let stored: Vec<Vec<u8>> = if supercompression == SUPERCOMPRESSION_ZSTD {
        level_data
            .iter()
            .map(|data| zstd_compress(data, ZSTD_LEVEL))
            .collect::<Result<_, _>>()?
    } else {
        level_data.clone()
    };

    let mut global_data = Vec::new();
    if settings.etc1s {
        let count = |value: u32, what: &str| {
            u16::try_from(value).map_err(|_| format!("encoder output has {value} {what}"))
        };
        global_data.extend_from_slice(&count(basis.endpoints.0, "endpoints")?.to_le_bytes());
        global_data.extend_from_slice(&count(basis.selectors.0, "selectors")?.to_le_bytes());
        u32_le(&mut global_data, basis.endpoints.1.len() as u32);
        u32_le(&mut global_data, basis.selectors.1.len() as u32);
        u32_le(&mut global_data, basis.tables.len() as u32);
        u32_le(&mut global_data, 0);
        global_data.extend_from_slice(&image_descs);
        global_data.extend_from_slice(basis.endpoints.1);
        global_data.extend_from_slice(basis.selectors.1);
        global_data.extend_from_slice(basis.tables);
    }

    let dfd = dfd(settings, has_alpha);
    let kvd_offset = HEADER_LEN + LEVEL_ENTRY_LEN * level_count + dfd.len();
    let mut entries: Vec<KeyValue> = vec![(
        b"KTXwriter\0".to_vec(),
        [KTX_WRITER.as_bytes(), b"\0"].concat(),
    )];
    let mut key_values = key_value_data(&entries);
    if supercompression == SUPERCOMPRESSION_NONE {
        // create_ktx2_file's dummy key, which ends the key/value data on a 16-byte boundary.
        let mut padding = (16 - ((kvd_offset + key_values.len()) & 15)) & 15;
        if padding != 0 {
            if padding < 6 {
                padding += 16;
            }
            let mut key = vec![127u8; padding - 6];
            key.push(0);
            entries.push((key, vec![0]));
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            key_values = key_value_data(&entries);
        }
    }

    let mut out = vec![0u8; HEADER_LEN + LEVEL_ENTRY_LEN * level_count];
    let dfd_offset = out.len();
    out.extend_from_slice(&dfd);
    debug_assert_eq!(out.len(), kvd_offset);
    out.extend_from_slice(&key_values);
    let (mut sgd_offset, mut sgd_length) = (0u64, 0u64);
    if !global_data.is_empty() {
        out.resize(out.len().next_multiple_of(8), 0);
        sgd_offset = out.len() as u64;
        sgd_length = global_data.len() as u64;
        out.extend_from_slice(&global_data);
    }
    if supercompression == SUPERCOMPRESSION_NONE {
        out.resize(out.len().next_multiple_of(16), 0);
    }
    let mut index = vec![(0u64, 0u64, 0u64); level_count];
    for level in (0..level_count).rev() {
        let uncompressed = if settings.etc1s {
            0
        } else {
            level_data[level].len() as u64
        };
        index[level] = (out.len() as u64, stored[level].len() as u64, uncompressed);
        out.extend_from_slice(&stored[level]);
    }

    let mut header = Vec::with_capacity(HEADER_LEN + LEVEL_ENTRY_LEN * level_count);
    header.extend_from_slice(&KTX2_IDENTIFIER);
    for value in [
        0,                       // vkFormat: VK_FORMAT_UNDEFINED
        1,                       // typeSize
        mips[0].width,           // pixelWidth
        mips[0].height,          // pixelHeight
        0,                       // pixelDepth
        0,                       // layerCount
        1,                       // faceCount
        level_count as u32,      // levelCount
        supercompression,        // supercompressionScheme
        dfd_offset as u32,       // dfdByteOffset
        dfd.len() as u32,        // dfdByteLength
        kvd_offset as u32,       // kvdByteOffset
        key_values.len() as u32, // kvdByteLength
    ] {
        u32_le(&mut header, value);
    }
    u64_le(&mut header, sgd_offset);
    u64_le(&mut header, sgd_length);
    for (offset, length, uncompressed) in index {
        u64_le(&mut header, offset);
        u64_le(&mut header, length);
        u64_le(&mut header, uncompressed);
    }
    out[..header.len()].copy_from_slice(&header);
    Ok(out)
}

/// Encodes caller-authored levels (already checked by the caller) to KTX2.
pub(crate) fn encode(mips: &[MipLevel<'_>], settings: Settings) -> Result<Vec<u8>, String> {
    let mut compressor = BasisCompressor::new();
    if !compressor.init(&params(mips, settings)) {
        return Err("the encoder refused its parameters".into());
    }
    let status = compressor.process();
    if status != BasisCompressorErrorCode::Success {
        return Err(format!("the encoder failed ({status:?})"));
    }
    let has_alpha = compressor.get_any_source_image_has_alpha();
    let basis = read_basis(
        compressor.get_output_basis_file(),
        mips,
        settings,
        has_alpha,
    )?;
    write_ktx2(&basis, mips, settings, has_alpha)
}
