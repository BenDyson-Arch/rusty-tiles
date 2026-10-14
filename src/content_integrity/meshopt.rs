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
/// Post-decode `EXT_meshopt_compression` filters this decoder applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Filter {
    None,
    /// Unit vectors: four 8-bit (stride 4) or 16-bit (stride 8) components per element.
    Octahedral,
}
impl Filter {
    /// `"NONE"` or `"OCTAHEDRAL"`; any other filter is outside this decoder.
    pub(crate) fn parse(name: &str) -> Option<Self> {
        match name {
            "NONE" => Some(Self::None),
            "OCTAHEDRAL" => Some(Self::Octahedral),
            _ => None,
        }
    }
}
pub(crate) fn decode(
    compressed: &[u8],
    count: usize,
    stride: usize,
    mode: Mode,
) -> Result<Vec<u8>, FormatError> {
    decode_filtered(compressed, count, stride, mode, Filter::None)
}
/// [`decode`], then the filter. OCTAHEDRAL requires ATTRIBUTES with stride 4 or 8, and
/// every element must be a valid K-bit octahedral code (pinned extension text, Appendix
/// B): the third component encodes 1.0 as `(1 << (K - 1)) - 1` for some 2 <= K <= 8 or
/// 16 (the component width), and `|x|, |y|` do not exceed it. Anything else is
/// InvalidInput, never a decoded guess.
pub(crate) fn decode_filtered(
    compressed: &[u8],
    count: usize,
    stride: usize,
    mode: Mode,
    filter: Filter,
) -> Result<Vec<u8>, FormatError> {
    if filter == Filter::Octahedral
        && (!matches!(mode, Mode::Attributes) || (stride != 4 && stride != 8))
    {
        return Err(invalid(
            "meshopt OCTAHEDRAL filter needs ATTRIBUTES mode and byteStride 4 or 8",
        ));
    }
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
    if filter == Filter::Octahedral {
        octahedral(&mut decoded, count, stride)?;
    }
    copy_words(&decoded, length)
}
/// The OCTAHEDRAL filter alone on uncompressed filtered bytes: the same element checks
/// and linked decoder filter [`decode_filtered`] applies after decompression. Meshopt
/// compression is lossless, so this is what a consumer decodes from the stream.
pub(crate) fn unfilter_octahedral(
    filtered: &[u8],
    count: usize,
    stride: usize,
) -> Result<Vec<u8>, FormatError> {
    if (stride != 4 && stride != 8) || count.checked_mul(stride) != Some(filtered.len()) {
        return Err(invalid("meshopt OCTAHEDRAL input shape"));
    }
    let mut words = Vec::<u32>::new();
    words
        .try_reserve_exact(filtered.len().div_ceil(4))
        .map_err(|_| limit("allocation failed"))?;
    words.extend(filtered.chunks(4).map(|c| {
        let mut b = [0u8; 4];
        b[..c.len()].copy_from_slice(c);
        u32::from_ne_bytes(b)
    }));
    octahedral(&mut words, count, stride)?;
    copy_words(&words, filtered.len())
}
/// Checks every octahedral element, then applies the linked decoder's filter in place
/// (no further allocation, so the decode peak bound is unchanged).
fn octahedral(words: &mut [u32], count: usize, stride: usize) -> Result<(), FormatError> {
    // The element checks read little-endian glTF bytes from native words and the native
    // filter works on native-endian integers: both agree only on little-endian hosts.
    if cfg!(target_endian = "big") {
        return Err(FormatError::Unsupported(
            "meshopt OCTAHEDRAL decoding on a big-endian host".into(),
        ));
    }
    let component = stride / 4;
    let byte = |at: usize| words[at / 4].to_ne_bytes()[at % 4];
    let read = |at: usize| -> i32 {
        if component == 1 {
            i32::from(byte(at) as i8)
        } else {
            i32::from(i16::from_le_bytes([byte(at), byte(at + 1)]))
        }
    };
    for element in 0..count {
        let base = element * stride;
        let one = read(base + 2 * component);
        let bits = component as u32 * 8;
        let valid_one = one > 0 && (one + 1).count_ones() == 1 && one < 1 << (bits - 1);
        if !valid_one || read(base).abs() > one || read(base + component).abs() > one {
            return Err(FormatError::InvalidInput(format!(
                "meshopt OCTAHEDRAL element {element} is not a valid K-bit octahedral code"
            )));
        }
    }
    // SAFETY: `words` is a live, 4-byte-aligned buffer of at least count * stride bytes
    // (decode allocated it for this stream); stride is 4 or 8 as the filter requires.
    unsafe { meshopt::ffi::meshopt_decodeFilterOct(words.as_mut_ptr().cast(), count, stride) };
    Ok(())
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
