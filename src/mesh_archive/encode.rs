//! Narrow uncompressed, embedded GLB leaf writer. No converter/runtime policy.
use super::{partition::Leaf, source::Geometry};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error)]
pub(super) enum EncodeError<E> {
    #[error("invalid prepared geometry: {0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Json(serde_json::Error),
    #[error("encoding checkpoint failed")]
    Checkpoint(E),
}
fn append_vec3(buffer: &mut Vec<u8>, values: [f32; 3]) {
    for value in values {
        buffer.extend_from_slice(&value.to_le_bytes());
    }
}
pub(super) fn leaf<E>(
    geometry: &Geometry,
    leaf: &Leaf,
    mut check: impl FnMut() -> Result<(), E>,
) -> Result<Vec<u8>, EncodeError<E>> {
    let mut groups: BTreeMap<(Option<usize>, bool), Vec<usize>> = BTreeMap::new();
    for &index in &leaf.triangles {
        check().map_err(EncodeError::Checkpoint)?;
        let triangle = geometry
            .triangles
            .get(index)
            .ok_or(EncodeError::Invalid("triangle index"))?;
        groups
            .entry((triangle.material, triangle.normals.is_some()))
            .or_default()
            .push(index);
    }
    let mut materials = Vec::new();
    let mut material_ids = BTreeMap::new();
    for (material, _) in groups.keys() {
        if let Some(material) = material.filter(|material| !material_ids.contains_key(material)) {
            let value = geometry
                .materials
                .get(material)
                .ok_or(EncodeError::Invalid("material index"))?;
            material_ids.insert(material, materials.len());
            materials.push(value.clone());
        }
    }
    let mut buffer = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut primitives = Vec::new();
    for ((material, normals), triangles) in groups {
        let count = triangles
            .len()
            .checked_mul(3)
            .ok_or(EncodeError::Invalid("vertex count overflow"))?;
        let offset = buffer.len();
        let mut minimum = [f32::INFINITY; 3];
        let mut maximum = [f32::NEG_INFINITY; 3];
        for &index in &triangles {
            check().map_err(EncodeError::Checkpoint)?;
            for position in geometry.triangles[index].positions {
                for axis in 0..3 {
                    minimum[axis] = minimum[axis].min(position[axis]);
                    maximum[axis] = maximum[axis].max(position[axis]);
                }
                append_vec3(&mut buffer, position);
            }
        }
        let view = views.len();
        views.push(
            json!({"buffer":0,"byteOffset":offset,"byteLength":buffer.len()-offset,"target":34962}),
        );
        let position = accessors.len();
        accessors.push(json!({"bufferView":view,"componentType":5126,"count":count,"type":"VEC3","min":minimum,"max":maximum}));
        let mut attributes = json!({"POSITION":position});
        if normals {
            let offset = buffer.len();
            for &index in &triangles {
                check().map_err(EncodeError::Checkpoint)?;
                let values = geometry.triangles[index]
                    .normals
                    .ok_or(EncodeError::Invalid("normal group"))?;
                for normal in values {
                    append_vec3(&mut buffer, normal);
                }
            }
            let view = views.len();
            views.push(json!({"buffer":0,"byteOffset":offset,"byteLength":buffer.len()-offset,"target":34962}));
            let normal = accessors.len();
            accessors
                .push(json!({"bufferView":view,"componentType":5126,"count":count,"type":"VEC3"}));
            attributes["NORMAL"] = json!(normal);
        }
        let mut primitive = json!({"attributes":attributes,"mode":4});
        if let Some(material) = material {
            primitive["material"] = json!(material_ids[&material]);
        }
        primitives.push(primitive);
    }
    let mut document = json!({"asset":{"version":"2.0","generator":"rusty-tiles F1a"},
        "buffers":[{"byteLength":buffer.len()}],"bufferViews":views,"accessors":accessors,
        "meshes":[{"primitives":primitives}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0});
    if !materials.is_empty() {
        document["materials"] = Value::Array(materials);
    }
    let mut json = serde_json::to_vec(&document).map_err(EncodeError::Json)?;
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let size = 12usize
        .checked_add(8)
        .and_then(|n| n.checked_add(json.len()))
        .and_then(|n| n.checked_add(8))
        .and_then(|n| n.checked_add(buffer.len()))
        .and_then(|n| u32::try_from(n).ok())
        .ok_or(EncodeError::Invalid("GLB length"))?;
    let mut output = Vec::with_capacity(size as usize);
    output.extend_from_slice(b"glTF");
    output.extend_from_slice(&2u32.to_le_bytes());
    output.extend_from_slice(&size.to_le_bytes());
    output.extend_from_slice(&(json.len() as u32).to_le_bytes());
    output.extend_from_slice(b"JSON");
    output.extend_from_slice(&json);
    output.extend_from_slice(&(buffer.len() as u32).to_le_bytes());
    output.extend_from_slice(b"BIN\0");
    output.extend_from_slice(&buffer);
    Ok(output)
}
