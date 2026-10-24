//! A KTX2 header, level-index and DFD reader written from the KTX 2.0
//! specification, independent of the vendored Basis Universal parser. The
//! post-encode verification uses it to check the container fields the encoder
//! claims to have written before trusting the vendored decoder.

const IDENTIFIER: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];
const HEADER_LEN: usize = 80;
const LEVEL_ENTRY_LEN: usize = 24;

pub(super) const SUPERCOMPRESSION_NONE: u32 = 0;
pub(super) const SUPERCOMPRESSION_BASIS_LZ: u32 = 1;
pub(super) const SUPERCOMPRESSION_ZSTD: u32 = 2;

pub(super) const MODEL_ETC1S: u8 = 163;
pub(super) const MODEL_UASTC: u8 = 166;
pub(super) const TRANSFER_LINEAR: u8 = 1;
pub(super) const TRANSFER_SRGB: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Level {
    pub offset: u64,
    pub length: u64,
    pub uncompressed_length: u64,
}

#[derive(Debug)]
pub(super) struct Container {
    pub vk_format: u32,
    pub type_size: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub layers: u32,
    pub faces: u32,
    pub level_count: u32,
    pub supercompression: u32,
    pub global_data_length: u64,
    pub color_model: u8,
    pub transfer: u8,
    pub levels: Vec<Level>,
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

fn in_bounds(bytes: &[u8], offset: u64, length: u64) -> bool {
    offset
        .checked_add(length)
        .is_some_and(|end| end <= bytes.len() as u64)
}

pub(super) fn parse(bytes: &[u8]) -> Result<Container, String> {
    if bytes.len() < HEADER_LEN || bytes[..12] != IDENTIFIER {
        return Err("not a KTX2 file".into());
    }
    let field = |at| u32_at(bytes, at).ok_or("truncated KTX2 header");
    let level_count_field = field(40)?;
    let level_count = level_count_field.max(1) as usize;
    let index_end = level_count
        .checked_mul(LEVEL_ENTRY_LEN)
        .and_then(|len| len.checked_add(HEADER_LEN))
        .filter(|&end| end <= bytes.len())
        .ok_or("KTX2 level index exceeds the file")?;
    let mut levels = Vec::with_capacity(level_count);
    for entry in (HEADER_LEN..index_end).step_by(LEVEL_ENTRY_LEN) {
        let level = Level {
            offset: u64_at(bytes, entry).ok_or("truncated level index")?,
            length: u64_at(bytes, entry + 8).ok_or("truncated level index")?,
            uncompressed_length: u64_at(bytes, entry + 16).ok_or("truncated level index")?,
        };
        if level.length == 0 || !in_bounds(bytes, level.offset, level.length) {
            return Err(format!("KTX2 level {} range is invalid", levels.len()));
        }
        levels.push(level);
    }
    let dfd_offset = u64::from(field(48)?);
    let dfd_length = u64::from(field(52)?);
    if dfd_length < 16 || !in_bounds(bytes, dfd_offset, dfd_length) {
        return Err("KTX2 DFD range is invalid".into());
    }
    let dfd = dfd_offset as usize;
    if u64::from(u32_at(bytes, dfd).ok_or("truncated DFD")?) != dfd_length {
        return Err("KTX2 DFD total size disagrees with the index".into());
    }
    let descriptor = u32_at(bytes, dfd + 4).ok_or("truncated DFD")?;
    let version = u32_at(bytes, dfd + 8).ok_or("truncated DFD")?;
    if descriptor != 0 || version & 0xFFFF != 2 {
        return Err("KTX2 DFD is not a Khronos basic descriptor block version 2".into());
    }
    let global_data_offset = u64_at(bytes, 64).ok_or("truncated KTX2 header")?;
    let global_data_length = u64_at(bytes, 72).ok_or("truncated KTX2 header")?;
    if global_data_length != 0 && !in_bounds(bytes, global_data_offset, global_data_length) {
        return Err("KTX2 supercompression global data range is invalid".into());
    }
    Ok(Container {
        vk_format: field(12)?,
        type_size: field(16)?,
        width: field(20)?,
        height: field(24)?,
        depth: field(28)?,
        layers: field(32)?,
        faces: field(36)?,
        level_count: level_count_field,
        supercompression: field(44)?,
        global_data_length,
        color_model: bytes[dfd + 12],
        transfer: bytes[dfd + 14],
        levels,
    })
}
