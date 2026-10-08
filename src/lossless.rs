//! Tile serialization. Meshopt is a byte codec here, never a quantizer.
use crate::{
    error::Error,
    glb::{self, FallbackOffsets, MeshoptLayout, MeshoptStream},
    glb_write::{self, TilePrimitive},
};
use serde_json::{json, Value};

/// Mesh tiles keep four-byte views, source fallback offsets and an explicit
/// `NONE` filter on every compressed view.
const MESH_MESHOPT: MeshoptLayout = MeshoptLayout {
    align: 4,
    fallback: FallbackOffsets::Source,
    explicit_filter: true,
};

pub fn write(
    prims: &[TilePrimitive],
    materials: &[Value],
    compressed: bool,
) -> Result<Vec<u8>, Error> {
    write_with_features(prims, materials, compressed, &[])
}

pub(crate) fn write_with_features(
    prims: &[TilePrimitive],
    materials: &[Value],
    compressed: bool,
    features: &[(u32, String)],
) -> Result<Vec<u8>, Error> {
    let (root, mut bin) = glb_write::build(prims)?;
    // The typed root carries f32 material factors. serde_json::to_value would
    // widen them (0.22 -> 0.2199999988079071), so convert through JSON text to
    // keep the shortest f32 spelling the tiles have always carried. Only the
    // small JSON document takes this path; the binary is never copied.
    let mut doc: Value = serde_json::from_slice(&serde_json::to_vec(&root)?)?;
    for (i, template) in materials.iter().enumerate() {
        let texture = doc["materials"][i]["pbrMetallicRoughness"]["baseColorTexture"].take();
        doc["materials"][i] = template.clone();
        if !texture.is_null() {
            if doc["materials"][i]["pbrMetallicRoughness"].is_null() {
                doc["materials"][i]["pbrMetallicRoughness"] = json!({});
            }
            doc["materials"][i]["pbrMetallicRoughness"]["baseColorTexture"] = texture;
        }
    }
    let mut required = std::collections::BTreeSet::new();
    let images = doc["images"].clone();
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
        glb::add_extension(&mut doc, extension, true)?;
    }
    if !features.is_empty() {
        let mut builder = crate::metadata::MetadataGlb::from_parts(doc, bin);
        crate::metadata::attach_node_features(&mut builder, features)?;
        (doc, bin) = builder.into_parts();
    }
    if !compressed {
        return glb::encode_glb(&doc, &bin);
    }
    let vertex_count = prims.iter().map(|p| p.positions.len()).max().unwrap_or(0);
    // Only primitive index views use triangle compression. Scalar feature
    // IDs are attribute streams and must retain their exact element order.
    let index_views: std::collections::BTreeSet<_> = doc["meshes"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|m| m["primitives"].as_array().into_iter().flatten())
        .filter_map(|p| p["indices"].as_u64())
        .filter_map(|i| doc["accessors"][i as usize]["bufferView"].as_u64())
        .collect();
    let mut roles = vec![None; doc["bufferViews"].as_array().map_or(0, Vec::len)];
    for accessor in doc["accessors"].as_array().into_iter().flatten() {
        let Some(view) = accessor["bufferView"].as_u64() else {
            continue;
        };
        if let Some(role @ None) = roles.get_mut(view as usize) {
            let count = accessor["count"]
                .as_u64()
                .ok_or_else(|| Error::msg("missing accessor count"))?;
            *role = Some((count as usize, index_views.contains(&view)));
        }
    }
    // Metadata supports 64-bit values. Keep the builder alignment after the
    // compression rewrite; legacy mesh content retains its four-byte layout.
    let layout = MeshoptLayout {
        align: if features.is_empty() { 4 } else { 8 },
        ..MESH_MESHOPT
    };
    let packed = glb::meshopt_compress(&mut doc, &bin, layout, |view, length| {
        let Some((count, indices)) = roles[view] else {
            return Ok(None);
        };
        if count == 0 {
            return Err(Error::msg("invalid meshopt stride"));
        }
        let stride = length / count;
        Ok(Some(if indices {
            MeshoptStream::TriangleIndices {
                count,
                width: stride,
                vertex_count,
            }
        } else {
            MeshoptStream::Attributes { count, stride }
        }))
    })?;
    glb::encode_glb(&doc, &packed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(glb: &[u8]) -> (Value, Vec<u8>) {
        let glb = gltf::Glb::from_slice(glb).unwrap();
        (
            serde_json::from_slice(&glb.json).unwrap(),
            glb.bin.unwrap().into_owned(),
        )
    }

    fn grid(n: u32) -> TilePrimitive {
        let mut prim = TilePrimitive::default();
        for y in 0..=n {
            for x in 0..=n {
                prim.positions
                    .push([x as f32, y as f32, (x * y) as f32 * 0.01]);
                prim.normals.push([0., 0., 1.]);
            }
        }
        let w = n + 1;
        for y in 0..n {
            for x in 0..n {
                let i = y * w + x;
                prim.indices
                    .extend_from_slice(&[i, i + 1, i + w + 1, i, i + w + 1, i + w]);
            }
        }
        prim
    }

    /// Production meshopt output is smaller, declares the extension and its
    /// fallback buffer, and every compressed view decodes to the raw bytes.
    #[test]
    fn meshopt_streams_decode_to_the_uncompressed_views() {
        let prims = [grid(80)];
        let raw = write(&prims, &[], false).unwrap();
        let packed = write(&prims, &[], true).unwrap();
        assert!(packed.len() < raw.len());
        let (raw_doc, raw_bin) = parts(&raw);
        let (doc, bin) = parts(&packed);
        assert_eq!(
            doc["extensionsRequired"],
            json!(["EXT_meshopt_compression"])
        );
        assert_eq!(
            doc["buffers"][1]["extensions"]["EXT_meshopt_compression"]["fallback"],
            true
        );
        assert_eq!(doc["accessors"], raw_doc["accessors"]);
        for (i, view) in doc["bufferViews"].as_array().unwrap().iter().enumerate() {
            let original = &raw_doc["bufferViews"][i];
            let offset = original["byteOffset"].as_u64().unwrap() as usize;
            let length = original["byteLength"].as_u64().unwrap() as usize;
            let expected = &raw_bin[offset..offset + length];
            let e = &view["extensions"]["EXT_meshopt_compression"];
            assert_eq!(e["filter"], "NONE");
            let start = e["byteOffset"].as_u64().unwrap() as usize;
            assert_eq!(start % 4, 0);
            let data = &bin[start..start + e["byteLength"].as_u64().unwrap() as usize];
            let count = e["count"].as_u64().unwrap() as usize;
            if e["mode"] == "TRIANGLES" {
                // The index codec may rotate a triangle's corners; winding
                // and triangle order are preserved.
                assert_eq!(e["byteStride"], 2);
                let decoded = meshopt::encoding::decode_index_buffer::<u16>(data, count).unwrap();
                let expected: Vec<u16> = expected
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|&v| u16::from_le_bytes(v))
                    .collect();
                for (a, b) in decoded.chunks(3).zip(expected.chunks(3)) {
                    assert!((0..3).any(|r| (0..3).all(|k| a[(k + r) % 3] == b[k])));
                }
            } else {
                assert_eq!(e["mode"], "ATTRIBUTES");
                let decoded =
                    meshopt::encoding::decode_vertex_buffer::<[u8; 12]>(data, count).unwrap();
                assert_eq!(decoded.concat(), expected);
            }
        }
    }
}
