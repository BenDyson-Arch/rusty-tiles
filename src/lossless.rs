//! Tile serialization. Meshopt is a byte codec here, never a quantizer.
use crate::{
    error::Error,
    glb_write::{write_glb, TilePrimitive},
};
use serde_json::{json, Value};

pub fn write(
    prims: &[TilePrimitive],
    materials: &[Value],
    compressed: bool,
) -> Result<Vec<u8>, Error> {
    let bytes = write_glb(prims)?;
    let glb = gltf::Glb::from_slice(&bytes)?;
    let mut doc: Value = serde_json::from_slice(&glb.json)?;
    for (i, template) in materials.iter().enumerate() {
        let texture = doc["materials"][i]["pbrMetallicRoughness"]["baseColorTexture"].clone();
        doc["materials"][i] = template.clone();
        if !texture.is_null() {
            if doc["materials"][i]["pbrMetallicRoughness"].is_null() {
                doc["materials"][i]["pbrMetallicRoughness"] = json!({});
            }
            doc["materials"][i]["pbrMetallicRoughness"]["baseColorTexture"] = texture;
        }
    }
    let images = doc["images"].as_array().cloned().unwrap_or_default();
    let mut required = std::collections::BTreeSet::new();
    for texture in doc
        .get_mut("textures")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        let Some(i) = texture["source"].as_u64() else {
            continue;
        };
        let extension = match images[i as usize]["mimeType"].as_str() {
            Some("image/ktx2") => "KHR_texture_basisu",
            Some("image/webp") => "EXT_texture_webp",
            _ => continue,
        };
        texture["extensions"] = json!({extension: {"source":i}});
        texture.as_object_mut().unwrap().remove("source");
        required.insert(extension);
    }
    for extension in required {
        for key in ["extensionsUsed", "extensionsRequired"] {
            if doc[key].is_null() {
                doc[key] = json!([]);
            }
            let list = doc[key].as_array_mut().unwrap();
            if !list.iter().any(|v| v == extension) {
                list.push(json!(extension));
            }
        }
    }
    let raw = glb.bin.as_ref().unwrap();
    let mut bin = Vec::new();
    if compressed {
        let accessors = doc["accessors"].as_array().unwrap().clone();
        for (vi, view) in doc["bufferViews"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .enumerate()
        {
            let offset = view["byteOffset"].as_u64().unwrap_or(0) as usize;
            let len = view["byteLength"].as_u64().unwrap() as usize;
            let data = &raw[offset..offset + len];
            while bin.len() % 4 != 0 {
                bin.push(0);
            }
            let dst = bin.len();
            if let Some(a) = accessors
                .iter()
                .find(|a| a["bufferView"].as_u64() == Some(vi as u64))
            {
                let count = a["count"].as_u64().unwrap() as usize;
                let stride = len / count;
                let indices = a["type"] == "SCALAR";
                let encoded = if indices {
                    let idx: Vec<u32> = match stride {
                        2 => data
                            .chunks_exact(2)
                            .map(|v| u16::from_le_bytes(v.try_into().unwrap()) as u32)
                            .collect(),
                        4 => data
                            .chunks_exact(4)
                            .map(|v| u32::from_le_bytes(v.try_into().unwrap()))
                            .collect(),
                        _ => return Err(Error::msg("unsupported index width")),
                    };
                    meshopt::encoding::encode_index_buffer(
                        &idx,
                        prims.iter().map(|p| p.positions.len()).max().unwrap(),
                    )
                    .map_err(|e| Error::msg(format!("meshopt indices: {e}")))?
                } else {
                    if stride == 0 || stride > 256 || stride % 4 != 0 {
                        return Err(Error::msg("invalid meshopt stride"));
                    }
                    // Source slices have count * stride initialized bytes. The codec
                    // accepts byte data and does not require native float alignment.
                    unsafe {
                        let cap = meshopt::ffi::meshopt_encodeVertexBufferBound(count, stride);
                        let mut out = vec![0u8; cap];
                        let n = meshopt::ffi::meshopt_encodeVertexBuffer(
                            out.as_mut_ptr(),
                            cap,
                            data.as_ptr().cast(),
                            count,
                            stride,
                        );
                        if n == 0 {
                            return Err(Error::msg("meshopt vertex encoding failed"));
                        }
                        out.truncate(n);
                        out
                    }
                };
                bin.extend_from_slice(&encoded);
                view["buffer"] = json!(1);
                view["extensions"] = json!({"EXT_meshopt_compression": {
                    "buffer":0,"byteOffset":dst,"byteLength":encoded.len(),
                    "byteStride":stride,"count":count,"mode":if indices {"TRIANGLES"} else {"ATTRIBUTES"},"filter":"NONE"
                }});
            } else {
                bin.extend_from_slice(data);
                view["byteOffset"] = json!(dst);
            }
        }
        for key in ["extensionsUsed", "extensionsRequired"] {
            if doc[key].is_null() {
                doc[key] = json!([]);
            }
            doc[key]
                .as_array_mut()
                .unwrap()
                .push(json!("EXT_meshopt_compression"));
        }
        doc["buffers"] = json!([
            {"byteLength":bin.len()},
            {"byteLength":raw.len(),"extensions":{"EXT_meshopt_compression":{"fallback":true}}}
        ]);
    } else {
        bin.extend_from_slice(raw);
    }
    encode_glb(doc, bin)
}

pub fn encode_glb(doc: Value, mut bin: Vec<u8>) -> Result<Vec<u8>, Error> {
    let mut json = serde_json::to_vec(&doc)?;
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let len = 28usize
        .checked_add(json.len())
        .and_then(|n| n.checked_add(bin.len()))
        .ok_or_else(|| Error::msg("GLB size overflow"))?;
    if len > u32::MAX as usize {
        return Err(Error::msg("individual GLB exceeds 4 GiB"));
    }
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(len as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    Ok(out)
}
