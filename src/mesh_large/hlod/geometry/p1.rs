//! P1: one guard-by-construction contraction (`collapse`) of a cell whose primary build
//! failed, before the retry ladder. It contracts the merged children onto the parent
//! contract and simplifies them in one pass that never makes what the fold guard,
//! topology check or interface check refuses; a border vertex that only a face on three
//! shared points could remove is rescued by an interior Steiner vertex on the canonical
//! lattice (`collapse::Problem::steiner`). Its proposal is still assessed like every other
//! (decode validity, S2), so P1 only adds a way to build a cell.
//!
//! Two passes: stage 0 under the default policy; stage 1 also licenses new edges and faces
//! on shared points of the seam segments this cell owns (N1's seam-owner licence, declared
//! as last-resort stage 1).
use super::super::{
    borders::Contract,
    interfaces::{InterfaceRow, Key},
    store::{StoppingBound, TileGeometry, TileKey},
};
use super::{
    collapse, folds, interface_points, merge, missing, topology, BorderMap, Built, Merged,
};
use crate::{mesh_large::leaves::geometry::LatticePlan, JobError};
use rustc_hash::FxHashSet as HashSet;
use std::borrow::Cow;

/// Upper estimate of what P1 holds at once for `vertices` merged vertices, `faces` faces and
/// `rows` child interface rows: per vertex its merged copy, class, mapping, f64 position,
/// quadric, face list, stamps and heap entries; per face the merged face and representative,
/// the four edge and face hash tables at a load of 7/16, and the face lists' entries; per
/// row the merged child interface and its copy in the contraction guard. The primary build's
/// remaining hand-over is included. Calibrated with margin against the complete-parent
/// controls (`contraction_memory`), whose vertex-dense strips are the worst case measured.
fn workspace_bytes(vertices: usize, faces: usize, rows: usize) -> u64 {
    const PER_VERTEX: u64 = 600;
    const PER_FACE: u64 = 600;
    const PER_ROW: u64 = 100;
    let n = |x: usize| u64::try_from(x).unwrap_or(u64::MAX);
    n(vertices)
        .saturating_mul(PER_VERTEX)
        .saturating_add(n(faces).saturating_mul(PER_FACE))
        .saturating_add(n(rows).saturating_mul(PER_ROW))
}

/// P1 runs within its job's charge: the per-triangle geometry workspace plus the encode
/// allowance, which nothing uses until the published tile is encoded, after P1 has
/// released its state. Beyond it (vertex-dense inputs at full size) the ladder runs as
/// before without P1.
fn fits(vertices: usize, faces: usize, rows: usize) -> bool {
    let faces64 = u64::try_from(faces).unwrap_or(u64::MAX);
    workspace_bytes(vertices, faces, rows)
        <= faces64
            .saturating_mul(super::super::JOB_BYTES_PER_TRIANGLE)
            .saturating_add(super::super::JOB_ENCODE_BYTES)
}

/// The P1 proposal for `tile`, with the last-resort stage that licensed it (0 or 1), or
/// none. `caps` are the build's caps (target first, hard cap last).
pub(super) fn attempt(
    tile: TileKey,
    children: &[TileGeometry],
    caps: &[usize],
    contract: &Contract,
    mapping: &BorderMap<'_>,
    lattice: &LatticePlan,
) -> Result<Option<(u8, Built)>, JobError> {
    let (Some(&target), Some(&cap)) = (caps.first(), caps.last()) else {
        return Ok(None);
    };
    // A contract that alone needs more than the cap cannot be kept (production never
    // builds one; `build_tile` filters it out).
    if contract.minimum_triangles() > cap {
        return Ok(None);
    }
    let rows: usize = children.iter().map(|c| c.interface.len()).sum();
    let vertices: usize = children.iter().map(|c| c.positions.len()).sum();
    let faces: usize = children.iter().map(|c| c.triangles.len()).sum();
    if !fits(vertices, faces, rows) {
        return Ok(None);
    }
    let m = merge(children);
    let interface = &contract.interface;
    let children_error = children
        .iter()
        .map(|c| c.geometric_error)
        .fold(0., f64::max);
    let floor = contract.scale.max(children_error);
    let (mut prob, mapped0, previous) =
        collapse::problem(&m, children, interface, mapping, target, 0.)?;
    prob.limit = cap;
    prob.steiner = true;
    let snap = |p: [f32; 3], r: u32| {
        m.ells
            .get(r as usize)
            .and_then(|&ell| lattice.canonical(p, ell).ok())
            .unwrap_or(p)
    };
    prob.snap = Some(&snap);
    let policy = folds::Policy::last_resort(1, tile.leaf_begin, tile.leaf_end, &contract.holders);
    let owns_segment = |a: u32, b: u32| match (m.keys.get(a as usize), m.keys.get(b as usize)) {
        (Some(&a), Some(&b)) => policy.owns_segment(a, b),
        _ => false,
    };
    let owns_face = |f: [u32; 3]| {
        let keys = f.map(|v| m.keys.get(v as usize).copied());
        match keys {
            [Some(a), Some(b), Some(c)] => policy.owns_face([a, b, c]),
            _ => false,
        }
    };
    let segments: HashSet<(Key, Key)> = interface
        .iter()
        .filter(|r| !r.is_vertex())
        .map(|r| (r.a, r.b))
        .collect();
    let points = interface_points(interface);
    // `faces` over `m` (the merged children with any Steiner vertices) as a proposal, if it
    // keeps every contract segment and point, passes the contraction fold guard under
    // `policy` and changes topology only within `floor`.
    let publishable = |m: &Merged,
                       faces: &[[u32; 3]],
                       mapped: &[Option<(Key, Key, f64, u8)>],
                       policy: folds::Policy|
     -> Option<Built> {
        let (lost_edges, lost_points) = missing(faces, &m.keys, &segments, &points);
        if !lost_edges.is_empty() || !lost_points.is_empty() {
            return None;
        }
        let guard = folds::Source::for_contraction(
            &m.positions,
            &m.triangles,
            interface,
            &previous,
            mapped,
        )
        .with_policy(policy);
        if guard.violations(&m.positions, faces).contains(&true) {
            return None;
        }
        let change = topology::changes_noting(m, faces, interface, floor, &mut false)?;
        let mut remap = vec![u32::MAX; m.keys.len()];
        let mut geometry = TileGeometry {
            key: tile,
            ells: Vec::new(),
            positions: Vec::new(),
            triangles: Vec::with_capacity(faces.len()),
            representative: vec![0; faces.len()],
            primitives: m.representative.first().copied().into_iter().collect(),
            interface: interface.to_vec(),
            geometric_error: 0.,
            published_error: 0.,
            surface_error: 0.,
            decoded_override: Vec::new(),
            texture: Default::default(),
            lease: None,
        };
        let mut locked = Vec::new();
        for t in faces {
            let mut out = [0u32; 3];
            for (o, &v) in out.iter_mut().zip(t) {
                let slot = remap.get_mut(v as usize)?;
                if *slot == u32::MAX {
                    *slot = u32::try_from(geometry.positions.len()).ok()?;
                    geometry.ells.push(*m.ells.get(v as usize)?);
                    geometry.positions.push(*m.positions.get(v as usize)?);
                    locked.push(points.contains(m.keys.get(v as usize)?));
                }
                *o = *slot;
            }
            geometry.triangles.push(out);
        }
        Some(Built {
            locked,
            stopping: StoppingBound::TriangleBudget,
            simplifier_error: 0.,
            children_error,
            input_triangles: u32::try_from(m.triangles.len()).unwrap_or(u32::MAX),
            protected_faces: 0,
            retries: 0,
            last_resort: true,
            staged: false,
            last_resort_stage: 0,
            rejection: None,
            primary: false,
            p1: true,
            relief_stuck: 0,
            interface_vertices: u32::try_from(points.len()).unwrap_or(u32::MAX),
            interface_segments: u32::try_from(segments.len()).unwrap_or(u32::MAX),
            reoriented: 0,
            topology: change.receipt,
            change,
            controls: Default::default(),
            search: Default::default(),
            geometry,
        })
    };
    for stage in [0u8, 1] {
        if stage == 1 {
            prob.owns_segment = Some(&owns_segment);
            prob.owns_face = Some(&owns_face);
        }
        let (Some(faces), _, added) = collapse::simplify_added(&prob) else {
            continue;
        };
        if faces.len() > cap || faces.is_empty() {
            continue;
        }
        // Only a Steiner rescue adds vertices; otherwise the merged mesh is used as is.
        let (with_added, mapped) = if added.is_empty() {
            (Cow::Borrowed(&m), Cow::Borrowed(&mapped0))
        } else {
            let (mut m, mut mapped) = (m.clone(), mapped0.clone());
            collapse::extend_added(&mut m, &mut mapped, &added);
            (Cow::Owned(m), Cow::Owned(mapped))
        };
        let stage_policy = if stage == 1 {
            policy.clone()
        } else {
            folds::Policy::default()
        };
        if let Some(built) = publishable(&with_added, &faces, &mapped, stage_policy) {
            return Ok(Some((stage, built)));
        }
    }
    Ok(None)
}
