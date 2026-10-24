//! D1: a parent from its immediate children by position-welded meshopt simplification.
//!
//! Only the parent's outer interface with other ownership subtrees is locked, as complete
//! segments (shared source edges) and shared vertices (point contacts and junctions);
//! interior child boundaries and UV/attribute/primitive seams are free. When meshopt still
//! deletes a required segment, the input faces incident to it are protected (kept
//! verbatim, their vertices locked) and the rest is simplified again, a bounded number of
//! times. A tile that cannot keep its interface within the cap is `unsupported`.
use super::{
    interfaces::{key, InterfaceRow, Key},
    store::{SourcePrimitive, StoppingBound, TileGeometry, TileKey},
};
use crate::{JobError, JobErrorKind};
use std::collections::{HashMap, HashSet};

/// Protect-and-retry rounds after the first simplification.
pub(crate) const RETRIES: u32 = 3;

fn state(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.into())
}

/// A built parent, before encoding.
#[derive(Debug)]
pub(crate) struct Built {
    pub geometry: TileGeometry,
    /// Per vertex: on the parent's outer interface (locked).
    pub locked: Vec<bool>,
    pub stopping: StoppingBound,
    pub simplifier_error: f64,
    pub children_error: f64,
    pub input_triangles: u32,
    pub protected_faces: u32,
    pub retries: u32,
    pub interface_vertices: u32,
    pub interface_segments: u32,
    /// Output faces turned to (or dropped beside) their source orientation (`orient`).
    pub reoriented: u32,
    /// Triangle visits of the representative selection (linear: `representatives`).
    pub representative_visits: u64,
}

/// Children merged into one vertex space welded by canonical decoded position, in child
/// (slot) order.
struct Merged {
    keys: Vec<Key>,
    ells: Vec<u8>,
    positions: Vec<[f32; 3]>,
    triangles: Vec<[u32; 3]>,
    representative: Vec<SourcePrimitive>,
}

fn merge(children: &[TileGeometry]) -> Merged {
    let triangles: usize = children.iter().map(|c| c.triangles.len()).sum();
    let mut index: HashMap<Key, u32> = HashMap::with_capacity(triangles);
    let mut merged = Merged {
        keys: Vec::new(),
        ells: Vec::new(),
        positions: Vec::new(),
        triangles: Vec::with_capacity(triangles),
        representative: Vec::with_capacity(triangles),
    };
    for child in children {
        let mut local = Vec::with_capacity(child.positions.len());
        for (v, p) in child.positions.iter().enumerate() {
            let next = merged.keys.len() as u32;
            let id = *index.entry(key(*p)).or_insert(next);
            if id == next {
                merged.keys.push(key(*p));
                merged.ells.push(child.ells[v]);
                merged.positions.push(*p);
            } else {
                // A finer lattice still holds the point exactly.
                let ell = &mut merged.ells[id as usize];
                *ell = (*ell).max(child.ells[v]);
            }
            local.push(id);
        }
        for (t, tri) in child.triangles.iter().enumerate() {
            merged.triangles.push(tri.map(|v| local[v as usize]));
            merged
                .representative
                .push(child.primitives[child.representative[t] as usize]);
        }
    }
    // Triangles whose corners decode onto fewer than three points carry no surface: drop
    // them, unless nothing else is left (then the parent publishes the same zero-area
    // geometry its leaves publish).
    let surface = |t: &[u32; 3]| t[0] != t[1] && t[1] != t[2] && t[0] != t[2];
    if merged.triangles.iter().any(surface) {
        let keep: Vec<bool> = merged.triangles.iter().map(surface).collect();
        let mut i = 0;
        merged.triangles.retain(|_| (keep[i], i += 1).0);
        let mut i = 0;
        merged.representative.retain(|_| (keep[i], i += 1).0);
    }
    merged
}

fn edge(a: Key, b: Key) -> (Key, Key) {
    (a.min(b), a.max(b))
}

/// Required segments and points (canonical keys) missing from `triangles`.
fn missing(
    triangles: &[[u32; 3]],
    keys: &[Key],
    segments: &HashSet<(Key, Key)>,
    points: &HashSet<Key>,
) -> (Vec<(Key, Key)>, Vec<Key>) {
    let mut present_edges = HashSet::with_capacity(triangles.len() * 3);
    let mut present_points = HashSet::with_capacity(triangles.len() * 3);
    for t in triangles {
        for k in 0..3 {
            let (a, b) = (keys[t[k] as usize], keys[t[(k + 1) % 3] as usize]);
            present_edges.insert(edge(a, b));
            present_points.insert(a);
        }
    }
    let mut edges: Vec<_> = segments
        .iter()
        .filter(|e| !present_edges.contains(*e))
        .copied()
        .collect();
    let mut lost: Vec<_> = points
        .iter()
        .filter(|v| !present_points.contains(*v))
        .copied()
        .collect();
    edges.sort_unstable();
    lost.sort_unstable();
    (edges, lost)
}

/// meshopt over a subset of the merged triangles, compacted to the vertices they use.
/// Returns triangles in merged vertex indices and the absolute error estimate.
fn simplify(
    merged: &Merged,
    faces: &[[u32; 3]],
    locked: &dyn Fn(u32) -> bool,
    target: usize,
) -> (Vec<[u32; 3]>, f64) {
    let mut local: HashMap<u32, u32> = HashMap::with_capacity(faces.len() * 2);
    let mut global = Vec::new();
    let mut indices = Vec::with_capacity(faces.len() * 3);
    for f in faces {
        for &v in f {
            let next = global.len() as u32;
            let id = *local.entry(v).or_insert(next);
            if id == next {
                global.push(v);
            }
            indices.push(id);
        }
    }
    let positions: Vec<[f32; 3]> = global
        .iter()
        .map(|&v| merged.positions[v as usize])
        .collect();
    let locks: Vec<u8> = global.iter().map(|&v| u8::from(locked(v))).collect();
    let mut destination = vec![0u32; indices.len()];
    let mut error = 0f32;
    // SAFETY: every pointer covers the lengths passed with it: `indices` and `destination`
    // hold `indices.len()` u32 (meshopt's worst-case output is the input count),
    // `positions` holds `global.len()` tightly packed f32x3 (stride 12), `locks` one byte per
    // vertex, and no attributes are passed. Indices are all below `global.len()`.
    let count = unsafe {
        meshopt::ffi::meshopt_simplifyWithAttributes(
            destination.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            positions.as_ptr().cast::<f32>(),
            positions.len(),
            12,
            std::ptr::null(),
            0,
            std::ptr::null(),
            0,
            locks.as_ptr(),
            target.saturating_mul(3),
            f32::MAX,
            meshopt::SimplifyOptions::ErrorAbsolute.bits(),
            &mut error,
        )
    };
    destination.truncate(count);
    let triangles = destination
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| t.map(|v| global[v as usize]))
        .collect();
    (triangles, f64::from(error))
}

/// A triangle rotated to start at its least vertex: equal for the same oriented face.
fn oriented(t: &[u32; 3]) -> [u32; 3] {
    let k = (0..3).min_by_key(|&k| t[k]).unwrap();
    [t[k], t[(k + 1) % 3], t[(k + 2) % 3]]
}
fn reversed(t: &[u32; 3]) -> [u32; 3] {
    oriented(&[t[0], t[2], t[1]])
}

/// Keep every output face on its source orientation. Collapsing an edge across a fold (two
/// sheets of the surface back to back) can land a face exactly on an opposite-facing input
/// face: its own normal did not flip (meshopt's check), but it now duplicates that face's
/// corners in reverse. Such a face, the exact reverse of an input face and not itself one,
/// is dropped where its source-facing twin is in the output (the same corners, edges and
/// area remain) and turned to the source's orientation otherwise. Returns the faces fixed.
fn orient(merged: &Merged, output: &mut Vec<[u32; 3]>) -> u32 {
    let input: HashSet<[u32; 3]> = merged.triangles.iter().map(oriented).collect();
    let present: HashSet<[u32; 3]> = output.iter().map(oriented).collect();
    let mut fixed = 0;
    output.retain_mut(|t| {
        let degenerate = t[0] == t[1] || t[1] == t[2] || t[0] == t[2];
        if degenerate || input.contains(&oriented(t)) || !input.contains(&reversed(t)) {
            return true;
        }
        fixed += 1;
        if present.contains(&reversed(t)) {
            return false;
        }
        t.swap(1, 2);
        true
    });
    fixed
}

/// For each output triangle, the representative of an input triangle at one of its corners.
/// Each merged vertex first picks its incident input triangle whose centroid is nearest the
/// vertex (ties: earlier triangle); each output triangle then takes, of its three corners'
/// picks, the one whose centroid is nearest its own (ties: earlier triangle). Every input
/// and output triangle is visited three times, whatever the valence: `O(input + output)`
/// (the visits are returned). Provisional until S2's source index selects the nearest source
/// primitive (D4).
fn representatives(merged: &Merged, output: &[[u32; 3]]) -> (Vec<SourcePrimitive>, u64) {
    let centroid = |t: &[u32; 3]| -> [f64; 3] {
        let p = t.map(|v| merged.positions[v as usize].map(f64::from));
        std::array::from_fn(|a| (p[0][a] + p[1][a] + p[2][a]) / 3.)
    };
    let d2 = |a: [f64; 3], b: [f64; 3]| (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>();
    let mut visits = 0u64;
    let mut pick = vec![(f64::INFINITY, u32::MAX); merged.positions.len()];
    for (i, t) in merged.triangles.iter().enumerate() {
        let c = centroid(t);
        for &v in t {
            visits += 1;
            let at = merged.positions[v as usize].map(f64::from);
            let candidate = (d2(c, at), i as u32);
            let best = &mut pick[v as usize];
            if candidate < *best {
                *best = candidate;
            }
        }
    }
    let reps = output
        .iter()
        .map(|t| {
            let c = centroid(t);
            let mut best = (f64::INFINITY, u32::MAX);
            for &v in t {
                visits += 1;
                let i = pick[v as usize].1;
                if i == u32::MAX {
                    continue;
                }
                let candidate = (d2(centroid(&merged.triangles[i as usize]), c), i);
                if candidate < best {
                    best = candidate;
                }
            }
            merged.representative[best.1 as usize]
        })
        .collect();
    (reps, visits)
}

/// Build `key`'s parent from its children (in slot order) under the first of `caps` (each a
/// triangle limit, tried in order) it can keep its interface within.
///
/// The interface is exact (shared points and segments only), so a parent that cannot keep
/// it within its last cap has a genuine limit (D1: its exact interface, or topology plain
/// simplification cannot reduce, D3). It returns `None`: its coarse tile is **omitted**,
/// never published and never refused, and so is every ancestor's (the walk propagates it),
/// so its capped descendants are the refinement frontier.
pub(crate) fn build_parent(
    tile: TileKey,
    children: &[TileGeometry],
    caps: &[usize],
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<Option<Built>, JobError> {
    check()?;
    let merged = merge(children);
    if merged.triangles.is_empty() {
        return Err(state(format!(
            "coarse tile f{}-d{} has no source triangle",
            tile.first, tile.depth
        )));
    }
    let children_error = children
        .iter()
        .map(|c| c.geometric_error)
        .fold(0., f64::max);
    // The parent's outer interface: children's rows whose leaf interval leaves the parent.
    let mut interface: Vec<InterfaceRow> = children
        .iter()
        .flat_map(|c| c.interface.iter())
        .filter(|r| r.outer(tile.leaf_begin, tile.leaf_end))
        .copied()
        .collect();
    super::interfaces::merge_rows(&mut interface);
    if caps.is_empty() {
        return Err(state("a parent build needs at least one cap"));
    }
    for &cap in caps {
        match attempt(tile, &merged, &interface, cap, children_error) {
            Err(e) if e.kind() == JobErrorKind::Unsupported => continue,
            built => return built.map(Some),
        }
    }
    Ok(None)
}

/// One build of the parent from `merged` children under `interface`; `unsupported` when
/// its interface cannot fit `cap`.
fn attempt(
    tile: TileKey,
    merged: &Merged,
    interface: &[InterfaceRow],
    cap: usize,
    children_error: f64,
) -> Result<Built, JobError> {
    let segments: HashSet<(Key, Key)> = interface
        .iter()
        .filter(|r| !r.is_vertex())
        .map(|r| (r.a, r.b))
        .collect();
    let vertices: HashSet<Key> = interface
        .iter()
        .filter(|r| r.is_vertex())
        .map(|r| r.a)
        .collect();
    // By induction every child kept its own outer interface, a superset of these.
    let (lost_edges, lost_points) = missing(&merged.triangles, &merged.keys, &segments, &vertices);
    if !lost_edges.is_empty() || !lost_points.is_empty() {
        return Err(state(format!(
            "children of f{}-d{} lost {} interface segments and {} interface vertices",
            tile.first,
            tile.depth,
            lost_edges.len(),
            lost_points.len()
        )));
    }
    let locked: Vec<bool> = merged.keys.iter().map(|k| vertices.contains(k)).collect();
    let input = merged.triangles.len();
    let mut protected = vec![false; input];
    let mut retries = 0;
    let mut reoriented = 0;
    let degenerate_only = merged
        .triangles
        .iter()
        .all(|t| t[0] == t[1] || t[1] == t[2] || t[0] == t[2]);
    let (output, simplifier_error, stopping) = if input <= cap {
        (merged.triangles.clone(), 0., StoppingBound::Input)
    } else if degenerate_only {
        (
            merged.triangles[..cap].to_vec(),
            0.,
            StoppingBound::TriangleBudget,
        )
    } else {
        let (mut output, mut error) =
            simplify(merged, &merged.triangles, &|v| locked[v as usize], cap);
        reoriented = orient(merged, &mut output);
        loop {
            let (edges, points) = missing(&output, &merged.keys, &segments, &vertices);
            if edges.is_empty() && points.is_empty() {
                break;
            }
            if retries == RETRIES {
                return Err(JobError::new(
                    JobErrorKind::Unsupported,
                    format!(
                        "coarse tile f{}-d{}: simplification still drops {} interface segments \
                         and {} interface vertices after {RETRIES} protected retries \
                         ({input} input triangles, {} protected, cap {cap})",
                        tile.first,
                        tile.depth,
                        edges.len(),
                        points.len(),
                        protected.iter().filter(|p| **p).count()
                    ),
                ));
            }
            retries += 1;
            let edges: HashSet<(Key, Key)> = edges.into_iter().collect();
            let points: HashSet<Key> = points.into_iter().collect();
            for (i, t) in merged.triangles.iter().enumerate() {
                let o = t.map(|v| merged.keys[v as usize]);
                if (0..3)
                    .any(|k| edges.contains(&edge(o[k], o[(k + 1) % 3])) || points.contains(&o[k]))
                {
                    protected[i] = true;
                }
            }
            let keep: Vec<[u32; 3]> = merged
                .triangles
                .iter()
                .zip(&protected)
                .filter(|(_, p)| **p)
                .map(|(t, _)| *t)
                .collect();
            let rest: Vec<[u32; 3]> = merged
                .triangles
                .iter()
                .zip(&protected)
                .filter(|(_, p)| !**p)
                .map(|(t, _)| *t)
                .collect();
            let kept: HashSet<u32> = keep.iter().flatten().copied().collect();
            let (mut reduced, e) = if rest.is_empty() {
                (Vec::new(), 0.)
            } else {
                simplify(
                    merged,
                    &rest,
                    &|v| locked[v as usize] || kept.contains(&v),
                    cap.saturating_sub(keep.len()),
                )
            };
            reduced.extend(keep);
            output = reduced;
            reoriented = orient(merged, &mut output);
            error = e;
        }
        if output.len() > cap {
            return Err(JobError::new(
                JobErrorKind::Unsupported,
                format!(
                    "coarse tile f{}-d{}: its locked outer interface ({} segments, {} vertices) \
                     and {} protected faces cannot be simplified to the {cap}-triangle cap \
                     ({input} input triangles, {} after simplification)",
                    tile.first,
                    tile.depth,
                    segments.len(),
                    vertices.len(),
                    protected.iter().filter(|p| **p).count(),
                    output.len()
                ),
            ));
        } else {
            (output, error, StoppingBound::TriangleBudget)
        }
    };
    if output.is_empty() {
        return Err(JobError::new(
            JobErrorKind::Unsupported,
            format!(
                "coarse tile f{}-d{} simplified to no triangles",
                tile.first, tile.depth
            ),
        ));
    }
    if !simplifier_error.is_finite() {
        return Err(state("meshopt reported a nonfinite error"));
    }
    let (reps, representative_visits) = representatives(merged, &output);
    // Compact to the vertices the output uses, in first-use order.
    let mut remap = vec![u32::MAX; merged.keys.len()];
    let mut geometry = TileGeometry {
        key: tile,
        ells: Vec::new(),
        positions: Vec::new(),
        triangles: Vec::with_capacity(output.len()),
        representative: Vec::with_capacity(output.len()),
        primitives: Vec::new(),
        interface: interface.to_vec(),
        geometric_error: 0.,
        lease: None,
    };
    let mut out_locked = Vec::new();
    for t in &output {
        geometry.triangles.push(t.map(|v| {
            let slot = &mut remap[v as usize];
            if *slot == u32::MAX {
                *slot = geometry.positions.len() as u32;
                geometry.ells.push(merged.ells[v as usize]);
                geometry.positions.push(merged.positions[v as usize]);
                out_locked.push(locked[v as usize]);
            }
            *slot
        }));
    }
    let mut table: Vec<SourcePrimitive> = reps.clone();
    table.sort_unstable();
    table.dedup();
    geometry.representative = reps
        .iter()
        .map(|r| table.binary_search(r).unwrap() as u32)
        .collect();
    geometry.primitives = table;
    geometry.interface.shrink_to_fit();
    geometry.ells.shrink_to_fit();
    geometry.positions.shrink_to_fit();
    geometry.triangles.shrink_to_fit();
    geometry.representative.shrink_to_fit();
    geometry.primitives.shrink_to_fit();
    Ok(Built {
        locked: out_locked,
        stopping,
        simplifier_error,
        children_error,
        input_triangles: input as u32,
        protected_faces: protected.iter().filter(|p| **p).count() as u32,
        retries,
        interface_vertices: vertices.len() as u32,
        interface_segments: segments.len() as u32,
        reoriented,
        representative_visits,
        geometry,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merged(triangles: Vec<[u32; 3]>) -> Merged {
        Merged {
            keys: Vec::new(),
            ells: Vec::new(),
            positions: Vec::new(),
            triangles,
            representative: Vec::new(),
        }
    }

    /// Sol round 7: a locked fan of `m` faces about one corner (every face kept) once cost
    /// `m * m` visits picking representatives. Each vertex now picks once and each output
    /// face looks at its three corners' picks: exactly three visits per input and per output
    /// face.
    #[test]
    fn representative_selection_is_linear_in_a_locked_fan() {
        for m in [1024u32, 2048, 4096, 8192, 16384] {
            let mut positions = vec![[0f32; 3]];
            let mut triangles = Vec::new();
            for i in 0..=m {
                let a = f64::from(i) / f64::from(m) * std::f64::consts::PI;
                positions.push([a.cos() as f32, a.sin() as f32, 0.]);
            }
            for i in 0..m {
                triangles.push([0, i + 1, i + 2]);
            }
            let representative: Vec<SourcePrimitive> = (0..m)
                .map(|i| SourcePrimitive {
                    node: 0,
                    mesh: 0,
                    primitive: 0,
                    expanded_ordinal: u64::from(i),
                    material: u32::MAX,
                })
                .collect();
            let merged = Merged {
                keys: positions.iter().map(|p| key(*p)).collect(),
                ells: vec![0; positions.len()],
                positions,
                triangles: triangles.clone(),
                representative,
            };
            let (reps, visits) = representatives(&merged, &triangles);
            assert_eq!(visits, 6 * u64::from(m), "fan {m}");
            assert_eq!(reps.len(), m as usize);
        }
    }

    /// S5 round 5: a face collapsed onto an opposite-facing input face is dropped beside its
    /// source-facing twin, turned where it stands alone, and left alone where the input holds
    /// both orientations (back-to-back coincident sheets) or it reverses no input face.
    #[test]
    fn simplified_faces_keep_their_source_orientation() {
        let input = merged(vec![[0, 1, 2], [2, 1, 3]]);
        let mut beside = vec![[1, 2, 0], [0, 2, 1], [1, 3, 4]];
        assert_eq!(orient(&input, &mut beside), 1);
        assert_eq!(beside, vec![[1, 2, 0], [1, 3, 4]]);
        let mut alone = vec![[2, 1, 0], [1, 3, 4]];
        assert_eq!(orient(&input, &mut alone), 1);
        assert_eq!(alone, vec![[2, 0, 1], [1, 3, 4]]);
        let both = merged(vec![[0, 1, 2], [0, 2, 1]]);
        let mut kept = vec![[0, 2, 1], [1, 2, 0]];
        assert_eq!(orient(&both, &mut kept), 0);
        assert_eq!(kept, vec![[0, 2, 1], [1, 2, 0]]);
    }
}
