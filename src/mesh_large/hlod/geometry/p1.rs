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
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

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
    let m = merge(children);
    let interface = &contract.interface;
    let children_error = children
        .iter()
        .map(|c| c.geometric_error)
        .fold(0., f64::max);
    let floor = contract.scale.max(children_error);
    let mut previous: Vec<InterfaceRow> = children
        .iter()
        .flat_map(|c| c.interface.iter().copied())
        .collect();
    super::super::interfaces::merge_rows(&mut previous);
    let child_border = interface_points(&previous);
    let mut mapped_keys: HashMap<Key, Option<(Key, Key, f64, u8)>> = HashMap::default();
    for k in &m.keys {
        if child_border.contains(k) {
            mapped_keys.insert(*k, mapping(*k)?);
        }
    }
    let lookup = |k: Key| mapped_keys.get(&k).copied().flatten();
    let (mut prob, mapped0, previous) =
        collapse::problem(&m, children, interface, &lookup, target, 0.);
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
        let mut with_added = m.clone();
        let mut mapped = mapped0.clone();
        collapse::extend_added(&mut with_added, &mut mapped, &added);
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
