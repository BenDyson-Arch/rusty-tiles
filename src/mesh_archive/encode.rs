//! Uncompressed GLB geometry with prepared shared image references.
//! No filesystem, source discovery, or publication policy.
use super::{
    partition::Leaf,
    source::{Geometry, Image},
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    convert::Infallible,
};

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

pub(super) fn image_name(source_id: usize, image: &Image) -> String {
    format!("textures/{source_id}.{}", image.extension)
}

fn index<E>(value: &Value) -> Result<usize, EncodeError<E>> {
    value
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or(EncodeError::Invalid("resource index"))
}

fn material_texture<E>(material: &Value) -> Result<Option<usize>, EncodeError<E>> {
    material
        .get("pbrMetallicRoughness")
        .and_then(|pbr| pbr.get("baseColorTexture"))
        .map(|info| index(&info["index"]))
        .transpose()
}

fn texture_image<E>(geometry: &Geometry, texture_id: usize) -> Result<usize, EncodeError<E>> {
    let texture = geometry
        .textures
        .get(texture_id)
        .ok_or(EncodeError::Invalid("texture index"))?;
    let source = index(&texture["source"])?;
    geometry
        .images
        .get(source)
        .ok_or(EncodeError::Invalid("image index"))?;
    Ok(source)
}

/// Exact image closure of the expanded selected geometry. IDs remain source IDs
/// so all leaves can share one deterministic archive member per accepted image.
pub(super) fn used_images(geometry: &Geometry) -> Result<Vec<usize>, EncodeError<Infallible>> {
    let material_ids: BTreeSet<_> = geometry
        .triangles
        .iter()
        .filter_map(|t| t.material)
        .collect();
    let mut image_ids = BTreeSet::new();
    for material_id in material_ids {
        let material = geometry
            .materials
            .get(material_id)
            .ok_or(EncodeError::Invalid("material index"))?;
        if let Some(texture_id) = material_texture(material)? {
            image_ids.insert(texture_image(geometry, texture_id)?);
        }
    }
    Ok(image_ids.into_iter().collect())
}

pub(super) fn leaf<E>(
    geometry: &Geometry,
    leaf: &Leaf,
    mut check: impl FnMut() -> Result<(), E>,
) -> Result<Vec<u8>, EncodeError<E>> {
    let mut groups: BTreeMap<(Option<usize>, bool, bool), Vec<usize>> = BTreeMap::new();
    for &index in &leaf.triangles {
        check().map_err(EncodeError::Checkpoint)?;
        let triangle = geometry
            .triangles
            .get(index)
            .ok_or(EncodeError::Invalid("triangle index"))?;
        groups
            .entry((
                triangle.material,
                triangle.normals.is_some(),
                triangle.texcoords.is_some(),
            ))
            .or_default()
            .push(index);
    }
    let material_ids: BTreeMap<_, _> = groups
        .keys()
        .filter_map(|(id, _, _)| *id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .enumerate()
        .map(|(local, source)| (source, local))
        .collect();
    let mut texture_sources = BTreeSet::new();
    for source in material_ids.keys() {
        check().map_err(EncodeError::Checkpoint)?;
        let material = geometry
            .materials
            .get(*source)
            .ok_or(EncodeError::Invalid("material index"))?;
        if let Some(texture) = material_texture(material)? {
            texture_sources.insert(texture);
        }
    }
    let texture_ids: BTreeMap<_, _> = texture_sources
        .into_iter()
        .enumerate()
        .map(|(local, source)| (source, local))
        .collect();
    let mut image_sources = BTreeSet::new();
    let mut sampler_sources = BTreeSet::new();
    for source in texture_ids.keys() {
        image_sources.insert(texture_image(geometry, *source)?);
        if let Some(sampler) = geometry.textures[*source].get("sampler") {
            let sampler = index(sampler)?;
            geometry
                .samplers
                .get(sampler)
                .ok_or(EncodeError::Invalid("sampler index"))?;
            sampler_sources.insert(sampler);
        }
    }
    let image_ids: BTreeMap<_, _> = image_sources
        .into_iter()
        .enumerate()
        .map(|(local, source)| (source, local))
        .collect();
    let sampler_ids: BTreeMap<_, _> = sampler_sources
        .into_iter()
        .enumerate()
        .map(|(local, source)| (source, local))
        .collect();
    let mut materials = Vec::with_capacity(material_ids.len());
    for source in material_ids.keys() {
        let mut material = geometry.materials[*source].clone();
        if let Some(texture) = material_texture(&material)? {
            material["pbrMetallicRoughness"]["baseColorTexture"]["index"] =
                json!(texture_ids[&texture]);
        }
        materials.push(material);
    }
    let mut textures = Vec::with_capacity(texture_ids.len());
    for source in texture_ids.keys() {
        let mut texture = geometry.textures[*source].clone();
        texture["source"] = json!(image_ids[&texture_image(geometry, *source)?]);
        if let Some(sampler) = texture.get("sampler") {
            texture["sampler"] = json!(sampler_ids[&index::<E>(sampler)?]);
        }
        textures.push(texture);
    }
    let images: Vec<_> = image_ids.keys().map(|source| {
        let image = &geometry.images[*source];
        json!({"uri": format!("../{}", image_name(*source, image)), "mimeType": image.mime_type})
    }).collect();
    let samplers: Vec<_> = sampler_ids
        .keys()
        .map(|source| geometry.samplers[*source].clone())
        .collect();
    let mut buffer = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut primitives = Vec::new();
    for ((material, normals, texcoords), triangles) in groups {
        if !texcoords
            && material.is_some_and(|source| {
                geometry.materials[source]
                    .get("pbrMetallicRoughness")
                    .is_some_and(|pbr| pbr.get("baseColorTexture").is_some())
            })
        {
            return Err(EncodeError::Invalid("textured primitive lacks TEXCOORD_0"));
        }
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
        if texcoords {
            let offset = buffer.len();
            for &index in &triangles {
                check().map_err(EncodeError::Checkpoint)?;
                let values = geometry.triangles[index]
                    .texcoords
                    .ok_or(EncodeError::Invalid("texcoord group"))?;
                for texcoord in values {
                    for value in texcoord {
                        buffer.extend_from_slice(&value.to_le_bytes());
                    }
                }
            }
            let view = views.len();
            views.push(json!({"buffer":0,"byteOffset":offset,"byteLength":buffer.len()-offset,"target":34962}));
            let texcoord = accessors.len();
            accessors
                .push(json!({"bufferView":view,"componentType":5126,"count":count,"type":"VEC2"}));
            attributes["TEXCOORD_0"] = json!(texcoord);
        }
        let mut primitive = json!({"attributes":attributes,"mode":4});
        if let Some(material) = material {
            primitive["material"] = json!(material_ids[&material]);
        }
        primitives.push(primitive);
    }
    let mut document = json!({"asset":{"version":"2.0","generator":"rusty-tiles F1b1"},
        "buffers":[{"byteLength":buffer.len()}],"bufferViews":views,"accessors":accessors,
        "meshes":[{"primitives":primitives}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0});
    if !materials.is_empty() {
        document["materials"] = Value::Array(materials);
    }
    if !textures.is_empty() {
        document["textures"] = Value::Array(textures);
        document["images"] = Value::Array(images);
    }
    if !samplers.is_empty() {
        document["samplers"] = Value::Array(samplers);
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
