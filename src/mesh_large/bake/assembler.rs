//! Translate the bake-owned appearance packet into the real, verifying root writer.
//! POSITION, TRS, source identities and all original metadata remain root-owned.
use super::{invalid, json_charge, resource, types::*};
use crate::{
    gltf_source::source::TextureSlot,
    mesh_large::{bake_pass as root, leaves::id_texture, lod::handoff},
    JobError,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Group {
    gltf: u32,
    /// Material class representative (H28): source materials of one class merge.
    material: u32,
    pages: [Option<usize>; 5],
    ids: Vec<usize>,
    no_raster: bool,
    /// Source attributes the faces' leaves carry (NORMAL, COLOR).
    attributes: u8,
}

#[derive(Debug)]
pub(super) struct PreparedPacket {
    pub node: root::PreparedNode,
    pub writer_memory_bytes: u64,
}

/// Admission of the translated packet precedes its page/attribute/material copies.
/// The root writer must independently reserve its own complete writer lease.
fn translation_charge(p: &PreparedNode) -> Result<u64, JobError> {
    let mut bytes = 4096u64;
    let mut add = |n| {
        bytes = bytes
            .checked_add(n)
            .ok_or_else(|| resource("prepared adapter ownership sum"))?;
        Ok::<_, JobError>(())
    };
    add((p.corners.len() as u64)
        .checked_mul(160)
        .ok_or_else(|| resource("prepared adapter corners"))?)?;
    add((p.face_indices.len() as u64)
        .checked_mul(512)
        .ok_or_else(|| resource("prepared adapter groups"))?)?;
    for page in &p.pages {
        add(page.bytes.len() as u64)?;
        add(json_charge(&page.sampler) + 2048)?;
    }
    // Worst case: every face is its own group, and each group clones exactly one
    // material (its class representative), never the whole table. So faces times the
    // largest single material bounds all clones, however many materials the tile has.
    let largest = p
        .source_materials
        .values()
        .map(json_charge)
        .max()
        .unwrap_or(0);
    add(largest
        .checked_mul(p.face_indices.len() as u64)
        .ok_or_else(|| resource("prepared adapter materials"))?)?;
    for packet in &p.ids {
        add((packet.faces.len() as u64)
            .checked_mul(16)
            .ok_or_else(|| resource("prepared adapter categorical faces"))?)?;
        add((packet.sets.len() as u64)
            .checked_mul(2048)
            .ok_or_else(|| resource("prepared adapter categorical declarations"))?)?;
        for page in &packet.texture.pages {
            add(page
                .levels
                .first()
                .ok_or_else(|| invalid("categorical page has no base PNG"))?
                .len() as u64
                + 4096)?;
        }
    }
    // Actual root decoded identities and the core identity conversion coexist
    // until bake_node's audit finishes. Reserve their dynamic feature owners
    // outside the complete writer's remaining allowance.
    for face in &p.face_identities {
        add(std::mem::size_of::<root::FaceIdentity>() as u64
            + std::mem::size_of::<FaceIdentity>() as u64)?;
        add((face.features.len() as u64)
            .checked_mul(
                std::mem::size_of::<FeatureIdentity>() as u64
                    + std::mem::size_of::<(u32, Option<u32>)>() as u64,
            )
            .ok_or_else(|| resource("assembled dynamic identity ownership"))?)?;
    }
    if bytes > p.assembly_memory_bytes {
        return Err(resource(
            "prepared adapter exceeds complete assembly residual lease",
        ));
    }
    Ok(bytes)
}

fn append_image(
    out: &mut root::PreparedNode,
    bytes: &[u8],
    mime: &'static str,
    sampler: &Value,
) -> Result<u32, JobError> {
    let sampler_index = match out.samplers.iter().position(|s| s == sampler) {
        Some(index) => index,
        None => {
            out.samplers.push(sampler.clone());
            out.samplers.len() - 1
        }
    };
    let image = out.images.len();
    out.images.push(root::PreparedImage {
        bytes: bytes.to_vec(),
        mime,
    });
    let texture =
        u32::try_from(out.textures.len()).map_err(|_| resource("assembled texture index"))?;
    out.textures.push(if mime == "image/ktx2" {
        json!({"sampler":sampler_index,"extensions":{"KHR_texture_basisu":{"source":image}}})
    } else {
        json!({"source":image,"sampler":sampler_index})
    });
    Ok(texture)
}

fn binding_mut(material: &mut Value, slot: usize) -> Option<&mut Value> {
    match slot {
        0 => material
            .get_mut("pbrMetallicRoughness")?
            .get_mut("baseColorTexture"),
        1 => material
            .get_mut("pbrMetallicRoughness")?
            .get_mut("metallicRoughnessTexture"),
        2 => material.get_mut("normalTexture"),
        3 => material.get_mut("occlusionTexture"),
        4 => material.get_mut("emissiveTexture"),
        _ => None,
    }
}
fn detach(material: &mut Value, slot: usize) {
    let (owner, key) = match slot {
        0 => (material.get_mut("pbrMetallicRoughness"), "baseColorTexture"),
        1 => (
            material.get_mut("pbrMetallicRoughness"),
            "metallicRoughnessTexture",
        ),
        2 => (Some(material), "normalTexture"),
        3 => (Some(material), "occlusionTexture"),
        _ => (Some(material), "emissiveTexture"),
    };
    if let Some(object) = owner.and_then(Value::as_object_mut) {
        object.remove(key);
    }
}

/// Every face appears once, possibly split into a distinct render binding. This
/// never rewrites frozen source material labels or copied feature declarations.
pub(super) fn prepare(
    frozen: &handoff::FrozenNode,
    p: &PreparedNode,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<PreparedPacket, JobError> {
    let translated = translation_charge(p)?;
    if p.face_indices.len() != frozen.faces.len()
        || p.face_identities.len() != frozen.faces.len()
        || p.face_pages.len() != frozen.faces.len()
        || p.face_attributes.len() != frozen.faces.len()
    {
        return Err(invalid("prepared adapter frozen face closure"));
    }
    let mut no_raster = vec![false; frozen.faces.len()];
    for &face in &p.no_raster_faces {
        let flag = no_raster
            .get_mut(face)
            .ok_or_else(|| invalid("no-raster face index"))?;
        if std::mem::replace(flag, true) {
            return Err(invalid("no-raster face duplicated"));
        }
    }
    let mut out = root::PreparedNode::default();
    let mut texture_indices = Vec::with_capacity(p.pages.len());
    for page in &p.pages {
        check()?;
        texture_indices.push(append_image(
            &mut out,
            &page.bytes,
            page.mime,
            &page.sampler,
        )?);
    }
    let mut categorical = Vec::with_capacity(p.ids.len());
    for packet in &p.ids {
        check()?;
        if packet.sets.len() != packet.texture.sets.len()
            || packet.sets.len() != packet.texture.layouts.len()
            || packet.sets.len() != packet.texture.feature_counts.len()
        {
            return Err(invalid("categorical adapter layout closure"));
        }
        let mut textures = Vec::with_capacity(packet.texture.pages.len());
        for page in &packet.texture.pages {
            textures.push(append_image(
                &mut out,
                page.levels
                    .first()
                    .ok_or_else(|| invalid("categorical base page missing"))?,
                "image/png",
                &id_texture::sampler(),
            )?);
        }
        let mut entries = Vec::with_capacity(packet.sets.len());
        for (index, identity) in packet.sets.iter().enumerate() {
            let layout = &packet.texture.layouts[index];
            let count = *packet.texture.feature_counts[index]
                .first()
                .ok_or_else(|| invalid("categorical base feature count"))?;
            let reference = id_texture::TextureRef {
                texture: *textures
                    .get(layout.page)
                    .ok_or_else(|| invalid("categorical layout page"))?
                    as usize,
                channels: layout.channels.clone(),
                null_feature_id: packet.texture.sets[index].null(),
                feature_count: count,
            };
            let label = format!("baked_feature_id_{}", identity.set);
            let mut declaration = reference
                .feature_id(0, &label)
                .ok_or_else(|| invalid("occupied categorical page has no feature"))?;
            if let Some(table) = identity.table {
                declaration["propertyTable"] = json!(table);
            }
            entries.push(declaration);
        }
        categorical.push(entries);
    }
    let mut groups: BTreeMap<Group, Vec<u32>> = BTreeMap::new();
    for (face_index, face) in frozen.faces.iter().enumerate() {
        check()?;
        let source = frozen
            .primitives
            .get(face.source as usize)
            .ok_or_else(|| invalid("prepared adapter source row"))?;
        let class = *p
            .material_classes
            .get(&source.material)
            .ok_or_else(|| invalid("prepared adapter material class closure"))?;
        let pages = p.face_pages[face_index];
        if no_raster[face_index] && pages.iter().any(Option::is_some) {
            return Err(invalid("no-raster face unexpectedly owns a texture page"));
        }
        for (slot, page) in pages.iter().enumerate() {
            if let Some(page) = page {
                let encoded = p
                    .pages
                    .get(*page)
                    .ok_or_else(|| invalid("prepared binding page"))?;
                if encoded.material != class || super::slot_index(encoded.slot) != slot {
                    return Err(invalid("prepared binding material class/slot mismatch"));
                }
            }
        }
        let ids: Vec<usize> = p
            .ids
            .iter()
            .enumerate()
            .filter_map(|(i, packet)| packet.faces.contains(&face_index).then_some(i))
            .collect();
        if no_raster[face_index] && !ids.is_empty() {
            return Err(invalid(
                "no-raster face unexpectedly owns a categorical page",
            ));
        }
        groups
            .entry(Group {
                gltf: face.gltf,
                material: class,
                pages,
                ids,
                no_raster: no_raster[face_index],
                attributes: p.face_attributes[face_index],
            })
            .or_default()
            .push(u32::try_from(face_index).map_err(|_| resource("prepared face index"))?);
    }
    for (group, faces) in groups {
        check()?;
        let mut material = p
            .source_materials
            .get(&group.material)
            .ok_or_else(|| invalid("prepared source material closure"))?
            .clone();
        for slot in 0..5 {
            match group.pages[slot] {
                None => detach(&mut material, slot),
                Some(page) => {
                    let binding = binding_mut(&mut material, slot)
                        .and_then(Value::as_object_mut)
                        .ok_or_else(|| invalid("prepared page has no source texture binding"))?;
                    binding.insert("index".into(), json!(texture_indices[page]));
                    binding.insert("texCoord".into(), json!(0));
                    // Original transforms are already absorbed by source sampling.
                    if let Some(extensions) =
                        binding.get_mut("extensions").and_then(Value::as_object_mut)
                    {
                        extensions.remove("KHR_texture_transform");
                        if extensions.is_empty() {
                            binding.remove("extensions");
                        }
                    }
                    if slot == super::slot_index(TextureSlot::Normal)
                        && binding.get("scale").and_then(Value::as_f64) != Some(1.)
                    {
                        return Err(invalid("proxy normal scale was not absorbed once"));
                    }
                }
            }
        }
        let material_index = u32::try_from(out.materials.len())
            .map_err(|_| resource("prepared local material index"))?;
        out.materials.push(material);
        let mut primitive = root::PreparedPrimitive {
            gltf: group.gltf,
            faces,
            material: Some(material_index),
            // The owner checks this flag: only a no-raster group may detach source slots.
            no_raster: group.no_raster,
            ..Default::default()
        };
        // Shading inputs follow the leaves: NORMAL and COLOR_0 only where the source
        // primitive's leaves carry them (a source without normals renders unlit in
        // viewers, leaves and proxies alike). A baked normal map is authored in the
        // emitted NORMAL/TANGENT frame, so it brings both; otherwise TANGENT, which only
        // a normal map reads, is not published.
        let normal_map = group.pages[super::slot_index(TextureSlot::Normal)].is_some();
        if normal_map || group.attributes & crate::mesh_large::attributes::NORMAL != 0 {
            primitive.normals = Some(Vec::new());
        }
        if normal_map {
            primitive.tangents = Some(Vec::new());
        }
        if group.attributes & crate::mesh_large::attributes::COLOR != 0 {
            primitive.colors = Some(Vec::new());
        }
        let textured = group.pages.iter().any(Option::is_some) || !group.ids.is_empty();
        if textured {
            primitive.uvs.push(Vec::new());
        }
        for &face in &primitive.faces {
            let face = face as usize;
            for (corner_index, &index) in p.face_indices[face].iter().enumerate() {
                let corner = p
                    .corners
                    .get(index as usize)
                    .ok_or_else(|| invalid("prepared render corner index"))?;
                if corner.face != face || corner.corner as usize != corner_index {
                    return Err(invalid("prepared render corner orientation"));
                }
                if textured {
                    primitive.uvs[0].push(corner.uv);
                }
                if let Some(n) = &mut primitive.normals {
                    n.push(corner.normal);
                }
                if let Some(t) = &mut primitive.tangents {
                    t.push(corner.tangent);
                }
                if let Some(c) = &mut primitive.colors {
                    c.push(corner.color);
                }
            }
        }
        for id in group.ids {
            primitive
                .extra_feature_ids
                .extend(categorical[id].iter().cloned());
        }
        out.primitives.push(primitive);
    }
    Ok(PreparedPacket {
        node: out,
        writer_memory_bytes: p
            .assembly_memory_bytes
            .checked_sub(translated)
            .ok_or_else(|| resource("complete writer residual after translated packet"))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    impl EncodedPage {
        fn clone_page(&self) -> Self {
            Self {
                bytes: self.bytes.clone(),
                mime: self.mime,
                size: self.size,
                slot: self.slot,
                material: self.material,
                sampler: self.sampler.clone(),
                receipt: self.receipt.clone(),
            }
        }
    }
    fn prepared() -> PreparedNode {
        let frozen = super::super::integration::handoff_fixture();
        let core =
            super::super::integration::frozen_node(&frozen, Vec::new(), &[], 1 << 20, &mut || {
                Ok(())
            })
            .unwrap();
        let corners = core
            .faces
            .iter()
            .enumerate()
            .flat_map(|(face, f)| {
                (0..3).map(move |corner| RenderCorner {
                    face,
                    corner,
                    position: f.words[corner as usize],
                    transform: f.transform,
                    uv: [0.25, 0.5],
                    normal: [0., 0., 1.],
                    tangent: [1., 0., 0., 1.],
                    color: [0.2, 0.3, 0.4, 1.],
                })
            })
            .collect();
        PreparedNode {
            assembly_memory_bytes: 1 << 20,
            no_raster_faces: vec![1],
            face_attributes: vec![
                crate::mesh_large::attributes::NORMAL
                    | crate::mesh_large::attributes::COLOR;
                2
            ],
            corners,
            face_indices: vec![[0, 1, 2], [3, 4, 5]],
            face_identities: core.faces.iter().map(FrozenFace::identity).collect(),
            pages: vec![EncodedPage {
                bytes: vec![1, 2, 3],
                mime: "image/png",
                size: [4, 4],
                slot: TextureSlot::Normal,
                material: 5,
                sampler: json!({"minFilter":9729,"magFilter":9729}),
                receipt: Value::Null,
            }],
            material_classes: BTreeMap::from([(5, 5)]),
            source_materials: BTreeMap::from([(
                5,
                json!({
                    "pbrMetallicRoughness":{"roughnessFactor":0.3},
                    "normalTexture":{"index":99,"scale":1.,"texCoord":2,
                        "extensions":{"KHR_texture_transform":{"offset":[0.1,0.2]}}}
                }),
            )]),
            face_pages: vec![[None, None, Some(0), None, None], [None; 5]],
            ids: Vec::new(),
            receipts: Value::Null,
        }
    }
    #[test]
    fn mixed_no_raster_material_detaches_only_its_binding_and_preserves_corner_attributes() {
        let frozen = super::super::integration::handoff_fixture();
        let p = prepared();
        let packet = prepare(&frozen, &p, &mut || Ok(())).unwrap();
        assert!(packet.writer_memory_bytes < p.assembly_memory_bytes);
        let output = packet.node;
        assert_eq!(output.primitives.len(), 2);
        let active = output.primitives.iter().find(|p| p.faces == [0]).unwrap();
        let empty = output.primitives.iter().find(|p| p.faces == [1]).unwrap();
        assert!(empty.uvs.is_empty());
        assert_eq!(
            empty.colors.as_ref().unwrap(),
            &vec![[0.2, 0.3, 0.4, 1.]; 3]
        );
        let material = &output.materials[active.material.unwrap() as usize];
        assert_eq!(material["normalTexture"]["index"], json!(0));
        assert_eq!(material["normalTexture"]["scale"], json!(1.));
        assert_eq!(material["normalTexture"]["texCoord"], json!(0));
        assert!(material["normalTexture"].get("extensions").is_none());
        let empty_material = &output.materials[empty.material.unwrap() as usize];
        assert!(empty_material.get("normalTexture").is_none());
        assert_eq!(
            empty_material["pbrMetallicRoughness"]["roughnessFactor"],
            json!(0.3)
        );
    }
    /// H28: faces of different source materials of one class share one output primitive,
    /// one material and the class page; another class keeps its own primitive and page.
    #[test]
    fn faces_of_one_material_class_share_a_primitive() {
        let mut frozen = super::super::integration::handoff_fixture();
        frozen.primitives[1].material = 6;
        let mut p = prepared();
        p.no_raster_faces.clear();
        p.face_pages = vec![[None, None, Some(0), None, None]; 2];
        p.material_classes = BTreeMap::from([(5, 5), (6, 5)]);
        let packet = prepare(&frozen, &p, &mut || Ok(())).unwrap();
        assert_eq!(packet.node.primitives.len(), 1);
        assert_eq!(packet.node.primitives[0].faces, [0, 1]);
        assert_eq!(packet.node.materials.len(), 1);
        assert_eq!(packet.node.textures.len(), 1);
        // A face of another class cannot read this class's page...
        p.material_classes.insert(6, 6);
        assert_eq!(
            prepare(&frozen, &p, &mut || Ok(())).unwrap_err().kind(),
            crate::JobErrorKind::InvalidState
        );
        // ...and on its own class page it is its own primitive and material.
        let mut page = p.pages[0].clone_page();
        page.material = 6;
        p.pages.push(page);
        p.face_pages[1] = [None, None, Some(1), None, None];
        let six = p.source_materials[&5].clone();
        p.source_materials.insert(6, six);
        let packet = prepare(&frozen, &p, &mut || Ok(())).unwrap();
        assert_eq!(packet.node.primitives.len(), 2);
        assert_eq!(packet.node.materials.len(), 2);
    }
    /// Each output group clones one material, so a tile with many (even large) source
    /// materials is charged faces x the largest one, not faces x their sum; and the
    /// charge still covers every material clone when each face is its own group.
    #[test]
    fn translation_charge_scales_with_the_largest_material_not_the_table() {
        let mut p = prepared();
        let big = json!({"extras": "x".repeat(100_000)});
        for id in 100..140u32 {
            p.source_materials.insert(id, big.clone());
            p.material_classes.insert(id, id);
        }
        // 2 faces x 40 x 100 kB is 8 MB under the sum rule; 2 x 100 kB bounds the clones.
        p.assembly_memory_bytes = 1 << 20;
        let frozen = super::super::integration::handoff_fixture();
        let charged = translation_charge(&p).expect("table size must not multiply by faces");
        assert!(charged < 1 << 20);
        prepare(&frozen, &p, &mut || Ok(())).unwrap();
        // Sound: both faces on the largest material clone it twice.
        let mut frozen = frozen;
        frozen.primitives[0].material = 100;
        frozen.primitives[1].material = 100;
        p.no_raster_faces = vec![0, 1];
        p.face_pages = vec![[None; 5]; 2];
        p.face_attributes[1] = 0;
        let packet = prepare(&frozen, &p, &mut || Ok(())).unwrap();
        let cloned: u64 = packet.node.materials.iter().map(json_charge).sum();
        assert!(translation_charge(&p).unwrap() >= cloned);
    }
    #[test]
    fn translation_refuses_unabsorbed_scale_mislabeled_page_and_unsupported_budget() {
        let frozen = super::super::integration::handoff_fixture();
        let mut p = prepared();
        p.source_materials.get_mut(&5).unwrap()["normalTexture"]["scale"] = json!(2.);
        assert_eq!(
            prepare(&frozen, &p, &mut || Ok(())).unwrap_err().kind(),
            crate::JobErrorKind::InvalidState
        );
        let mut p = prepared();
        p.pages[0].material = 6;
        assert_eq!(
            prepare(&frozen, &p, &mut || Ok(())).unwrap_err().kind(),
            crate::JobErrorKind::InvalidState
        );
        let mut p = prepared();
        p.assembly_memory_bytes = 1;
        assert_eq!(
            prepare(&frozen, &p, &mut || Ok(())).unwrap_err().kind(),
            crate::JobErrorKind::ResourceLimit
        );
    }

    /// Structural adapter fixture only. These table rows are not an authentic
    /// capture, immutable sink or certified H17 member, and never enter bake_one.
    fn structural_writer_fixture() -> (handoff::FrozenNode, PreparedNode) {
        use id_texture::{IdPage, IdSet, IdTexture, SetLayout};
        let mut frozen = super::super::integration::handoff_fixture();
        frozen.gltf[0].feature_ids = json!([
            {"attribute":0,"featureCount":2,"propertyTable":0,"label":"source_primitive"},
            {"attribute":1,"featureCount":2,"propertyTable":1,"label":"proxy_source"},
            {"attribute":7,"featureCount":45,"propertyTable":2,"label":"custom"}
        ]);
        for (i, face) in frozen.faces.iter_mut().enumerate() {
            face.features = vec![
                (i as u32, Some(0)),
                (i as u32, Some(1)),
                (33 + i as u32 * 11, Some(2)),
            ];
        }
        frozen.tables = vec![
            handoff::FrozenTable {
                class: "source_primitive".into(),
                count: 2,
                columns: vec![
                    ("node_index".into(), "UINT32".into(), vec![10, 11]),
                    ("mesh_index".into(), "UINT32".into(), vec![20, 21]),
                    ("primitive_index".into(), "UINT32".into(), vec![30, 31]),
                    ("expanded_ordinal".into(), "UINT64".into(), vec![40, 41]),
                    ("material_index".into(), "UINT32".into(), vec![5, 5]),
                ],
            },
            handoff::FrozenTable {
                class: "proxy_source".into(),
                count: 2,
                columns: vec![
                    ("triangle_index".into(), "UINT64".into(), vec![100, 101]),
                    ("fill".into(), "UINT8".into(), vec![0, 0]),
                ],
            },
            handoff::FrozenTable {
                class: "custom".into(),
                count: 45,
                columns: vec![("value".into(), "UINT32".into(), (0..45).collect())],
            },
            handoff::FrozenTable {
                class: "unreferenced_VF".into(),
                count: 1,
                columns: vec![
                    ("V".into(), "UINT64".into(), vec![(1u64 << 53) + 7]),
                    ("F".into(), "FLOAT64".into(), vec![0.125f64.to_bits()]),
                ],
            },
        ];
        let mut p = prepared();
        p.face_identities =
            super::super::integration::frozen_node(&frozen, Vec::new(), &[], 1 << 20, &mut || {
                Ok(())
            })
            .unwrap()
            .faces
            .iter()
            .map(FrozenFace::identity)
            .collect();
        p.assembly_memory_bytes = 16 << 20;
        p.pages.clear();
        let slots = [
            TextureSlot::BaseColor,
            TextureSlot::MetallicRoughness,
            TextureSlot::Normal,
            TextureSlot::Occlusion,
            TextureSlot::Emissive,
        ];
        for (i, slot) in slots.into_iter().enumerate() {
            p.pages.push(EncodedPage {
                bytes: small_png([20 + i as u8, 80, 140, 255]),
                mime: "image/png",
                size: [4, 4],
                slot,
                material: 5,
                sampler: json!({"minFilter":9729,"magFilter":9729,"wrapS":33071,"wrapT":33071}),
                receipt: Value::Null,
            });
        }
        p.face_pages[0] = [Some(0), Some(1), Some(2), Some(3), Some(4)];
        p.source_materials.insert(5, json!({
            "pbrMetallicRoughness":{"baseColorFactor":[0.3,0.4,0.5,1.],"baseColorTexture":{"index":99},
                "metallicFactor":0.7,"roughnessFactor":0.3,"metallicRoughnessTexture":{"index":99}},
            "normalTexture":{"index":99,"scale":1.},"occlusionTexture":{"index":99,"strength":0.6},
            "emissiveTexture":{"index":99},"emissiveFactor":[0.1,0.2,0.3]
        }));
        p.ids.push(IdPacket {
            page: 0,
            faces: vec![0],
            sets: vec![
                FeatureIdentity {
                    set: 0,
                    table: Some(0),
                    value: 0,
                },
                FeatureIdentity {
                    set: 1,
                    table: Some(1),
                    value: 0,
                },
                FeatureIdentity {
                    set: 7,
                    table: Some(2),
                    value: 0,
                },
            ],
            texture: IdTexture {
                sets: vec![IdSet { bytes: 1 }; 3],
                layouts: vec![
                    SetLayout {
                        page: 0,
                        channels: vec![0],
                    },
                    SetLayout {
                        page: 0,
                        channels: vec![1],
                    },
                    SetLayout {
                        page: 0,
                        channels: vec![2],
                    },
                ],
                pages: vec![IdPage {
                    levels: vec![small_png([0, 0, 33, 255])],
                    sizes: vec![[4, 4]],
                }],
                feature_counts: vec![vec![1]; 3],
                receipts: Vec::new(),
            },
        });
        (frozen, p)
    }
    fn small_png(pixel: [u8; 4]) -> Vec<u8> {
        let mut output = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(4, 4, image::Rgba(pixel)))
            .write_to(&mut output, image::ImageFormat::Png)
            .unwrap();
        output.into_inner()
    }
    fn bounded_structural_assembly(
        frozen: &handoff::FrozenNode,
        packet: &PreparedPacket,
        tile_bytes: u64,
    ) -> root::Assembly {
        // This fixture bounds every input owner: two faces, six render corners,
        // six <=256-byte images and <=128 table values. Reserve a conservative
        // 128x input amplification plus 1 MiB for this tiny free-writer control.
        // This is not the missing production writer/native accounting interface.
        assert_eq!(frozen.faces.len(), 2);
        assert_eq!(packet.node.images.len(), 6);
        assert!(packet
            .node
            .images
            .iter()
            .all(|image| image.bytes.len() <= 256));
        assert!(
            frozen
                .tables
                .iter()
                .flat_map(|t| &t.columns)
                .map(|column| column.2.len())
                .sum::<usize>()
                <= 128
        );
        let input = serde_json::to_vec(&packet.node.materials).unwrap().len() as u64
            + packet
                .node
                .images
                .iter()
                .map(|image| image.bytes.len() as u64)
                .sum::<u64>()
            + 4096;
        assert!(input.checked_mul(128).unwrap() + (1 << 20) <= packet.writer_memory_bytes);
        // Source material 5 with the fixture's literal factors (bindings to texture 0).
        let mut materials = vec![json!({}); 5];
        materials.push(json!({
            "pbrMetallicRoughness":{"baseColorFactor":[0.3,0.4,0.5,1.],"baseColorTexture":{"index":0},
                "metallicFactor":0.7,"roughnessFactor":0.3,"metallicRoughnessTexture":{"index":0}},
            "normalTexture":{"index":0,"scale":1.},"occlusionTexture":{"index":0,"strength":0.6},
            "emissiveTexture":{"index":0},"emissiveFactor":[0.1,0.2,0.3]
        }));
        let sources = crate::gltf_source::source::parse_materials_for_tests(
            &json!({"materials":materials,"textures":[{"source":0}]}),
        );
        root::assemble(
            frozen,
            &packet.node,
            &sources,
            tile_bytes,
            packet.writer_memory_bytes,
            &|| Ok(()),
        )
        .unwrap()
    }
    #[test]
    fn structural_packet_runs_real_complete_writer_and_independent_pbr_attribute_readback() {
        use crate::mesh_large::lod::codec;
        let (frozen, p) = structural_writer_fixture();
        let packet = prepare(&frozen, &p, &mut || Ok(())).unwrap();
        let root::Assembly::Fit { glb, decoded, .. } =
            bounded_structural_assembly(&frozen, &packet, 1 << 20)
        else {
            panic!("tiny structural member must fit")
        };
        let back = codec::readback(&glb).unwrap();
        let geometry = codec::decode(&glb).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(geometry.tables[3].columns["V"], vec![(1u64 << 53) + 7]);
        assert_eq!(geometry.tables[3].components["F"], "FLOAT64");
        for (primitive_index, primitive) in packet.node.primitives.iter().enumerate() {
            for &face in &primitive.faces {
                let decoded = decoded.iter().find(|d| d.face == face).unwrap();
                assert_eq!(decoded.source.0, 10 + face);
                assert_eq!(decoded.origin.triangle, 100 + u64::from(face));
                assert_eq!(decoded.material, 5);
                assert_eq!(decoded.features[2], (33 + face * 11, Some(2)));
            }
            let attrs = &back.attributes[primitive_index];
            assert_eq!(
                attrs["NORMAL"]
                    .iter()
                    .map(|x| x.to_bits())
                    .collect::<Vec<_>>(),
                vec![[0f32, 0., 1.]; 3]
                    .into_iter()
                    .flatten()
                    .map(f32::to_bits)
                    .collect::<Vec<_>>()
            );
            // TANGENT only where a baked normal map reads it (the rasterising face).
            if primitive.faces == [0] {
                assert_eq!(
                    attrs["TANGENT"],
                    vec![[1f32, 0., 0., 1.]; 3]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                );
            } else {
                assert!(!attrs.contains_key("TANGENT"));
            }
            assert_eq!(
                attrs["COLOR_0"],
                vec![[0.2f32, 0.3, 0.4, 1.]; 3]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
            );
            if primitive.faces == [0] {
                assert_eq!(
                    attrs["TEXCOORD_0"],
                    vec![[0.25f32, 0.5]; 3]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                );
                let material = &back.doc["materials"][primitive.material.unwrap() as usize];
                for slot in 0..5 {
                    let mut copy = material.clone();
                    let binding = binding_mut(&mut copy, slot).unwrap();
                    let index = binding["index"].as_u64().unwrap() as usize;
                    let source = back.doc["textures"][index]["source"].as_u64().unwrap() as usize;
                    let rgba = image::load_from_memory(&back.images[source])
                        .unwrap()
                        .to_rgba8();
                    assert_eq!(rgba.get_pixel(0, 0).0, [20 + slot as u8, 80, 140, 255]);
                }
                assert_eq!(material["normalTexture"]["scale"], json!(1.));
                let ids = geometry.primitives[primitive_index]
                    .feature_ids
                    .as_array()
                    .unwrap();
                assert_eq!(ids.len(), 6);
                assert_eq!(ids[2]["attribute"], json!(7));
                assert_eq!(ids[5]["propertyTable"], json!(2));
                let texture = ids[5]["texture"]["index"].as_u64().unwrap() as usize;
                let sampler = back.doc["textures"][texture]["sampler"].as_u64().unwrap() as usize;
                assert_eq!(back.doc["samplers"][sampler]["minFilter"], json!(9728));
                assert_eq!(back.doc["samplers"][sampler]["magFilter"], json!(9728));
            } else {
                assert!(attrs.get("TEXCOORD_0").is_none());
                let material = &back.doc["materials"][primitive.material.unwrap() as usize];
                assert!(material.get("normalTexture").is_none());
                assert!(material["pbrMetallicRoughness"]
                    .get("baseColorTexture")
                    .is_none());
            }
        }
        assert_eq!(
            back.images,
            packet
                .node
                .images
                .iter()
                .map(|image| image.bytes.clone())
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn structural_writer_physical_budget_uses_actual_complete_member_length() {
        let (frozen, p) = structural_writer_fixture();
        let packet = prepare(&frozen, &p, &mut || Ok(())).unwrap();
        let root::Assembly::Fit { glb, .. } =
            bounded_structural_assembly(&frozen, &packet, 1 << 20)
        else {
            panic!("tiny structural member must fit")
        };
        let actual = glb.len() as u64;
        assert!(
            actual
                > packet
                    .node
                    .images
                    .iter()
                    .map(|image| image.bytes.len() as u64)
                    .sum::<u64>()
        );
        let root::Assembly::PhysicalBudget { actual_bytes } =
            bounded_structural_assembly(&frozen, &packet, actual - 1)
        else {
            panic!("complete member must exceed one-byte-short budget")
        };
        assert_eq!(actual_bytes, actual);
        assert!(matches!(
            bounded_structural_assembly(&frozen, &packet, actual),
            root::Assembly::Fit { .. }
        ));
    }
}
