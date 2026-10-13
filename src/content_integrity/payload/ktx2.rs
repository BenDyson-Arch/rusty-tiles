//! C1 container integrity of a `KHR_texture_basisu` KTX2 image.
//!
//! Rules come from the pinned KHR_texture_basisu README (KhronosGroup/glTF
//! 1bde276f) and the KTX 2.0 specification v2.0.4 (KhronosGroup/KTX-Specification
//! 30cb4530) with its BasisLZ global-data appendix. Only the container is read:
//! header, index, level index, the first DFD block, key/value framing and
//! BasisLZ image descriptors. No level is inflated, transcoded or decoded, and
//! nothing is allocated. Work is linear in the image's bytes (the key/value scan
//! reads each byte once) plus at most 32 levels and descriptors, over bytes the
//! caller has already admitted and charged; the caller inspects each image
//! resource once (one pass per bufferView however many images share it).
use super::{invalid, unsupported, FormatResult};

pub(super) const IDENTIFIER: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];
const HEADER: u64 = 80;
const LEVEL: u64 = 24;
const NONE: u32 = 0;
const BASIS_LZ: u32 = 1;
const ZSTD: u32 = 2;
const MODEL_ETC1S: u8 = 163;
const MODEL_UASTC: u8 = 166;
const TRANSFER_SRGB: u8 = 2;
const FLAG_PREMULTIPLIED: u8 = 1;
/// Bytes per 4x4 UASTC block, and the `lcm(texel block size, 4)` level
/// alignment of unsupercompressed UASTC.
const UASTC_BLOCK: u64 = 16;

fn word(bytes: &[u8], at: u64) -> FormatResult<u32> {
    let at = usize::try_from(at).map_err(|_| invalid("KTX2 field outside the image"))?;
    let raw = at
        .checked_add(4)
        .and_then(|end| bytes.get(at..end))
        .ok_or_else(|| invalid("KTX2 field outside the image"))?;
    Ok(u32::from_le_bytes(raw.try_into().unwrap()))
}
fn long(bytes: &[u8], at: u64) -> FormatResult<u64> {
    Ok(u64::from(word(bytes, at)?) | (u64::from(word(bytes, at + 4)?) << 32))
}
/// The in-file byte range `[offset, offset + length)`.
fn section<'a>(bytes: &'a [u8], offset: u64, length: u64, what: &str) -> FormatResult<&'a [u8]> {
    offset
        .checked_add(length)
        .filter(|end| *end <= bytes.len() as u64)
        .map(|end| &bytes[offset as usize..end as usize])
        .ok_or_else(|| invalid(format!("KTX2 {what} range exceeds the image")))
}
fn align_up(value: u64, to: u64) -> FormatResult<u64> {
    value
        .checked_next_multiple_of(to)
        .ok_or_else(|| invalid("KTX2 offset overflow"))
}

#[cfg(test)]
thread_local! {
    /// Containers inspected on this thread: controls prove one pass per view.
    pub(super) static INSPECTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Check one KTX2 image referenced by a `KHR_texture_basisu` `source`.
/// `material` is true when a material texture-info role uses the texture,
/// which makes the extension's 2D and premultiplied-alpha requirements apply.
pub(super) fn inspect(bytes: &[u8], material: bool) -> FormatResult<()> {
    #[cfg(test)]
    INSPECTIONS.with(|n| n.set(n.get() + 1));
    if bytes.get(..12) != Some(&IDENTIFIER[..]) {
        return Err(invalid("KTX2 identifier is absent"));
    }
    if (bytes.len() as u64) < HEADER {
        return Err(invalid("KTX2 header is truncated"));
    }
    let field = |i: u64| word(bytes, 12 + 4 * i);
    let (vk_format, type_size) = (field(0)?, field(1)?);
    let (width, height, depth) = (field(2)?, field(3)?, field(4)?);
    let (layers, faces, level_count, scheme) = (field(5)?, field(6)?, field(7)?, field(8)?);
    // KTX texture-type table; BasisLZ/ETC1S and UASTC are block-compressed,
    // so pixelHeight must not be 0.
    if width == 0 || height == 0 || !(faces == 1 || faces == 6) {
        return Err(invalid("KTX2 texture type is invalid for Basis Universal"));
    }
    if faces == 6 && (height != width || depth != 0) {
        return Err(invalid("KTX2 cubemap dimensions are invalid"));
    }
    if depth != 0 || layers != 0 || faces != 1 {
        let message = "KHR_texture_basisu KTX2 image is not a 2D texture";
        return Err(if material {
            invalid(message)
        } else {
            unsupported(message)
        });
    }
    if vk_format != 0 || type_size != 1 {
        return Err(invalid(
            "Basis Universal KTX2 must use VK_FORMAT_UNDEFINED and typeSize 1",
        ));
    }
    if width % 4 != 0 || height % 4 != 0 {
        return Err(invalid(
            "KHR_texture_basisu pixelWidth and pixelHeight must be multiples of 4",
        ));
    }
    // levelCount 0 is not allowed for block-compressed formats; at most
    // floor(log2(max(width, height))) + 1 levels exist.
    let max_levels = 32 - width.max(height).leading_zeros();
    if level_count == 0 || level_count > max_levels {
        return Err(invalid(
            "KTX2 levelCount is inconsistent with the dimensions",
        ));
    }
    let levels = u64::from(level_count);
    let index_end = HEADER + LEVEL * levels;
    if (bytes.len() as u64) < index_end {
        return Err(invalid("KTX2 level index is truncated"));
    }

    // Data Format Descriptor: it follows the level index; its first block is
    // the Khronos basic descriptor block, version 2.
    let (dfd_offset, dfd_length) = (u64::from(field(9)?), u64::from(field(10)?));
    if dfd_offset != index_end {
        return Err(invalid("KTX2 DFD does not follow the level index"));
    }
    let dfd = section(bytes, dfd_offset, dfd_length, "DFD")?;
    if dfd_length < 28 || dfd_length % 4 != 0 || u64::from(word(dfd, 0)?) != dfd_length {
        return Err(invalid("KTX2 dfdTotalSize disagrees with dfdByteLength"));
    }
    let block = u64::from(word(dfd, 8)? >> 16);
    if word(dfd, 4)? != 0
        || word(dfd, 8)? & 0xFFFF != 2
        || block < 24
        || (block - 24) % 16 != 0
        || 4 + block > dfd_length
    {
        return Err(invalid(
            "KTX2 DFD is not one Khronos basic block, version 2",
        ));
    }
    let samples = (block - 24) / 16;
    // channelType is the low nibble of each sample's fourth byte; 0xFF marks
    // an absent sample.
    let channel = |i: u64| {
        if i < samples {
            dfd[(4 + 24 + 16 * i + 3) as usize] & 0x0F
        } else {
            0xFF
        }
    };
    let channels = [channel(0), channel(1)];
    let model = dfd[12];
    // KHR_texture_basisu header-field rules for each Basis Universal format.
    match model {
        MODEL_ETC1S => {
            if scheme != BASIS_LZ {
                return Err(invalid(format!(
                    "ETC1S KTX2 must use BasisLZ supercompression, not scheme {scheme}"
                )));
            }
            if !matches!(
                (samples, channels),
                (1, [0 | 3, 0xFF]) | (2, [0, 15]) | (2, [3, 4])
            ) {
                return Err(invalid(
                    "ETC1S KTX2 DFD channels are not RGB, RGBA, R or RG",
                ));
            }
        }
        MODEL_UASTC => {
            if !matches!(scheme, NONE | ZSTD) {
                return Err(invalid(format!(
                    "UASTC KTX2 must use no or Zstandard supercompression, not scheme {scheme}"
                )));
            }
            if !matches!((samples, channels), (1, [0 | 3 | 4 | 6, 0xFF])) {
                return Err(invalid("UASTC KTX2 DFD channel is not RGB, RGBA, R or RG"));
            }
        }
        _ => {
            return Err(invalid("KHR_texture_basisu image is not ETC1S or UASTC"));
        }
    }
    // Red and Red-Green textures (ETC1S RRR first, UASTC_RRR, UASTC_RG) MUST NOT
    // use the sRGB transfer function; material textures MUST NOT be premultiplied.
    let red = (model == MODEL_ETC1S && channels[0] == 3)
        || (model == MODEL_UASTC && matches!(channels[0], 4 | 6));
    if red && dfd[14] == TRANSFER_SRGB {
        return Err(invalid("Red and Red-Green KTX2 textures must not use sRGB"));
    }
    if material && dfd[15] & FLAG_PREMULTIPLIED != 0 {
        return Err(invalid("KTX2 material texture must not be premultiplied"));
    }
    if dfd[16..20] != [3, 3, 0, 0] {
        return Err(invalid("Basis Universal KTX2 texel block is not 4x4"));
    }

    // Key/value data follows the DFD; pairs are 4-byte aligned and sum
    // exactly to kvdByteLength. KTXswizzle must be rgba and KTXorientation rd.
    let mut end = dfd_offset + dfd_length;
    let (kvd_offset, kvd_length) = (u64::from(field(11)?), u64::from(field(12)?));
    if kvd_length == 0 {
        if kvd_offset != 0 {
            return Err(invalid(
                "KTX2 kvdByteOffset must be 0 without key/value data",
            ));
        }
    } else {
        if kvd_offset != end {
            return Err(invalid("KTX2 key/value data does not follow the DFD"));
        }
        let kvd = section(bytes, kvd_offset, kvd_length, "key/value data")?;
        let mut at = 0u64;
        while at < kvd_length {
            let n = u64::from(word(kvd, at)?);
            let pair = section(kvd, at + 4, n, "key/value pair")?;
            let nul = pair.iter().position(|b| *b == 0);
            if n < 2 || nul.is_none_or(|nul| nul == 0) {
                return Err(invalid("KTX2 key/value pair has no NUL-terminated key"));
            }
            let (key, value) = pair.split_at(nul.unwrap());
            let value = &value[1..];
            let value = value.strip_suffix(&[0]).unwrap_or(value);
            if (key == b"KTXswizzle" && value != b"rgba")
                || (key == b"KTXorientation" && value != b"rd")
            {
                return Err(invalid(
                    "KHR_texture_basisu requires KTXswizzle rgba and KTXorientation rd",
                ));
            }
            at = align_up(at + 4 + n, 4)?;
        }
        if at != kvd_length {
            return Err(invalid("KTX2 key/value pairs do not sum to kvdByteLength"));
        }
        end += kvd_length;
    }

    // Supercompression global data: BasisLZ only, at the next 8-byte boundary.
    let (sgd_offset, sgd_length) = (long(bytes, 64)?, long(bytes, 72)?);
    let mut global = None;
    if sgd_length == 0 {
        if sgd_offset != 0 || scheme == BASIS_LZ {
            return Err(invalid("KTX2 supercompression global data is inconsistent"));
        }
    } else {
        if scheme != BASIS_LZ || sgd_offset != align_up(end, 8)? {
            return Err(invalid("KTX2 supercompression global data is misplaced"));
        }
        global = Some(section(bytes, sgd_offset, sgd_length, "global data")?);
        end = sgd_offset + sgd_length;
    }

    // Mip Level Array: smallest level first, packed in order up to the end of
    // the image; each level's lengths agree with its scheme and dimensions.
    let alignment = if scheme == NONE { UASTC_BLOCK } else { 1 };
    let mut level_lengths = [0u64; 32];
    let mut cursor = end;
    for p in (0..levels).rev() {
        let entry = HEADER + LEVEL * p;
        let (offset, length, inflated) = (
            long(bytes, entry)?,
            long(bytes, entry + 8)?,
            long(bytes, entry + 16)?,
        );
        if offset != align_up(cursor, alignment)? {
            return Err(invalid(format!(
                "KTX2 level {p} is not at its packed offset"
            )));
        }
        if length == 0 {
            return Err(invalid(format!("KTX2 level {p} is empty")));
        }
        cursor = section(bytes, offset, length, "level").map(|_| offset + length)?;
        let blocks = |size: u32| u64::from((size >> p).max(1).div_ceil(4));
        let uastc = u128::from(blocks(width)) * u128::from(blocks(height)) * 16;
        let agrees = match scheme {
            BASIS_LZ => inflated == 0,
            NONE => u128::from(length) == uastc && u128::from(inflated) == uastc,
            _ => u128::from(inflated) == uastc,
        };
        if !agrees {
            return Err(invalid(format!(
                "KTX2 level {p} byte lengths disagree with its size"
            )));
        }
        level_lengths[p as usize] = length;
    }
    if cursor != bytes.len() as u64 {
        return Err(invalid("KTX2 image has bytes after its base level"));
    }

    // BasisLZ global data: one ImageDesc per level of a 2D texture, and no
    // trailing bytes; every slice lies inside its level.
    if let Some(global) = global {
        let lengths = [4, 8, 12].map(|at| word(global, at).map(u64::from));
        let extended = u64::from(word(global, 16)?);
        let declared = lengths
            .into_iter()
            .try_fold(20 + 20 * levels + extended, |sum, n| Ok(sum + n?))?;
        if declared != global.len() as u64 || extended != 0 {
            return Err(invalid("BasisLZ global data length is inconsistent"));
        }
        for (p, length) in level_lengths[..levels as usize].iter().enumerate() {
            let desc = |i: u64| word(global, 20 + 20 * p as u64 + 4 * i).map(u64::from);
            let (flags, rgb_offset, rgb_length) = (desc(0)?, desc(1)?, desc(2)?);
            let (alpha_offset, alpha_length) = (desc(3)?, desc(4)?);
            let alpha_ok = if samples == 1 {
                alpha_offset == 0 && alpha_length == 0
            } else {
                alpha_length != 0 && alpha_offset + alpha_length <= *length
            };
            if flags != 0 || rgb_length == 0 || rgb_offset + rgb_length > *length || !alpha_ok {
                return Err(invalid(format!("BasisLZ image descriptor {p} is invalid")));
            }
        }
    }
    Ok(())
}
