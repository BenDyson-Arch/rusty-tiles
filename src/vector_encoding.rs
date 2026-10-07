//! Lossless buffer-view compression for generated vector content. Metadata and
//! polygon/feature accessor identities remain unchanged; no vertex reordering.
use crate::{
    error::Error,
    glb::{self, FallbackOffsets, MeshoptLayout, MeshoptStream},
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path};

/// Vector content keeps eight-byte views (64-bit property tables) in both the
/// compressed and the repacked fallback buffer.
const VECTOR_MESHOPT: MeshoptLayout = MeshoptLayout {
    align: 8,
    fallback: FallbackOffsets::Packed,
    explicit_filter: false,
};

pub fn compress_file(path: &Path) -> Result<Value, Error> {
    let input = std::fs::read(path)?;
    let output = compress_bytes(&input)?;
    std::fs::write(path, &output)?;
    Ok(json!({"beforeBytes":input.len(),"afterBytes":output.len()}))
}

pub(crate) fn compress_bytes(input: &[u8]) -> Result<Vec<u8>, Error> {
    let glb = gltf::binary::Glb::from_slice(input)?;
    let mut doc: Value = serde_json::from_slice(&glb.json)?;
    let source = glb
        .bin
        .ok_or_else(|| Error::msg("vector GLB has no binary chunk"))?;
    let binary = compress_document(&mut doc, &source)?;
    glb::encode_glb(&doc, &binary)
}

/// Compress every accessor-backed view of a vector document in place as an
/// attribute stream (positions, feature IDs, polygon offsets and restart
/// loop indices alike); other views (metadata) stay raw. Returns the binary.
pub(crate) fn compress_document(doc: &mut Value, source: &[u8]) -> Result<Vec<u8>, Error> {
    let mut layouts = BTreeMap::new();
    for accessor in doc["accessors"]
        .as_array()
        .ok_or_else(|| Error::msg("missing accessors"))?
    {
        let view = accessor["bufferView"]
            .as_u64()
            .ok_or_else(|| Error::msg("missing accessor view"))? as usize;
        let count = accessor["count"]
            .as_u64()
            .ok_or_else(|| Error::msg("missing accessor count"))? as usize;
        let components = match accessor["type"].as_str() {
            Some("VEC3") => 3,
            Some("SCALAR") => 1,
            _ => return Err(Error::msg("unsupported vector accessor")),
        };
        let width = match accessor["componentType"].as_u64() {
            Some(5123) => 2,
            Some(5125 | 5126) => 4,
            _ => return Err(Error::msg("unsupported vector component")),
        };
        let stride = doc["bufferViews"][view]["byteStride"]
            .as_u64()
            .map(|v| v as usize)
            .unwrap_or(components * width);
        layouts.insert(view, (count, stride));
    }
    glb::meshopt_compress(doc, source, VECTOR_MESHOPT, |view, length| {
        let Some(&(count, stride)) = layouts.get(&view) else {
            return Ok(None);
        };
        if count.checked_mul(stride) != Some(length) {
            return Err(Error::msg("vector view layout mismatch"));
        }
        if !matches!(stride, 4 | 8 | 12) {
            return Err(Error::msg("unsupported vector stride"));
        }
        Ok(Some(MeshoptStream::Attributes { count, stride }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accessor_streams_decode_exactly_and_metadata_stays_raw() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tile.glb");
        let mut data = Vec::new();
        for i in 0..1000 {
            for v in [i as f32 * 0.1, 0.0, 0.0] {
                data.extend(v.to_le_bytes());
            }
        }
        let position_size = data.len();
        for _ in 0..1000 {
            data.extend(0u32.to_le_bytes());
        }
        let ids_size = data.len() - position_size;
        for i in 0..1000u32 {
            data.extend(i.to_le_bytes());
        }
        let raw_offset = data.len();
        data.extend(b"metadata");
        let original = data.clone();
        let doc = json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":data.len()}],
            "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":position_size},
                {"buffer":0,"byteOffset":position_size,"byteLength":ids_size},
                {"buffer":0,"byteOffset":position_size+ids_size,"byteLength":4000},
                {"buffer":0,"byteOffset":raw_offset,"byteLength":8}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":1000,"type":"VEC3"},
                {"bufferView":1,"componentType":5125,"count":1000,"type":"SCALAR"},
                {"bufferView":2,"componentType":5125,"count":1000,"type":"SCALAR"}]});
        let source = glb::encode_glb(&doc, &data).unwrap();
        std::fs::write(&path, &source).unwrap();
        let report = compress_file(&path).unwrap();
        assert!(report["afterBytes"].as_u64().unwrap() < report["beforeBytes"].as_u64().unwrap());
        let encoded = std::fs::read(path).unwrap();
        let glb = gltf::binary::Glb::from_slice(&encoded).unwrap();
        let result: Value = serde_json::from_slice(&glb.json).unwrap();
        let bin = glb.bin.unwrap();
        for (i, range) in [
            (0, 0..position_size),
            (1, position_size..position_size + ids_size),
            (2, position_size + ids_size..raw_offset),
        ] {
            let e = &result["bufferViews"][i]["extensions"]["EXT_meshopt_compression"];
            let start = e["byteOffset"].as_u64().unwrap() as usize;
            let end = start + e["byteLength"].as_u64().unwrap() as usize;
            let decoded = if i == 0 {
                meshopt::encoding::decode_vertex_buffer::<[u8; 12]>(&bin[start..end], 1000)
                    .unwrap()
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
            } else {
                meshopt::encoding::decode_vertex_buffer::<[u8; 4]>(&bin[start..end], 1000)
                    .unwrap()
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
            };
            assert_eq!(decoded, original[range]);
        }
        let v = &result["bufferViews"][3];
        let offset = v["byteOffset"].as_u64().unwrap() as usize;
        assert_eq!(&bin[offset..offset + 8], b"metadata");
        assert_eq!(v["buffer"], 0);
        assert_eq!(
            result["buffers"][1]["extensions"]["EXT_meshopt_compression"]["fallback"],
            true
        );
    }
}
