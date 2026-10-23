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

/// For each output triangle, the representative of the input triangle sharing one of its
/// corners whose centroid is nearest its own (ties: earlier input triangle). Provisional
/// until S2's source index selects the nearest source primitive (D4).
fn representatives(merged: &Merged, output: &[[u32; 3]]) -> Vec<SourcePrimitive> {
    let n = merged.positions.len();
    let mut start = vec![0u32; n + 1];
    for t in &merged.triangles {
        for &v in t {
            start[v as usize + 1] += 1;
        }
    }
    for i in 0..n {
        start[i + 1] += start[i];
    }
    let mut fill = start.clone();
    let mut incident = vec![0u32; merged.triangles.len() * 3];
    for (i, t) in merged.triangles.iter().enumerate() {
        for &v in t {
            incident[fill[v as usize] as usize] = i as u32;
            fill[v as usize] += 1;
        }
    }
    let centroid = |t: &[u32; 3]| -> [f64; 3] {
        let p = t.map(|v| merged.positions[v as usize].map(f64::from));
        std::array::from_fn(|a| (p[0][a] + p[1][a] + p[2][a]) / 3.)
    };
    output
        .iter()
        .map(|t| {
            let c = centroid(t);
            let mut best = (f64::INFINITY, u32::MAX);
            for &v in t {
                for &i in &incident[start[v as usize] as usize..start[v as usize + 1] as usize] {
                    let d = centroid(&merged.triangles[i as usize]);
                    let d2 = (0..3).map(|a| (d[a] - c[a]).powi(2)).sum::<f64>();
                    if (d2, i) < best {
                        best = (d2, i);
                    }
                }
            }
            merged.representative[best.1 as usize]
        })
        .collect()
}

/// Build `key`'s parent from its children (in slot order) under a `cap`-triangle limit.
pub(crate) fn build_parent(
    tile: TileKey,
    children: &[TileGeometry],
    cap: usize,
) -> Result<Built, JobError> {
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
    interface.sort_unstable_by_key(InterfaceRow::key);
    interface.dedup_by_key(|r| r.key());
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
            simplify(&merged, &merged.triangles, &|v| locked[v as usize], cap);
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
                    &merged,
                    &rest,
                    &|v| locked[v as usize] || kept.contains(&v),
                    cap.saturating_sub(keep.len()),
                )
            };
            reduced.extend(keep);
            output = reduced;
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
        }
        (output, error, StoppingBound::TriangleBudget)
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
    let reps = representatives(&merged, &output);
    // Compact to the vertices the output uses, in first-use order.
    let mut remap = vec![u32::MAX; merged.keys.len()];
    let mut geometry = TileGeometry {
        key: tile,
        ells: Vec::new(),
        positions: Vec::new(),
        triangles: Vec::with_capacity(output.len()),
        representative: Vec::with_capacity(output.len()),
        primitives: Vec::new(),
        interface,
        geometric_error: 0.,
        lease: None,
        measurement: None,
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
        geometry,
    })
}
