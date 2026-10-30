//! D11: bounded parents from immediate children and a common, certified border contract.
//! Boundary reconstruction precedes meshopt reduction. Stalled interiors get bounded
//! topology cleanup and remeshing; only the reported capped frontier remains afterwards.
#[cfg(test)]
mod combined_controls;
mod constrained;
#[cfg(test)]
mod contraction_memory;
pub(crate) mod folds;
pub(crate) mod locations;
pub(crate) mod measurement;
mod sheets;
mod topology;
pub(crate) use topology::Receipt as TopologyReceipt;

use super::{
    borders::Contract,
    interfaces::{key, InterfaceRow, Key},
    store::{SourcePrimitive, StoppingBound, TileGeometry, TileKey},
};
use crate::{JobError, JobErrorKind};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

type BorderMap<'a> = dyn Fn(Key) -> Result<Option<(Key, Key, f64, u8)>, JobError> + 'a;

/// Protect-and-retry rounds after the first simplification.
pub(crate) const RETRIES: u32 = 3;
// Geometry proposals have no S2/audit write until accepted. This separate fixed
// allowance lets local carrier protection converge without multiplying measured
// hole-opening attempts or their scratch reservation.
const FOLD_RETRIES: u32 = 8;
// Ordinary and independent-sheet strategies share ten reduction passes. Reserve
// one for the second strategy; exhausting this allowance enters constructive
// remeshing, rather than supplying evidence of cap infeasibility.
const REDUCTION_PASSES: u32 = 10;

fn state(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.into())
}

/// Grow by distinct keys, never by the duplicated endpoint iterator's size hint.
fn interface_points(interface: &[InterfaceRow]) -> HashSet<Key> {
    let mut points = HashSet::default();
    for row in interface {
        points.insert(row.a);
        points.insert(row.b);
    }
    points
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
    /// Published by the last-resort build (`build_parent`).
    pub last_resort: bool,
    /// Proposed by the staged build or the last resort. Only an accepted one is audited:
    /// its rejected proposals revisit the earlier builds' strategies
    /// (`audit::topology_attempt`).
    pub staged: bool,
    /// The last-resort stage (1-3) that proposed it, 0 otherwise (`Resort::N1`..`N3`).
    pub last_resort_stage: u8,
    /// Border vertices N3 snapped from an A1 hand-over (`Contracted::stuck`).
    pub relief_stuck: u32,
    pub interface_vertices: u32,
    pub interface_segments: u32,
    /// Historical receipt field; proposals are rejected rather than reoriented.
    pub reoriented: u32,
    pub topology: topology::Receipt,
    pub change: topology::Change,
    pub controls: measurement::Controls,
}

pub(crate) struct Outcome {
    pub built: Option<Built>,
    pub contraction_failed: bool,
    /// All measured proposals, including rejected candidates and the accepted publication.
    pub attempted_topology: TopologyReceipt,
    pub construction_dropped: Vec<[[f32; 3]; 3]>,
    /// A border contraction the primary build started but could not finish (A1 border
    /// relief); the last resort's N3 continues from it (`Contracted`).
    pub relief: Option<Contracted>,
}

/// A border contraction handed to the last resort's N3 (A1 border relief): the merged
/// children with every accepted collapse applied (vertex indexing, keys, positions and ells
/// of `merge(children)`, unchanged) and the border vertices still to remove, each onto its
/// chord. N3 snaps only those (`snap_stuck`) and neither re-merges nor re-contracts.
pub(super) struct Contracted {
    pub merged: Merged,
    /// Sorted by vertex.
    pub stuck: Vec<Stuck>,
}

/// A relieved border vertex left on its chord: its index in `Contracted::merged`, its key,
/// the chord's retained endpoints in source-arc order and its parameter along it (t <= 0.5
/// is nearer `chord[0]`).
pub(super) struct Stuck {
    pub vertex: u32,
    pub key: Key,
    pub chord: [Key; 2],
    pub t: f64,
}

/// N3 on a handed-over contraction: each stuck vertex snaps to its nearer chord endpoint
/// (no voting: a pure function of the hand-over), degenerate faces go, and the result must
/// keep every interface item, leave the topology unchanged and pass `guard`: the merged
/// children's contraction licences under N3's policy (folds admitted, a new face or edge on
/// shared points only in their seam owner). None refuses (the cell is omitted).
fn snap_stuck(
    reference: &Merged,
    seed: &Contracted,
    interface: &[InterfaceRow],
    guard: &folds::Source,
) -> Option<Merged> {
    let m = &seed.merged;
    if m.keys != reference.keys {
        return None;
    }
    let ids: HashMap<Key, u32> = m
        .keys
        .iter()
        .enumerate()
        .map(|(i, k)| (*k, i as u32))
        .collect();
    let mut map: Vec<u32> = (0..m.keys.len() as u32).collect();
    for s in &seed.stuck {
        if m.keys.get(s.vertex as usize) != Some(&s.key) {
            return None;
        }
        map[s.vertex as usize] = *ids.get(&s.chord[usize::from(s.t > 0.5)])?;
    }
    let triangles: Vec<[u32; 3]> = m
        .triangles
        .iter()
        .map(|t| t.map(|v| map[v as usize]))
        .filter(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2])
        .collect();
    let segments = interface
        .iter()
        .filter(|r| !r.is_vertex())
        .map(|r| (r.a, r.b))
        .collect();
    let (edges, points) = missing_owned(&triangles, &m.keys, segments, interface_points(interface));
    if !edges.is_empty() || !points.is_empty() || triangles.is_empty() {
        return None;
    }
    topology::changes(reference, &triangles, interface, 0.)?;
    if guard.violations(&m.positions, &triangles).contains(&true) {
        return None;
    }
    Some(Merged {
        representative: vec![reference.representative[0]; triangles.len()],
        triangles,
        ..m.clone()
    })
}

/// Children merged into one vertex space welded by canonical decoded position, in child
/// (slot) order.
#[derive(Clone)]
pub(super) struct Merged {
    keys: Vec<Key>,
    ells: Vec<u8>,
    positions: Vec<[f32; 3]>,
    triangles: Vec<[u32; 3]>,
    representative: Vec<SourcePrimitive>,
}

fn merge(children: &[TileGeometry]) -> Merged {
    let _cpu = super::cpu_profile::start(
        super::cpu_profile::Stage::Merge,
        children.iter().map(|c| c.triangles.len()).sum(),
    );
    let triangles: usize = children.iter().map(|c| c.triangles.len()).sum();
    let mut index: HashMap<Key, u32> =
        HashMap::with_capacity_and_hasher(triangles, Default::default());
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
    missing_owned(triangles, keys, segments.clone(), points.clone())
}

/// Consume temporary queries instead of retaining a second copy during the scan.
fn missing_owned(
    triangles: &[[u32; 3]],
    keys: &[Key],
    missing_edges: HashSet<(Key, Key)>,
    missing_points: HashSet<Key>,
) -> (Vec<(Key, Key)>, Vec<Key>) {
    missing_among(triangles, keys, missing_edges, missing_points, &|_| true)
}

/// Vertices whose Key is a required point or a required segment's endpoint: only
/// their corners and edges can match a query, so the scan skips every other probe.
fn required_corners(
    keys: &[Key],
    segments: &HashSet<(Key, Key)>,
    points: &HashSet<Key>,
) -> Vec<bool> {
    let ends: HashSet<Key> = segments.iter().flat_map(|&(a, b)| [a, b]).collect();
    keys.iter()
        .map(|k| ends.contains(k) || points.contains(k))
        .collect()
}

/// [`missing_owned`], probing only corners for which `relevant` holds. Exact when
/// `relevant` covers every required point and both endpoints of every segment.
fn missing_among(
    triangles: &[[u32; 3]],
    keys: &[Key],
    mut missing_edges: HashSet<(Key, Key)>,
    mut missing_points: HashSet<Key>,
    relevant: &dyn Fn(u32) -> bool,
) -> (Vec<(Key, Key)>, Vec<Key>) {
    for t in triangles {
        for k in 0..3 {
            let (u, w) = (t[k], t[(k + 1) % 3]);
            if !relevant(u) {
                continue;
            }
            let a = keys[u as usize];
            if relevant(w) {
                missing_edges.remove(&edge(a, keys[w as usize]));
            }
            missing_points.remove(&a);
        }
        if missing_edges.is_empty() && missing_points.is_empty() {
            return (Vec::new(), Vec::new());
        }
    }
    let mut edges: Vec<_> = missing_edges.into_iter().collect();
    let mut lost: Vec<_> = missing_points.into_iter().collect();
    edges.sort_unstable();
    lost.sort_unstable();
    (edges, lost)
}

// Staged reduction (`Resort::Staged`), fixed bounds: steps, tries of a refused step, its
// least size, carrier-merge rounds and passes.
const STAGES: u32 = 120;
const STAGE_TRIES: u32 = 10;
const MIN_STEP: usize = 4;
const MERGE_ROUNDS: u32 = 4;
const MERGE_PASSES: u32 = 32;

/// Reduce toward the cap in steps from the current mesh (the first a third of the
/// excess). The unchanged guard, the interface items and component survival judge each
/// step; a refused step keeps its own faces around the flagged vertices and is redone,
/// then halves. At a plateau border carriers merge, then the steps resume. The error is
/// the largest step's meshopt estimate against its previous step (merges add none): a
/// lower bound, as S2 measures the published G.
fn staged_reduce(
    merged: &Merged,
    source: &[[u32; 3]],
    interface: &[InterfaceRow],
    locked: &[bool],
    cap: usize,
    policy: &folds::Policy,
) -> (Vec<[u32; 3]>, f64) {
    let guard = folds::Source::new(&merged.positions, source)
        .with_boundary(interface)
        .with_policy(policy.clone());
    let segments: HashSet<(Key, Key)> = interface
        .iter()
        .filter(|r| !r.is_vertex())
        .map(|r| (r.a, r.b))
        .collect();
    let points = interface_points(interface);
    let (mut current, mut error) = (merged.triangles.clone(), 0f64);
    for _ in 0..MERGE_ROUNDS {
        let mut step = (current.len().saturating_sub(cap) / 3).max(1);
        for _ in 0..STAGES {
            if current.len() <= cap {
                break;
            }
            step = step.min(current.len() - cap);
            let target = current.len() - step;
            let component = topology::components(&Merged {
                triangles: current.clone(),
                ..merged.clone()
            });
            let mut seen = HashSet::default();
            let mut keep: Vec<bool> = current
                .iter()
                .map(|t| seen.insert(component[t[0] as usize]))
                .collect();
            let mut done = false;
            for _ in 0..STAGE_TRIES {
                let (kept, rest): (Vec<_>, Vec<_>) =
                    current.iter().zip(&keep).partition(|(_, k)| **k);
                let kept: Vec<[u32; 3]> = kept.into_iter().map(|(t, _)| *t).collect();
                let fixed: HashSet<u32> = kept.iter().flatten().copied().collect();
                let rest: Vec<[u32; 3]> = rest.into_iter().map(|(t, _)| *t).collect();
                let (mut out, e) = simplify(
                    merged,
                    &rest,
                    &|v| locked[v as usize] || fixed.contains(&v),
                    target.saturating_sub(kept.len()),
                );
                out.extend(kept);
                let mut bad = guard.violations(&merged.positions, &out);
                let (edges, lost) = missing(&out, &merged.keys, &segments, &points);
                for (v, k) in merged.keys.iter().enumerate() {
                    bad[v] |= lost.contains(k) || edges.iter().any(|(a, b)| a == k || b == k);
                }
                if !bad.contains(&true) {
                    if out.len() < current.len() {
                        (current, error, done) = (out, error.max(e), true);
                    }
                    break;
                }
                let before = keep.iter().filter(|k| **k).count();
                for (k, t) in keep.iter_mut().zip(&current) {
                    *k |= t.iter().any(|&v| bad[v as usize]);
                }
                if keep.iter().filter(|k| **k).count() == before {
                    break;
                }
            }
            if !done {
                if step <= MIN_STEP {
                    break;
                }
                step /= 2;
            }
        }
        if current.len() <= cap
            || !carrier_merge(merged, &guard, locked, &points, &mut current, cap)
        {
            break;
        }
    }
    (current, error)
}

/// N2/N3: faces the source holds only the other way round are turned; N2 also drops
/// coincident copies beyond the source's count (`folds::Source::settle`).
fn settle(
    merged: &Merged,
    source: &[[u32; 3]],
    policy: &folds::Policy,
    resort: Resort,
    out: &mut Vec<[u32; 3]>,
) {
    if policy.folds {
        folds::Source::new(&merged.positions, source).settle(
            &merged.positions,
            out,
            resort != Resort::N3,
        );
    }
}

// N3 clustering bounds: cell-size doublings from the shortest edge, then bisections
// toward the finest cell that fits.
const CLUSTER_DOUBLINGS: u32 = 64;
const CLUSTER_BISECTIONS: u32 = 12;

/// N3 (`Resort::N3`): vertex clustering. Every locked vertex (interface points, opening
/// pins, open boundaries touching the interface) is its own cluster, so a face on a contract edge keeps that edge with its third
/// corner clustered, and no edge between two locked vertices appears that was not there.
/// Unlocked vertices merge per input component and grid cell (no face spans two
/// components, as topology accounting requires) onto the member nearest the cell's mean
/// (least index on ties); degenerate faces go and the rest settle (`settle`). The cell starts at
/// the shortest edge, doubles until the output fits, then bisects toward the finest fit.
/// The error is the largest move onto a representative.
fn cluster_reduce(
    merged: &Merged,
    source: &[[u32; 3]],
    interface: &[InterfaceRow],
    locked: &[bool],
    cap: usize,
    policy: &folds::Policy,
) -> (Vec<[u32; 3]>, f64) {
    let guard = folds::Source::new(&merged.positions, source)
        .with_boundary(interface)
        .with_policy(policy.clone());
    let faces = &merged.triangles;
    let component = topology::components(merged);
    let touching = topology::touching_boundary(merged, interface);
    let locked: Vec<bool> = locked
        .iter()
        .zip(&touching)
        .map(|(a, b)| *a || *b)
        .collect();
    let p = |v: u32| merged.positions[v as usize].map(f64::from);
    let mut free: Vec<u32> = faces
        .iter()
        .flatten()
        .copied()
        .filter(|&v| !locked[v as usize])
        .collect();
    free.sort_unstable();
    free.dedup();
    if free.is_empty() {
        return (faces.clone(), 0.);
    }
    let mut lo = [f64::INFINITY; 3];
    for &v in &free {
        for (l, x) in lo.iter_mut().zip(p(v)) {
            *l = l.min(x);
        }
    }
    let shortest = faces
        .iter()
        .flat_map(|t| (0..3).map(move |j| (t[j], t[(j + 1) % 3])))
        .map(|(a, b)| distance(p(a), p(b)))
        .filter(|d| *d > 0.)
        .fold(f64::INFINITY, f64::min);
    let run = |h: f64| -> (Vec<[u32; 3]>, f64) {
        let cell = |v: u32| {
            let q = p(v);
            let c = [0, 1, 2].map(|i| ((q[i] - lo[i]) / h).floor() as i64);
            (component[v as usize], c)
        };
        // Per (component, grid cell): coordinate sum, member count and members.
        type Cell = ([f64; 3], u32, Vec<u32>);
        let mut cells: HashMap<(usize, [i64; 3]), Cell> = HashMap::default();
        for &v in &free {
            let c = cells.entry(cell(v)).or_insert(([0.; 3], 0, Vec::new()));
            for (s, x) in c.0.iter_mut().zip(p(v)) {
                *s += x;
            }
            c.1 += 1;
            c.2.push(v);
        }
        let mut map: Vec<u32> = (0..merged.positions.len() as u32).collect();
        let mut error = 0f64;
        for (sum, n, members) in cells.values() {
            let mean = sum.map(|x| x / *n as f64);
            let rep = *members
                .iter()
                .min_by(|a, b| {
                    distance(p(**a), mean)
                        .total_cmp(&distance(p(**b), mean))
                        .then(a.cmp(b))
                })
                .unwrap();
            for &v in members {
                map[v as usize] = rep;
                error = error.max(distance(p(v), p(rep)));
            }
        }
        let mut out: Vec<[u32; 3]> = faces
            .iter()
            .map(|t| t.map(|v| map[v as usize]))
            .filter(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2])
            .collect();
        guard.settle(&merged.positions, &mut out, false);
        (out, error)
    };
    let (mut fine, mut h) = (shortest, shortest);
    let mut best = run(h);
    if best.0.len() <= cap || !h.is_finite() {
        return best;
    }
    let mut fit = None;
    for _ in 0..CLUSTER_DOUBLINGS {
        h *= 2.;
        best = run(h);
        if best.0.len() <= cap {
            fit = Some(h);
            break;
        }
        fine = h;
    }
    if let Some(mut coarse) = fit {
        for _ in 0..CLUSTER_BISECTIONS {
            let mid = (fine + coarse) / 2.;
            let r = run(mid);
            if r.0.len() <= cap {
                (coarse, best) = (mid, r);
            } else {
                fine = mid;
            }
        }
    }
    best
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>().sqrt()
}

/// Merge border carriers (unlocked vertices next to an interface point, off free edges)
/// into an adjacent carrier, shortest edge first, never onto an interface point. A
/// collapse keeps each moved face's orientation and area and every other vertex's last
/// face, and passes the guard on the affected stars (exact: they hold every face on a
/// changed edge). A pass's collapses have disjoint stars; a pass the whole-mesh guard
/// refuses is dropped. Whether any merged.
fn carrier_merge(
    merged: &Merged,
    guard: &folds::Source,
    locked: &[bool],
    points: &HashSet<Key>,
    faces: &mut Vec<[u32; 3]>,
    cap: usize,
) -> bool {
    let p = &merged.positions;
    let border = |v: u32| points.contains(&merged.keys[v as usize]);
    let mut any = false;
    for _ in 0..MERGE_PASSES {
        if faces.len() <= cap {
            break;
        }
        let mut star: HashMap<u32, Vec<usize>> = HashMap::default();
        let mut uses: HashMap<(u32, u32), u32> = HashMap::default();
        for (i, t) in faces.iter().enumerate() {
            for k in 0..3 {
                star.entry(t[k]).or_default().push(i);
                let (a, b) = (t[k], t[(k + 1) % 3]);
                *uses.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        let free: HashSet<u32> = uses
            .iter()
            .filter(|(_, n)| **n != 2)
            .flat_map(|(e, _)| [e.0, e.1])
            .collect();
        let carrier = |v: u32| {
            !locked[v as usize]
                && !border(v)
                && !free.contains(&v)
                && star[&v]
                    .iter()
                    .any(|&f| faces[f].iter().any(|&u| border(u)))
        };
        let mut candidates: Vec<(f64, u32, u32)> = uses
            .keys()
            .filter(|&&(a, b)| carrier(a) && carrier(b))
            .map(|&(a, b)| {
                let d = (0..3)
                    .map(|i| (f64::from(p[a as usize][i]) - f64::from(p[b as usize][i])).powi(2))
                    .sum();
                (d, a, b)
            })
            .collect();
        candidates.sort_by(|x, y| x.0.total_cmp(&y.0).then((x.1, x.2).cmp(&(y.1, y.2))));
        let (mut next, mut dead) = (faces.clone(), vec![false; faces.len()]);
        let (mut alive, mut used, mut done) = (faces.len(), HashSet::default(), 0);
        for &(_, a, b) in &candidates {
            if alive <= cap {
                break;
            }
            if used.contains(&a) || used.contains(&b) {
                continue;
            }
            for (from, to) in [(a, b), (b, a)] {
                let change: Option<HashMap<usize, Option<[u32; 3]>>> = star[&from]
                    .iter()
                    .map(|&f| {
                        let t = faces[f];
                        if t.contains(&to) {
                            return Some((f, None));
                        }
                        let n = t.map(|v| if v == from { to } else { v });
                        let (o, q) = (topology::normal(p, t), topology::normal(p, n));
                        let kept = q.iter().any(|&x| x != 0.)
                            && (0..3).map(|i| o[i] * q[i]).sum::<f64>() > 0.;
                        kept.then_some((f, Some(n)))
                    })
                    .collect();
                let Some(change) = change else { continue };
                let orphan = change.iter().filter(|(_, n)| n.is_none()).any(|(f, _)| {
                    faces[*f].iter().any(|&v| {
                        v != from
                            && star[&v]
                                .iter()
                                .all(|g| change.get(g).is_some_and(Option::is_none))
                    })
                });
                let mut ids: HashSet<usize> = star[&to].iter().copied().collect();
                for v in change.values().flatten().flatten() {
                    ids.extend(star[v].iter().copied());
                }
                let local: Vec<[u32; 3]> = ids
                    .iter()
                    .filter_map(|f| change.get(f).copied().unwrap_or(Some(faces[*f])))
                    .collect();
                if orphan || guard.rejects(p, &local) {
                    continue;
                }
                for &f in star[&from].iter().chain(&star[&to]) {
                    used.extend(faces[f]);
                }
                for (f, n) in change {
                    match n {
                        Some(n) => next[f] = n,
                        None => (dead[f], alive) = (true, alive - 1),
                    }
                }
                done += 1;
                break;
            }
        }
        let out: Vec<[u32; 3]> = next
            .into_iter()
            .zip(&dead)
            .filter(|(_, d)| !**d)
            .map(|(t, _)| t)
            .collect();
        if done == 0 || guard.violations(p, &out).contains(&true) {
            break;
        }
        (*faces, any) = (out, true);
    }
    any
}

/// meshopt over a subset of the merged triangles, compacted to the vertices they use.
/// Returns triangles in merged vertex indices and the absolute error estimate.
fn simplify(
    merged: &Merged,
    faces: &[[u32; 3]],
    locked: &dyn Fn(u32) -> bool,
    target: usize,
) -> (Vec<[u32; 3]>, f64) {
    let mut local = vec![u32::MAX; merged.positions.len()];
    simplify_mapped(merged, faces, locked, target, &mut local)
}

fn simplify_mapped(
    merged: &Merged,
    faces: &[[u32; 3]],
    locked: &dyn Fn(u32) -> bool,
    target: usize,
    local: &mut [u32],
) -> (Vec<[u32; 3]>, f64) {
    let _cpu = super::cpu_profile::start(super::cpu_profile::Stage::Simplify, faces.len());
    // In pinned meshopt 0.6.2, a satisfied target or all locked vertices leaves
    // the copied index buffer untouched: no collapse, pruning or solve runs for
    // our ErrorAbsolute-only options. Avoid allocating its quadrics/adjacency
    // for these common tiny sheets. A finite extent is needed for its final
    // sqrt(0) * extent to equal zero; unusual coordinates retain the FFI path.
    if !faces.is_empty()
        && (target == faces.len() || faces.iter().flatten().all(|&v| locked(v)))
        && finite_extent(&merged.positions, faces)
    {
        let _fixed =
            super::cpu_profile::start(super::cpu_profile::Stage::FixedSimplify, faces.len());
        return (faces.to_vec(), 0.);
    }
    // Merged indices are dense and bounded to eight capped children. Preserve first-use
    // order without hashing each corner, including when simplifying a protected subset.
    let mut global = Vec::new();
    let mut indices = Vec::with_capacity(faces.len() * 3);
    for f in faces {
        for &v in f {
            let next = global.len() as u32;
            let id = &mut local[v as usize];
            if *id == u32::MAX {
                *id = next;
                global.push(v);
            }
            indices.push(*id);
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
    for &v in &global {
        local[v as usize] = u32::MAX;
    }
    (triangles, f64::from(error))
}

fn finite_extent(positions: &[[f32; 3]], faces: &[[u32; 3]]) -> bool {
    let mut lo = [f32::MAX; 3];
    let mut hi = [-f32::MAX; 3];
    for &v in faces.iter().flatten() {
        for (i, x) in positions[v as usize].into_iter().enumerate() {
            if !x.is_finite() {
                return false;
            }
            lo[i] = lo[i].min(x);
            hi[i] = hi[i].max(x);
        }
    }
    (0..3).all(|i| (hi[i] - lo[i]).is_finite())
}

/// A triangle rotated to start at its least vertex: equal for the same oriented face.
fn oriented(t: &[u32; 3]) -> [u32; 3] {
    let k = (0..3).min_by_key(|&k| t[k]).unwrap();
    [t[k], t[(k + 1) % 3], t[(k + 2) % 3]]
}
/// Reconstruct the common contracted boundary before ordinary reduction. Only capped
/// immediate children enter this job. After a stalled reduction, explicitly remesh the
/// interior; omission is the reported last resort and propagates upwards in the walk.
/// Only when that fails, and with `last_resorts` (production: always), the whole build
/// runs again with fix 2's last resorts, then with staged reduction (`Resort`), so no tile
/// an earlier build publishes changes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_parent(
    tile: TileKey,
    children: &[TileGeometry],
    caps: &[usize],
    contract: &Contract,
    mapping: &BorderMap<'_>,
    assess: &mut dyn FnMut(&mut Built) -> Result<bool, JobError>,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
    last_resorts: bool,
) -> Result<Outcome, JobError> {
    let mut run = |r, seed| {
        build(
            tile, children, caps, contract, mapping, assess, check, r, seed,
        )
    };
    let mut first = run(Resort::None, None)?;
    if first.built.is_some() || !last_resorts {
        return Ok(first);
    }
    // A1 hand-over from the primary contraction, kept for N3 (`Contracted`).
    let relief = first.relief.take();
    let mut last = run(Resort::Fix2, None)?;
    last.attempted_topology.add(first.attempted_topology);
    for resort in [Resort::Staged, Resort::N1, Resort::N2, Resort::N3] {
        if last.built.is_some() {
            break;
        }
        let mut next = run(resort, relief.as_ref())?;
        next.attempted_topology.add(last.attempted_topology);
        if let (Some(b), Some(relief)) = (&mut next.built, &relief) {
            if resort == Resort::N3 {
                b.relief_stuck = relief.stuck.len() as u32;
            }
        }
        last = next;
    }
    if let Some(b) = &mut last.built {
        b.last_resort = true;
    }
    Ok(last)
}

/// A build's last resorts. `Fix2`: contraction rebuilds also flip interface diagonals and
/// join slivers (`constrained::contract`), an attempt also pins touching paths (`attempt`),
/// and coarse drops and wall panels follow every strategy. `Staged`: develop's strategies,
/// each reduction staged against the fold guard (`staged`). `N1`-`N3` (D1/D11 amendment
/// A2) replace omission: develop's first strategies only, staged, with the guard relaxed
/// by `folds::Policy`: N1 admits new shared-point faces and edges in their seam owner,
/// N2 also admits folds, N3 also reduces by clustering with every locked vertex fixed
/// (`cluster_reduce`).
#[derive(Clone, Copy, PartialEq)]
enum Resort {
    None,
    Fix2,
    Staged,
    N1,
    N2,
    N3,
}

impl Resort {
    /// The last-resort stage, 0 before it.
    fn stage(self) -> u8 {
        match self {
            Self::N1 => 1,
            Self::N2 => 2,
            Self::N3 => 3,
            _ => 0,
        }
    }
    /// Whether the first reduction is staged or clustered (without contact pins).
    fn staged(self) -> bool {
        matches!(self, Self::Staged | Self::N1 | Self::N2 | Self::N3)
    }
}

/// One build with the given last resorts.
#[allow(clippy::too_many_arguments)]
fn build(
    tile: TileKey,
    children: &[TileGeometry],
    caps: &[usize],
    contract: &Contract,
    mapping: &BorderMap<'_>,
    assess: &mut dyn FnMut(&mut Built) -> Result<bool, JobError>,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
    resort: Resort,
    seed: Option<&Contracted>,
) -> Result<Outcome, JobError> {
    let fix2 = resort == Resort::Fix2;
    check()?;
    if caps.is_empty() {
        return Err(state("a parent build needs at least one cap"));
    }
    let mut merged = merge(children);
    let reference = merged.clone();
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
    let interface = &contract.interface;
    let policy =
        folds::Policy::last_resort(resort.stage(), tile.leaf_begin, tile.leaf_end, interface);
    let n3 = (resort == Resort::N3).then_some(&policy);
    let mut topology = topology::Receipt::default();
    let mut attempted = topology::Receipt::default();
    // Coarsening only merges owners: every live parent border is a child border.
    // Nested canonical endpoints therefore make the union of child interfaces a
    // complete lookup domain. Interior vertices need no disk-backed source search.
    let mut previous_interface =
        Vec::with_capacity(children.iter().map(|c| c.interface.len()).sum());
    for child in children {
        previous_interface.extend_from_slice(&child.interface);
    }
    super::interfaces::merge_rows(&mut previous_interface);
    previous_interface.shrink_to_fit();
    let child_border = interface_points(&previous_interface);
    let lookup = |k| {
        if child_border.contains(&k) {
            mapping(k)
        } else {
            Ok(None)
        }
    };
    if let Some(seed) = seed.filter(|_| resort == Resort::N3) {
        // A1 hand-over: continue the primary contraction, snapping only its stuck vertices,
        // licensed like any contraction (`folds::Source::for_contraction`).
        let mapped = reference
            .keys
            .iter()
            .map(|&k| lookup(k))
            .collect::<Result<Vec<_>, JobError>>()?;
        let guard = folds::Source::for_contraction(
            &reference.positions,
            &reference.triangles,
            interface,
            &previous_interface,
            &mapped,
        )
        .with_policy(policy.clone());
        let Some(snapped) = snap_stuck(&reference, seed, interface, &guard) else {
            return Ok(Outcome {
                built: None,
                contraction_failed: true,
                attempted_topology: attempted,
                construction_dropped: Vec::new(),
                relief: None,
            });
        };
        merged = snapped;
    } else {
        if !topology::contract(
            &mut merged,
            interface,
            &previous_interface,
            &lookup,
            &mut topology,
            fix2,
            n3,
        )? {
            attempted.add(topology);
            topology = Default::default();
            if !recover_contraction(
                &mut merged,
                &reference,
                interface,
                &previous_interface,
                &lookup,
                caps,
                &mut topology,
                check,
                fix2,
                n3,
            )? {
                attempted.add(topology);
                return Ok(Outcome {
                    built: None,
                    contraction_failed: true,
                    attempted_topology: attempted,
                    construction_dropped: Vec::new(),
                    relief: None,
                });
            }
        }
    }
    let contracted_faces = merged.triangles.clone();
    let contracted_representative = merged.representative.clone();
    let floor = contract.scale.max(children_error);
    // Components may go only under cap pressure: the contracted input exceeds the hard
    // cap (validators refuse a drop whose children fit it). Below it, a zero floor
    // licenses no drop, even for a smaller D11 target.
    let pressure = contracted_faces.len() > *caps.last().unwrap();
    // N3's clustering may drop or close any border-free component or hole: declared,
    // ordered smallest first and covered by S2's measured G like every drop.
    let drops = if resort == Resort::N3 {
        f64::INFINITY
    } else if pressure {
        floor
    } else {
        0.
    };
    let mut opening_pins = HashSet::default();
    for (i, &cap) in caps.iter().enumerate() {
        if caps[..i].contains(&cap) {
            continue;
        }
        for preserve_holes in [false, true] {
            if let Some(mut b) = assessed_attempt(
                tile,
                &merged,
                interface,
                cap,
                children_error,
                &reference,
                &contracted_faces,
                drops,
                preserve_holes,
                &mut opening_pins,
                resort,
                &mut attempted,
                assess,
                check,
            )? {
                attempted.add(topology);
                b.topology.add(topology);
                return Ok(Outcome {
                    built: Some(b),
                    contraction_failed: false,
                    attempted_topology: attempted,
                    construction_dropped: Vec::new(),
                    relief: None,
                });
            }
        }
    }
    // The last resort runs develop's first strategies only (fixed work per stage).
    if resort.stage() > 0 {
        attempted.add(topology);
        return Ok(Outcome {
            built: None,
            contraction_failed: false,
            attempted_topology: attempted,
            construction_dropped: Vec::new(),
            relief: None,
        });
    }
    // D11: simplifier stagnation alone is not cap infeasibility. Deduplicate and remove
    // eligible interior components, then remesh with increasingly coarse interior cells.
    let construction_dropped = topology::cleanup(&mut merged, interface, drops, &mut topology);
    if let Some(receipt) = constrained::remesh(&mut merged, interface, *caps.last().unwrap()) {
        topology.add(receipt);
    }
    for round in 0..=6 {
        check()?;
        if round != 0 && !topology::remesh(&mut merged, interface, round, &mut topology) {
            continue;
        }
        for preserve_holes in [false, true] {
            if let Some(mut b) = assessed_attempt(
                tile,
                &merged,
                interface,
                *caps.last().unwrap(),
                children_error,
                &reference,
                &contracted_faces,
                drops,
                preserve_holes,
                &mut opening_pins,
                resort,
                &mut attempted,
                assess,
                check,
            )? {
                // Final comparison already counts actual changes against the reference.
                topology.dropped_components = 0;
                topology.closed_holes = 0;
                topology.opened_holes = 0;
                attempted.add(topology);
                b.topology.add(topology);
                return Ok(Outcome {
                    built: Some(b),
                    contraction_failed: false,
                    attempted_topology: attempted,
                    construction_dropped: Vec::new(),
                    relief: None,
                });
            }
        }
    }
    // D11 coarse drops (owner decision 2026-10-28): only once every strategy above has
    // failed, and only for cap pressure: the contracted input exceeds the cap (validators
    // refuse a coarse drop whose children fit it). Whole components with no border
    // contact go smallest first: the least prefix that fits, found by doubling and then
    // bisection (at most 2 log2(C) + 2 proposals; each prefix licenses drops below its
    // last diagonal). S2's fixed controls on every dropped triangle carry their loss into
    // the published G.
    let mut base = merged.clone();
    base.triangles = contracted_faces.clone();
    base.representative = contracted_representative;
    let candidates = if pressure {
        coarse_candidates(&base, interface, floor)
    } else {
        Vec::new()
    };
    let n = candidates.len();
    let (mut failed, mut fitted) = (0, None);
    let mut next = (n > 0).then_some(1);
    while let Some(k) = next {
        check()?;
        let license = candidates[k - 1].diagonal.next_up();
        merged.triangles.clone_from(&base.triangles);
        merged.representative.clone_from(&base.representative);
        let mut receipt = topology::Receipt::default();
        topology::cleanup(&mut merged, interface, license, &mut receipt);
        if let Some(r) = constrained::remesh(&mut merged, interface, *caps.last().unwrap()) {
            receipt.add(r);
        }
        let mut built = None;
        for preserve_holes in [false, true] {
            built = assessed_attempt(
                tile,
                &merged,
                interface,
                *caps.last().unwrap(),
                children_error,
                &reference,
                &contracted_faces,
                license,
                preserve_holes,
                &mut opening_pins,
                resort,
                &mut attempted,
                assess,
                check,
            )?;
            if built.is_some() {
                break;
            }
        }
        // The final comparison counts every published change against the reference.
        receipt.dropped_components = 0;
        receipt.closed_holes = 0;
        receipt.opened_holes = 0;
        topology.add(receipt);
        match built {
            Some(b) => fitted = Some((k, b)),
            None => failed = k,
        }
        next = match &fitted {
            None if k < n => Some((2 * k).min(n)),
            Some((fit, _)) if fit - failed > 1 => Some(failed + (fit - failed) / 2),
            _ => None,
        };
    }
    // In the last-resort build, once every strategy above failed (a parent that fits in
    // either build never reaches this). Panels: every exactly coplanar chart rebuilt from its boundary
    // (exact geometry, fewer interior samples), then the remesh rounds. Then, under cap
    // pressure, coarse-drop prefixes followed by the same sequence: all candidates are
    // probed first, so a parent no drop can fit costs one sequence; if they fit, bisect
    // for the least prefix (at most log2(C) + 1 more sequences).
    if fitted.is_none() && fix2 {
        let mut sequence = |merged: &mut Merged,
                            license: f64,
                            rounds: u32,
                            receipt: &mut topology::Receipt,
                            attempted: &mut TopologyReceipt|
         -> Result<Option<(Built, u32)>, JobError> {
            merged.triangles.clone_from(&base.triangles);
            merged.representative.clone_from(&base.representative);
            topology::cleanup(merged, interface, license, receipt);
            if let Some(r) = constrained::remesh_panels(merged, interface) {
                receipt.add(r);
            }
            for round in 0..=rounds {
                check()?;
                if round != 0 && !topology::remesh(merged, interface, round, receipt) {
                    continue;
                }
                for preserve_holes in [false, true] {
                    if let Some(b) = assessed_attempt(
                        tile,
                        merged,
                        interface,
                        *caps.last().unwrap(),
                        children_error,
                        &reference,
                        &contracted_faces,
                        license,
                        preserve_holes,
                        &mut opening_pins,
                        Resort::Fix2,
                        attempted,
                        assess,
                        check,
                    )? {
                        return Ok(Some((b, round)));
                    }
                }
            }
            Ok(None)
        };
        let mut run =
            |merged: &mut Merged, license: f64, rounds: u32, attempted: &mut TopologyReceipt| {
                let mut receipt = topology::Receipt::default();
                let built = sequence(merged, license, rounds, &mut receipt, attempted);
                receipt.dropped_components = 0;
                receipt.closed_holes = 0;
                receipt.opened_holes = 0;
                built.map(|b| (b, receipt))
            };
        let (b, receipt) = run(&mut merged, drops, 6, &mut attempted)?;
        topology.add(receipt);
        if let Some((b, _)) = b {
            fitted = Some((0, b));
        } else if n > 0 {
            let license = |k: usize| candidates[k - 1].diagonal.next_up();
            let (b, receipt) = run(&mut merged, license(n), 6, &mut attempted)?;
            topology.add(receipt);
            // The bisection stops each prefix at the remesh round all candidates needed
            // (a bound on work; the prefix it finds fits by construction).
            if let Some((b, rounds)) = b {
                let (mut failed, mut fit) = (0, n);
                fitted = Some((n, b));
                while fit - failed > 1 {
                    let k = failed + (fit - failed) / 2;
                    let (b, receipt) = run(&mut merged, license(k), rounds, &mut attempted)?;
                    topology.add(receipt);
                    match b {
                        Some((b, _)) => {
                            fit = k;
                            fitted = Some((k, b));
                        }
                        None => failed = k,
                    }
                }
            }
        }
    }
    if let Some((_, mut b)) = fitted {
        let kept: HashSet<Key> = b
            .geometry
            .triangles
            .iter()
            .flatten()
            .map(|&v| key(b.geometry.positions[v as usize]))
            .collect();
        b.topology.coarse_dropped_components = candidates
            .iter()
            .filter(|c| !c.keys.iter().any(|k| kept.contains(k)))
            .count() as u64;
        attempted.add(topology);
        topology.dropped_components = 0;
        topology.closed_holes = 0;
        topology.opened_holes = 0;
        b.topology.add(topology);
        return Ok(Outcome {
            built: Some(b),
            contraction_failed: false,
            attempted_topology: attempted,
            construction_dropped: Vec::new(),
            relief: None,
        });
    }
    attempted.add(topology);
    Ok(Outcome {
        built: None,
        contraction_failed: false,
        attempted_topology: attempted,
        construction_dropped,
        relief: None,
    })
}

/// A component with no border contact at or above the G floor, which only a coarse drop
/// can remove.
struct Candidate {
    diagonal: f64,
    keys: Vec<Key>,
}

/// Coarse-drop candidates in drop order: bounding-box diagonal, then triangle count, then
/// least key. One component pass and one sort: O(F + C log C).
fn coarse_candidates(m: &Merged, interface: &[InterfaceRow], floor: f64) -> Vec<Candidate> {
    #[derive(Default)]
    struct Component {
        lo: [f64; 3],
        hi: [f64; 3],
        touch: bool,
        triangles: u64,
        keys: Vec<Key>,
    }
    let component = topology::components(m);
    let border = interface_points(interface);
    let mut components: HashMap<usize, Component> = HashMap::default();
    let mut seen = vec![false; m.keys.len()];
    for t in &m.triangles {
        let c = components
            .entry(component[t[0] as usize])
            .or_insert_with(|| Component {
                lo: [f64::MAX; 3],
                hi: [f64::MIN; 3],
                ..Default::default()
            });
        c.triangles += 1;
        for &v in t {
            let p = m.positions[v as usize].map(f64::from);
            for ((lo, hi), x) in c.lo.iter_mut().zip(&mut c.hi).zip(p) {
                *lo = lo.min(x);
                *hi = hi.max(x);
            }
            let k = m.keys[v as usize];
            c.touch |= border.contains(&k);
            if !std::mem::replace(&mut seen[v as usize], true) {
                c.keys.push(k);
            }
        }
    }
    let mut candidates: Vec<(f64, u64, Key, Vec<Key>)> = components
        .into_values()
        .filter_map(|mut c| {
            let diagonal = (0..3)
                .map(|a| (c.hi[a] - c.lo[a]).powi(2))
                .sum::<f64>()
                .sqrt();
            c.keys.sort_unstable();
            (!c.touch && diagonal >= floor).then(|| (diagonal, c.triangles, c.keys[0], c.keys))
        })
        .collect();
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    candidates
        .into_iter()
        .map(|(diagonal, _, _, keys)| Candidate { diagonal, keys })
        .collect()
}

/// Failed contraction is not an omission decision. Preserve the old border samples
/// while reducing separate sheets at both caps, then try constructive and six bounded
/// interior remeshes. Every final contraction is checked against the original faces.
#[allow(clippy::too_many_arguments)]
fn recover_contraction(
    merged: &mut Merged,
    reference: &Merged,
    interface: &[InterfaceRow],
    previous: &[InterfaceRow],
    mapping: &BorderMap<'_>,
    caps: &[usize],
    receipt: &mut TopologyReceipt,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
    fallbacks: bool,
    folds: Option<&folds::Policy>,
) -> Result<bool, JobError> {
    let border = interface_points(previous);
    let locked: Vec<_> = merged.keys.iter().map(|k| border.contains(k)).collect();
    drop(border);
    for round in 0..caps.len() + 7 {
        check()?;
        merged.triangles.clone_from(&reference.triangles);
        merged.representative.clone_from(&reference.representative);
        let mut attempt = TopologyReceipt::default();
        if round < caps.len() {
            if caps[..round].contains(&caps[round]) {
                continue;
            }
            let (faces, _) = sheets::simplify(
                merged,
                &merged.triangles,
                &|v| locked[v as usize],
                caps[round],
            );
            if faces == merged.triangles {
                continue;
            }
            merged.triangles = faces;
            merged
                .representative
                .resize(merged.triangles.len(), reference.representative[0]);
        } else if round == caps.len() {
            let Some(r) = constrained::remesh(merged, previous, reference.triangles.len()) else {
                continue;
            };
            attempt.add(r);
        } else if !topology::remesh(merged, previous, (round - caps.len()) as u32, &mut attempt) {
            receipt.rejected_remesh_rounds += attempt.rejected_remesh_rounds;
            continue;
        }
        if topology::contract_from(
            merged,
            interface,
            previous,
            mapping,
            &mut attempt,
            Some(&reference.triangles),
            fallbacks,
            folds,
        )? {
            receipt.add(attempt);
            return Ok(true);
        }
        receipt.rejected_remesh_rounds += attempt.rejected_remesh_rounds;
    }
    Ok(false)
}

/// Keep the source face ring around a rejected new hole before considering omission.
/// Pins grow monotonically and retries are bounded; no patch or new boundary is invented.
#[allow(clippy::too_many_arguments)]
fn assessed_attempt(
    tile: TileKey,
    merged: &Merged,
    interface: &[InterfaceRow],
    cap: usize,
    children_error: f64,
    reference: &Merged,
    contracted_faces: &[[u32; 3]],
    floor: f64,
    preserve_holes: bool,
    opening_pins: &mut HashSet<Key>,
    resort: Resort,
    attempted: &mut TopologyReceipt,
    assess: &mut dyn FnMut(&mut Built) -> Result<bool, JobError>,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<Option<Built>, JobError> {
    for round in 0..=RETRIES {
        check()?;
        let proposal = attempt(
            tile,
            merged,
            interface,
            cap,
            children_error,
            reference,
            contracted_faces,
            floor,
            preserve_holes,
            opening_pins,
            resort,
        );
        let mut b = match proposal {
            Ok(b) => b,
            Err(e) if e.kind() == JobErrorKind::Unsupported => return Ok(None),
            Err(e) => return Err(e),
        };
        b.retries += round;
        b.staged = resort.staged();
        b.last_resort_stage = resort.stage();
        let accepted = assess(&mut b)?;
        if !accepted {
            b.change.receipt.rejected_hole_closures += b.change.receipt.closed_holes;
            b.change.receipt.rejected_hole_openings += b.change.receipt.opened_holes;
        }
        attempted.add(b.change.receipt);
        if accepted {
            return Ok(Some(b));
        }
        if round == RETRIES || b.change.receipt.opened_holes == 0 {
            break;
        }
        let rim: HashSet<Key> = b
            .change
            .opening_boundary
            .iter()
            .flatten()
            .map(|p| key(*p))
            .collect();
        let before = opening_pins.len();
        for t in &reference.triangles {
            let keys = t.map(|v| reference.keys[v as usize]);
            if keys.iter().any(|k| rim.contains(k)) {
                opening_pins.extend(keys);
            }
        }
        if opening_pins.len() == before {
            break;
        }
    }
    Ok(None)
}

/// One bounded parent proposal. Unsupported means its protected repair did not
/// produce an eligible capped result; it is not a proof of cap infeasibility. With `Fix2`,
/// when both modes fail after the touching-boundary comparison refused a proposal, reduce
/// once more with the anchored paths of touching boundaries locked
/// (`topology::touching_pins`).
#[allow(clippy::too_many_arguments)]
fn attempt(
    tile: TileKey,
    merged: &Merged,
    interface: &[InterfaceRow],
    cap: usize,
    children_error: f64,
    reference: &Merged,
    contracted_faces: &[[u32; 3]],
    floor: f64,
    preserve_holes: bool,
    opening_pins: &HashSet<Key>,
    resort: Resort,
) -> Result<Built, JobError> {
    let mut refused = false;
    let mut remaining = REDUCTION_PASSES;
    let reduce = |locked: &HashSet<Key>, sheets, remaining: &mut u32, refused: &mut bool| {
        attempt_mode(
            tile,
            merged,
            interface,
            cap,
            children_error,
            reference,
            contracted_faces,
            floor,
            preserve_holes,
            locked,
            sheets,
            remaining,
            refused,
            resort,
        )
    };
    // An overprotected ring is a failed strategy, not a cap lower bound. Both
    // strategies retain the same source guards and pre-contraction reference.
    let result = match reduce(opening_pins, false, &mut remaining, &mut refused) {
        Err(e) if e.kind() == JobErrorKind::Unsupported => {
            reduce(opening_pins, true, &mut remaining, &mut refused)
        }
        result => result,
    };
    match result {
        Err(e) if e.kind() == JobErrorKind::Unsupported && resort == Resort::Fix2 && refused => {
            let border = interface_points(interface);
            let mut locked = opening_pins.clone();
            locked.extend(
                merged
                    .keys
                    .iter()
                    .zip(topology::touching_pins(merged, interface))
                    .filter(|(k, pin)| *pin && !border.contains(*k))
                    .map(|(k, _)| *k),
            );
            if locked.len() == opening_pins.len() {
                Err(e)
            } else {
                reduce(&locked, false, &mut { REDUCTION_PASSES }, &mut false)
            }
        }
        result => result,
    }
}

#[allow(clippy::too_many_arguments)]
fn attempt_mode(
    tile: TileKey,
    merged: &Merged,
    interface: &[InterfaceRow],
    cap: usize,
    children_error: f64,
    reference: &Merged,
    contracted_faces: &[[u32; 3]],
    floor: f64,
    preserve_holes: bool,
    opening_pins: &HashSet<Key>,
    separate_sheets: bool,
    remaining: &mut u32,
    touching: &mut bool,
    resort: Resort,
) -> Result<Built, JobError> {
    let policy =
        folds::Policy::last_resort(resort.stage(), tile.leaf_begin, tile.leaf_end, interface);
    let _cpu =
        super::cpu_profile::start(super::cpu_profile::Stage::Attempt, merged.triangles.len());
    let required = || {
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
        (segments, vertices)
    };
    let (segments, vertices) = required();
    let (interface_segments, interface_vertices) = (segments.len(), vertices.len());
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
    let corners = required_corners(&merged.keys, &segments, &vertices);
    let mut locked: Vec<bool> = if preserve_holes {
        topology::protected(merged, interface, true).0
    } else {
        merged.keys.iter().map(|k| vertices.contains(k)).collect()
    };
    let contact_pins = folds::contact_pins(&merged.keys, &merged.triangles, interface);
    for (v, lock) in locked.iter_mut().enumerate() {
        *lock |= opening_pins.contains(&merged.keys[v]);
    }
    // Staged reduction checks every step with the guard, so it needs no contact pins.
    let staged_lock = (resort.staged() && !separate_sheets).then(|| locked.clone());
    for (lock, &pin) in locked.iter_mut().zip(&contact_pins) {
        *lock |= pin;
    }
    drop(contact_pins);
    drop(segments);
    drop(vertices);
    let input = merged.triangles.len();
    let mut protected = vec![false; input];
    let mut retries = 0;
    let degenerate_only = merged
        .triangles
        .iter()
        .all(|t| t[0] == t[1] || t[1] == t[2] || t[0] == t[2]);
    let mut accepted_change = None;
    let (mut output, simplifier_error, stopping) = if input <= cap {
        (merged.triangles.clone(), 0., StoppingBound::Input)
    } else if degenerate_only {
        (
            merged.triangles[..cap].to_vec(),
            0.,
            StoppingBound::TriangleBudget,
        )
    } else {
        let mut reduce = |faces: &[[u32; 3]], lock: &dyn Fn(u32) -> bool, target| {
            let reserve = u32::from(!separate_sheets);
            if *remaining <= reserve {
                return Err(JobError::new(
                    JobErrorKind::Unsupported,
                    "protected reduction exhausted its shared strategy budget",
                ));
            }
            *remaining -= 1;
            Ok(if separate_sheets {
                sheets::simplify(merged, faces, lock, target)
            } else {
                simplify(merged, faces, lock, target)
            })
        };
        let (mut output, mut error) = match &staged_lock {
            Some(lock) if resort == Resort::N3 => {
                cluster_reduce(merged, contracted_faces, interface, lock, cap, &policy)
            }
            Some(lock) => staged_reduce(merged, contracted_faces, interface, lock, cap, &policy),
            None => reduce(&merged.triangles, &|v| locked[v as usize], cap)?,
        };
        settle(merged, contracted_faces, &policy, resort, &mut output);
        let mut topology_protected = false;
        let mut fold_retries = 0;
        // Only one immutable post-contraction source per retry sequence. Release
        // the interface query sets before the guard, and all repair sets before
        // meshopt, so these maps fit the same parent workspace charge.
        let mut fold_source = None;
        loop {
            let (segments, vertices) = required();
            let (edges, points) = missing_among(&output, &merged.keys, segments, vertices, &|v| {
                corners[v as usize]
            });
            let invalid = fold_source
                .get_or_insert_with(|| {
                    folds::Source::new(&merged.positions, contracted_faces)
                        .with_boundary(interface)
                        .with_policy(policy.clone())
                })
                .violations(&merged.positions, &output);
            let valid = edges.is_empty() && points.is_empty() && !invalid.iter().any(|v| *v);
            if valid {
                // Topology comparison has its own tables; do not overlap them
                // with a retained fold source. Rebuild only if it rejects.
                fold_source = None;
                accepted_change =
                    topology::changes_noting(reference, &output, interface, floor, touching);
                if accepted_change.is_some() {
                    break;
                }
            }
            let previously_protected = protected.iter().filter(|p| **p).count();
            if valid || fold_retries == FOLD_RETRIES {
                fold_source = None;
                if topology_protected {
                    return Err(JobError::new(
                        JobErrorKind::Unsupported,
                        format!("coarse tile f{}-d{}: protected reduction exhausted its repair budget for topology and orientation ({input} input triangles, {} protected, cap {cap})",
                            tile.first, tile.depth, protected.iter().filter(|p| **p).count()),
                    ));
                }
                // Keep these carriers in the SAME partition as all previous fold
                // protection. A fresh whole-mesh reduction would discard that evidence.
                for (keep, support) in protected.iter_mut().zip(topology::support_faces(
                    merged,
                    interface,
                    preserve_holes,
                )) {
                    *keep |= support;
                }
                topology_protected = true;
                fold_retries = 0;
            } else {
                fold_retries += 1;
            }
            retries += 1;
            let edges: HashSet<(Key, Key)> = edges.into_iter().collect();
            let points: HashSet<Key> = points.into_iter().collect();
            let mut carried = if separate_sheets {
                vec![false; invalid.len()]
            } else {
                Vec::new()
            };
            for (i, t) in merged.triangles.iter().enumerate() {
                let o = t.map(|v| merged.keys[v as usize]);
                if (0..3)
                    .any(|k| edges.contains(&edge(o[k], o[(k + 1) % 3])) || points.contains(&o[k]))
                {
                    protected[i] = true;
                }
                // A full vertex star can consume the cap even on a two-sided plate.
                // The sheet retry retains one fresh local carrier per offending
                // corner per round; all failures are rechecked on the next proposal.
                let needed = t
                    .iter()
                    .any(|&v| invalid[v as usize] && (!separate_sheets || !carried[v as usize]));
                if needed && (!separate_sheets || !protected[i]) {
                    protected[i] = true;
                    if separate_sheets {
                        for &v in t {
                            carried[v as usize] = true;
                        }
                    }
                }
            }
            drop(carried);
            if protected.iter().filter(|p| **p).count() == previously_protected {
                return Err(JobError::new(
                    JobErrorKind::Unsupported,
                    "protected reduction made no progress",
                ));
            }
            drop(edges);
            drop(points);
            drop(invalid);
            if separate_sheets {
                // Partition incidence workspace and the cached Source must not
                // overlap; the regular path keeps its established retry cache.
                fold_source = None;
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
                reduce(
                    &rest,
                    &|v| locked[v as usize] || kept.contains(&v),
                    cap.saturating_sub(keep.len()),
                )?
            };
            reduced.extend(keep);
            output = reduced;
            settle(merged, contracted_faces, &policy, resort, &mut output);
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
                    interface_segments,
                    interface_vertices,
                    protected.iter().filter(|p| **p).count(),
                    output.len()
                ),
            ));
        } else {
            (output, error, StoppingBound::TriangleBudget)
        }
    };
    if matches!(stopping, StoppingBound::Input)
        && folds::Source::new(&merged.positions, contracted_faces)
            .with_boundary(interface)
            .with_policy(policy.clone())
            .violations(&merged.positions, &output)
            .iter()
            .any(|v| *v)
    {
        return Err(JobError::new(
            JobErrorKind::Unsupported,
            "remeshing introduces coincident or folded faces",
        ));
    }
    if output.is_empty() {
        return Err(JobError::new(
            JobErrorKind::Unsupported,
            format!(
                "coarse tile f{}-d{} simplified to no triangles",
                tile.first, tile.depth
            ),
        ));
    }
    // Drops go smallest first (validators check it): extra smaller drops, all licensed,
    // invalidate the loop's comparison.
    if topology::order_drops(reference, &mut output, interface, children_error) {
        accepted_change = None;
        // Meshopt kept only smaller border-free components than one it dropped: this
        // strategy fails, and the next cap, remesh or D11 stage runs (its cleanup keeps
        // the largest).
        if output.is_empty() {
            return Err(JobError::new(
                JobErrorKind::Unsupported,
                format!(
                    "coarse tile f{}-d{}: drop order would empty the tile",
                    tile.first, tile.depth
                ),
            ));
        }
    }
    // The successful loop check used these same immutable inputs. Preserve its
    // witnesses before compaction; input/degenerate-only paths still compare here.
    let topology_change = accepted_change
        .or_else(|| topology::changes_noting(reference, &output, interface, floor, touching));
    let Some(change) = topology_change else {
        return Err(JobError::new(
            JobErrorKind::Unsupported,
            "simplification removes a border-connected component or touching boundary loop",
        ));
    };
    if !simplifier_error.is_finite() {
        return Err(state("meshopt reported a nonfinite error"));
    }

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
    geometry.primitives.push(merged.representative[0]);
    geometry.representative.resize(output.len(), 0);
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
        last_resort: false,
        staged: false,
        last_resort_stage: 0,
        relief_stuck: 0,
        interface_vertices: interface_vertices as u32,
        interface_segments: interface_segments as u32,
        reoriented: 0,
        topology: change.receipt,
        change,
        controls: Default::default(),
        geometry,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A1 → N3 hand-over: border chain a-m-b with m left on chord a-b (t 0.25) and an
    /// interior apex c. `snap_stuck` moves m onto a, drops the face it degenerates and
    /// keeps the chord, licensed as any contraction is; a snap that leaves no face on the
    /// chord, or a stuck entry naming another vertex, is refused.
    #[test]
    fn handed_over_stuck_vertices_snap_onto_their_chords() {
        let p = [[0., 0., 0.], [1., 0.1, 0.], [2., 0., 0.], [1., 1., 0.]];
        let k: Vec<Key> = p.iter().map(|&q| key(q)).collect();
        let (a, m, b) = (k[0], k[1], k[2]);
        let merged = |triangles: Vec<[u32; 3]>| Merged {
            keys: k.clone(),
            ells: vec![0; 4],
            positions: p.to_vec(),
            representative: vec![
                SourcePrimitive {
                    node: 0,
                    mesh: 0,
                    primitive: 0,
                    expanded_ordinal: 0,
                    material: u32::MAX,
                };
                triangles.len()
            ],
            triangles,
        };
        let interface = [
            InterfaceRow::new(0, a, a, 0, 1),
            InterfaceRow::new(0, b, b, 0, 1),
            InterfaceRow::new(0, a.min(b), a.max(b), 0, 1),
        ];
        let previous = [
            InterfaceRow::new(0, a.min(m), a.max(m), 0, 1),
            InterfaceRow::new(0, m.min(b), m.max(b), 0, 1),
        ];
        let mapped = [None, Some((a, b, 0.25, 0)), None, None];
        let stuck = || {
            vec![Stuck {
                vertex: 1,
                key: m,
                chord: [a, b],
                t: 0.25,
            }]
        };
        let check = |triangles: Vec<[u32; 3]>, stuck: Vec<Stuck>| {
            let reference = merged(triangles);
            let guard = folds::Source::for_contraction(
                &reference.positions,
                &reference.triangles,
                &interface,
                &previous,
                &mapped,
            )
            .with_policy(folds::Policy::last_resort(3, 0, 1, &interface));
            let seed = Contracted {
                merged: reference.clone(),
                stuck,
            };
            snap_stuck(&reference, &seed, &interface, &guard).map(|m| m.triangles)
        };
        assert_eq!(
            check(vec![[0, 1, 3], [1, 2, 3]], stuck()),
            Some(vec![[0, 2, 3]])
        );
        assert_eq!(check(vec![[0, 1, 2]], stuck()), None);
        let mut wrong = stuck();
        wrong[0].key = b;
        assert_eq!(check(vec![[0, 1, 3], [1, 2, 3]], wrong), None);
    }

    #[test]
    fn fixed_sheet_shortcut_matches_pinned_meshopt_with_degeneracies() {
        for seed in 0..32u32 {
            let positions: Vec<_> = (0..12)
                .map(|i| {
                    if seed % 4 == 0 {
                        [0., -0., 0.]
                    } else {
                        [((i * 7 + seed) % 5) as f32, (i % 3) as f32, 0.]
                    }
                })
                .collect();
            // Includes repeated positions/indices, duplicate and reversed faces,
            // nonmanifold shared edges and unused vertices. No cleanup is implied.
            let triangles = vec![
                [1, 2, 3],
                [3, 2, 1],
                [1, 2, 3],
                [1, 2, 4],
                [1, 5, 2],
                [1, 1, 6],
                [7, 8, 9],
            ];
            let merged = Merged {
                keys: positions.iter().copied().map(key).collect(),
                ells: vec![1; positions.len()],
                positions,
                triangles: triangles.clone(),
                representative: vec![],
            };
            for (locked, target) in [(true, 0), (true, 3), (false, triangles.len())] {
                let indices: Vec<_> = triangles.iter().flatten().copied().collect();
                let mut actual = vec![0; indices.len()];
                let locks = vec![u8::from(locked); merged.positions.len()];
                let mut error = -1f32;
                // SAFETY: buffers cover the declared indices/vertices, positions
                // are tightly packed f32x3, and all indices are in range.
                let n = unsafe {
                    meshopt::ffi::meshopt_simplifyWithAttributes(
                        actual.as_mut_ptr(),
                        indices.as_ptr(),
                        indices.len(),
                        merged.positions.as_ptr().cast::<f32>(),
                        merged.positions.len(),
                        12,
                        std::ptr::null(),
                        0,
                        std::ptr::null(),
                        0,
                        locks.as_ptr(),
                        target * 3,
                        f32::MAX,
                        meshopt::SimplifyOptions::ErrorAbsolute.bits(),
                        &mut error,
                    )
                };
                actual.truncate(n);
                assert_eq!(actual, indices);
                assert_eq!(error.to_bits(), 0);
                let mut local = vec![u32::MAX; merged.positions.len()];
                let (output, estimate) =
                    simplify_mapped(&merged, &triangles, &|_| locked, target, &mut local);
                assert_eq!(output, triangles);
                assert_eq!(estimate.to_bits(), 0);
                assert!(local.iter().all(|&i| i == u32::MAX));
            }
        }
        assert!(!finite_extent(
            &[[f32::MAX, 0., 0.], [-f32::MAX, 0., 0.]],
            &[[0, 1, 0]]
        ));
        assert!(!finite_extent(&[[f32::NAN, 0., 0.]], &[[0, 0, 0]]));
    }

    #[test]
    fn parent_mapping_queries_only_child_interfaces_and_keeps_the_contracted_arc() {
        let parent = TileKey {
            first: 0,
            end: 2,
            depth: 0,
            leaf_begin: 0,
            leaf_end: 1,
        };
        let positions = vec![
            [0., 0., 0.],
            [0.5, 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [1., 1., 0.],
        ];
        let [a, middle, b] = [key(positions[0]), key(positions[1]), key(positions[2])];
        let child = TileGeometry {
            key: TileKey { depth: 1, ..parent },
            positions,
            ells: vec![1; 5],
            triangles: vec![[0, 1, 3], [1, 4, 3], [1, 2, 4]],
            representative: vec![0; 3],
            primitives: vec![SourcePrimitive {
                node: 0,
                mesh: 0,
                primitive: 0,
                expanded_ordinal: 0,
                material: u32::MAX,
            }],
            interface: vec![
                InterfaceRow::new(0, a, middle, 0, 1),
                InterfaceRow::new(0, middle, b, 0, 1),
            ],
            geometric_error: 0.,
            lease: None,
        };
        let calls = std::cell::Cell::new(0);
        let built = build_parent(
            parent,
            &[child],
            &[8],
            &Contract {
                depth: 0,
                interface: vec![InterfaceRow::new(0, a, b, 0, 1)],
                certificates: vec![],
                terminal_vertices: vec![],
                scale: 0.,
            },
            &|p| {
                assert!([a, middle, b].contains(&p), "interior source lookup");
                calls.set(calls.get() + 1);
                Ok((p == middle).then_some((a, b, 0.5, 1)))
            },
            &mut |_| Ok(true),
            &|| Ok(()),
            true,
        )
        .unwrap()
        .built
        .unwrap();
        assert_eq!(calls.get(), 3);
        let geometry = built.geometry;
        let keys: Vec<_> = geometry.positions.iter().copied().map(key).collect();
        let absent = missing(
            &geometry.triangles,
            &keys,
            &[edge(a, b)].into_iter().collect(),
            &HashSet::default(),
        );
        assert!(absent.0.is_empty());
        assert!(built.topology.contracted_vertices > 0);
    }

    #[test]
    fn thin_tetrahedron_collapse_retries_instead_of_publishing_fresh_twins() {
        let positions = vec![
            [0., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [0.25, 0.25, 0.01],
            [1. / 3., 1. / 3., 0.],
        ];
        let triangles = vec![
            [0, 1, 4],
            [1, 2, 4],
            [2, 0, 4],
            [0, 3, 1],
            [1, 3, 2],
            [2, 3, 0],
        ];
        let merged = Merged {
            keys: positions.iter().copied().map(key).collect(),
            ells: vec![0; positions.len()],
            positions,
            representative: Vec::new(),
            triangles,
        };
        let source = folds::Source::new(&merged.positions, &merged.triangles);
        let (raw, _) = simplify(&merged, &merged.triangles, &|v| v < 3, 2);
        assert!(
            source
                .violations(&merged.positions, &raw)
                .iter()
                .any(|v| *v),
            "{raw:?}"
        );
        let interface = (0..3)
            .map(|v| InterfaceRow::new(0, merged.keys[v], merged.keys[v], 0, 1))
            .collect::<Vec<_>>();
        let mut merged = merged;
        merged.representative.resize(
            merged.triangles.len(),
            SourcePrimitive {
                node: 0,
                mesh: 0,
                primitive: 0,
                expanded_ordinal: 0,
                material: u32::MAX,
            },
        );
        let tile = TileKey {
            first: 0,
            end: 6,
            depth: 0,
            leaf_begin: 0,
            leaf_end: 1,
        };
        let pins = HashSet::default();
        assert!(attempt(
            tile,
            &merged,
            &interface,
            2,
            0.,
            &merged,
            &merged.triangles,
            1.,
            false,
            &pins,
            Resort::None,
        )
        .is_err());
        let built = attempt(
            tile,
            &merged,
            &interface,
            6,
            0.,
            &merged,
            &merged.triangles,
            1.,
            false,
            &pins,
            Resort::None,
        )
        .unwrap();
        assert_eq!(built.geometry.triangles.len(), 6);
        assert_eq!(built.reoriented, 0);
    }
}
