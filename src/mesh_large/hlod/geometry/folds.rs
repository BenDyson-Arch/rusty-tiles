//! Reject introduced face coincidences and overlapping edge fans before publication.
//! This is a proposal check: it never removes, reverses or moves a face.
use super::{key, Key};
use crate::mesh_large::hlod::interfaces::InterfaceRow;
use rustc_hash::FxHashMap as HashMap;
use rustc_hash::FxHashSet as HashSet;
#[path = "folds_contract.rs"]
mod contract;

type Face = [Key; 3];
type Edge = (Key, Key);
type Incidence = (usize, u8, u32);

fn face(mut f: Face) -> (Face, usize) {
    let k = (0..3).min_by_key(|&i| f[i]).unwrap();
    f.rotate_left(k);
    let direction = usize::from(f[1] > f[2]);
    if direction != 0 {
        f.swap(1, 2);
    }
    (f, direction)
}

pub(crate) struct Source {
    faces: HashMap<Face, [u32; 2]>,
    edges: HashMap<Edge, [u32; 2]>,
    boundary: HashSet<Key>,
    policy: Policy,
}

/// The last resort's relaxations (D1/D11 amendment A2); the default relaxes nothing.
/// `least`, `greatest`: the shared points whose least, respectively greatest, holding DFS
/// leaf lies in this cell. A new edge between two shared points, or a new face on three, is
/// admitted only in its seam owner (N1, `owns_segment`, `owns_face`): every same-depth
/// cell derives the same single owner from the canonical keys. `folds`: reversed faces,
/// coincident groups and same-direction books are admitted (N2, N3), declared and
/// recounted by the validators. Degenerate faces and every other shared-point rule stay.
#[derive(Clone, Default)]
pub(crate) struct Policy {
    pub least: HashSet<Key>,
    pub greatest: HashSet<Key>,
    pub folds: bool,
}

/// The published canonical domain: a signed zero is positive zero.
fn published(k: Key) -> Key {
    k.map(|w| if w == 0x8000_0000 { 0 } else { w })
}

/// The seam side of a segment between published keys `a < b`: the top bit of FNV-1a 64
/// over their little-endian words, `a` first. false: the least holder of `a`; true: its
/// greatest. The validators compute the same bit independently.
pub(crate) fn greatest_side(a: Key, b: Key) -> bool {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for w in a.into_iter().chain(b) {
        for byte in w.to_le_bytes() {
            h = (h ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h >> 63 == 1
}

impl Policy {
    /// Stage 1-3 relaxations for `tile` (over leaves `[begin, end)`), from its contract.
    pub(crate) fn last_resort(stage: u8, begin: u32, end: u32, interface: &[InterfaceRow]) -> Self {
        if stage == 0 {
            return Self::default();
        }
        let holding = |leaf: fn(&InterfaceRow) -> u32| {
            interface
                .iter()
                .filter(|r| r.is_vertex() && (begin..end).contains(&leaf(r)))
                .map(|r| r.a)
                .collect()
        };
        Self {
            least: holding(|r| r.lo),
            greatest: holding(|r| r.hi),
            folds: stage >= 2,
        }
    }

    /// The segment `{a, b}` between shared points belongs to the cell holding the least or
    /// greatest holding leaf (`greatest_side`) of its lesser published point: exactly one
    /// cell per depth.
    pub(crate) fn owns_segment(&self, a: Key, b: Key) -> bool {
        let (a, b) = if published(a) <= published(b) {
            (a, b)
        } else {
            (b, a)
        };
        if greatest_side(published(a), published(b)) {
            self.greatest.contains(&a)
        } else {
            self.least.contains(&a)
        }
    }

    /// A face on three shared points belongs to the owner of its span segment, least to
    /// greatest published corner. On a seam chain that is the edge a new sliver face adds.
    pub(crate) fn owns_face(&self, mut f: Face) -> bool {
        f.sort_unstable_by_key(|&k| published(k));
        self.owns_segment(f[0], f[2])
    }
}

impl Source {
    pub(crate) fn new(positions: &[[f32; 3]], triangles: &[[u32; 3]]) -> Self {
        let _cpu = super::super::cpu_profile::start(
            super::super::cpu_profile::Stage::Source,
            triangles.len(),
        );
        let mut result = Self {
            // Reserve the face bound and the common surface-edge count. This is
            // only a capacity hint: disconnected/nonmanifold inputs still grow.
            faces: HashMap::with_capacity_and_hasher(triangles.len(), Default::default()),
            edges: HashMap::with_capacity_and_hasher(triangles.len() * 3 / 2, Default::default()),
            boundary: HashSet::default(),
            policy: Policy::default(),
        };
        for t in triangles {
            let keys = t.map(|v| key(positions[v as usize]));
            let (f, d) = face(keys);
            result.faces.entry(f).or_default()[d] += 1;
            for j in 0..3 {
                let (a, b) = (keys[j], keys[(j + 1) % 3]);
                result.edges.entry((a.min(b), a.max(b))).or_default()[usize::from(a > b)] += 1;
            }
        }
        result
    }

    pub(crate) fn with_boundary(
        mut self,
        rows: &[crate::mesh_large::hlod::interfaces::InterfaceRow],
    ) -> Self {
        let _cpu = super::super::cpu_profile::start(
            super::super::cpu_profile::Stage::Boundary,
            rows.len(),
        );
        // Grow with distinct keys rather than the row iterator's (often duplicate)
        // size hint. A parent source's checked boundary has at most 3m keys.
        for r in rows {
            self.boundary.insert(r.a);
            self.boundary.insert(r.b);
        }
        self
    }

    pub(crate) fn with_policy(mut self, policy: Policy) -> Self {
        self.policy = policy;
        self
    }

    /// Turn faces the source holds only the other way round and, with `once`, drop
    /// coincident copies beyond the source's count per orientation (at least one stays):
    /// N2 keeps its folds' surfaces once, oriented like the source. N3 keeps every copy, so
    /// clustering leaves edge counts even wherever the input was closed.
    pub(crate) fn settle(&self, positions: &[[f32; 3]], triangles: &mut Vec<[u32; 3]>, once: bool) {
        let mut kept: HashMap<Face, [u32; 2]> = HashMap::default();
        triangles.retain_mut(|t| {
            let (f, mut d) = face(t.map(|v| key(positions[v as usize])));
            let source = self.faces.get(&f).copied().unwrap_or_default();
            if source[d] == 0 && source[1 - d] != 0 {
                t.swap(1, 2);
                d = 1 - d;
            }
            let k = kept.entry(f).or_default();
            let total = k[0] + k[1];
            let keep = !once || total == 0 || k[d] < source[d];
            k[d] += u32::from(keep);
            keep
        });
    }

    /// `canonical` gives each corner the first vertex index with its Key, so equal
    /// sorted canonical triples are exactly equal faces (`face` is the sorted Key triple).
    fn bad_faces(
        &self,
        positions: &[[f32; 3]],
        triangles: &[[u32; 3]],
        canonical: &[[u32; 3]],
    ) -> (Vec<bool>, Vec<bool>) {
        let _cpu = super::super::cpu_profile::start(
            super::super::cpu_profile::Stage::Faces,
            triangles.len(),
        );
        let mut invalid = vec![false; triangles.len()];
        let mut original = vec![false; triangles.len()];
        let mut faces = Vec::with_capacity(triangles.len());
        for (i, (t, ids)) in triangles.iter().zip(canonical).enumerate() {
            let (f, d) = face(t.map(|v| key(positions[v as usize])));
            let source = self.faces.get(&f).copied().unwrap_or_default();
            // A source face license is independent of proposal validity below.
            original[i] = source[d] != 0;
            // Reject the old orient() repair rather than flipping or removing a face.
            invalid[i] = (source[d] == 0 && source[1 - d] != 0 && !self.policy.folds)
                // Two disjoint children can independently close the same three
                // shared points into opposite faces. Their union would license
                // a new twin as "input". Retain the local source strip instead.
                || (source == [0, 0]
                    && f.iter().all(|p| self.boundary.contains(p))
                    && !self.policy.owns_face(f))
                || (source == [0, 0]
                    && super::topology::normal(positions, *t)
                        .iter()
                        .all(|&n| n == 0.));
            let mut group = *ids;
            group.sort_unstable();
            faces.push((group, d as u8, i as u32));
        }
        // Grouping only: counts and marks are the same in any order within a group.
        faces.sort_unstable_by_key(|r| r.0);
        let mut at = 0;
        while at < faces.len() {
            let end = at + faces[at..].partition_point(|r| r.0 == faces[at].0);
            if end - at > 1 && !self.policy.folds {
                let mut counts = [0; 2];
                for &(_, d, _) in &faces[at..end] {
                    counts[d as usize] += 1;
                }
                let i = faces[at].2 as usize;
                let (f, _) = face(triangles[i].map(|v| key(positions[v as usize])));
                let source = self.faces.get(&f).copied().unwrap_or_default();
                if counts[0] > source[0] || counts[1] > source[1] {
                    for &(_, _, i) in &faces[at..end] {
                        invalid[i as usize] = true;
                    }
                }
            }
            at = end;
        }
        (invalid, original)
    }

    /// Whether this edge (in either direction) is a source edge.
    pub(crate) fn licensed_edge(&self, positions: &[[f32; 3]], a: u32, b: u32) -> bool {
        let (a, b) = (key(positions[a as usize]), key(positions[b as usize]));
        self.edges.contains_key(&(a.min(b), a.max(b)))
    }

    /// Whether this face (in either orientation) is a source face.
    pub(crate) fn licensed(&self, positions: &[[f32; 3]], t: [u32; 3]) -> bool {
        let (f, _) = face(t.map(|v| key(positions[v as usize])));
        self.faces.get(&f).is_some_and(|s| *s != [0, 0])
    }

    /// Corners of rejected faces, in vertex order. One flat sorted incidence array
    /// avoids an allocation for every edge. More than four wings cannot be pairwise
    /// nonacute in their plane, so the fallback never enumerates high-valence pairs.
    pub(crate) fn violations(&self, positions: &[[f32; 3]], triangles: &[[u32; 3]]) -> Vec<bool> {
        self.check(positions, triangles, false)
    }

    /// A whole-panel fan has no certified contraction license for moved wings.
    /// Retain the boundary count ceiling and also check changed boundary fans.
    pub(super) fn remesh_violations(
        &self,
        positions: &[[f32; 3]],
        triangles: &[[u32; 3]],
    ) -> Vec<bool> {
        self.check(positions, triangles, true)
    }

    /// Any face rejected; no vertex-sized work.
    pub(super) fn rejects(&self, positions: &[[f32; 3]], triangles: &[[u32; 3]]) -> bool {
        self.rejected_faces(positions, triangles, false)
            .contains(&true)
    }

    fn check(
        &self,
        positions: &[[f32; 3]],
        triangles: &[[u32; 3]],
        changed_boundary: bool,
    ) -> Vec<bool> {
        let invalid = self.rejected_faces(positions, triangles, changed_boundary);
        #[cfg(test)]
        VERTEX_FLAGS.with(|n| n.set(n.get() + positions.len()));
        let mut vertices = vec![false; positions.len()];
        for (t, invalid) in triangles.iter().zip(invalid) {
            if invalid {
                for &v in t {
                    vertices[v as usize] = true;
                }
            }
        }
        vertices
    }

    fn rejected_faces(
        &self,
        positions: &[[f32; 3]],
        triangles: &[[u32; 3]],
        changed_boundary: bool,
    ) -> Vec<bool> {
        let incidence_cpu = super::super::cpu_profile::start(
            super::super::cpu_profile::Stage::Incidence,
            triangles.len(),
        );
        // Only referenced vertices consume workspace. Equal Keys share the first
        // referenced index, including duplicate positions; signed zeros stay distinct.
        // Drop the reserved 3m-entry lookup before allocating the 72m-byte edge
        // array; only the 12m-byte corner IDs overlap that array (64-bit sizes).
        let canonical = {
            let mut by_key = HashMap::<Key, u32>::with_capacity_and_hasher(
                triangles.len() * 3,
                Default::default(),
            );
            let mut canonical = Vec::with_capacity(triangles.len());
            for t in triangles {
                canonical.push(t.map(|v| *by_key.entry(key(positions[v as usize])).or_insert(v)));
            }
            canonical
        };
        drop(incidence_cpu);
        let (mut invalid, original) = self.bad_faces(positions, triangles, &canonical);
        let incidence_cpu = super::super::cpu_profile::start(
            super::super::cpu_profile::Stage::Incidence,
            triangles.len(),
        );
        let mut edges = Vec::<((u32, u32), Incidence)>::with_capacity(triangles.len() * 3);
        for (i, ids) in canonical.iter().enumerate() {
            let keys = ids.map(|v| key(positions[v as usize]));
            for j in 0..3 {
                let (a, b, c) = (ids[j], ids[(j + 1) % 3], ids[(j + 2) % 3]);
                // Numeric IDs group edges only; direction retains Key ordering.
                edges.push((
                    (a.min(b), a.max(b)),
                    (i, u8::from(keys[j] > keys[(j + 1) % 3]), c),
                ));
            }
        }
        drop(canonical);
        // Grouping only: every per-edge decision below (direction counts, unchanged
        // licences, `distinct_sides`, which fails on a fifth wing, a zero normal or any
        // acute pair) is independent of the order within a group.
        edges.sort_unstable_by_key(|e| e.0);
        drop(incidence_cpu);
        let _cpu =
            super::super::cpu_profile::start(super::super::cpu_profile::Stage::Fans, edges.len());
        let mut at = 0;
        while at < edges.len() {
            let ids = edges[at].0;
            let end = at + edges[at..].partition_point(|r| r.0 == ids);
            let (a, b) = (
                key(positions[ids.0 as usize]),
                key(positions[ids.1 as usize]),
            );
            let edge = (a.min(b), a.max(b));
            let mut counts = [0; 2];
            for &(_, (_, d, _)) in &edges[at..end] {
                counts[d as usize] += 1;
            }
            let source = self.edges.get(&edge).copied().unwrap_or_default();
            // N1: a new edge between two shared points this cell owns.
            let owned = source == [0, 0] && self.policy.owns_segment(edge.0, edge.1);
            let boundary =
                !owned && self.boundary.contains(&edge.0) && self.boundary.contains(&edge.1);
            let unsupported = if boundary && (0..2).any(|d| counts[d] > source[d]) {
                // A new edge between shared contacts could be created by either
                // neighbour independently. Only an existing source edge or a
                // chord certified over its entire previous arc supplies support.
                true
            } else if (boundary && !changed_boundary) || self.policy.folds {
                false
            } else {
                (0..2).any(|d| {
                    if counts[d] < 2 {
                        return false;
                    }
                    // Child incidence is not automatically source multiplicity: a
                    // child may have admitted a new, nonacute book spine. Only carry
                    // its license unchanged; moving a wing must pass the side test
                    // again, even when balanced 2/2 counts are unchanged.
                    let unchanged = counts[d] <= source[d]
                        && edges[at..end]
                            .iter()
                            .filter(|(_, (_, direction, _))| *direction == d as u8)
                            .all(|&(_, (i, _, _))| original[i]);
                    !unchanged
                        && (counts[1 - d] == 0
                            || !distinct_sides(
                                edge,
                                edges[at..end]
                                    .iter()
                                    .filter(|(_, (_, direction, _))| *direction == d as u8)
                                    .map(|(_, (_, _, third))| key(positions[*third as usize])),
                            ))
                })
            };
            if unsupported {
                for &(_, (i, _, _)) in &edges[at..end] {
                    invalid[i] = true;
                }
            }
            at = end;
        }
        invalid
    }
}

/// Faces on a fold, as every validator recounts them from the decoded payload alone:
/// faces of a coincident group (two or more on one corner set, either orientation), and
/// faces using an edge that two or more faces use in the same direction. A last-resort
/// tile declares this count (`asset.extras.rustyTiles.lastResort.foldFaces`).
pub(crate) fn fold_faces(positions: &[[f32; 3]], triangles: &[[u32; 3]]) -> u64 {
    let keys: Vec<Face> = triangles
        .iter()
        .map(|t| t.map(|v| key(positions[v as usize])))
        .collect();
    let mut groups: HashMap<Face, u32> = HashMap::default();
    let mut edges: HashMap<(Key, Key), u32> = HashMap::default();
    for k in &keys {
        if k[0] == k[1] || k[1] == k[2] || k[0] == k[2] {
            continue;
        }
        *groups.entry(face(*k).0).or_default() += 1;
        for j in 0..3 {
            *edges.entry((k[j], k[(j + 1) % 3])).or_default() += 1;
        }
    }
    keys.iter()
        .filter(|k| {
            !(k[0] == k[1] || k[1] == k[2] || k[0] == k[2])
                && (groups[&face(**k).0] > 1 || (0..3).any(|j| edges[&(k[j], k[(j + 1) % 3])] > 1))
        })
        .count() as u64
}

/// The widest fold of a tile (D1/D11 amendment A3, reported, never checked): over each
/// fold face (`fold_faces`), at its corners, edge midpoints and centroid, the distance
/// to the nearest `reference` face facing the same way plus the nearest facing the other
/// way, i.e. how far apart the two layers it folds lie. Distances beyond `reach` count as
/// `reach` (a fold with no second layer within it reports `2 * reach`).
pub(crate) fn fold_separation(
    positions: &[[f32; 3]],
    triangles: &[[u32; 3]],
    reference: &[([f32; 3], [f32; 3], [f32; 3])],
    reach: f64,
) -> f64 {
    let p = |q: [f32; 3]| q.map(f64::from);
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let keys: Vec<Face> = triangles
        .iter()
        .map(|t| t.map(|v| key(positions[v as usize])))
        .collect();
    let mut groups: HashMap<Face, u32> = HashMap::default();
    let mut edges: HashMap<(Key, Key), u32> = HashMap::default();
    for k in &keys {
        *groups.entry(face(*k).0).or_default() += 1;
        for j in 0..3 {
            *edges.entry((k[j], k[(j + 1) % 3])).or_default() += 1;
        }
    }
    let cell = reach.max(1e-9);
    let mut grid: HashMap<[i64; 3], Vec<usize>> = HashMap::default();
    let at = |q: [f64; 3]| q.map(|x| (x / cell).floor() as i64);
    for (i, f) in reference.iter().enumerate() {
        let (a, b) = (at(p(f.0)), at(p(f.1)));
        let c = at(p(f.2));
        for x in a[0].min(b[0]).min(c[0])..=a[0].max(b[0]).max(c[0]) {
            for y in a[1].min(b[1]).min(c[1])..=a[1].max(b[1]).max(c[1]) {
                for z in a[2].min(b[2]).min(c[2])..=a[2].max(b[2]).max(c[2]) {
                    grid.entry([x, y, z]).or_default().push(i);
                }
            }
        }
    }
    let closest = |q: [f64; 3], f: &([f32; 3], [f32; 3], [f32; 3])| {
        let (a, b, c) = (p(f.0), p(f.1), p(f.2));
        let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(q, a));
        let (d1, d2) = (dot(ab, ap), dot(ac, ap));
        let bp = sub(q, b);
        let (d3, d4) = (dot(ab, bp), dot(ac, bp));
        let cp = sub(q, c);
        let (d5, d6) = (dot(ab, cp), dot(ac, cp));
        let (va, vb, vc) = (d3 * d6 - d5 * d4, d5 * d2 - d1 * d6, d1 * d4 - d3 * d2);
        let point = if d1 <= 0. && d2 <= 0. {
            a
        } else if d3 >= 0. && d4 <= d3 {
            b
        } else if d6 >= 0. && d5 <= d6 {
            c
        } else if vc <= 0. && d1 >= 0. && d3 <= 0. {
            let v = d1 / (d1 - d3);
            [a[0] + ab[0] * v, a[1] + ab[1] * v, a[2] + ab[2] * v]
        } else if vb <= 0. && d2 >= 0. && d6 <= 0. {
            let w = d2 / (d2 - d6);
            [a[0] + ac[0] * w, a[1] + ac[1] * w, a[2] + ac[2] * w]
        } else if va <= 0. && d4 - d3 >= 0. && d5 - d6 >= 0. {
            let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
            [
                b[0] + (c[0] - b[0]) * w,
                b[1] + (c[1] - b[1]) * w,
                b[2] + (c[2] - b[2]) * w,
            ]
        } else {
            let d = 1. / (va + vb + vc);
            let (v, w) = (vb * d, vc * d);
            [
                a[0] + ab[0] * v + ac[0] * w,
                a[1] + ab[1] * v + ac[1] * w,
                a[2] + ab[2] * v + ac[2] * w,
            ]
        };
        dot(sub(q, point), sub(q, point)).sqrt()
    };
    let mut widest = 0f64;
    for (t, k) in triangles.iter().zip(&keys) {
        let fold = groups[&face(*k).0] > 1 || (0..3).any(|j| edges[&(k[j], k[(j + 1) % 3])] > 1);
        if !fold || k[0] == k[1] || k[1] == k[2] || k[0] == k[2] {
            continue;
        }
        let v = t.map(|i| p(positions[i as usize]));
        let n = cross(sub(v[1], v[0]), sub(v[2], v[0]));
        let mid =
            |a: [f64; 3], b: [f64; 3]| [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2., (a[2] + b[2]) / 2.];
        let centre = [0, 1, 2].map(|i| (v[0][i] + v[1][i] + v[2][i]) / 3.);
        for q in [
            v[0],
            v[1],
            v[2],
            mid(v[0], v[1]),
            mid(v[1], v[2]),
            mid(v[2], v[0]),
            centre,
        ] {
            let mut side = [reach; 2];
            let c = at(q);
            for x in c[0] - 1..=c[0] + 1 {
                for y in c[1] - 1..=c[1] + 1 {
                    for z in c[2] - 1..=c[2] + 1 {
                        for &i in grid.get(&[x, y, z]).into_iter().flatten() {
                            let f = &reference[i];
                            let m = cross(sub(p(f.1), p(f.0)), sub(p(f.2), p(f.0)));
                            let s = usize::from(dot(m, n) < 0.);
                            side[s] = side[s].min(closest(q, f));
                        }
                    }
                }
            }
            widest = widest.max(side[0] + side[1]);
        }
    }
    widest
}

#[cfg(test)]
thread_local! {
    pub(super) static VERTEX_FLAGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
#[path = "folds_compact_tests.rs"]
mod compact_tests;

/// Retain a local carrier before reduction can join two shared contacts. Pinning
/// the free common neighbour is smaller than later freezing both contact stars.
/// The proposal guard still checks multi-step collapses and every changed fan.
pub(super) fn contact_pins(
    keys: &[Key],
    triangles: &[[u32; 3]],
    interface: &[crate::mesh_large::hlod::interfaces::InterfaceRow],
) -> Vec<bool> {
    let _cpu = super::super::cpu_profile::start(
        super::super::cpu_profile::Stage::Contacts,
        triangles.len(),
    );
    let boundary = super::interface_points(interface);
    let mut first = vec![u32::MAX; keys.len()];
    let mut pins = vec![false; keys.len()];
    for t in triangles {
        for i in 0..3 {
            let v = t[i] as usize;
            if boundary.contains(&keys[v]) {
                continue;
            }
            for j in [1, 2] {
                let other = t[(i + j) % 3];
                if boundary.contains(&keys[other as usize]) {
                    if first[v] == u32::MAX {
                        first[v] = other;
                    } else if first[v] != other {
                        pins[v] = true;
                    }
                }
            }
        }
    }
    pins
}

fn distinct_sides(edge: Edge, thirds: impl Iterator<Item = Key>) -> bool {
    let _cpu = super::super::cpu_profile::start(super::super::cpu_profile::Stage::ExactFans, 1);
    let mut normals = Vec::<[Vec<f64>; 3]>::with_capacity(4);
    for c in thirds {
        if normals.len() == 4 {
            return false;
        }
        let n = exact_normal(edge, c);
        if n.iter().all(Vec::is_empty) || normals.iter().any(|u| acute(u, &n)) {
            return false;
        }
        normals.push(n);
    }
    true
}

/// Grow an exact nonoverlapping expansion in place (TwoSum, zero elimination).
/// Degree-four terms over finite f32 coordinates are multiples of 2^-596 with
/// magnitudes below 2^522, safely inside f64's normal range, including residuals.
fn add(expansion: &mut Vec<f64>, mut q: f64) {
    let mut written = 0;
    for i in 0..expansion.len() {
        let e = expansion[i];
        let sum = q + e;
        let b = sum - q;
        let residual = (q - (sum - b)) + (e - b);
        if residual != 0. {
            expansion[written] = residual;
            written += 1;
        }
        q = sum;
    }
    expansion.truncate(written);
    if q != 0. {
        expansion.push(q);
    }
}

/// (b-a) cross (c-a), with differences expanded before multiplication. A pair
/// of f32 coordinates multiplies exactly in f64, even across its entire exponent
/// range. Keeping the cancellation terms avoids a translation-dependent epsilon.
fn exact_normal(edge: Edge, c: Key) -> [Vec<f64>; 3] {
    let p = |k: Key| k.map(|x| f64::from(f32::from_bits(x)));
    let (a, b, c) = (p(edge.0), p(edge.1), p(c));
    std::array::from_fn(|i| {
        let (j, k) = ((i + 1) % 3, (i + 2) % 3);
        let mut terms = Vec::with_capacity(8);
        for (j, k, sign) in [(j, k, 1.), (k, j, -1.)] {
            for x in [b[j], -a[j]] {
                for y in [c[k], -a[k]] {
                    add(&mut terms, sign * x * y);
                }
            }
        }
        terms
    })
}

/// Cross-product dot sign equals the perpendicular wings' dot sign. Split every
/// product with its exact fused remainder; no angular tolerance or normalization.
fn acute(a: &[Vec<f64>; 3], b: &[Vec<f64>; 3]) -> bool {
    let mut sum = Vec::new();
    for (a, b) in a.iter().zip(b) {
        for &a in a {
            for &b in b {
                let p = a * b;
                add(&mut sum, a.mul_add(b, -p));
                add(&mut sum, p);
            }
        }
    }
    sum.last().is_some_and(|&x| x > 0.)
}

/// Encoding keeps face correspondence, so test every normal as well as multiplicity.
/// A reversed ordinary 1/1 edge is otherwise invisible to incidence counts.
pub(crate) fn decode_valid(
    before: &[[f32; 3]],
    after: &[[f32; 3]],
    triangles: &[[u32; 3]],
    boundary: &[crate::mesh_large::hlod::interfaces::InterfaceRow],
) -> bool {
    decode_valid_with(before, after, triangles, boundary, &Policy::default())
}

/// `decode_valid` under a last-resort policy.
pub(crate) fn decode_valid_with(
    before: &[[f32; 3]],
    after: &[[f32; 3]],
    triangles: &[[u32; 3]],
    boundary: &[crate::mesh_large::hlod::interfaces::InterfaceRow],
    policy: &Policy,
) -> bool {
    let _cpu =
        super::super::cpu_profile::start(super::super::cpu_profile::Stage::Decode, triangles.len());
    // An identity decode has exactly the source face/edge multiplicities and
    // oriented-face licenses. Finite f32 normals have positive finite f64 self-dot
    // whenever nonzero. Compare bits because signed zero is part of a canonical key.
    if before.len() == after.len()
        && before.iter().zip(after).all(|(a, b)| {
            a.iter()
                .zip(b)
                .all(|(a, b)| a.is_finite() && a.to_bits() == b.to_bits())
        })
    {
        return true;
    }
    decode_changed(before, after, triangles, boundary, policy)
}

fn decode_changed(
    before: &[[f32; 3]],
    after: &[[f32; 3]],
    triangles: &[[u32; 3]],
    boundary: &[crate::mesh_large::hlod::interfaces::InterfaceRow],
    policy: &Policy,
) -> bool {
    // New decoded keys can acquire an owner in another tile, outside the source
    // incidence graph. Retain canonical source keys; merging onto an existing key
    // is still subject to the face, boundary-edge and orientation checks below.
    let keys: HashSet<_> = before.iter().copied().map(key).collect();
    if after.iter().any(|&p| !keys.contains(&key(p))) {
        return false;
    }
    for &t in triangles {
        let a = super::topology::normal(before, t);
        let b = super::topology::normal(after, t);
        let old_area = a.iter().any(|&x| x != 0.);
        let new_area = b.iter().any(|&x| x != 0.);
        // A last-resort tile admits folds; a face decoding to zero area never.
        if old_area != new_area
            || (old_area && !policy.folds && a.iter().zip(b).map(|(a, b)| a * b).sum::<f64>() <= 0.)
        {
            return false;
        }
    }
    !Source::new(before, triangles)
        .with_boundary(boundary)
        .with_policy(policy.clone())
        .violations(after, triangles)
        .iter()
        .any(|v| *v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_decode_agrees_with_full_guard_across_finite_f32_domain() {
        // Exercise the unchanged full guard, including degenerate faces, twins,
        // multiplicity, acute books, signed zero and extreme/subnormal exponents.
        for exponent in [-149, -126, -80, -1, 0, 60, 100, 125] {
            let s = 2f64.powi(exponent) as f32;
            let p = [
                [0., -0., 0.],
                [s, 0., 0.],
                [0., s, 0.],
                [0., s, s],
                [-s, -s, -s],
                [f32::MAX, f32::MIN_POSITIVE, -f32::MAX],
            ];
            let faces = [
                [0, 1, 2],
                [0, 1, 3],
                [1, 0, 2],
                [1, 0, 4],
                [0, 1, 2],
                [0, 0, 0],
                [2, 5, 4],
            ];
            for n in 0..=faces.len() {
                for boundary in [Vec::new(), boundary(&p)] {
                    assert!(decode_changed(
                        &p,
                        &p,
                        &faces[..n],
                        &boundary,
                        &Policy::default()
                    ));
                    assert!(decode_valid(&p, &p, &faces[..n], &boundary));
                }
            }
            for v in 0..p.len() {
                let mut changed = p;
                changed[v] = p[(v + 1) % p.len()];
                assert_eq!(
                    decode_valid(&p, &changed, &faces, &boundary(&p)),
                    decode_changed(&p, &changed, &faces, &boundary(&p), &Policy::default()),
                );
            }
        }
    }

    fn boundary(points: &[[f32; 3]]) -> Vec<crate::mesh_large::hlod::interfaces::InterfaceRow> {
        points
            .iter()
            .map(|&p| {
                crate::mesh_large::hlod::interfaces::InterfaceRow::new(0, key(p), key(p), 0, 1)
            })
            .collect()
    }

    #[test]
    fn neighbouring_children_cannot_create_a_common_new_edge_or_face() {
        let p = [
            [0., 0., 0.],
            [0., 1., 0.],
            [2., 0., -1.],
            [3., 0., -1.],
            [0., 0.5, -0.125],
            [2., 0., 1.],
            [3., 0., 1.],
            [0., 0.5, 0.125],
        ];
        let contacts = boundary(&p[..2]);
        for (c, d, m) in [(2, 3, 4), (5, 6, 7)] {
            let input = [
                [0, m, c],
                [m, 1, c],
                [1, m, d],
                [m, 0, d],
                [0, c, d],
                [1, d, c],
            ];
            let output = [[0, 1, c], [1, 0, d], [0, c, d], [1, d, c]];
            // Each child alone looks like an ordinary 1/1 tetrahedral sheet.
            assert!(!Source::new(&p, &input)
                .violations(&p, &output)
                .iter()
                .any(|v| *v));
            let source = Source::new(&p, &input).with_boundary(&contacts);
            assert!(source.violations(&p, &output).iter().any(|v| *v));
            assert!(!source.violations(&p, &input).iter().any(|v| *v));
        }
        let p = [
            [0., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [0.25, 0.25, 0.001],
        ];
        let contacts = boundary(&p[..3]);
        let fan = [[0, 1, 3], [1, 2, 3], [2, 0, 3]];
        // All three old boundary edges exist, so the face guard is needed too.
        assert!(Source::new(&p, &fan)
            .with_boundary(&contacts)
            .violations(&p, &[[0, 1, 2]])
            .iter()
            .any(|v| *v));
        assert!(!Source::new(&p, &[[0, 1, 2]])
            .with_boundary(&contacts)
            .violations(&p, &[[0, 1, 2]])
            .iter()
            .any(|v| *v));
    }

    #[test]
    fn contact_carriers_pin_the_free_neighbour_not_whole_shared_stars() {
        let p = [
            [0., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [0.25, 0.25, 0.01],
            [2., 0., 0.],
            [2., 1., 0.],
        ];
        let keys: Vec<_> = p.into_iter().map(key).collect();
        // Vertex 3 joins contacts 0 and 1. Vertex 4 sees contact 0 repeatedly,
        // but has only that one contact and must remain free to simplify.
        let triangles = [[0, 3, 2], [1, 2, 3], [0, 4, 5], [0, 5, 4]];
        assert_eq!(
            contact_pins(&keys, &triangles, &boundary(&p[..2])),
            [false, false, true, true, false, false]
        );
    }

    #[test]
    fn decoding_cannot_invent_owner_keys_or_close_a_shared_face() {
        let before = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0.001, 0.001, 0.]];
        let triangles = [[3, 1, 2]];
        let mut after = before;
        after[3] = [0.002, 0.002, 0.];
        assert!(!decode_valid(&before, &after, &triangles, &[]));
        after[3] = before[0];
        assert!(decode_valid(&before, &after, &triangles, &[]));
        assert!(!decode_valid(
            &before,
            &after,
            &triangles,
            &boundary(&before[..3])
        ));
    }

    #[test]
    fn balanced_fans_check_both_sides_and_cannot_inherit_a_new_acute_wing() {
        let mut p = [
            [0., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [0., 0., 1.],
            [0., -1., 0.],
            [0., 0., -1.],
        ];
        let source = Source::new(&p, &[[0, 1, 2], [1, 0, 4]]);
        let fan = [[0, 1, 2], [0, 1, 3], [1, 0, 4], [1, 0, 5]];
        assert!(!source.violations(&p, &fan).iter().any(|v| *v));
        let child = Source::new(&p, &fan);
        for (corner, side) in [(3, 1.), (5, -1.)] {
            let before = p[corner];
            // Positive but far below the old 1e-6 angular allowance.
            p[corner][1] = side * f32::from_bits(1);
            assert!(source.violations(&p, &fan).iter().any(|v| *v));
            assert!(child.violations(&p, &fan).iter().any(|v| *v));
            p[corner] = before;
        }
        // Original acute nonmanifold sheets remain legal when kept exactly.
        p[3][1] = 0.5;
        assert!(!Source::new(&p, &fan)
            .violations(&p, &fan)
            .iter()
            .any(|v| *v));
    }

    #[test]
    fn wing_sign_handles_full_f32_range_and_exact_right_angles() {
        for (large, small) in [(f32::MAX, f32::from_bits(1)), (1., 1.), (1e-30, 1e30)] {
            let edge = (key([large, small, 0.]), key([-large, small, 0.]));
            let a = exact_normal(edge, key([0., small, small]));
            for (z, expected) in [(-small, false), (0., false), (small, true)] {
                let b = exact_normal(edge, key([0., 2. * small, z]));
                assert_eq!(acute(&a, &b), expected);
                assert!(a.iter().any(|v| !v.is_empty()));
                assert!(b.iter().any(|v| !v.is_empty()));
            }
        }
    }

    #[test]
    fn expansion_wing_sign_matches_integer_reference() {
        let mut seed = 117u64;
        let mut next = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            ((seed >> 32) as i32 % (1 << 20)) as i128
        };
        for _ in 0..128 {
            let p: [[i128; 3]; 4] = std::array::from_fn(|_| std::array::from_fn(|_| next()));
            let normal = |c: usize| {
                let d: [i128; 3] = std::array::from_fn(|k| p[1][k] - p[0][k]);
                let u: [i128; 3] = std::array::from_fn(|k| p[c][k] - p[0][k]);
                std::array::from_fn::<_, 3, _>(|i| {
                    let (j, k) = ((i + 1) % 3, (i + 2) % 3);
                    d[j] * u[k] - d[k] * u[j]
                })
            };
            let expected = normal(2)
                .iter()
                .zip(normal(3))
                .map(|(a, b)| a * b)
                .sum::<i128>();
            let keys = p.map(|p| key(p.map(|v| v as f32)));
            let a = exact_normal((keys[0], keys[1]), keys[2]);
            let b = exact_normal((keys[0], keys[1]), keys[3]);
            assert_eq!(acute(&a, &b), expected > 0);
        }
    }

    #[test]
    fn new_twins_and_acute_flaps_are_rejected_but_source_sheets_survive() {
        let p = [
            [0., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [0., 1., 0.015],
            [0., -1., 0.],
            [0., 0., 1.],
        ];
        let source = Source::new(&p, &[[0, 1, 2]]);
        for output in [
            vec![[0, 1, 2], [0, 2, 1]],
            vec![[0, 1, 2], [0, 1, 2]],
            vec![[0, 1, 2], [0, 1, 3], [1, 0, 4]],
        ] {
            assert!(source.violations(&p, &output).iter().any(|v| *v));
        }
        let book = vec![[0, 1, 2], [0, 1, 5], [1, 0, 4]];
        assert!(!source.violations(&p, &book).iter().any(|v| *v));
        let twins = vec![[0, 1, 2], [0, 2, 1]];
        assert!(!Source::new(&p, &twins)
            .violations(&p, &twins)
            .iter()
            .any(|v| *v));
    }

    #[test]
    fn final_decode_coincidence_is_detected_even_with_distinct_vertex_indices() {
        let p = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 1., 0.001]];
        let triangles = [[0, 1, 2], [1, 0, 3]];
        let source = Source::new(&p, &triangles);
        assert!(!source.violations(&p, &triangles).iter().any(|v| *v));
        let mut decoded = p;
        decoded[3] = decoded[2];
        assert!(source.violations(&decoded, &triangles).iter().any(|v| *v));
    }
    #[test]
    fn final_decode_cannot_invert_an_ordinary_manifold_face() {
        let p = [
            [0., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [0., -1., 0.],
            [0., 0.9, 0.01],
        ];
        let triangles = [[0, 1, 2], [1, 0, 3]];
        let mut folded = p;
        folded[3] = p[4];
        assert!(decode_valid(&p, &p, &triangles, &[]));
        assert!(!decode_valid(&p, &folded, &triangles, &[]));
    }
}
