//! Lossless buffer-view compression for generated vector content. Metadata and
//! polygon/feature accessor identities remain unchanged; no vertex reordering.
use crate::error::Error;
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path};

fn encode<const N: usize>(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let values: Vec<[u8; N]> = bytes
        .chunks_exact(N)
        .map(|v| v.try_into().unwrap())
        .collect();
    meshopt::encoding::encode_vertex_buffer(&values).map_err(|e| Error::msg(e.to_string()))
}

pub fn compress_file(path: &Path) -> Result<Value, Error> {
    let input = std::fs::read(path)?;
    let glb = gltf::binary::Glb::from_slice(&input)?;
    let mut doc: Value = serde_json::from_slice(&glb.json)?;
    let source = glb
        .bin
        .ok_or_else(|| Error::msg("vector GLB has no binary chunk"))?;
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
    let views = doc["bufferViews"]
        .as_array_mut()
        .ok_or_else(|| Error::msg("missing views"))?;
    let mut binary = Vec::new();
    let mut virtual_size = 0;
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
        while binary.len() % 8 != 0 {
            binary.push(0);
        }
        let start = binary.len();
        if let Some(&(count, stride)) = layouts.get(&index) {
            if count.checked_mul(stride) != Some(length) {
                return Err(Error::msg("vector view layout mismatch"));
            }
            let packed = match stride {
                4 => encode::<4>(bytes)?,
                8 => encode::<8>(bytes)?,
                12 => encode::<12>(bytes)?,
                _ => return Err(Error::msg("unsupported vector stride")),
            };
            while virtual_size % 8 != 0 {
                virtual_size += 1;
            }
            view["buffer"] = json!(1);
            view["byteOffset"] = json!(virtual_size);
            virtual_size += length;
            view["extensions"] = json!({"EXT_meshopt_compression":{"buffer":0,"byteOffset":start,
                "byteLength":packed.len(),"byteStride":stride,"count":count,"mode":"ATTRIBUTES"}});
            binary.extend(packed);
        } else {
            view["buffer"] = json!(0);
            view["byteOffset"] = json!(start);
            binary.extend(bytes);
        }
    }
    doc["buffers"] = json!([{"byteLength":binary.len()}, {"byteLength":virtual_size,
        "extensions":{"EXT_meshopt_compression":{"fallback":true}}}]);
    for name in ["extensionsUsed", "extensionsRequired"] {
        if doc.get(name).is_none() {
            doc[name] = json!([]);
        }
        doc[name]
            .as_array_mut()
            .ok_or_else(|| Error::msg("invalid extension list"))?
            .push(json!("EXT_meshopt_compression"));
    }
    let json = serde_json::to_vec(&doc)?;
    let output = gltf::binary::Glb {
        header: gltf::binary::Header {
            magic: *b"glTF",
            version: 2,
            length: 0,
        },
        json: std::borrow::Cow::Owned(json),
        bin: Some(std::borrow::Cow::Owned(binary)),
    }
    .to_vec()?;
    std::fs::write(path, &output)?;
    Ok(json!({"beforeBytes":input.len(),"afterBytes":output.len()}))
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
        let json = serde_json::to_vec(&doc).unwrap();
        let source = gltf::binary::Glb {
            header: gltf::binary::Header {
                magic: *b"glTF",
                version: 2,
                length: 0,
            },
            json: std::borrow::Cow::Owned(json),
            bin: Some(std::borrow::Cow::Owned(data)),
        }
        .to_vec()
        .unwrap();
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
