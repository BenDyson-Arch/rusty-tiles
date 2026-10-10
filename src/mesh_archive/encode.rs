//! Uncompressed GLB geometry with prepared shared image references.
//! No filesystem, source discovery, or publication policy.
use super::{
    approximation::ProxyGeometry,
    partition::{Bounds, Leaf},
    source::{Geometry, Image, Triangle},
};
use serde_json::{json, Value};
mod identity;
mod proxy_identity;
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
fn append_vector<const N: usize>(buffer: &mut Vec<u8>, values: [f32; N]) {
    for value in values {
        buffer.extend_from_slice(&value.to_le_bytes());
    }
}

pub(super) fn image_name(source_id: usize, image: &Image) -> String {
    format!("textures/{source_id}.{}", image.extension)
}

pub(super) fn leaf_name(id: usize) -> String {
    format!("t/{id}.glb")
}

pub(super) const ROOT_PROXY_NAME: &str = "t/root.glb";

#[derive(Clone, Copy)]
pub(super) enum RootContent {
    Routing { omission_error_metres: f64 },
    Proxy { geometric_error_metres: f64 },
}

/// One explicit hierarchy in tile-local metres. Placement is already resolved;
/// root transforms apply to both content and conservative local box half axes.
pub(super) fn tileset<E>(
    leaves: &[Leaf],
    bounds: Bounds,
    root_transform: [f64; 16],
    root_content: RootContent,
    omission_error_metres: f64,
    mut check: impl FnMut() -> Result<(), E>,
) -> Result<Vec<u8>, EncodeError<E>> {
    let mut children = Vec::with_capacity(leaves.len());
    for (id, leaf) in leaves.iter().enumerate() {
        check().map_err(EncodeError::Checkpoint)?;
        children.push(json!({
            "boundingVolume": {"box": leaf.bounds.box_values()},
            "geometricError": 0.0,
            "content": {"uri": leaf_name(id)}
        }));
    }
    let root_error = match root_content {
        RootContent::Routing {
            omission_error_metres,
        } => omission_error_metres,
        RootContent::Proxy {
            geometric_error_metres,
        } => geometric_error_metres,
    };
    let mut document = json!({
        "asset": {"version": "1.1"},
        "geometricError": omission_error_metres,
        "root": {
            "boundingVolume": {"box": bounds.box_values()},
            "transform": root_transform,
            "geometricError": root_error,
            "refine": "REPLACE",
            "children": children
        }
    });
    if matches!(root_content, RootContent::Proxy { .. }) {
        document["root"]["content"] = json!({"uri":ROOT_PROXY_NAME});
    }
    serde_json::to_vec(&document).map_err(EncodeError::Json)
}

fn index<E>(value: &Value) -> Result<usize, EncodeError<E>> {
    value
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or(EncodeError::Invalid("resource index"))
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
        for binding in material.bindings() {
            image_ids.insert(texture_image(geometry, binding.texture)?);
        }
    }
    Ok(image_ids.into_iter().collect())
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct AttributeLayout {
    normals: bool,
    tangents: bool,
    texcoords: [bool; 2],
    colors: bool,
}
impl AttributeLayout {
    fn of(triangle: &Triangle) -> Self {
        Self {
            normals: triangle.normals.is_some(),
            tangents: triangle.tangents.is_some(),
            texcoords: triangle.texcoords.map(|set| set.is_some()),
            colors: triangle.colors.is_some(),
        }
    }
}

/// One encoding path for every prepared companion attribute. The group layout
/// guarantees consistent channel presence, and every channel follows the same
/// expanded corner order as POSITION.
fn write_attribute<E, const N: usize>(
    geometry: &Geometry,
    triangles: &[usize],
    buffer: &mut Vec<u8>,
    views: &mut Vec<Value>,
    accessors: &mut Vec<Value>,
    values: impl Fn(&Triangle) -> Option<[[f32; N]; 3]>,
    check: &mut impl FnMut() -> Result<(), E>,
) -> Result<usize, EncodeError<E>> {
    let kind = match N {
        2 => "VEC2",
        3 => "VEC3",
        4 => "VEC4",
        _ => return Err(EncodeError::Invalid("attribute width")),
    };
    let offset = buffer.len();
    for &index in triangles {
        check().map_err(EncodeError::Checkpoint)?;
        let corners =
            values(&geometry.triangles[index]).ok_or(EncodeError::Invalid("attribute group"))?;
        for corner in corners {
            append_vector(buffer, corner);
        }
    }
    let view = views.len();
    views.push(
        json!({"buffer":0,"byteOffset":offset,"byteLength":buffer.len()-offset,"target":34962}),
    );
    let accessor = accessors.len();
    accessors.push(
        json!({"bufferView":view,"componentType":5126,"count":triangles.len()*3,"type":kind}),
    );
    Ok(accessor)
}

pub(super) fn validate_identity_budget(
    geometry: &Geometry,
    leaves: &[Leaf],
    proxy: Option<&ProxyGeometry>,
    mut check: impl FnMut() -> Result<(), crate::JobError>,
) -> Result<(), crate::JobError> {
    let leaf_bytes = identity::name_bytes(geometry, leaves, &mut check)?;
    let proxy_bytes = proxy.map_or(Ok(0), |proxy| {
        proxy_identity::name_bytes(geometry, proxy, &mut check)
    })?;
    let total = leaf_bytes.checked_add(proxy_bytes).ok_or_else(|| {
        crate::JobError::new(
            crate::JobErrorKind::Unsupported,
            "emitted source names overflow",
        )
    })?;
    if total > 8 * 1024 * 1024 {
        return Err(crate::JobError::new(
            crate::JobErrorKind::Unsupported,
            "emitted source node names exceed 8 MiB archive ceiling",
        ));
    }
    Ok(())
}

/// Encode already admitted, certified positions and region provenance. Source
/// material preparation owns the opaque/untextured eligibility decision.
pub(super) fn proxy(
    geometry: &Geometry,
    proxy: &ProxyGeometry,
    mut check: impl FnMut() -> Result<(), crate::JobError>,
) -> Result<Vec<u8>, crate::JobError> {
    use crate::{JobError, JobErrorKind};
    let invalid = |message: &str| JobError::new(JobErrorKind::InvalidState, message.to_owned());
    let identity = proxy_identity::Plan::new(geometry, proxy, &mut check)?;
    let mut groups: BTreeMap<Option<usize>, Vec<usize>> = BTreeMap::new();
    for (index, triangle) in proxy.triangles.iter().enumerate() {
        check()?;
        groups
            .entry(proxy.regions[triangle.region].material)
            .or_default()
            .push(index);
    }
    let material_ids: BTreeMap<_, _> = groups
        .keys()
        .flatten()
        .copied()
        .enumerate()
        .map(|(local, source)| (source, local))
        .collect();
    let mut materials = Vec::with_capacity(material_ids.len());
    for &source in material_ids.keys() {
        check()?;
        let material = geometry
            .materials
            .get(source)
            .ok_or_else(|| invalid("prepared proxy material index"))?;
        materials.push(
            material
                .remap(&BTreeMap::new())
                .map_err(|_| invalid("prepared proxy material has unresolved textures"))?,
        );
    }
    let mut buffer = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut primitives = Vec::new();
    for (material, triangles) in groups {
        let offset = buffer.len();
        let mut minimum = [f32::INFINITY; 3];
        let mut maximum = [f32::NEG_INFINITY; 3];
        for &index in &triangles {
            check()?;
            for position in proxy.triangles[index].positions {
                for axis in 0..3 {
                    if !position[axis].is_finite() {
                        return Err(invalid("prepared proxy nonfinite position"));
                    }
                    minimum[axis] = minimum[axis].min(position[axis]);
                    maximum[axis] = maximum[axis].max(position[axis]);
                }
                append_vector(&mut buffer, position);
            }
        }
        let view = views.len();
        views.push(
            json!({"buffer":0,"byteOffset":offset,"byteLength":buffer.len()-offset,"target":34962}),
        );
        let position = accessors.len();
        accessors.push(json!({"bufferView":view,"componentType":5126,"count":triangles.len()*3,"type":"VEC3","min":minimum,"max":maximum}));
        let mut attributes = json!({"POSITION":position});
        let features = identity.append_attributes(
            &triangles,
            &mut buffer,
            &mut views,
            &mut accessors,
            &mut attributes,
            &mut check,
        )?;
        let mut primitive =
            json!({"attributes":attributes,"mode":4,"extensions":{"EXT_mesh_features":features}});
        if let Some(material) = material {
            primitive["material"] = json!(material_ids[&material]);
        }
        primitives.push(primitive);
    }
    let metadata = identity.append_metadata(&mut buffer, &mut views, &mut check)?;
    let mut document = json!({"asset":{"version":"2.0","generator":"rusty-tiles F1d1"},
        "extensionsUsed":["EXT_mesh_features","EXT_structural_metadata"],
        "extensions":{"EXT_structural_metadata":metadata},
        "buffers":[{"byteLength":buffer.len()}],"bufferViews":views,"accessors":accessors,
        "meshes":[{"primitives":primitives}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0});
    if !materials.is_empty() {
        document["materials"] = Value::Array(materials);
    }
    glb(document, buffer).map_err(|error| match error {
        EncodeError::Invalid(message) => invalid(message),
        EncodeError::Json(error) => invalid(&format!("encode proxy JSON: {error}")),
        EncodeError::Checkpoint(error) => error,
    })
}

pub(super) fn leaf<E>(
    geometry: &Geometry,
    leaf: &Leaf,
    mut check: impl FnMut() -> Result<(), E>,
) -> Result<Vec<u8>, EncodeError<E>> {
    let identity = identity::Plan::new(geometry, leaf, &mut check)?;
    let mut groups: BTreeMap<(Option<usize>, AttributeLayout), Vec<usize>> = BTreeMap::new();
    for &index in &leaf.triangles {
        check().map_err(EncodeError::Checkpoint)?;
        let triangle = geometry
            .triangles
            .get(index)
            .ok_or(EncodeError::Invalid("triangle index"))?;
        groups
            .entry((triangle.material, AttributeLayout::of(triangle)))
            .or_default()
            .push(index);
    }
    let material_ids: BTreeMap<_, _> = groups
        .keys()
        .filter_map(|(id, _)| *id)
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
        for binding in material.bindings() {
            texture_sources.insert(binding.texture);
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
        materials.push(
            geometry.materials[*source]
                .remap(&texture_ids)
                .map_err(|_| EncodeError::Invalid("material texture remapping"))?,
        );
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
    for ((material, layout), triangles) in groups {
        if material.is_some_and(|source| {
            geometry.materials[source]
                .bindings()
                .any(|binding| !layout.texcoords[binding.texcoord])
        }) {
            return Err(EncodeError::Invalid(
                "textured primitive lacks bound UV set",
            ));
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
                append_vector(&mut buffer, position);
            }
        }
        let view = views.len();
        views.push(
            json!({"buffer":0,"byteOffset":offset,"byteLength":buffer.len()-offset,"target":34962}),
        );
        let position = accessors.len();
        accessors.push(json!({"bufferView":view,"componentType":5126,"count":count,"type":"VEC3","min":minimum,"max":maximum}));
        let mut attributes = json!({"POSITION":position});
        if layout.normals {
            attributes["NORMAL"] = json!(write_attribute(
                geometry,
                &triangles,
                &mut buffer,
                &mut views,
                &mut accessors,
                |t| t.normals,
                &mut check
            )?);
        }
        if layout.tangents {
            attributes["TANGENT"] = json!(write_attribute(
                geometry,
                &triangles,
                &mut buffer,
                &mut views,
                &mut accessors,
                |t| t.tangents,
                &mut check
            )?);
        }
        for (set, present) in layout.texcoords.into_iter().enumerate() {
            if present {
                attributes[format!("TEXCOORD_{set}")] = json!(write_attribute(
                    geometry,
                    &triangles,
                    &mut buffer,
                    &mut views,
                    &mut accessors,
                    |t| t.texcoords[set],
                    &mut check
                )?);
            }
        }
        if layout.colors {
            attributes["COLOR_0"] = json!(write_attribute(
                geometry,
                &triangles,
                &mut buffer,
                &mut views,
                &mut accessors,
                |t| t.colors,
                &mut check
            )?);
        }
        let features = identity.append_attributes(
            &triangles,
            &mut buffer,
            &mut views,
            &mut accessors,
            &mut attributes,
            &mut check,
        )?;
        let mut primitive =
            json!({"attributes":attributes,"mode":4,"extensions":{"EXT_mesh_features":features}});
        if let Some(material) = material {
            primitive["material"] = json!(material_ids[&material]);
        }
        primitives.push(primitive);
    }
    let metadata = identity.append_metadata(&mut buffer, &mut views, &mut check)?;
    // Eight-byte alignment is storage inside buffer zero. GLB chunk padding
    // beyond the declared buffer length may contain at most three bytes.
    buffer.resize(buffer.len().next_multiple_of(8), 0);
    let mut document = json!({"asset":{"version":"2.0","generator":"rusty-tiles F1c2"},
        "extensionsUsed":["EXT_mesh_features","EXT_structural_metadata"],
        "extensions":{"EXT_structural_metadata":metadata},
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
    glb(document, buffer)
}

/// Shared envelope only: prepared leaf and proxy identities keep separate plans.
fn glb<E>(mut document: Value, mut buffer: Vec<u8>) -> Result<Vec<u8>, EncodeError<E>> {
    buffer.resize(buffer.len().next_multiple_of(8), 0);
    document["buffers"][0]["byteLength"] = json!(buffer.len());
    let mut json = serde_json::to_vec(&document).map_err(EncodeError::Json)?;
    while !(20 + json.len()).is_multiple_of(8) {
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
