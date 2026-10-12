//! Narrow locked native byte decoder shared by payload integrity and vector identity.
use super::FormatError;
#[derive(Clone, Copy)]
pub(crate) enum Mode {
    Attributes,
    Triangles,
    Indices,
}
impl Mode {
    /// The only first byte the pinned `EXT_meshopt_compression` text admits for this
    /// mode (KhronosGroup/glTF 5ccac1cb, Appendix A): attributes `0xa0`, triangles
    /// `0xe1`, indices `0xd1`. The linked decoder also accepts other codec versions.
    pub(crate) const fn header(self) -> u8 {
        match self {
            Mode::Attributes => 0xa0,
            Mode::Triangles => 0xe1,
            Mode::Indices => 0xd1,
        }
    }
}
/// Rejects a stream whose header byte the pinned extension forbids for `mode`,
/// before the decoder sees it.
pub(crate) fn check_header(compressed: &[u8], mode: Mode) -> Result<(), FormatError> {
    match compressed.first() {
        Some(&byte) if byte == mode.header() => Ok(()),
        Some(&byte) => Err(FormatError::InvalidInput(format!(
            "meshopt stream header 0x{byte:02x} is not the 0x{:02x} EXT_meshopt_compression requires",
            mode.header()
        ))),
        None => Err(invalid("meshopt stream header missing")),
    }
}
fn invalid(s: &str) -> FormatError {
    FormatError::InvalidInput(s.into())
}
fn limit(s: &str) -> FormatError {
    FormatError::ResourceLimit(s.into())
}
pub(crate) fn scratch(length: usize) -> Result<usize, FormatError> {
    length
        .checked_mul(2)
        .and_then(|n| n.checked_add(3))
        .ok_or_else(|| limit("meshopt peak bound overflow"))
}
pub(crate) fn decode(
    compressed: &[u8],
    count: usize,
    stride: usize,
    mode: Mode,
) -> Result<Vec<u8>, FormatError> {
    let length = count
        .checked_mul(stride)
        .ok_or_else(|| limit("meshopt decoded size overflow"))?;
    let words = length
        .checked_add(3)
        .map(|n| n / 4)
        .ok_or_else(|| limit("meshopt aligned output overflow"))?;
    let backing = words
        .checked_mul(4)
        .ok_or_else(|| limit("meshopt aligned allocation overflow"))?;
    let peak = backing
        .checked_add(length)
        .ok_or_else(|| limit("meshopt temporary storage overflow"))?;
    if peak > scratch(length)? || backing > isize::MAX as usize || length > isize::MAX as usize {
        return Err(limit("meshopt temporary storage bound exceeded"));
    }
    let mut decoded = Vec::<u32>::new();
    decoded
        .try_reserve_exact(words)
        .map_err(|_| limit("allocation failed"))?;
    decoded.resize(words, 0);
    let code = unsafe {
        match mode {
            Mode::Attributes => meshopt::ffi::meshopt_decodeVertexBuffer(
                decoded.as_mut_ptr().cast(),
                count,
                stride,
                compressed.as_ptr(),
                compressed.len(),
            ),
            Mode::Triangles => meshopt::ffi::meshopt_decodeIndexBuffer(
                decoded.as_mut_ptr().cast(),
                count,
                stride,
                compressed.as_ptr(),
                compressed.len(),
            ),
            Mode::Indices => meshopt::ffi::meshopt_decodeIndexSequence(
                decoded.as_mut_ptr().cast(),
                count,
                stride,
                compressed.as_ptr(),
                compressed.len(),
            ),
        }
    };
    if code != 0 {
        return Err(invalid("invalid meshopt compressed stream"));
    }
    copy_words(&decoded, length)
}

pub(crate) fn copy_words(words: &[u32], length: usize) -> Result<Vec<u8>, FormatError> {
    let backing = words
        .len()
        .checked_mul(4)
        .ok_or_else(|| limit("meshopt copy backing overflow"))?;
    if backing < length || backing - length > 3 {
        return Err(invalid("meshopt copy backing length mismatch"));
    }
    let mut out = Vec::new();
    out.try_reserve_exact(length)
        .map_err(|_| limit("meshopt output allocation failed"))?;
    for word in words {
        let take = (length - out.len()).min(4);
        out.extend_from_slice(&word.to_ne_bytes()[..take]);
    }
    Ok(out)
}
