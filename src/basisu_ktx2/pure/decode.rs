//! A validating KTX2 front end for the pure-Rust transcoder, following the acceptance of the
//! vendored `basist::ktx2_transcoder` (`init`, `start_transcoding`, `get_image_level_info`,
//! `transcode_image_level` to RGBA32 with no flags) for 2D ETC1S and UASTC LDR 4x4 files.
//!
//! The block decoders are ktx-rs's ports of `basisu_lowlevel_etc1s_transcoder` and
//! `basisu_lowlevel_uastc_ldr_4x4_transcoder`; Zstandard levels are decompressed by `ruzstd`.
//! Every read is bounds-checked and every allocation is bounded by the level being decoded.
//! Where this front end is stricter than the vendored transcoder it says so; it never accepts
//! a file the vendored transcoder rejects, and when both accept, the decoded texels are equal
//! (`shadow_tests`).

use ktx::basisu::etc1s::{LowlevelEtc1sTranscoder, TranscodeStatus};
use ktx::basisu::uastc_ldr::LowlevelUastcLdr4x4Transcoder;
use ktx::basisu::{TranscoderState, TranscoderTextureFormat};

use super::{
    MODEL_ETC1S, MODEL_UASTC, SUPERCOMPRESSION_BASIS_LZ, SUPERCOMPRESSION_NONE,
    SUPERCOMPRESSION_ZSTD,
};

const IDENTIFIER: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];
const HEADER_LEN: u64 = 80;
const LEVEL_ENTRY_LEN: usize = 24;
/// `BASISU_MAX_SUPPORTED_TEXTURE_DIMENSION`.
const MAX_DIMENSION: u32 = 16_384;
/// `KTX2_MAX_SUPPORTED_LEVEL_COUNT`.
const MAX_LEVELS: u32 = 16;
/// `MAX_SANE_LEVEL_UNCOMP_SIZE` in `ktx2_transcoder::init`.
const MAX_SANE_LEVEL_UNCOMPRESSED: u64 = 2048 * 1024 * 1024;
const ETC1S_GLOBAL_HEADER_LEN: usize = 20;
const ETC1S_IMAGE_DESC_LEN: usize = 20;
/// `KTX2_DF_CHANNEL_UASTC_RGBA` and `KTX2_DF_CHANNEL_UASTC_RRRG`.
const CHANNEL_RGBA: u32 = 3;
const CHANNEL_RRRG: u32 = 5;

fn u32_at(bytes: &[u8], at: u64) -> Option<u32> {
    let at = usize::try_from(at).ok()?;
    Some(u32::from_le_bytes(
        bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn u64_at(bytes: &[u8], at: u64) -> Option<u64> {
    let at = usize::try_from(at).ok()?;
    Some(u64::from_le_bytes(
        bytes.get(at..at.checked_add(8)?)?.try_into().ok()?,
    ))
}

fn bytes_at(bytes: &[u8], offset: u64, length: u64) -> Option<&[u8]> {
    let start = usize::try_from(offset).ok()?;
    bytes.get(start..start.checked_add(usize::try_from(length).ok()?)?)
}

const ZSTD_MAGIC: u32 = 0xFD2F_B528;

/// `ZSTD_decompress` into `out`, with the checks zstd 1.5.7 makes that ruzstd leaves to its
/// caller: a frame's reserved descriptor bit must be clear, its decoded size must equal its
/// Frame_Content_Size when present, and its content checksum must match when present.
/// Frames are decoded in sequence and skippable frames skipped, as zstd does; the output
/// never grows past `out` (ruzstd's default window limit, 2^27 bytes, is zstd's
/// ZSTD_WINDOWLOG_LIMIT_DEFAULT). Returns the number of bytes written.
fn zstd_decompress(mut input: &[u8], out: &mut [u8]) -> Result<usize, String> {
    use ruzstd::decoding::errors::{FrameDecoderError, ReadFrameHeaderError};
    use ruzstd::decoding::{BlockDecodingStrategy, FrameDecoder};
    use std::io::Read;
    let error = |e: FrameDecoderError| format!("Zstandard level did not decompress: {e}");
    let mut decoder = FrameDecoder::new();
    let mut written = 0usize;
    while !input.is_empty() {
        let descriptor = match input {
            [a, b, c, d, fhd, ..] if u32::from_le_bytes([*a, *b, *c, *d]) == ZSTD_MAGIC => {
                Some(*fhd)
            }
            _ => None,
        };
        if descriptor.is_some_and(|fhd| fhd & 0x08 != 0) {
            return Err("Zstandard level did not decompress: reserved frame header bit set".into());
        }
        match decoder.init(&mut input) {
            Ok(()) => {}
            Err(FrameDecoderError::ReadFrameHeaderError(ReadFrameHeaderError::SkipFrame {
                length,
                ..
            })) => {
                input = input
                    .get(length as usize..)
                    .ok_or_else(|| error(FrameDecoderError::FailedToSkipFrame))?;
                continue;
            }
            Err(e) => return Err(error(e)),
        }
        let frame_start = written;
        loop {
            decoder
                .decode_blocks(&mut input, BlockDecodingStrategy::UptoBytes(1 << 20))
                .map_err(error)?;
            written += decoder
                .read(&mut out[written..])
                .map_err(|e| error(FrameDecoderError::FailedToDrainDecodebuffer(e)))?;
            if decoder.can_collect() != 0 {
                return Err(error(FrameDecoderError::TargetTooSmall));
            }
            if decoder.is_finished() {
                break;
            }
        }
        // Frame_Content_Size is present when its flag is set or the frame is single-segment.
        let has_size = descriptor.is_some_and(|fhd| fhd >> 6 != 0 || fhd & 0x20 != 0);
        if has_size && decoder.content_size() != (written - frame_start) as u64 {
            return Err(
                "Zstandard level did not decompress: frame size disagrees with its header".into(),
            );
        }
        if let Some(stored) = decoder.get_checksum_from_data() {
            if decoder.get_calculated_checksum() != Some(stored) {
                return Err("Zstandard level did not decompress: content checksum mismatch".into());
            }
        }
    }
    Ok(written)
}

#[derive(Clone, Copy, Debug)]
struct Level {
    offset: u64,
    length: u64,
    uncompressed: u64,
}

/// What `ktx2_transcoder::init` and `start_transcoding` establish, for the supported subset.
struct Ktx2<'a> {
    data: &'a [u8],
    width: u32,
    height: u32,
    etc1s: bool,
    supercompression: u32,
    has_alpha: bool,
    levels: Vec<Level>,
    /// ETC1S: the decoded codebooks and the per-level image descriptors
    /// `(rgb offset, rgb length, alpha offset, alpha length)` within the level.
    etc1s_codec: Option<(LowlevelEtc1sTranscoder, Vec<[u32; 4]>)>,
}

/// `ktx2_transcoder::read_key_values`: whether the key/value data parses as the transcoder
/// parses it, and whether a `KTXanimData` key is present.
fn read_key_values(data: &[u8], offset: u32, length: u32) -> Result<bool, String> {
    if length == 0 {
        return if offset != 0 {
            Err("key/value data has an offset but no length".into())
        } else {
            Ok(false)
        };
    }
    if u64::from(offset) < HEADER_LEN || u64::from(offset) + u64::from(length) > data.len() as u64 {
        return Err("key/value data range is invalid".into());
    }
    // `at` indexes the file: entry padding is relative to its start.
    let (mut at, end) = (offset as usize, (offset + length) as usize);
    let mut video = false;
    while end - at > 4 {
        let entry = u32::from_le_bytes(data[at..at + 4].try_into().unwrap_or_default()) as usize;
        at += 4;
        if entry < 2 || end - at < entry {
            return Err("key/value entry length is invalid".into());
        }
        let Some(nul) = data[at..at + entry].iter().position(|&b| b == 0) else {
            return Err("key/value entry key is not terminated".into());
        };
        video |= &data[at..at + nul] == b"KTXanimData";
        at += entry;
        let padding = (4 - (at & 3)) & 3;
        if end - at < padding {
            return Err("key/value entry padding exceeds the data".into());
        }
        at += padding;
    }
    Ok(video)
}

impl<'a> Ktx2<'a> {
    fn parse(data: &'a [u8]) -> Result<Self, String> {
        if data.len() > u32::MAX as usize {
            return Err("file is larger than 4 GiB".into());
        }
        let size = data.len() as u64;
        if size <= HEADER_LEN || data[..12] != IDENTIFIER {
            return Err("not a KTX2 file".into());
        }
        let field = |at: u64| u32_at(data, at).unwrap_or_default();
        let (vk_format, type_size, width, height, depth) =
            (field(12), field(16), field(20), field(24), field(28));
        let (layers, faces, level_count, supercompression) =
            (field(32), field(36), field(40), field(44));
        let (dfd_offset, dfd_length, kvd_offset, kvd_length) =
            (field(48), field(52), field(56), field(60));
        let sgd_offset = u64_at(data, 64).unwrap_or_default();
        let sgd_length = u64_at(data, 72).unwrap_or_default();
        // Stricter than the vendored transcoder, which also reads other formats, cubemaps
        // and arrays: this front end decodes one 2D ETC1S or UASTC LDR 4x4 image chain.
        if vk_format != 0 {
            return Err(format!(
                "vkFormat {vk_format} is not a Basis Universal format"
            ));
        }
        if type_size != 1 || width < 1 || height < 1 || depth > 0 {
            return Err("header describes no 2D image".into());
        }
        if width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(format!(
                "{width}x{height} exceeds {MAX_DIMENSION} texels per edge"
            ));
        }
        if faces != 1 || layers > 1 {
            return Err(format!("{faces} faces and {layers} layers are unsupported"));
        }
        if !(1..=MAX_LEVELS).contains(&level_count) {
            return Err(format!("{level_count} levels is outside 1..={MAX_LEVELS}"));
        }
        if !matches!(
            supercompression,
            SUPERCOMPRESSION_NONE | SUPERCOMPRESSION_BASIS_LZ | SUPERCOMPRESSION_ZSTD
        ) {
            return Err(format!(
                "supercompression scheme {supercompression} is unsupported"
            ));
        }
        if supercompression == SUPERCOMPRESSION_BASIS_LZ
            && (sgd_offset < HEADER_LEN || sgd_offset > size || sgd_length > size - sgd_offset)
        {
            return Err("supercompression global data range is invalid".into());
        }
        let index_end = HEADER_LEN + (level_count as usize * LEVEL_ENTRY_LEN) as u64;
        if index_end > size {
            return Err("level index exceeds the file".into());
        }
        let mut levels = Vec::with_capacity(level_count as usize);
        for i in 0..u64::from(level_count) {
            let entry = HEADER_LEN + i * LEVEL_ENTRY_LEN as u64;
            let level = Level {
                offset: u64_at(data, entry).unwrap_or_default(),
                length: u64_at(data, entry + 8).unwrap_or_default(),
                uncompressed: u64_at(data, entry + 16).unwrap_or_default(),
            };
            if level.offset < HEADER_LEN
                || level.length == 0
                || level.offset > size
                || level.length > size - level.offset
                || level.uncompressed >= MAX_SANE_LEVEL_UNCOMPRESSED
                || (supercompression == SUPERCOMPRESSION_BASIS_LZ && level.uncompressed != 0)
                || (supercompression == SUPERCOMPRESSION_ZSTD && level.uncompressed == 0)
            {
                return Err(format!("level {i} index entry is invalid"));
            }
            levels.push(level);
        }
        if dfd_length != 44 && dfd_length != 60 {
            return Err(format!("DFD length {dfd_length} is not 44 or 60"));
        }
        if u64::from(dfd_offset) + u64::from(dfd_length) > size
            || u64::from(dfd_offset) < HEADER_LEN
        {
            return Err("DFD range is invalid".into());
        }
        let dfd = u64::from(dfd_offset);
        let dfd_word = |index: u64| field(dfd + index * 4);
        if dfd_word(0) != dfd_length {
            return Err("DFD total size disagrees with the index".into());
        }
        if kvd_length != 0 && dfd_word(0) != kvd_offset.wrapping_sub(dfd_offset) {
            return Err("key/value data does not follow the DFD".into());
        }
        let bits = dfd_word(3);
        let (model, transfer) = (bits & 255, (bits >> 16) & 255);
        if transfer != 1 && transfer != 2 {
            return Err(format!("DFD transfer function {transfer} is unsupported"));
        }
        let chan0 = (dfd_word(7) >> 24) & 15;
        let (etc1s, has_alpha) = match model as u8 {
            MODEL_ETC1S => (true, dfd_length == 60),
            MODEL_UASTC => (false, chan0 == CHANNEL_RGBA || chan0 == CHANNEL_RRRG),
            _ => {
                return Err(format!(
                    "DFD colour model {model} is not ETC1S or UASTC LDR 4x4"
                ))
            }
        };
        let video = read_key_values(data, kvd_offset, kvd_length)?;
        // Stricter: the scheme must be the one each format is written with, and ETC1S video
        // (P-frames transcoded against earlier frames) is unsupported.
        let scheme_ok = if etc1s {
            supercompression == SUPERCOMPRESSION_BASIS_LZ
        } else {
            supercompression != SUPERCOMPRESSION_BASIS_LZ
        };
        if !scheme_ok {
            return Err(format!(
                "supercompression scheme {supercompression} does not suit the format"
            ));
        }
        if etc1s && video {
            return Err("ETC1S video (KTXanimData) is unsupported".into());
        }
        let mut file = Ktx2 {
            data,
            width,
            height,
            etc1s,
            supercompression,
            has_alpha,
            levels,
            etc1s_codec: None,
        };
        if etc1s {
            file.etc1s_codec = Some(file.etc1s_global_data(sgd_offset, sgd_length)?);
        }
        Ok(file)
    }

    /// `ktx2_transcoder::decompress_etc1s_global_data`.
    fn etc1s_global_data(
        &self,
        sgd_offset: u64,
        sgd_length: u64,
    ) -> Result<(LowlevelEtc1sTranscoder, Vec<[u32; 4]>), String> {
        let global =
            bytes_at(self.data, sgd_offset, sgd_length).ok_or("global data range is invalid")?;
        // Stricter: the vendored reader copies the 20-byte header before checking the length.
        if global.len() < ETC1S_GLOBAL_HEADER_LEN {
            return Err("ETC1S global data is shorter than its header".into());
        }
        let word =
            |at: usize| u32::from_le_bytes(global[at..at + 4].try_into().unwrap_or_default());
        let endpoint_count = u32::from(u16::from_le_bytes([global[0], global[1]]));
        let selector_count = u32::from(u16::from_le_bytes([global[2], global[3]]));
        let (endpoints_len, selectors_len, tables_len, extended_len) =
            (word(4), word(8), word(12), word(16));
        if endpoints_len == 0 || selectors_len == 0 || tables_len == 0 {
            return Err("ETC1S global data omits a codebook or table".into());
        }
        if endpoint_count == 0 || selector_count == 0 {
            return Err("ETC1S global data has no endpoints or selectors".into());
        }
        let image_count = self.levels.len();
        let descs_len = ETC1S_IMAGE_DESC_LEN as u64 * image_count as u64;
        let needed = ETC1S_GLOBAL_HEADER_LEN as u64
            + descs_len
            + u64::from(endpoints_len)
            + u64::from(selectors_len)
            + u64::from(tables_len)
            + u64::from(extended_len);
        if needed > sgd_length {
            return Err("ETC1S global data is shorter than its sections".into());
        }
        let mut descs = Vec::with_capacity(image_count);
        for i in 0..image_count {
            let at = ETC1S_GLOBAL_HEADER_LEN + i * ETC1S_IMAGE_DESC_LEN;
            let desc = [word(at + 4), word(at + 8), word(at + 12), word(at + 16)];
            if desc[1] == 0 || (self.has_alpha && desc[3] == 0) {
                return Err(format!("ETC1S image {i} has an empty slice"));
            }
            descs.push(desc);
        }
        let mut at = ETC1S_GLOBAL_HEADER_LEN + descs_len as usize;
        let mut section = |length: u32| {
            let bytes = &global[at..at + length as usize];
            at += length as usize;
            bytes
        };
        let endpoints = section(endpoints_len);
        let selectors = section(selectors_len);
        let tables = section(tables_len);
        let mut codec = LowlevelEtc1sTranscoder::new();
        if !codec.decode_tables(tables) {
            return Err("ETC1S tables did not decode".into());
        }
        if !codec.decode_palettes(endpoint_count, endpoints, selector_count, selectors) {
            return Err("ETC1S codebooks did not decode".into());
        }
        Ok((codec, descs))
    }

    /// `ktx2_transcoder::decompress_level_data` for a UASTC Zstandard level of exactly
    /// `expected` bytes. Stricter: the vendored transcoder allocates any recorded size below
    /// 2 GiB and reads the level's first `expected` bytes; here the recorded size must be the
    /// level's block bytes, which bounds the allocation by the level.
    fn zstd_level(&self, level: &Level, expected: u64) -> Result<Vec<u8>, String> {
        if level.uncompressed != expected {
            return Err(format!(
                "Zstandard level records {} bytes, expected {expected}",
                level.uncompressed
            ));
        }
        let compressed =
            bytes_at(self.data, level.offset, level.length).ok_or("level range is invalid")?;
        let mut out = vec![0u8; expected as usize];
        let written = zstd_decompress(compressed, &mut out)?;
        if written as u64 != expected {
            return Err(format!(
                "Zstandard level decompressed to {written} bytes, expected {expected}"
            ));
        }
        Ok(out)
    }

    /// `ktx2_transcoder::transcode_image_level(level, 0, 0, RGBA32, flags 0)`.
    fn decode_level(&self, level_index: u32) -> Result<(u32, u32, Vec<u8>), String> {
        let level = *self
            .levels
            .get(level_index as usize)
            .ok_or_else(|| format!("level {level_index} does not exist"))?;
        let width = (self.width >> level_index).max(1);
        let height = (self.height >> level_index).max(1);
        let (blocks_x, blocks_y) = (width.div_ceil(4), height.div_ceil(4));
        let pixels = width * height;
        let mut out = vec![0u8; pixels as usize * 4];
        let mut state = TranscoderState::new();
        let status = if let Some((codec, descs)) = &self.etc1s_codec {
            let [rgb_offset, rgb_length, alpha_offset, alpha_length] = descs[level_index as usize];
            codec.transcode_image(
                TranscoderTextureFormat::RGBA32,
                &mut out,
                pixels,
                self.data,
                blocks_x,
                blocks_y,
                width,
                height,
                level_index,
                level.offset + u64::from(rgb_offset),
                rgb_length,
                if alpha_length != 0 {
                    level.offset + u64::from(alpha_offset)
                } else {
                    0
                },
                alpha_length,
                0,
                self.has_alpha,
                false,
                0,
                &mut state,
                0,
            )
        } else {
            let image_len = u64::from(blocks_x) * u64::from(blocks_y) * 16;
            let decompressed;
            let blocks = if self.supercompression == SUPERCOMPRESSION_ZSTD {
                decompressed = self.zstd_level(&level, image_len)?;
                &decompressed[..]
            } else {
                // Uncompressed: the level must hold at least the image's blocks.
                if level.length < image_len {
                    return Err(format!(
                        "level {level_index} holds {} bytes, expected {image_len}",
                        level.length
                    ));
                }
                bytes_at(self.data, level.offset, image_len).ok_or("level range is invalid")?
            };
            LowlevelUastcLdr4x4Transcoder::new().transcode_image(
                TranscoderTextureFormat::RGBA32,
                &mut out,
                pixels,
                blocks,
                blocks_x,
                blocks_y,
                width,
                height,
                level_index,
                0,
                image_len as u32,
                0,
                self.has_alpha,
                false,
                0,
                &mut state,
                0,
                -1,
                -1,
            )
        };
        match status {
            TranscodeStatus::Ok => Ok((width, height, out)),
            other => Err(format!("level {level_index} did not transcode ({other:?})")),
        }
    }
}

/// Decodes one level to RGBA8: `(width, height, rgba)`, the level's dimensions as the
/// transcoder reports them. A decoder panic (the block decoders are ports of C++ that
/// assumes well-formed input in places) is returned as an error, never propagated.
pub(crate) fn decode_level(ktx2: &[u8], level: u32) -> Result<(u32, u32, Vec<u8>), String> {
    std::panic::catch_unwind(|| Ktx2::parse(ktx2)?.decode_level(level))
        .unwrap_or_else(|_| Err("the transcoder panicked on this file".into()))
}
