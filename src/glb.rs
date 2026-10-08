//! Shared GLB container plumbing: chunk framing, alignment, buffer-view
//! appends, extension declarations and the one `EXT_meshopt_compression`
//! view rewriter used by mesh and vector content.
//!
//! Every writer keeps its glTF document and binary buffer in memory and
//! serialises exactly once through [`encode_glb`].

use serde::Serialize;
use serde_json::{json, Value};

use crate::error::Error;

const HEADER_LEN: usize = 12;
const CHUNK_HEADER_LEN: usize = 8;
const MESHOPT: &str = "EXT_meshopt_compression";

/// Zero-pad `buf` to a multiple of `align` bytes.
pub(crate) fn pad_to(buf: &mut Vec<u8>, align: usize) {
    buf.resize(buf.len().next_multiple_of(align), 0);
}

/// Append `bytes` at the next `align` boundary and return their offset.
pub(crate) fn append_aligned(buf: &mut Vec<u8>, align: usize, bytes: &[u8]) -> usize {
    pad_to(buf, align);
    let offset = buf.len();
    buf.extend_from_slice(bytes);
    offset
}

/// Byte length [`encode_glb`] produces for a JSON chunk of `json_len` bytes
/// and a binary chunk of `bin_len` bytes, or an error past the 32-bit limit.
pub(crate) fn glb_len(json_len: usize, bin_len: usize) -> Result<usize, Error> {
    (HEADER_LEN + 2 * CHUNK_HEADER_LEN)
        .checked_add(json_len.next_multiple_of(4))
        .and_then(|n| n.checked_add(bin_len.next_multiple_of(4)))
        .filter(|&n| u32::try_from(n).is_ok())
        .ok_or_else(|| Error::Data("GLB exceeds the 32-bit format size limit".into()))
}

/// Serialised JSON length of `document`, without materialising it.
#[cfg(any(test, feature = "native-geospatial"))]
pub(crate) fn json_len<T: Serialize + ?Sized>(document: &T) -> Result<usize, Error> {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 += buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    serde_json::to_writer(&mut count, document)?;
    Ok(count.0)
}

/// Frame a glTF document and binary buffer as GLB 2.0: header, JSON chunk
/// padded with spaces and BIN chunk padded with zeros, both to four bytes.
/// The total length is checked against the format's 32-bit limit.
pub(crate) fn encode_glb<T: Serialize + ?Sized>(
    document: &T,
    bin: &[u8],
) -> Result<Vec<u8>, Error> {
    let mut out = Vec::with_capacity(HEADER_LEN + 2 * CHUNK_HEADER_LEN + 4096 + bin.len());
    out.extend_from_slice(&[0; HEADER_LEN + CHUNK_HEADER_LEN]);
    serde_json::to_writer(&mut out, document)?;
    let json_len = out.len() - HEADER_LEN - CHUNK_HEADER_LEN;
    let total = glb_len(json_len, bin.len())?;
    out.resize(out.len().next_multiple_of(4), b' ');
    let json_chunk = out.len() - HEADER_LEN - CHUNK_HEADER_LEN;
    out.reserve_exact(total - out.len());
    out.extend_from_slice(&(bin.len().next_multiple_of(4) as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(bin);
    pad_to(&mut out, 4);
    debug_assert_eq!(out.len(), total);
    out[..4].copy_from_slice(b"glTF");
    out[4..8].copy_from_slice(&2u32.to_le_bytes());
    out[8..12].copy_from_slice(&(total as u32).to_le_bytes());
    out[12..16].copy_from_slice(&(json_chunk as u32).to_le_bytes());
    out[16..20].copy_from_slice(b"JSON");
    Ok(out)
}

/// Declare `name` in `extensionsUsed` (and `extensionsRequired` when
/// `required`), creating the lists on first use and never duplicating names.
pub(crate) fn add_extension(document: &mut Value, name: &str, required: bool) -> Result<(), Error> {
    let lists: &[&str] = if required {
        &["extensionsUsed", "extensionsRequired"]
    } else {
        &["extensionsUsed"]
    };
    for &key in lists {
        if document[key].is_null() {
            document[key] = json!([]);
        }
        let list = document[key]
            .as_array_mut()
            .ok_or_else(|| Error::msg("invalid extension list"))?;
        if !list.iter().any(|v| v == name) {
            list.push(json!(name));
        }
    }
    Ok(())
}

/// How one buffer view is encoded. Every role is a lossless byte codec.
#[derive(Clone, Copy, Debug)]
pub(crate) enum MeshoptStream {
    /// Triangle-list indices (`mode: TRIANGLES`) of `width` bytes each.
    TriangleIndices {
        count: usize,
        width: usize,
        vertex_count: usize,
    },
    /// Fixed-stride elements (`mode: ATTRIBUTES`): vertex attributes, feature
    /// IDs and polygon offsets/loop indices including their restart values,
    /// which must keep exact element bytes rather than triangle semantics.
    Attributes { count: usize, stride: usize },
}

/// Where compressed views point in the uncompressed fallback buffer.
#[derive(Clone, Copy, Debug)]
pub(crate) enum FallbackOffsets {
    /// Keep each view's source `byteOffset`; the fallback spans the source.
    Source,
    /// Repack compressed views contiguously at the layout alignment.
    Packed,
}

/// Container layout for one family of meshopt-compressed content.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MeshoptLayout {
    /// Alignment of every view in the compressed (and packed fallback) buffer.
    pub align: usize,
    pub fallback: FallbackOffsets,
    /// Write `"filter":"NONE"` explicitly on compressed views.
    pub explicit_filter: bool,
}

/// Compress buffer views of `document` (whose binary is `source`) with
/// `EXT_meshopt_compression`. `classify(view, byte_length)` selects the
/// stream role of each view; `None` copies the view verbatim. Accessors,
/// view identities and element order are unchanged. Returns the new binary.
pub(crate) fn meshopt_compress(
    document: &mut Value,
    source: &[u8],
    layout: MeshoptLayout,
    mut classify: impl FnMut(usize, usize) -> Result<Option<MeshoptStream>, Error>,
) -> Result<Vec<u8>, Error> {
    let views = document["bufferViews"]
        .as_array_mut()
        .ok_or_else(|| Error::msg("missing views"))?;
    let mut binary = Vec::with_capacity(source.len());
    let mut packed_size = 0usize;
    let mut indices = Vec::new();
    for (index, view) in views.iter_mut().enumerate() {
        let offset = view["byteOffset"].as_u64().unwrap_or(0) as usize;
        let length = view["byteLength"]
            .as_u64()
            .ok_or_else(|| Error::msg("missing view size"))? as usize;
        let end = offset
            .checked_add(length)
            .ok_or_else(|| Error::msg("view overflow"))?;
        let bytes = source
            .get(offset..end)
            .ok_or_else(|| Error::msg("view outside GLB"))?;
        pad_to(&mut binary, layout.align);
        let start = binary.len();
        let Some(stream) = classify(index, length)? else {
            binary.extend_from_slice(bytes);
            view["buffer"] = json!(0);
            view["byteOffset"] = json!(start);
            continue;
        };
        let (count, stride, mode) = match stream {
            MeshoptStream::TriangleIndices {
                count,
                width,
                vertex_count,
            } => {
                indices.clear();
                match width {
                    2 => indices.extend(
                        bytes
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .map(|&v| u16::from_le_bytes(v) as u32),
                    ),
                    4 => indices.extend(
                        bytes
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(|&v| u32::from_le_bytes(v)),
                    ),
                    _ => return Err(Error::msg("unsupported index width")),
                }
                encode_indices(&mut binary, &indices, vertex_count)?;
                (count, width, "TRIANGLES")
            }
            MeshoptStream::Attributes { count, stride } => {
                encode_attributes(&mut binary, bytes, count, stride)?;
                (count, stride, "ATTRIBUTES")
            }
        };
        view["buffer"] = json!(1);
        if let FallbackOffsets::Packed = layout.fallback {
            packed_size = packed_size.next_multiple_of(layout.align);
            view["byteOffset"] = json!(packed_size);
            packed_size += length;
        }
        let mut extension = json!({"buffer":0,"byteOffset":start,"byteLength":binary.len() - start,
            "byteStride":stride,"count":count,"mode":mode});
        if layout.explicit_filter {
            extension["filter"] = json!("NONE");
        }
        view["extensions"] = json!({ MESHOPT: extension });
    }
    let fallback = match layout.fallback {
        FallbackOffsets::Source => source.len(),
        FallbackOffsets::Packed => packed_size,
    };
    document["buffers"] = json!([{"byteLength":binary.len()},
        {"byteLength":fallback,"extensions":{MESHOPT:{"fallback":true}}}]);
    add_extension(document, MESHOPT, true)?;
    Ok(binary)
}

/// Append meshopt-encoded triangle indices to `out`.
fn encode_indices(out: &mut Vec<u8>, indices: &[u32], vertex_count: usize) -> Result<(), Error> {
    let start = out.len();
    // SAFETY: the destination is `bound` initialised bytes and the source is
    // a live u32 slice of `indices.len()` elements.
    unsafe {
        let bound = meshopt::ffi::meshopt_encodeIndexBufferBound(indices.len(), vertex_count);
        out.resize(start + bound, 0);
        let n = meshopt::ffi::meshopt_encodeIndexBuffer(
            out[start..].as_mut_ptr(),
            bound,
            indices.as_ptr(),
            indices.len(),
        );
        out.truncate(start + n);
    }
    Ok(())
}

/// Append a meshopt-encoded fixed-stride stream to `out`.
fn encode_attributes(
    out: &mut Vec<u8>,
    data: &[u8],
    count: usize,
    stride: usize,
) -> Result<(), Error> {
    if stride == 0 || stride > 256 || !stride.is_multiple_of(4) {
        return Err(Error::msg("invalid meshopt stride"));
    }
    if count.checked_mul(stride).is_none_or(|n| n > data.len()) {
        return Err(Error::msg("meshopt stream exceeds its view"));
    }
    let start = out.len();
    // SAFETY: `data` holds at least count * stride initialised bytes and the
    // destination is `bound` initialised bytes. The codec reads bytes and does
    // not require native element alignment.
    let n = unsafe {
        let bound = meshopt::ffi::meshopt_encodeVertexBufferBound(count, stride);
        out.resize(start + bound, 0);
        meshopt::ffi::meshopt_encodeVertexBuffer(
            out[start..].as_mut_ptr(),
            bound,
            data.as_ptr().cast(),
            count,
            stride,
        )
    };
    if n == 0 {
        return Err(Error::msg("meshopt vertex encoding failed"));
    }
    out.truncate(start + n);
    Ok(())
}

/// Small GLB builder for feature attributes and structural metadata. Views are
/// eight-byte aligned so numeric property tables can retain 64-bit source values.
#[derive(Clone)]
pub struct MetadataGlb {
    pub document: Value,
    binary: Vec<u8>,
}

impl MetadataGlb {
    const ALIGN: usize = 8;

    pub fn new(generator: &str) -> Self {
        Self {
            document: json!({
                "asset":{"version":"2.0","generator":generator},
                "buffers":[{"byteLength":0}], "bufferViews":[], "accessors":[],
                "scenes":[{"nodes":[0]}], "scene":0, "nodes":[{"mesh":0}]
            }),
            binary: Vec::new(),
        }
    }

    /// Continue authoring an existing glTF document and its binary buffer.
    pub fn from_parts(document: Value, binary: Vec<u8>) -> Self {
        Self { document, binary }
    }

    pub fn view(&mut self, bytes: &[u8]) -> usize {
        let offset = append_aligned(&mut self.binary, Self::ALIGN, bytes);
        let views = self.document["bufferViews"].as_array_mut().unwrap();
        views.push(json!({"buffer":0,"byteOffset":offset,"byteLength":bytes.len()}));
        views.len() - 1
    }

    pub fn accessor(&mut self, description: Value) -> usize {
        let accessors = self.document["accessors"].as_array_mut().unwrap();
        accessors.push(description);
        accessors.len() - 1
    }

    /// Replace a generated view while retaining accessor/metadata identities.
    #[cfg(feature = "native-geospatial")]
    pub fn replace_view(&mut self, index: usize, bytes: &[u8]) {
        let offset = append_aligned(&mut self.binary, Self::ALIGN, bytes);
        self.document["bufferViews"][index]["byteOffset"] = offset.into();
        self.document["bufferViews"][index]["byteLength"] = bytes.len().into();
    }

    #[cfg(feature = "native-geospatial")]
    pub fn compact_views(&mut self) {
        let mut binary = Vec::with_capacity(self.binary.len());
        for view in self.document["bufferViews"].as_array_mut().unwrap() {
            let offset = view["byteOffset"].as_u64().unwrap() as usize;
            let length = view["byteLength"].as_u64().unwrap() as usize;
            let bytes = &self.binary[offset..offset + length];
            view["byteOffset"] = append_aligned(&mut binary, Self::ALIGN, bytes).into();
        }
        self.binary = binary;
    }

    /// Record the final (four-byte padded) buffer length in the document.
    fn record_buffer_length(&mut self) {
        self.document["buffers"][0]["byteLength"] = self.binary.len().next_multiple_of(4).into();
    }

    /// Length of the GLB [`Self::finish`] would produce now, without
    /// serialising or copying the binary.
    #[cfg(feature = "native-geospatial")]
    pub fn encoded_len(&mut self) -> Result<usize, Error> {
        self.record_buffer_length();
        glb_len(json_len(&self.document)?, self.binary.len())
    }

    /// The finished document and its four-byte padded binary buffer, for
    /// callers that rewrite views (e.g. meshopt) before [`encode_glb`].
    pub fn into_parts(mut self) -> (Value, Vec<u8>) {
        self.record_buffer_length();
        pad_to(&mut self.binary, 4);
        (self.document, self.binary)
    }

    pub fn finish(mut self) -> Result<Vec<u8>, Error> {
        self.record_buffer_length();
        encode_glb(&self.document, &self.binary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;

    fn reference(json: &[u8], bin: &[u8]) -> Vec<u8> {
        gltf::Glb {
            header: gltf::binary::Header {
                magic: *b"glTF",
                version: 2,
                length: 0,
            },
            json: Cow::Borrowed(json),
            bin: Some(Cow::Borrowed(bin)),
        }
        .to_vec()
        .unwrap()
    }

    #[test]
    fn framing_matches_gltf_writer_for_every_padding() {
        for json_pad in 0..4 {
            for bin_len in 0..9 {
                let document = json!({"asset":{"version":"2.0"},"x":"a".repeat(json_pad)});
                let bin: Vec<u8> = (0..bin_len as u8).collect();
                let encoded = encode_glb(&document, &bin).unwrap();
                let text = serde_json::to_vec(&document).unwrap();
                assert_eq!(encoded, reference(&text, &bin));
                assert_eq!(encoded.len(), glb_len(text.len(), bin.len()).unwrap());
                assert_eq!(json_len(&document).unwrap(), text.len());
            }
        }
    }

    #[test]
    fn length_limit_is_checked() {
        assert!(glb_len(u32::MAX as usize, 0).is_err());
        // 28 bytes of headers plus a padded binary chunk.
        assert!(glb_len(0, u32::MAX as usize - 31).is_ok());
        assert!(glb_len(0, u32::MAX as usize - 30).is_err());
    }

    #[test]
    fn extensions_are_declared_once() {
        let mut document = json!({"extensionsUsed":["A"]});
        add_extension(&mut document, "A", true).unwrap();
        add_extension(&mut document, "B", false).unwrap();
        add_extension(&mut document, "B", true).unwrap();
        assert_eq!(document["extensionsUsed"], json!(["A", "B"]));
        assert_eq!(document["extensionsRequired"], json!(["A", "B"]));
    }
}
