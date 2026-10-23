//! Immutable H17 proxy appearance owner. Geometry is copied from the successful
//! finishing packet and independently audited after complete-member assembly.
mod assembler;
mod charts;
mod child_bridge;
#[cfg(test)]
mod controls;
mod delivery;
mod experiment;
mod frames;
mod ids;
pub(in crate::mesh_large) mod integration;
mod math;
mod mips;
mod packet;
#[cfg(test)]
mod probe_tests;
mod raster;
mod retained;
mod sampling;
mod source;
mod types;
use crate::{mesh_large::config::Profile, JobError, JobErrorKind};
pub(crate) use retained::ChildView;
#[cfg(test)]
pub(crate) use retained::{
    MIN_SLOTS as CHILD_TEXEL_MIN_SLOTS, SLOT_BYTES as CHILD_TEXEL_SLOT_BYTES,
};
use sha2::{Digest, Sha256};
pub(crate) use source::{CapturedSource, SourceImages};
use std::collections::BTreeMap;
pub(crate) use types::*;
fn invalid(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message)
}
fn resource(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::ResourceLimit, message)
}
fn unsupported(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::Unsupported, message)
}
fn add_charge(total: &mut u64, n: u64, limit: u64) -> Result<(), JobError> {
    *total = total
        .checked_add(n)
        .ok_or_else(|| resource("bake lease sum"))?;
    if *total > limit {
        return Err(resource("bake complete ownership lease"));
    }
    Ok(())
}
fn json_charge(value: &serde_json::Value) -> u64 {
    match value {
        serde_json::Value::String(s) => 64 + s.capacity() as u64,
        serde_json::Value::Array(a) => {
            64 + (a.capacity() as u64) * std::mem::size_of::<serde_json::Value>() as u64
                + a.iter().map(json_charge).sum::<u64>()
        }
        serde_json::Value::Object(o) => {
            64 + o
                .iter()
                .map(|(k, v)| 256 + k.capacity() as u64 + json_charge(v))
                .sum::<u64>()
        }
        _ => 64,
    }
}
fn packet_charge(
    f: &FrozenNode,
    children: &[ChildView],
    limits: BakeLimits,
) -> Result<u64, JobError> {
    let mut total = 0;
    add_charge(
        &mut total,
        f.original_glb.capacity() as u64,
        limits.memory_bytes,
    )?;
    // Audit map and PreparedNode each own a FaceIdentity and a feature clone.
    // Reserve the dynamic set counts BEFORE either audit allocation occurs;
    // arbitrary set count cannot hide behind a fixed per-face allowance.
    add_charge(
        &mut total,
        (f.faces.len() as u64)
            .checked_mul(2 * std::mem::size_of::<FaceIdentity>() as u64 + 256)
            .ok_or_else(|| resource("dynamic face audit lease"))?,
        limits.memory_bytes,
    )?;
    add_charge(
        &mut total,
        (f.faces.capacity() as u64)
            .checked_mul(std::mem::size_of::<FrozenFace>() as u64)
            .ok_or_else(|| resource("frozen faces"))?,
        limits.memory_bytes,
    )?;
    for face in &f.faces {
        add_charge(
            &mut total,
            (face.features.capacity() as u64) * std::mem::size_of::<FeatureIdentity>() as u64
                + (face.donor_ranges.capacity() as u64)
                    * std::mem::size_of::<std::ops::Range<u64>>() as u64,
            limits.memory_bytes,
        )?;
        add_charge(
            &mut total,
            (face.features.len() as u64)
                .checked_mul(2 * std::mem::size_of::<FeatureIdentity>() as u64)
                .ok_or_else(|| resource("dynamic feature audit clone lease"))?,
            limits.memory_bytes,
        )?;
    }
    // A child owns its faces and page metadata; its texels are resident only when the
    // child was held whole, else they stay in its packet behind the parent's texel cache.
    for c in children {
        add_charge(
            &mut total,
            (c.faces.capacity() as u64) * std::mem::size_of::<ChildFace>() as u64,
            limits.memory_bytes,
        )?;
        add_charge(&mut total, c.owned_bytes(), limits.memory_bytes)?;
    }
    add_charge(&mut total, limits.source_cache_bytes, limits.memory_bytes)?;
    Ok(total)
}
/// The most bytes a parent's child texel cache may own in production (see
/// [`retained::TexelCache`]): about nine thousand 16x16 tiles.
pub(crate) const CHILD_TEXEL_CACHE: u64 = 256 << 20;
/// Resolve texture participation once from the admitted source binding closure.
/// Scalar/COLOR-only faces retain generated normals and every immutable corner,
/// but have no atlas/decoded-UV/tangent-map sampling obligation.
///
/// Feature IDs alone do not make a face rasterise. Every frozen feature value is
/// constant over its face and published exactly as an integer attribute, as the
/// leaves publish it; a categorical page would only repaint those values. Without a
/// texture to follow, its density would fall to the geometric-error floor: on a small
/// untextured terrain that was seven 4096x4096 ID pages for one proxy, hundreds of MB
/// of tally workspace that a 1 GiB host refuses and a large host bakes for minutes.
fn raster_faces(f: &FrozenNode, source: &CapturedSource<'_>) -> Result<Vec<bool>, JobError> {
    f.faces
        .iter()
        .map(|face| {
            let textured = if face.material == u32::MAX {
                false
            } else {
                source
                    .captured
                    .materials
                    .get(face.material as usize)
                    .ok_or_else(|| invalid("immutable source material closure"))?
                    .bindings()
                    .next()
                    .is_some()
            };
            Ok(textured)
        })
        .collect()
}
/// Mandatory assembler feedback is the ONLY trigger for a density retry.
/// Fidelity/domain/cancellation failures leave immediately; no candidate may
/// escape until its complete GLB and all oriented identities are audited.
pub(crate) fn bake_node(
    f: &FrozenNode,
    source: &mut CapturedSource<'_>,
    children: &[ChildView],
    limits: BakeLimits,
    assemble: &mut Assemble<'_>,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<BakedNode, JobError> {
    check()?;
    if source.cache_limit() > limits.source_cache_bytes {
        return Err(invalid("captured source cache exceeds reserved bake lease"));
    }
    if !f.geometric_error.is_finite()
        || f.geometric_error < 0.
        || !f.child_texel_size.is_finite()
        || f.child_texel_size < 0.
        || limits.member_bytes == 0
        || limits.query_work == 0
    {
        return Err(invalid("immutable proxy bake packet/policy"));
    }
    if children.len() != usize::from(f.node.children) || children.len() > 8 {
        return Err(invalid("mandatory retained child preencode closure"));
    }
    if !children.is_empty() {
        // The at-most-eight immediate children must form the exact rank
        // partition of this actual tree node, including synthetic run paths.
        let mut ordered: [Option<&ChildView>; 8] = [None; 8];
        for (i, c) in children.iter().enumerate() {
            ordered[i] = Some(c);
        }
        ordered[..children.len()].sort_unstable_by_key(|c| c.unwrap().node.first);
        let mut next = f.node.first;
        let mut identities = std::collections::BTreeSet::new();
        for c in ordered[..children.len()].iter().flatten() {
            if c.node.first != next
                || c.node.first >= c.node.end
                || c.node.end > f.node.end
                || c.node.depth() != f.node.depth() + 1
                || c.node.parent() != Some(f.node.identity())
                || !identities.insert(c.node.identity())
            {
                return Err(invalid("immutable immediate-child identity/rank partition"));
            }
            next = c.node.end;
        }
        if next != f.node.end {
            return Err(invalid("immutable child rank partition incomplete"));
        }
    }
    let digest: [u8; 32] = Sha256::digest(&f.original_glb).into();
    if digest != f.original_sha256 {
        return Err(invalid("immutable certified GLB hash"));
    }
    let borrowed = packet_charge(f, children, limits)?;
    let mut identities = BTreeMap::new();
    for face in &f.faces {
        check()?;
        if face.donor_ranges.is_empty()
            || face
                .donor_ranges
                .iter()
                .any(|r| r.start >= r.end || r.start < f.node.first || r.end > f.node.end)
            || face.donor_ranges.windows(2).any(|r| r[0].end > r[1].start)
        {
            return Err(invalid("immutable sorted disjoint donor restrictions"));
        }
        if face.positions.iter().flatten().any(|v| !v.is_finite())
            || face
                .welds
                .iter()
                .flatten()
                .any(|v| !f32::from_bits(*v).is_finite())
            || face
                .fill_bound
                .is_some_and(|b| !b.is_finite() || b < 0. || b > f.geometric_error)
        {
            return Err(invalid("immutable proxy/fill domain"));
        }
        let scale = face.transform.scale.map(f32::from_bits);
        if scale[0] <= 0.
            || !scale[0].is_finite()
            || face.transform.scale != [face.transform.scale[0]; 3]
            || face
                .transform
                .translation
                .iter()
                .any(|v| !f32::from_bits(*v).is_finite())
        {
            return Err(invalid(
                "immutable H17 POSITION transform must be finite positive uniform",
            ));
        }
        for i in 0..3 {
            if face.transform.decode(face.words[i]).map(f32::to_bits)
                != face.positions[i].map(f32::to_bits)
            {
                return Err(invalid(
                    "frozen decoded position differs from raw POSITION/TRS words",
                ));
            }
        }
        if identities.insert(face.ordinal, face.identity()).is_some() {
            return Err(invalid("immutable face ordinal duplicated"));
        }
    }
    let raster_faces = raster_faces(f, source)?;
    let decodes_before = source.decodes();
    let available = limits
        .memory_bytes
        .checked_sub(borrowed)
        .ok_or_else(|| resource("bake borrowed packet lease"))?;
    source.begin(f.node, limits.query_work)?;
    // Correspondence queries read the node's source triangles from memory when a quarter
    // of the remaining lease holds them; the held bytes leave the lease.
    let resident = source.make_resident(available / 4, check)?;
    let available = available - resident;
    let k = match limits.profile {
        Profile::Fast => 2.,
        Profile::Balanced => 1.,
        Profile::Quality | Profile::Lossless => 0.5,
    };
    let child_density = children.iter().map(|c| c.texel_size).fold(0., f64::max);
    // A zero child spacing is a leaf with no rasterising bound face: no density floor.
    if children
        .iter()
        .any(|c| !c.texel_size.is_finite() || c.texel_size < 0.)
        || child_density != f.child_texel_size
    {
        return Err(invalid("actual child post-drop density handoff"));
    }
    let prescribed = (k * f.geometric_error / 16.).max(child_density);
    // The source heuristic supplies a finite starting density only when the
    // prescribed error/actual-child policy is zero. It never silently coarsens
    // positive policy: that requires complete-member PhysicalBudget feedback.
    let planned = if prescribed > 0. {
        prescribed
    } else {
        source.density_floor(check)?
    };
    // What each face's leaves carry, so the proxy publishes the same shading inputs.
    let mut face_attributes = Vec::with_capacity(f.faces.len());
    for face in &f.faces {
        let flags = source.primitive_flags(face.source, check)?;
        face_attributes.push(
            flags & (crate::mesh_large::attributes::NORMAL | crate::mesh_large::attributes::COLOR),
        );
    }
    let mut budget_receipts = Vec::new();
    let mut stats = BakeStats::default();
    let seconds = |t: std::time::Instant| t.elapsed().as_secs_f64();
    for density_drop in 0..=limits.max_density_drops {
        check()?;
        let texel = planned
            * 2f64
                .powi(i32::try_from(density_drop).map_err(|_| resource("proxy density exponent"))?);
        if !texel.is_finite() {
            return Err(resource("proxy density schedule overflow"));
        }
        let local = BakeLimits {
            memory_bytes: available,
            ..limits
        };
        let chart_bytes = (f.faces.len() as u64)
            .checked_mul(1024)
            .ok_or_else(|| resource("resident chart lease overflow"))?;
        let query_bytes = children
            .iter()
            .try_fold(0u64, |sum, c| {
                (c.faces.len() as u64)
                    .checked_mul(512)
                    .and_then(|b| sum.checked_add(b))
            })
            .ok_or_else(|| resource("resident child-query lease overflow"))?;
        // Children whose texels stay in their packets are read through one cache of
        // fixed size: an eighth of the lease, at most the policy's bytes.
        let texel_bytes = if children.iter().any(ChildView::reads_packet) {
            (available / 8).min(limits.child_texel_bytes)
        } else {
            0
        };
        let resident = chart_bytes
            .checked_add(query_bytes)
            .and_then(|b| b.checked_add(texel_bytes))
            .ok_or_else(|| resource("resident chart/query lease overflow"))?;
        // Combined admission precedes either owner's node-sized allocations;
        // each constructor receives only its own reservation.
        let raster_limits = BakeLimits {
            memory_bytes: available
                .checked_sub(resident)
                .ok_or_else(|| resource("resident chart/query lease"))?,
            ..local
        };
        let chart_limits = BakeLimits {
            memory_bytes: chart_bytes,
            ..local
        };
        let captured = source.captured;
        let started = std::time::Instant::now();
        let layout = charts::build(
            f,
            &raster_faces,
            chart_limits,
            texel,
            &captured.material_classes,
            check,
        )?;
        stats.charts_seconds += seconds(started);
        // H28: one output material per class, from its representative; every source
        // material of a class has equal literal factors, slots and slot filters.
        let material_classes: BTreeMap<u32, u32> = f
            .faces
            .iter()
            .map(|f| (f.material, source.class(f.material)))
            .collect();
        let mut materials = BTreeMap::new();
        for i in material_classes
            .values()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
        {
            let mut material = source.material(i)?;
            // Source normalScale is absorbed exactly once into reexpressed unit
            // proxy normals. The output authored TBN path therefore uses 1.
            if material.get("normalTexture").is_some() {
                material["normalTexture"]["scale"] = serde_json::json!(1.);
            }
            materials.insert(i, material);
        }
        // Every page encode refuses its lease when the pages' owned ledgers, its extra
        // chain ledger and its codec workspace exceed what remains (`delivery::encode`).
        // The layout fixes the pages and the bytes each owns once rasterised, which only
        // grow from there, so a tile that must refuse at an encode refuses here, with the
        // same reason, before the raster's work rather than after it.
        // The raster's own charge (its corner attributes and every page's workspace)
        // is likewise fixed by the layout and refused against its lease in the same way.
        let planned_pages = raster::planned_pages(f, &layout, source)?;
        let raster_charge = planned_pages
            .iter()
            .fold((f.faces.len() as u64).saturating_mul(2048), |sum, p| {
                sum.saturating_add(p.raster_charge)
            });
        if raster_charge > raster_limits.memory_bytes {
            return Err(resource("raster semantic pages exceed lease"));
        }
        let mut floor = (f.faces.len() as u64)
            .checked_mul(4096)
            .ok_or_else(|| resource("chart/attribute/identity owning lease"))?;
        for material in materials.values() {
            floor = floor.saturating_add(json_charge(material));
        }
        for p in &planned_pages {
            floor = floor.saturating_add(raster::page_bytes(p.size, p.slot).unwrap_or(u64::MAX));
        }
        for p in &planned_pages {
            let (extra, workspace) = delivery::need(p.size, 0, limits.profile)?;
            if floor.saturating_add(extra).saturating_add(workspace) > available {
                return Err(resource("proxy delivery codec plus semantic ledger peak"));
            }
        }
        let started = std::time::Instant::now();
        let mut query =
            sampling::ChildQuery::new(children, query_bytes, texel_bytes, limits.query_work)?;
        let mut raster = raster::build(f, &layout, source, &mut query, raster_limits, check)?;
        stats.child_tile_reads += query.tile_reads();
        drop(query);
        stats.raster_seconds += seconds(started);
        let mut encoded = Vec::new();
        let mut owned = 0u64;
        add_charge(
            &mut owned,
            (f.faces.len() as u64)
                .checked_mul(4096)
                .ok_or_else(|| resource("chart/attribute/identity owning lease"))?,
            available,
        )?;
        for material in materials.values() {
            add_charge(&mut owned, json_charge(material), available)?;
        }
        for page in &raster.pages {
            add_charge(
                &mut owned,
                (page.normal_samples.capacity() as u64)
                    * std::mem::size_of::<NormalSample>() as u64,
                available,
            )?;
            add_charge(
                &mut owned,
                page.rgba.capacity() as u64
                    + (page.support.capacity() as u64) * 4
                    + (page.samples.capacity() as u64)
                        * std::mem::size_of::<TerminalSample>() as u64,
                available,
            )?;
            for level in &page.levels {
                add_charge(
                    &mut owned,
                    (level.world_normals.capacity() as u64) * 24,
                    available,
                )?;
                add_charge(
                    &mut owned,
                    level.rgba.capacity() as u64
                        + (level.support.capacity() as u64) * 4
                        + (level.frames.capacity() as u64)
                            * std::mem::size_of::<frames::Frame>() as u64
                        + (level.samples.capacity() as u64)
                            * std::mem::size_of::<TerminalSample>() as u64,
                    available,
                )?;
            }
        }
        for mask in &raster.masks {
            add_charge(
                &mut owned,
                (mask.capacity() as u64)
                    * std::mem::size_of::<crate::mesh_large::leaves::mask::SurfaceSample>() as u64,
                available,
            )?;
        }
        for owner in &raster.owner {
            add_charge(
                &mut owned,
                (owner.capacity() as u64) * std::mem::size_of::<usize>() as u64,
                available,
            )?;
        }
        let started = std::time::Instant::now();
        for (i, page) in raster.pages.iter_mut().enumerate() {
            check()?;
            let material = source.material(page.material)?;
            let remaining = available
                .checked_sub(owned)
                .ok_or_else(|| resource("bake page output lease"))?;
            let output = delivery::encode(
                page,
                &raster.masks[i],
                &material,
                limits.profile,
                remaining,
                limits.query_work,
                check,
            )?;
            add_charge(&mut owned, output.bytes.capacity() as u64, available)?;
            for level in page.levels.iter().skip(1) {
                add_charge(
                    &mut owned,
                    (level.world_normals.capacity() as u64) * 24,
                    available,
                )?;
                add_charge(
                    &mut owned,
                    (level.rgba.capacity() as u64)
                        + (level.support.capacity() as u64) * 4
                        + (level.frames.capacity() as u64)
                            * std::mem::size_of::<frames::Frame>() as u64
                        + (level.samples.capacity() as u64)
                            * std::mem::size_of::<TerminalSample>() as u64,
                    available,
                )?;
            }
            encoded.push(output);
        }
        stats.encode_seconds += seconds(started);
        // H28: categorical ID pages only repeat the exact per-face feature-ID attributes,
        // so they are baked only on request (a future annotate pass).
        let ids = if limits.id_textures {
            ids::bake(
                f,
                &layout,
                available
                    .checked_sub(owned)
                    .ok_or_else(|| resource("categorical residual lease"))?,
                limits.query_work,
                check,
            )?
        } else {
            Vec::new()
        };
        for packet in &ids {
            for page in &packet.texture.pages {
                for bytes in &page.levels {
                    add_charge(&mut owned, bytes.capacity() as u64, available)?;
                }
            }
        }
        let mut corners = Vec::with_capacity(f.faces.len() * 3);
        let mut indices = Vec::with_capacity(f.faces.len());
        for (i, face) in f.faces.iter().enumerate() {
            let first = u32::try_from(corners.len())
                .map_err(|_| resource("proxy expanded corner count"))?;
            for c in 0..3 {
                corners.push(RenderCorner {
                    face: i,
                    corner: c as u8,
                    position: face.words[c],
                    transform: face.transform,
                    uv: layout.uv[i][c],
                    normal: raster.normal_attributes[i][c],
                    tangent: raster.tangent_attributes[i][c],
                    color: raster.colors[i][c],
                });
            }
            indices.push([first, first + 1, first + 2]);
        }
        let receipts = serde_json::json!({"appearance":raster.receipt,"density":{"planned":planned,"actual":raster.texel_size,"schedule":texel,"drops":density_drop,"physicalBudgetAttempts":budget_receipts},
            "charts":layout.charts.iter().map(|c|serde_json::json!({"faces":c.faces.len(),"sigmaMin":c.sigma,"compensation":1./c.sigma,"pixelsPerMetre":c.pixels_per_metre,"page":c.page,"size":c.size})).collect::<Vec<_>>(),
            "gutter":layout.gutter,"subTexelNoRasterFaces":layout.subtexel_no_raster,"sourceQueryWork":source.work(),"completeOwnedProvisional":owned,"normalScaleAbsorbed":true,"geometryReencoded":false});
        let prepared = PreparedNode {
            face_attributes: face_attributes.clone(),
            // Zero-area faces, and bound faces the layout demoted (sub-texel slivers).
            no_raster_faces: f
                .faces
                .iter()
                .enumerate()
                .filter_map(|(i, f)| {
                    (math::face_normal(f.positions.map(|p| p.map(f64::from))).is_none()
                        || (raster_faces[i] && !layout.raster_faces[i]))
                        .then_some(i)
                })
                .collect(),
            assembly_memory_bytes: available
                .checked_sub(owned)
                .ok_or_else(|| resource("complete assembly residual lease"))?,
            corners,
            face_indices: indices,
            face_identities: f.faces.iter().map(FrozenFace::identity).collect(),
            pages: encoded,
            source_materials: materials,
            material_classes: material_classes.clone(),
            face_pages: raster.face_pages.clone(),
            ids,
            receipts: receipts.clone(),
        };
        check()?;
        let started = std::time::Instant::now();
        let assembled = assemble(&prepared)?;
        stats.assemble_seconds += seconds(started);
        match assembled {
            Assembly::PhysicalBudget { actual_bytes } => {
                if actual_bytes <= limits.member_bytes {
                    return Err(invalid(
                        "assembler budget feedback below complete-member limit",
                    ));
                }
                budget_receipts.push(
                    serde_json::json!({"drop":density_drop,"actualCompleteBytes":actual_bytes}),
                );
            }
            Assembly::Fit { glb, decoded } => {
                check()?;
                if glb.capacity() as u64 > prepared.assembly_memory_bytes {
                    return Err(resource("assembler output exceeded residual lease"));
                }
                if glb.len() as u64 > limits.member_bytes {
                    return Err(invalid("assembler Fit exceeded complete-member byte limit"));
                }
                if glb.is_empty() || decoded.len() != identities.len() {
                    return Err(invalid("assembled proxy face closure"));
                }
                let mut audit = BTreeMap::new();
                for face in decoded {
                    if audit.insert(face.ordinal, face).is_some() {
                        return Err(invalid("assembled proxy face ordinal duplicated"));
                    }
                }
                if audit != identities {
                    return Err(invalid("assembled proxy changed oriented geometry/storage/provenance/feature identity"));
                }
                let faces = f
                    .faces
                    .iter()
                    .enumerate()
                    .map(|(i, face)| ChildFace {
                        source: face.source,
                        source_triangle: face.source_triangle,
                        material: face.material,
                        positions: face.positions,
                        frames: raster.frames[i],
                        bindings: raster.face_pages[i].map(|page| {
                            page.map(|page| ChildBinding {
                                page,
                                uv: layout.uv[i],
                            })
                        }),
                    })
                    .collect();
                return Ok(BakedNode {
                    glb,
                    preencode: PreencodeChild {
                        node: f.node,
                        texel_size: raster.texel_size,
                        faces,
                        pages: raster.pages,
                    },
                    receipts,
                    stats: BakeStats {
                        facing_fallback_samples: raster.facing_fallback_samples,
                        facing_fallback_faces: raster.facing_fallback_faces,
                        fill_fallback_samples: raster.fill_fallback_samples,
                        source_decodes: source.decodes().0 - decodes_before.0,
                        source_decode_seconds: source.decodes().1 - decodes_before.1,
                        ..stats
                    },
                });
            }
        }
    }
    Err(resource(
        "complete proxy member exceeds physical byte budget after bounded density schedule",
    ))
}
