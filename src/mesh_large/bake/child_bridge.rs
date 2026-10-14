//! Retained child closure, owned only until its immediate parent commits.
//! Leaf render witnesses must come from the accepted writer/readback; authored
//! source attributes cannot substitute for published corner frames or density.
use super::{invalid, packet, resource, types::*, unsupported};
use crate::{
    gltf_source::source::TextureSlot,
    mesh_large::{
        bake_pass::{BakePass, LeafPreencode, PreencodeStore},
        lod::handoff,
        records::NodeRecord,
    },
    JobError,
};

/// One accepted leaf primitive, in exactly the retained triangle order. The
/// owner supplies published positions/VS frames and expanded source identity.
pub(super) struct LeafPrimitiveWitness {
    pub faces: Vec<ChildFace>,
}
/// Renderer and terminal-source evidence currently absent from H8's retained
/// packet. Page entries are indexed by `(retained page, original material,
/// slot)` because one physical page can serve distinct literal bindings.
pub(super) struct LeafPageWitness {
    pub retained_page: u32,
    pub signal: SignalPage,
}
pub(super) struct LeafRendererWitness {
    pub node: NodeRecord,
    pub texel_size: f64,
    pub primitives: Vec<LeafPrimitiveWitness>,
    pub pages: Vec<LeafPageWitness>,
}

fn slot_name(slot: TextureSlot) -> &'static str {
    match slot {
        TextureSlot::BaseColor => "baseColorTexture",
        TextureSlot::MetallicRoughness => "metallicRoughnessTexture",
        TextureSlot::Normal => "normalTexture",
        TextureSlot::Occlusion => "occlusionTexture",
        TextureSlot::Emissive => "emissiveTexture",
    }
}
fn binding(material: &serde_json::Value, slot: TextureSlot) -> &serde_json::Value {
    if matches!(
        slot,
        TextureSlot::BaseColor | TextureSlot::MetallicRoughness
    ) {
        &material["pbrMetallicRoughness"][slot_name(slot)]
    } else {
        &material[slot_name(slot)]
    }
}
/// Convert only a complete accepted renderer/source witness. This function
/// derives actual slot page and TEXCOORD binding from retained literal material
/// JSON, verifies pre-codec pixels, and retains the normal scale exactly once.
pub(super) fn from_leaf(
    leaf: &LeafPreencode,
    mut witness: LeafRendererWitness,
    memory: u64,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<PreencodeChild, JobError> {
    if witness.primitives.len() != leaf.primitives.len() {
        return Err(invalid("leaf accepted primitive witness count"));
    }
    let count = witness
        .primitives
        .iter()
        .try_fold(0usize, |sum, p| sum.checked_add(p.faces.len()))
        .ok_or_else(|| resource("leaf child face count"))?;
    let face_bytes = (count as u64)
        .checked_mul(std::mem::size_of::<ChildFace>() as u64)
        .ok_or_else(|| resource("leaf child face lease"))?;
    let mut lease = face_bytes;
    let mut charge = |bytes: u64| -> Result<(), JobError> {
        lease = lease
            .checked_add(bytes)
            .ok_or_else(|| resource("leaf bridge combined lease overflow"))?;
        if lease > memory {
            return Err(resource("leaf bridge borrowed and owned memory lease"));
        }
        Ok(())
    };
    charge(
        (witness.primitives.capacity() as u64) * std::mem::size_of::<LeafPrimitiveWitness>() as u64,
    )?;
    for primitive in &witness.primitives {
        charge((primitive.faces.capacity() as u64) * std::mem::size_of::<ChildFace>() as u64)?;
    }
    charge((witness.pages.capacity() as u64) * std::mem::size_of::<LeafPageWitness>() as u64)?;
    charge((witness.pages.len() as u64) * std::mem::size_of::<SignalPage>() as u64)?;
    for page in &witness.pages {
        charge(packet::page_owned_bytes(&page.signal)?)?;
    }
    charge(
        (leaf.pages.capacity() as u64)
            * std::mem::size_of::<crate::mesh_large::leaves::prepared::PreencodePage>() as u64,
    )?;
    for page in &leaf.pages {
        charge(page.slot.capacity() as u64 + page.class.capacity() as u64)?;
        charge(
            (page.levels.capacity() as u64)
                * std::mem::size_of::<crate::mesh_large::leaves::prepared::PreencodeLevel>() as u64,
        )?;
        for level in &page.levels {
            charge(level.rgba.capacity() as u64)?;
        }
        if let Some(support) = &page.support {
            charge((support.capacity() as u64) * 4)?;
        }
    }
    charge(
        (leaf.primitives.capacity() as u64)
            * std::mem::size_of::<crate::mesh_large::bake_pass::LeafPrimitivePreencode>() as u64,
    )?;
    for primitive in &leaf.primitives {
        charge(
            (primitive.triangles.capacity() as u64)
                * std::mem::size_of::<(u32, u32, u32, u64)>() as u64,
        )?;
        charge((primitive.uvs.capacity() as u64) * std::mem::size_of::<Vec<[f32; 2]>>() as u64)?;
        for uv in &primitive.uvs {
            charge((uv.capacity() as u64) * std::mem::size_of::<[f32; 2]>() as u64)?;
        }
        if let Some(material) = &primitive.material {
            charge(super::json_charge(material))?;
        }
        for id in &primitive.feature_id_textures {
            charge(super::json_charge(id))?;
        }
    }
    for value in leaf.textures.iter().chain(&leaf.samplers) {
        charge(super::json_charge(value))?;
    }
    charge(
        ((leaf.textures.capacity() + leaf.samplers.capacity()) as u64)
            * std::mem::size_of::<serde_json::Value>() as u64,
    )?;
    let mut maximum_spacing = 0.0f64;
    let mut faces = Vec::new();
    faces
        .try_reserve_exact(count)
        .map_err(|_| resource("leaf child faces reserve"))?;
    for (primitive, owned) in leaf.primitives.iter().zip(&mut witness.primitives) {
        check()?;
        if owned.faces.len() != primitive.triangles.len() {
            return Err(invalid("leaf accepted face witness count"));
        }
        for (i, face) in owned.faces.iter_mut().enumerate() {
            let origin = primitive.triangles[i];
            if (
                face.source.node,
                face.source.mesh,
                face.source.primitive,
                face.source_triangle,
            ) != origin
            {
                return Err(invalid("leaf accepted original source witness"));
            }
            // No bindings supplied by the owner are trusted: derive each from
            // the exact accepted material, texture source and remapped UV set.
            face.bindings = [None; 5];
            let Some(material) = &primitive.material else {
                continue;
            };
            for slot in [
                TextureSlot::BaseColor,
                TextureSlot::MetallicRoughness,
                TextureSlot::Normal,
                TextureSlot::Occlusion,
                TextureSlot::Emissive,
            ] {
                let b = binding(material, slot);
                if b.is_null() {
                    continue;
                }
                let texture = b["index"]
                    .as_u64()
                    .and_then(|v| usize::try_from(v).ok())
                    .and_then(|i| leaf.textures.get(i))
                    .ok_or_else(|| invalid("leaf binding texture index"))?;
                let page = texture["source"]
                    .as_u64()
                    .or_else(|| texture["extensions"]["KHR_texture_basisu"]["source"].as_u64())
                    .and_then(|v| u32::try_from(v).ok())
                    .ok_or_else(|| invalid("leaf binding retained page"))?;
                let signal = witness
                    .pages
                    .iter()
                    .position(|p| {
                        p.retained_page == page
                            && p.signal.slot == slot
                            && p.signal.material == face.material
                    })
                    .ok_or_else(|| {
                        unsupported("accepted leaf terminal/frame page witness missing")
                    })?;
                let retained = leaf
                    .pages
                    .iter()
                    .find(|p| p.page == page)
                    .ok_or_else(|| invalid("leaf retained page metadata"))?;
                let role = slot_name(slot).strip_suffix("Texture").unwrap();
                let class = match slot {
                    TextureSlot::BaseColor | TextureSlot::Emissive => "colour",
                    TextureSlot::Normal => "normal",
                    _ => "data",
                };
                if retained.slot != role || retained.class != class {
                    return Err(invalid(
                        "leaf retained semantic slot/class differs from binding",
                    ));
                }
                let s = &witness.pages[signal].signal;
                let first = retained
                    .levels
                    .first()
                    .ok_or_else(|| invalid("leaf retained page has no base"))?;
                if s.width != retained.width
                    || s.height != retained.height
                    || s.rgba != first.rgba
                    || s.levels.len() < retained.levels.len()
                    || retained.levels.iter().zip(&s.levels).any(|(a, b)| {
                        a.width != b.width || a.height != b.height || a.rgba != b.rgba
                    })
                    || retained.support.as_ref().is_some_and(|q| *q != s.support)
                {
                    return Err(invalid(
                        "leaf page witness differs from actual pre-codec pixels/support",
                    ));
                }
                let set = b["texCoord"].as_u64().unwrap_or(0);
                let uv = usize::try_from(set)
                    .ok()
                    .and_then(|set| primitive.uvs.get(set))
                    .and_then(|uv| uv.get(i * 3..i * 3 + 3))
                    .ok_or_else(|| invalid("leaf literal TEXCOORD binding"))?;
                face.bindings[slot_index(slot)] = Some(ChildBinding {
                    page: signal,
                    uv: uv.try_into().unwrap(),
                });
                let positions = face.positions.map(|p| p.map(f64::from));
                let cross = super::math::cross(
                    super::math::sub(positions[1], positions[0]),
                    super::math::sub(positions[2], positions[0]),
                );
                if super::math::dot(cross, cross) > 0. {
                    let pixel_uv = std::array::from_fn(|i| {
                        [
                            f64::from(uv[i][0]) * f64::from(s.width),
                            f64::from(uv[i][1]) * f64::from(s.height),
                        ]
                    });
                    let spacing = super::math::pixel_size(positions, pixel_uv)
                        .ok_or_else(|| invalid("leaf accepted decoded UV spacing"))?;
                    maximum_spacing = maximum_spacing.max(spacing);
                }
                let sampler = texture
                    .get("sampler")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|i| usize::try_from(i).ok())
                    .and_then(|i| leaf.samplers.get(i));
                let logical = crate::mesh_large::leaves::crops::resolve(sampler)?;
                let actual = crate::mesh_large::leaves::crops::resolve(Some(&s.sampler))?;
                if logical != actual
                    || retained.min_filter != actual.min_filter
                    || retained.mag_filter != actual.mag_filter
                {
                    return Err(invalid("leaf accepted sampler witness"));
                }
                if slot == TextureSlot::Normal {
                    let scale = b
                        .get("scale")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(1.);
                    if !scale.is_finite() || scale.to_bits() != s.normal_scale.to_bits() {
                        return Err(invalid("leaf literal normal scale witness"));
                    }
                }
            }
        }
        faces.append(&mut owned.faces);
    }
    if !witness.texel_size.is_finite() || witness.texel_size < maximum_spacing {
        return Err(invalid(
            "leaf actual post-drop spacing witness understates decoded UV Jacobian",
        ));
    }
    let mut pages = Vec::new();
    pages
        .try_reserve_exact(witness.pages.len())
        .map_err(|_| resource("leaf child pages reserve"))?;
    for page in witness.pages {
        pages.push(page.signal);
    }
    let child = PreencodeChild {
        node: witness.node,
        texel_size: witness.texel_size,
        faces,
        pages,
    };
    // The exact packet validator also checks every source ledger/mip/frame and
    // the final density. Encoding here is unnecessary: validate without a copy.
    packet::validate(&child, check)?;
    Ok(child)
}

/// `memory` covers the resident child plus the encoded packet output. The
/// caller separately accounts other live baked-member/pass owners.
pub(super) fn persist(
    store: &PreencodeStore,
    child: &PreencodeChild,
    memory: u64,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<(), JobError> {
    if child
        .pages
        .iter()
        .any(|p| p.slot == TextureSlot::Normal && p.normal_scale.to_bits() != 1.0f64.to_bits())
    {
        return Err(invalid(
            "proxy preencode normal scale must be absorbed exactly once",
        ));
    }
    let bytes = packet::encode(child, memory, check)?;
    check()?;
    store.write_proxy(child.node.first, child.node.depth(), &bytes)
}
pub(super) fn load_children(
    pass: &BakePass<'_>,
    frozen: &handoff::FrozenNode,
    memory: u64,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<Vec<PreencodeChild>, JobError> {
    if frozen.children.len() > 8 {
        return Err(invalid("retained children exceed octants"));
    }
    if frozen.children.iter().any(|c| c.leaf.is_some()) {
        return Err(unsupported("H27 accepted H8 renderer-frame, terminal-source and actual-density witness is required before leaf consumption"));
    }
    // The root store currently exposes eager reads, without a file-size or
    // decoded-owner lease. Aggregate live scratch bounds each proxy file; use
    // a deliberately conservative bound before invoking its allocator. A
    // charged per-packet reader is an owner request for scalable operation.
    let live = pass.preencode.usage().0;
    if live.checked_mul(128).is_none_or(|n| n > memory) {
        return Err(resource("retained proxy eager-reader preflight lease"));
    }
    let owner = (frozen.children.len() as u64)
        .checked_mul(std::mem::size_of::<PreencodeChild>() as u64)
        .ok_or_else(|| resource("retained children outer owner overflow"))?;
    let residual = memory
        .checked_sub(owner)
        .ok_or_else(|| resource("retained children outer owner lease"))?;
    let mut out = Vec::new();
    out.try_reserve_exact(frozen.children.len())
        .map_err(|_| resource("retained children reserve"))?;
    let share = residual
        .checked_div(frozen.children.len().max(1) as u64)
        .unwrap();
    for child in &frozen.children {
        check()?;
        let bytes = pass
            .preencode
            .read_proxy(child.first, child.depth, share, pass.check)?;
        let decoded = packet::decode(&bytes, share, check)?;
        if decoded.node.first != child.first
            || decoded.node.end != child.end
            || decoded.node.depth() != child.depth
        {
            return Err(invalid("retained proxy child identity/rank"));
        }
        if decoded
            .pages
            .iter()
            .any(|p| p.slot == TextureSlot::Normal && p.normal_scale.to_bits() != 1.0f64.to_bits())
        {
            return Err(invalid("retained proxy normal scale was not absorbed"));
        }
        out.push(decoded);
    }
    Ok(out)
}
/// Invoke only after this parent has committed; failed attempts retain donors.
pub(super) fn release_children(
    store: &PreencodeStore,
    frozen: &handoff::FrozenNode,
) -> Result<(), JobError> {
    for child in &frozen.children {
        if let Some(leaf) = &child.leaf {
            store.release_leaf(leaf.index)?;
        } else {
            store.release_proxy(child.first, child.depth)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::frames::Frame;
    use super::*;
    use crate::{
        mesh_large::{
            bake_pass::LeafPrimitivePreencode,
            leaves::prepared::{PreencodeLevel, PreencodePage},
            records::{Bounds, NodeKind},
        },
        JobErrorKind,
    };
    fn fixture() -> (LeafPreencode, LeafRendererWitness) {
        let frame = Frame {
            tangent: [1., 0., 0.],
            bitangent: [0., 1., 0.],
            normal: [0., 0., 1.],
        };
        let sample = TerminalSample {
            source_rank: 4,
            barycentric: [0.25, 0.25, 0.5],
            centre: [0.5, 0.5, 0.],
            support_min: [0.; 3],
            support_max: [2., 1., 0.],
            weight: 7,
            centre_is_centroid: true,
        };
        let mut retained_pages = Vec::new();
        let mut witness_pages = Vec::new();
        for (page, slot, role, class, min, scale) in [
            (0, TextureSlot::BaseColor, "baseColor", "colour", 9728, 1.),
            (1, TextureSlot::Normal, "normal", "normal", 9987, 2.),
        ] {
            let rgba = vec![128, 128, 255, 255];
            retained_pages.push(PreencodePage {
                page,
                slot: role.into(),
                class: class.into(),
                width: 1,
                height: 1,
                levels: vec![PreencodeLevel {
                    width: 1,
                    height: 1,
                    rgba: rgba.clone(),
                }],
                support: Some(vec![7]),
                min_filter: min,
                mag_filter: 9728,
                density_drop: 1,
                source_mips: false,
            });
            witness_pages.push(LeafPageWitness{retained_page:page,signal:SignalPage{width:1,height:1,rgba:rgba.clone(),support:vec![7],slot,samples:vec![sample],levels:vec![SignalLevel{width:1,height:1,rgba,support:vec![7],frames:vec![frame],samples:vec![sample],world_normals:if slot==TextureSlot::Normal{vec![[0.,0.,1.]]}else{Vec::new()}}],sampler:serde_json::json!({"minFilter":min,"magFilter":9728,"wrapS":33071,"wrapT":33071}),material:9,receipt:serde_json::json!({"fixture":true}),normal_samples:Vec::new(),normal_scale:scale}});
        }
        let leaf = LeafPreencode {
            pages: retained_pages,
            primitives: vec![LeafPrimitivePreencode {
                material: Some(
                    serde_json::json!({"pbrMetallicRoughness":{"baseColorTexture":{"index":0,"texCoord":0}},"normalTexture":{"index":1,"texCoord":1,"scale":2.0}}),
                ),
                triangles: vec![(7, 8, 9, 11)],
                uvs: vec![
                    vec![[0., 0.], [0.5, 0.], [0., 1.]],
                    vec![[0., 0.], [1., 0.], [0., 1.]],
                ],
                feature_id_textures: Vec::new(),
            }],
            textures: vec![
                serde_json::json!({"source":0,"sampler":0}),
                serde_json::json!({"source":1,"sampler":1}),
            ],
            samplers: vec![
                serde_json::json!({"minFilter":9728,"magFilter":9728,"wrapS":33071,"wrapT":33071}),
                serde_json::json!({"minFilter":9987,"magFilter":9728,"wrapS":33071,"wrapT":33071}),
            ],
            witness: crate::mesh_large::leaves::witness::LeafWitness {
                texel_spacing: None,
                primitives: Vec::new(),
            },
        };
        let witness = LeafRendererWitness {
            node: NodeRecord {
                cell_depth: 1,
                cell: 0,
                path_len: 0,
                path: 0,
                last_octant: 0,
                kind: NodeKind::Leaf,
                children: 0,
                first: 4,
                end: 5,
                bytes: 0,
                bounds: Bounds {
                    min: [0.; 3],
                    max: [2., 1., 0.],
                },
            },
            texel_size: 4.0f64.next_up(),
            primitives: vec![LeafPrimitiveWitness {
                faces: vec![ChildFace {
                    source: SourceKey {
                        node: 7,
                        mesh: 8,
                        primitive: 9,
                        instance: 123,
                    },
                    source_triangle: 11,
                    material: 9,
                    positions: [[0., 0., 0.], [2., 0., 0.], [0., 1., 0.]],
                    frames: [frame; 3],
                    bindings: [None; 5],
                }],
            }],
            pages: witness_pages,
        };
        (leaf, witness)
    }
    #[test]
    fn leaf_bridge_maps_literal_slot_uv_sampler_scale_and_actual_spacing() {
        // This is a constructed retained-type/witness control, not a claim
        // that H8 currently exports the missing accepted renderer witness.
        let (leaf, witness) = fixture();
        let child = from_leaf(&leaf, witness, 1 << 20, &mut || Ok(())).unwrap();
        assert_eq!(child.faces[0].source.instance, 123);
        assert_eq!(child.faces[0].bindings[0].unwrap().page, 0);
        assert_eq!(child.faces[0].bindings[0].unwrap().uv[1], [0.5, 0.]);
        assert_eq!(child.faces[0].bindings[2].unwrap().page, 1);
        assert_eq!(child.faces[0].bindings[2].unwrap().uv[1], [1., 0.]);
        assert_eq!(child.pages[1].normal_scale, 2.);
        assert_eq!(child.pages[0].sampler["minFilter"], 9728);
        assert_eq!(child.pages[1].sampler["minFilter"], 9987);
        assert_eq!(child.texel_size, 4.0f64.next_up());
    }
    #[test]
    fn leaf_bridge_refuses_missing_or_mismatched_witness_and_low_lease() {
        let (leaf, witness) = fixture();
        assert!(matches!(
            from_leaf(&leaf, witness, 1, &mut || Ok(()))
                .err()
                .unwrap()
                .kind(),
            JobErrorKind::ResourceLimit
        ));
        let (leaf, mut witness) = fixture();
        witness.pages.pop();
        assert!(matches!(
            from_leaf(&leaf, witness, 1 << 20, &mut || Ok(()))
                .err()
                .unwrap()
                .kind(),
            JobErrorKind::Unsupported
        ));
        let (leaf, mut witness) = fixture();
        witness.texel_size = 2.;
        assert!(from_leaf(&leaf, witness, 1 << 20, &mut || Ok(())).is_err());
        let (leaf, mut witness) = fixture();
        witness.pages[1].signal.normal_scale = 1.;
        assert!(from_leaf(&leaf, witness, 1 << 20, &mut || Ok(())).is_err());
        let (leaf, mut witness) = fixture();
        witness.pages[0].signal.rgba[0] = 127;
        assert!(from_leaf(&leaf, witness, 1 << 20, &mut || Ok(())).is_err());
        let (mut leaf, witness) = fixture();
        leaf.pages[1].class = "data".into();
        assert!(from_leaf(&leaf, witness, 1 << 20, &mut || Ok(())).is_err());
        let (leaf, mut witness) = fixture();
        witness.primitives[0].faces[0].source_triangle = 12;
        assert!(from_leaf(&leaf, witness, 1 << 20, &mut || Ok(())).is_err());
    }
}
