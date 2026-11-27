//! Coarse self-intersections (#420, owner decision 2026-11-20): reported, never refused.
//!
//! Two distinct faces of one coarse tile may meet only in the simplex of their common
//! corners: nowhere with none, at the shared vertex with one, along the shared edge with
//! two. A pair that meets anywhere else is *improper*. Coarse tiles may self-intersect
//! within their certified geometricError, which bounds the two-sided distance to the
//! source, so this census never fails validation; it counts improper pairs per coarse tile
//! and splits them into pairs *present* in the tile's immediate children (both faces are
//! child faces, so the children cross there too; children that are leaves are the source)
//! and *new* ones.
//!
//! Every test is exact: `robust`'s adaptive orientation predicates on the f64 values of
//! the canonical f32 keys, with no tolerance. Faces are the drops recount's (three distinct
//! canonical corners); a collinear face is its hull segment. Two faces with the same three
//! corners are coincident (a fold, counted by `lastResort` and the border rules) and are not
//! counted here.
//!
//! The census has its own bounds per tile, never the validation's budgets: work
//! ([`tile_budget`]) and workspace ([`workspace_bytes`], within half the decoded-document
//! allowance the coarse replay has released). A tile over either is counted as incomplete,
//! without partial counts, so no input can turn the census into a refusal. Faces are
//! distinct by corner set, and "present" is judged against the children together: the
//! coarse tile replaces them together, and leaf children together are the source.
use super::{CanonicalPoint, Effort, Error};
use robust::{orient2d, orient3d, Coord, Coord3D};
use std::collections::HashSet;

type P = [f64; 3];

/// The listed tiles: those with the most new pairs, then present pairs, then lowest id.
pub(super) const LISTED_TILES: usize = 16;
/// Work units, against the per-tile budget: a grid cell insert, a candidate pair's box test,
/// and the exact test of a pair whose boxes overlap.
const CELL_UNITS: u64 = 1;
/// A face whose box spans more grid cells than this is a large face.
const LARGE_CELLS: f64 = 64.;
const CANDIDATE_UNITS: u64 = 1;
const EXACT_UNITS: u64 = 64;
/// A tile's crossing budget: a fixed allowance and a share per face.
const TILE_FLOOR_UNITS: u64 = 1 << 24;
const FACE_UNITS: u64 = 1 << 15;

pub(super) fn tile_budget(faces: usize) -> u64 {
    TILE_FLOOR_UNITS.saturating_add(FACE_UNITS.saturating_mul(faces as u64))
}

/// One listed tile: its preorder id (as in validation messages), cell depth, faces,
/// geometricError and improper pairs.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CrossingTile {
    pub(crate) tile: i64,
    pub(crate) depth: usize,
    pub(crate) faces: u64,
    pub(crate) geometric_error: f64,
    pub(crate) present: u64,
    pub(crate) new: u64,
}

// Validated geometricErrors are finite, so equality is an equivalence.
impl Eq for CrossingTile {}

/// The census over every coarse tile.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CoarseCrossings {
    /// Coarse tiles checked in full, and their faces.
    pub(crate) tiles: u64,
    pub(crate) faces: u64,
    /// Coarse tiles over the census's work or workspace bound (not counted below).
    pub(crate) incomplete_tiles: u64,
    /// Tiles with an improper pair, and with a new one.
    pub(crate) crossing_tiles: u64,
    pub(crate) new_tiles: u64,
    /// Improper pairs by cell depth.
    pub(crate) present_by_depth: Vec<u64>,
    pub(crate) new_by_depth: Vec<u64>,
    /// At most [`LISTED_TILES`] tiles, most new pairs first.
    pub(crate) listed: Vec<CrossingTile>,
}

impl CoarseCrossings {
    /// Census one coarse tile from its faces (one entry per placement) and its immediate
    /// children's faces together, by sorted corners. `workspace` bounds the census's
    /// memory ([`workspace_bytes`]); a tile over it or over its work budget is incomplete.
    pub(super) fn tile(
        &mut self,
        (tile, depth, geometric_error): (i64, usize, f64),
        entries: &[[CanonicalPoint; 3]],
        children: &HashSet<[CanonicalPoint; 3]>,
        workspace: usize,
        effort: &Effort,
    ) -> Result<(), Error> {
        let sorted = |f: [CanonicalPoint; 3]| {
            let mut k = f;
            k.sort_unstable();
            k
        };
        // The bound first, on the entries: nothing is built for a tile over it.
        if workspace_bytes(entries.len()) > workspace {
            self.incomplete_tiles += 1;
            return Ok(());
        }
        // Distinct faces by corner set, in first-entry order: a placement repeated at the
        // same position, or a repeated or reversed triangle, is one face.
        let mut seen = HashSet::new();
        let faces: Vec<[CanonicalPoint; 3]> = entries
            .iter()
            .copied()
            .filter(|&f| seen.insert(sorted(f)))
            .collect();
        drop(seen);
        let is_child: Vec<bool> = faces
            .iter()
            .map(|&f| children.contains(&sorted(f)))
            .collect();
        let (mut present, mut new) = (0u64, 0u64);
        let complete = improper_pairs(&faces, tile_budget(faces.len()), effort, &mut |i, j| {
            if is_child[i] && is_child[j] {
                present += 1;
            } else {
                new += 1;
            }
        })?;
        if !complete {
            self.incomplete_tiles += 1;
            return Ok(());
        }
        self.tiles += 1;
        self.faces += faces.len() as u64;
        if present + new == 0 {
            return Ok(());
        }
        self.crossing_tiles += 1;
        self.new_tiles += u64::from(new > 0);
        for v in [&mut self.present_by_depth, &mut self.new_by_depth] {
            if v.len() <= depth {
                v.resize(depth + 1, 0);
            }
        }
        self.present_by_depth[depth] += present;
        self.new_by_depth[depth] += new;
        self.listed.push(CrossingTile {
            tile,
            depth,
            faces: faces.len() as u64,
            geometric_error,
            present,
            new,
        });
        self.listed
            .sort_by(|a, b| (b.new, b.present, a.tile).cmp(&(a.new, a.present, b.tile)));
        self.listed.truncate(LISTED_TILES);
        Ok(())
    }
}

/// The census's largest workspace for a tile of `entries` face entries, per entry:
/// - the distinct-face set (a hash table at most three slots per face, control byte
///   included) and the distinct faces (a vector grown by doubling, so at most twice);
/// - the face's box, extent, grid span and two flags (vectors of exact length);
/// - up to [`LARGE_CELLS`] grid entries, a cell key and a face index each, in a vector
///   reserved exactly.
pub(super) fn workspace_bytes(entries: usize) -> usize {
    use std::mem::size_of;
    let face = size_of::<[CanonicalPoint; 3]>();
    let per = 3 * (face + 1)
        + 2 * face
        + size_of::<[P; 2]>()
        + size_of::<f64>()
        + size_of::<Option<([f64; 3], [f64; 3])>>()
        + 2 * size_of::<bool>()
        + GRID_ENTRIES * size_of::<([u64; 3], usize)>();
    entries.saturating_mul(per)
}
/// Grid entries per face at most ([`LARGE_CELLS`]).
const GRID_ENTRIES: usize = 64;

fn point(k: CanonicalPoint) -> P {
    k.map(|b| f64::from(f32::from_bits(b)))
}

/// Reports each improper pair `(i, j)`, `i < j`, of `faces` once to `found`; false when
/// the work exceeds `budget` units (the pairs reported so far are then partial). A uniform
/// grid at the faces' median box extent finds the candidates; each overlapping pair is tested
/// once, in the cell holding the lower corner of the boxes' overlap. A face spanning more than
/// [`LARGE_CELLS`] cells is tested against every face instead.
pub(super) fn improper_pairs(
    faces: &[[CanonicalPoint; 3]],
    budget: u64,
    effort: &Effort,
    found: &mut dyn FnMut(usize, usize),
) -> Result<bool, Error> {
    let mut used = 0u64;
    let mut spend = |n: u64| -> Result<bool, Error> {
        let before = used;
        used = used.saturating_add(n);
        // A cancellation checkpoint every 2^16 units.
        if before >> 16 != used >> 16 {
            effort.ticks.check()?;
        }
        Ok(used <= budget)
    };
    if faces.len() < 2 {
        return Ok(true);
    }
    let boxes: Vec<[P; 2]> = faces
        .iter()
        .map(|f| {
            let p = f.map(point);
            let lo = std::array::from_fn(|k| p[0][k].min(p[1][k]).min(p[2][k]));
            let hi = std::array::from_fn(|k| p[0][k].max(p[1][k]).max(p[2][k]));
            [lo, hi]
        })
        .collect();
    let origin: P = std::array::from_fn(|k| boxes.iter().map(|b| b[0][k]).fold(f64::MAX, f64::min));
    // Cells at the median face extent: a few large faces span many cells (or go to the
    // large list) rather than crowding every small face into a few cells.
    let mut extents: Vec<f64> = boxes
        .iter()
        .map(|b| (0..3).map(|k| b[1][k] - b[0][k]).fold(0., f64::max))
        .collect();
    let mid = extents.len() / 2;
    let (_, &mut extent, _) = extents.select_nth_unstable_by(mid, f64::total_cmp);
    drop(extents);
    // Distinct canonical points differ, so every face box has a positive extent; the floor
    // only guards a subnormal one.
    let h = extent.max(f64::MIN_POSITIVE);
    // Cells are floored f64 coordinates (integers), keyed by their bits.
    let cell = |p: P| -> [f64; 3] { std::array::from_fn(|k| ((p[k] - origin[k]) / h).floor()) };
    let key = |c: [f64; 3]| c.map(f64::to_bits);
    // Grid entries (cell, face), sorted: each face at most once per cell.
    // At most `LARGE_CELLS` per axis; where an offset is below the coordinates'
    // resolution a cell repeats, and repeats are removed.
    let steps = |lo: f64, hi: f64| {
        (0u32..64)
            .map(move |d| lo + f64::from(d))
            .take_while(move |&x| x <= hi)
    };
    let spans: Vec<Option<([f64; 3], [f64; 3])>> = boxes
        .iter()
        .map(|b| {
            let (lo, hi) = (cell(b[0]), cell(b[1]));
            ((0..3).map(|k| hi[k] - lo[k] + 1.).product::<f64>() <= LARGE_CELLS).then_some((lo, hi))
        })
        .collect();
    // Reserved exactly, so the vector never grows past its count.
    let count: usize = spans
        .iter()
        .flatten()
        .map(|(lo, hi)| {
            (0..3)
                .map(|k| steps(lo[k], hi[k]).count())
                .product::<usize>()
        })
        .sum();
    let mut entries: Vec<([u64; 3], usize)> = Vec::with_capacity(count);
    let mut large: Vec<usize> = Vec::new();
    let mut keys: Vec<[u64; 3]> = Vec::with_capacity(GRID_ENTRIES);
    for (i, span) in spans.iter().enumerate() {
        let Some((lo, hi)) = *span else {
            large.push(i);
            continue;
        };
        let steps = |k: usize| steps(lo[k], hi[k]);
        keys.clear();
        for x in steps(0) {
            for y in steps(1) {
                for z in steps(2) {
                    if !spend(CELL_UNITS)? {
                        return Ok(false);
                    }
                    keys.push(key([x, y, z]));
                }
            }
        }
        keys.sort_unstable();
        keys.dedup();
        entries.extend(keys.iter().map(|&k| (k, i)));
    }
    entries.sort_unstable();
    let mut is_large = vec![false; faces.len()];
    for &i in &large {
        is_large[i] = true;
    }
    let overlap = |a: &[P; 2], b: &[P; 2]| (0..3).all(|k| a[0][k] <= b[1][k] && b[0][k] <= a[1][k]);
    for &i in &large {
        for j in 0..faces.len() {
            // Each pair once: a large face against every small face, and against the
            // large faces after it.
            if j == i || (is_large[j] && j < i) {
                continue;
            }
            if !spend(CANDIDATE_UNITS)? {
                return Ok(false);
            }
            if !overlap(&boxes[i], &boxes[j]) {
                continue;
            }
            if !spend(EXACT_UNITS)? {
                return Ok(false);
            }
            if improper(faces[i], faces[j]) {
                found(i.min(j), i.max(j));
            }
        }
    }
    for run in entries.chunk_by(|a, b| a.0 == b.0) {
        let c = run[0].0;
        for (n, &(_, i)) in run.iter().enumerate() {
            for &(_, j) in &run[n + 1..] {
                if !spend(CANDIDATE_UNITS)? {
                    return Ok(false);
                }
                let (a, b) = (&boxes[i], &boxes[j]);
                if !overlap(a, b) {
                    continue;
                }
                let corner: P = std::array::from_fn(|k| a[0][k].max(b[0][k]));
                if key(cell(corner)) != c {
                    continue;
                }
                if !spend(EXACT_UNITS)? {
                    return Ok(false);
                }
                if improper(faces[i], faces[j]) {
                    found(i.min(j), i.max(j));
                }
            }
        }
    }
    Ok(true)
}

fn sign(x: f64) -> i8 {
    if x > 0. {
        1
    } else if x < 0. {
        -1
    } else {
        0
    }
}

fn o3(a: P, b: P, c: P, d: P) -> i8 {
    let c3 = |p: P| Coord3D {
        x: p[0],
        y: p[1],
        z: p[2],
    };
    sign(orient3d(c3(a), c3(b), c3(c), c3(d)))
}

/// The orientation of `a, b, c` with coordinate `drop` removed.
fn o2(a: P, b: P, c: P, drop: usize) -> i8 {
    let (i, j) = [(1, 2), (0, 2), (0, 1)][drop];
    let c2 = |p: P| Coord { x: p[i], y: p[j] };
    sign(orient2d(c2(a), c2(b), c2(c)))
}

/// `a, b, c` on one line: every coordinate projection is degenerate.
fn collinear(a: P, b: P, c: P) -> bool {
    (0..3).all(|d| o2(a, b, c, d) == 0)
}

/// A coordinate whose removal keeps the non-collinear `a, b, c` nondegenerate.
fn plane_axis(a: P, b: P, c: P) -> Option<usize> {
    (0..3).find(|&d| o2(a, b, c, d) != 0)
}

/// `p` within the box of `a, b` (on their line, the closed segment).
fn within(a: P, b: P, p: P) -> bool {
    (0..3).all(|k| a[k].min(b[k]) <= p[k] && p[k] <= a[k].max(b[k]))
}

/// A face's point set: a triangle, or for three collinear corners the segment between the
/// outer two.
#[derive(Clone, Copy, Debug)]
enum Hull {
    Point(P),
    Segment(P, P),
    Triangle([P; 3]),
}

/// The corner of three collinear points lying between the other two.
fn middle(p: [P; 3]) -> usize {
    let k = (0..3)
        .find(|&k| p[0][k] != p[1][k] || p[0][k] != p[2][k])
        .unwrap_or(0);
    (0..3)
        .find(|&i| {
            let (a, b) = (p[(i + 1) % 3][k], p[(i + 2) % 3][k]);
            a.min(b) <= p[i][k] && p[i][k] <= a.max(b)
        })
        .unwrap_or(0)
}

fn hull(p: [P; 3]) -> Hull {
    if collinear(p[0], p[1], p[2]) {
        let m = middle(p);
        Hull::Segment(p[(m + 1) % 3], p[(m + 2) % 3])
    } else {
        Hull::Triangle(p)
    }
}

/// Point `p` in the closed nondegenerate triangle `t`.
fn in_triangle(p: P, t: [P; 3]) -> bool {
    if o3(t[0], t[1], t[2], p) != 0 {
        return false;
    }
    let Some(d) = plane_axis(t[0], t[1], t[2]) else {
        return false;
    };
    let s = [
        o2(t[0], t[1], p, d),
        o2(t[1], t[2], p, d),
        o2(t[2], t[0], p, d),
    ];
    !(s.contains(&1) && s.contains(&-1))
}

/// The closed segments `pq` and `rs` meet.
fn segments_meet(p: P, q: P, r: P, s: P) -> bool {
    if o3(p, q, r, s) != 0 {
        return false;
    }
    if collinear(p, q, r) && collinear(p, q, s) {
        // One line: compare along a coordinate that varies on it.
        let Some(k) = (0..3).find(|&k| p[k] != q[k]) else {
            return within(r, s, p);
        };
        return p[k].min(q[k]) <= r[k].max(s[k]) && r[k].min(s[k]) <= p[k].max(q[k]);
    }
    // One plane; a projection that keeps it faithful.
    let Some(d) = [(p, q, r), (p, q, s), (r, s, p), (r, s, q)]
        .into_iter()
        .find_map(|(a, b, c)| plane_axis(a, b, c))
    else {
        return false;
    };
    let (d1, d2) = (o2(p, q, r, d), o2(p, q, s, d));
    let (d3, d4) = (o2(r, s, p, d), o2(r, s, q, d));
    if d1 * d2 < 0 && d3 * d4 < 0 {
        return true;
    }
    (d1 == 0 && within(p, q, r))
        || (d2 == 0 && within(p, q, s))
        || (d3 == 0 && within(r, s, p))
        || (d4 == 0 && within(r, s, q))
}

/// The closed segment `pq` meets the closed nondegenerate triangle `t`.
fn segment_meets_triangle(p: P, q: P, t: [P; 3]) -> bool {
    let (s1, s2) = (o3(t[0], t[1], t[2], p), o3(t[0], t[1], t[2], q));
    if s1 == s2 && s1 != 0 {
        return false;
    }
    if s1 == 0 && s2 == 0 {
        return in_triangle(p, t)
            || in_triangle(q, t)
            || (0..3).any(|i| segments_meet(p, q, t[i], t[(i + 1) % 3]));
    }
    // pq reaches the plane at one point: inside the closed triangle unless the line passes
    // two of its edges on opposite sides.
    let e = [
        o3(p, q, t[0], t[1]),
        o3(p, q, t[1], t[2]),
        o3(p, q, t[2], t[0]),
    ];
    !(e.contains(&1) && e.contains(&-1))
}

/// The closed hulls `a` and `b` meet.
fn meets(a: Hull, b: Hull) -> bool {
    use Hull::{Point, Segment, Triangle};
    match (a, b) {
        (Point(p), Point(q)) => p == q,
        (Point(p), Segment(r, s)) | (Segment(r, s), Point(p)) => {
            collinear(r, s, p) && within(r, s, p)
        }
        (Point(p), Triangle(t)) | (Triangle(t), Point(p)) => in_triangle(p, t),
        (Segment(p, q), Segment(r, s)) => segments_meet(p, q, r, s),
        (Segment(p, q), Triangle(t)) | (Triangle(t), Segment(p, q)) => {
            segment_meets_triangle(p, q, t)
        }
        // Two closed triangles meet exactly when an edge of one meets the other.
        (Triangle(s), Triangle(t)) => (0..3).any(|i| {
            segment_meets_triangle(s[i], s[(i + 1) % 3], t)
                || segment_meets_triangle(t[i], t[(i + 1) % 3], s)
        }),
    }
}

/// The parts of face `p` (corners `k`) away from its corner `v`: any point of the face
/// other than `v` lies on a ray from `v` that leaves the face through one of them.
fn away(p: [P; 3], k: [CanonicalPoint; 3], v: CanonicalPoint) -> Vec<Hull> {
    let i = k.iter().position(|&c| c == v).unwrap_or(0);
    let (a, b) = (p[(i + 1) % 3], p[(i + 2) % 3]);
    if !collinear(p[0], p[1], p[2]) {
        return vec![Hull::Segment(a, b)];
    }
    if middle(p) == i {
        vec![Hull::Point(a), Hull::Point(b)]
    } else {
        let m = middle(p);
        // The outer corner other than `v`.
        let far = (0..3).find(|&j| j != i && j != m).unwrap_or(0);
        vec![Hull::Point(p[far])]
    }
}

/// Faces `f` and `g` (canonical corners, each distinct) meet outside the simplex of their
/// common corners.
pub(super) fn improper(f: [CanonicalPoint; 3], g: [CanonicalPoint; 3]) -> bool {
    let common: Vec<CanonicalPoint> = f.iter().copied().filter(|c| g.contains(c)).collect();
    let (pf, pg) = (f.map(point), g.map(point));
    match common.as_slice() {
        [] => meets(hull(pf), hull(pg)),
        // Their meeting is convex and holds v; it holds another point exactly when a part
        // of one face away from v meets the other face.
        &[v] => {
            away(pf, f, v).into_iter().any(|h| meets(h, hull(pg)))
                || away(pg, g, v).into_iter().any(|h| meets(h, hull(pf)))
        }
        &[u, v] => {
            let third = |k: [CanonicalPoint; 3], p: [P; 3]| {
                p[k.iter().position(|&c| c != u && c != v).unwrap_or(0)]
            };
            let (x, y) = (third(f, pf), third(g, pg));
            let (pu, pv) = (point(u), point(v));
            match (collinear(pu, pv, x), collinear(pu, pv, y)) {
                // Distinct planes through uv meet on its line, which each face holds only
                // on uv; on one plane the faces must lie on opposite sides of it.
                (false, false) => {
                    o3(pu, pv, x, y) == 0 && {
                        let d = plane_axis(pu, pv, x).unwrap_or(0);
                        o2(pu, pv, x, d) * o2(pu, pv, y, d) > 0
                    }
                }
                // A triangle meets the line of uv on uv alone.
                (true, false) | (false, true) => false,
                // Both on the line of uv: improper when both reach past the same end.
                (true, true) => {
                    let k = (0..3).find(|&k| pu[k] != pv[k]).unwrap_or(0);
                    let (lo, hi) = (pu[k].min(pv[k]), pu[k].max(pv[k]));
                    (x[k] > hi && y[k] > hi) || (x[k] < lo && y[k] < lo)
                }
            }
        }
        // Coincident faces are folds, not crossings.
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pairs `improper_pairs` reports, sorted; `None` when incomplete.
    fn pairs_of(
        faces: &[[CanonicalPoint; 3]],
        budget: u64,
        effort: &Effort,
    ) -> Result<Option<Vec<(usize, usize)>>, Error> {
        let mut pairs = Vec::new();
        let complete = improper_pairs(faces, budget, effort, &mut |i, j| pairs.push((i, j)))?;
        pairs.sort_unstable();
        let n = pairs.len();
        pairs.dedup();
        assert_eq!(n, pairs.len(), "a pair reported twice");
        Ok(complete.then_some(pairs))
    }
    fn key(p: [f64; 3]) -> CanonicalPoint {
        p.map(|v| {
            let v = v as f32;
            if v == 0. {
                0
            } else {
                v.to_bits()
            }
        })
    }
    fn face(p: [[f64; 3]; 3]) -> [CanonicalPoint; 3] {
        p.map(key)
    }
    fn improper_both(f: [[f64; 3]; 3], g: [[f64; 3]; 3]) -> bool {
        let (f, g) = (face(f), face(g));
        let one = improper(f, g);
        assert_eq!(one, improper(g, f), "{f:?} {g:?}");
        // Corner order does not matter.
        assert_eq!(one, improper([f[1], f[2], f[0]], [g[2], g[0], g[1]]));
        one
    }

    const O: [f64; 3] = [0., 0., 0.];
    const X: [f64; 3] = [2., 0., 0.];
    const Y: [f64; 3] = [0., 2., 0.];

    #[test]
    fn faces_without_common_corners_must_not_meet() {
        // Apart, piercing the interior, and touching an edge with a corner.
        assert!(!improper_both(
            [O, X, Y],
            [[3., 3., -1.], [3., 3., 1.], [4., 3., 0.]]
        ));
        assert!(improper_both(
            [O, X, Y],
            [[0.5, 0.5, -1.], [0.5, 0.5, 1.], [5., 5., 5.]]
        ));
        assert!(improper_both(
            [O, X, Y],
            [[1., 0., 0.], [1., -1., 1.], [1., -1., -1.]]
        ));
        // Coplanar: overlapping, and one inside the other without crossing edges.
        assert!(improper_both(
            [O, X, Y],
            [[1., 1., 0.], [-1., 1., 0.], [1., -1., 0.]]
        ));
        assert!(improper_both(
            [O, X, Y],
            [[0.2, 0.2, 0.], [0.6, 0.2, 0.], [0.2, 0.6, 0.]]
        ));
        // Coplanar and apart across the hypotenuse.
        assert!(!improper_both(
            [O, X, Y],
            [[2., 1., 0.], [1., 2., 0.], [2., 2., 0.]]
        ));
    }

    #[test]
    fn faces_with_a_common_vertex_meet_only_there() {
        // Apart through the vertex, and folded onto each other on one plane.
        assert!(!improper_both([O, X, Y], [O, [-1., 0., 1.], [0., -1., 1.]]));
        assert!(improper_both([O, X, Y], [O, [1., 0.5, 0.], [0.5, 1., 0.]]));
        // Running along an edge from the vertex, and crossing the far edge.
        assert!(improper_both([O, X, Y], [O, [1., 0., 0.], [0., -1., 1.]]));
        assert!(improper_both([O, X, Y], [O, [3., 3., 1.], [3., 3., -1.]]));
        // A shared vertex and a touch elsewhere: the face's far edge reaches the other.
        assert!(improper_both([O, X, Y], [O, [1., 1., 0.], [1., 1., 1.]]));
    }

    #[test]
    fn faces_on_a_common_edge_meet_only_along_it() {
        // Opposite sides on one plane, distinct planes, and folded onto one side.
        assert!(!improper_both([O, X, Y], [X, O, [1., -1., 0.]]));
        assert!(!improper_both([O, X, Y], [X, O, [1., 1., 1.]]));
        assert!(improper_both([O, X, Y], [X, O, [1., 0.5, 0.]]));
        // A coincident face is a fold, not a crossing.
        assert!(!improper_both([O, X, Y], [Y, X, O]));
    }

    #[test]
    fn collinear_faces_are_their_segments() {
        let line = [[-1., 0.5, 0.], [0.5, 0.5, 0.], [3., 0.5, 0.]];
        // Through the triangle, beside it, and piercing it from off its plane.
        assert!(improper_both([O, X, Y], line));
        assert!(!improper_both([O, X, Y], line.map(|p| [p[0], p[1], 1.])));
        assert!(improper_both(
            [O, X, Y],
            [[0.5, 0.5, -1.], [0.5, 0.5, 0.25], [0.5, 0.5, 1.]]
        ));
        // A common outer corner: along an edge of the triangle, then away from it.
        assert!(improper_both([O, X, Y], [O, [1., 0., 0.], [3., 0., 0.]]));
        assert!(!improper_both([O, X, Y], [O, [-1., 0., 0.], [-3., 0., 0.]]));
        // The common vertex in the middle: one half into the triangle.
        assert!(improper_both([O, X, Y], [[-1., -1., 0.], O, [1., 1., 0.]]));
        assert!(!improper_both(
            [O, X, Y],
            [[-1., -1., 1.], O, [1., 1., -1.]]
        ));
        // A common edge: a triangle meets its line on the edge alone.
        assert!(!improper_both([O, X, Y], [O, X, [3., 0., 0.]]));
        // Two collinear faces on one line through a common edge: past the same end, then
        // past opposite ends.
        assert!(improper_both([O, X, [3., 0., 0.]], [O, X, [4., 0., 0.]]));
        assert!(!improper_both([O, X, [3., 0., 0.]], [O, X, [-1., 0., 0.]]));
        // Two collinear faces crossing at a corner of one only, meeting only at a common
        // corner, and on parallel lines.
        assert!(improper_both(
            [[-1., 0., 0.], [0., 0., 0.5], [1., 0., 1.]],
            [[0., -1., 0.5], [0., 0.5, 0.5], [0., 1., 0.5]]
        ));
        assert!(!improper_both(
            [[-1., 0., 0.], [0., 0., 0.5], [1., 0., 1.]],
            [[0., -1., 0.5], [0., 0., 0.5], [0., 1., 0.5]]
        ));
        assert!(!improper_both(
            [[-1., 0., 0.], [0., 0., 0.], [1., 0., 0.]],
            [[-1., 1., 0.], [0., 1., 0.], [1., 1., 0.]]
        ));
    }

    const CHORD: [[f64; 3]; 3] = [[0.25, 0.125, 1.], [0.25, 0.125, -1.], [2., 0.75, 0.]];

    /// A floor of unit squares, two faces each.
    fn floor() -> Vec<[CanonicalPoint; 3]> {
        let mut faces = Vec::new();
        for i in 0..4 {
            for j in 0..4 {
                let (x, y) = (f64::from(i), f64::from(j));
                faces.push(face([[x, y, 0.], [x + 1., y, 0.], [x + 1., y + 1., 0.]]));
                faces.push(face([[x, y, 0.], [x + 1., y + 1., 0.], [x, y + 1., 0.]]));
            }
        }
        faces
    }

    #[test]
    fn a_chord_through_the_floor_is_found_once_per_face_it_meets() {
        let effort = super::super::tests::effort_for_tests();
        let mut faces = floor();
        assert_eq!(pairs_of(&faces, u64::MAX, &effort).unwrap(), Some(vec![]));
        // A chord face standing on the floor: it meets z = 0 on the segment from
        // (0.25, 0.125) to (2, 0.75), through the interiors of square (0, 0)'s lower face and
        // both of square (1, 0)'s, and ends on the edge x = 2, touching square (2, 0)'s upper
        // face there at a point that is no corner of it.
        faces.push(CHORD.map(key));
        let pairs = pairs_of(&faces, u64::MAX, &effort).unwrap().unwrap();
        let chord_at = faces.len() - 1;
        assert!(pairs.iter().all(|&(i, j)| j == chord_at && i < chord_at));
        let mut met: Vec<usize> = pairs.iter().map(|&(i, _)| i).collect();
        met.sort_unstable();
        met.dedup();
        assert_eq!(met.len(), pairs.len(), "each pair once");
        // A brute-force count agrees.
        let brute = (0..chord_at)
            .filter(|&i| improper(faces[i], faces[chord_at]))
            .count();
        assert_eq!(pairs.len(), brute);
        assert_eq!(brute, 4);
        // Its far corner on the floor's edge: square (2, 0)'s upper face is touched.
        let touched = face([[2., 0., 0.], [3., 1., 0.], [2., 1., 0.]]);
        let at = faces.iter().position(|&f| f == touched).unwrap();
        assert!(pairs.contains(&(at, chord_at)));
        // The same pairs in any face order.
        let mut reversed = faces.clone();
        reversed.reverse();
        let back = pairs_of(&reversed, u64::MAX, &effort).unwrap().unwrap();
        assert_eq!(back.len(), pairs.len());
        // A tiny budget leaves the tile incomplete, never refused.
        assert_eq!(pairs_of(&faces, 10, &effort).unwrap(), None);
    }

    /// Seeded small faces near a lattice (shared corners, shared edges, coplanar and
    /// collinear faces arise often) and large faces across them: the grid and the large-face
    /// list find exactly the pairs a test of every pair finds, each once.
    #[test]
    fn the_grid_finds_exactly_the_pairs_a_test_of_every_pair_finds() {
        let effort = super::super::tests::effort_for_tests();
        let mut state = 0x420u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut faces = Vec::new();
        while faces.len() < 400 {
            let base: [f64; 3] = std::array::from_fn(|_| (next() % 6) as f64);
            let p: [[f64; 3]; 3] = std::array::from_fn(|_| {
                std::array::from_fn(|k| base[k] + (next() % 3) as f64 * 0.5)
            });
            let f = face(p);
            if f[0] != f[1] && f[1] != f[2] && f[0] != f[2] {
                faces.push(f);
            }
        }
        faces.push(face([[-1., -1., 1.], [9., -1., 1.], [-1., 9., 1.]]));
        faces.push(face([[-1., -1., 2.], [9., -1., 0.], [-1., 9., 3.]]));
        faces.push(face([[0., 0., -1.], [7., 7., 8.], [0., 7., 3.]]));
        let found = pairs_of(&faces, u64::MAX, &effort).unwrap().unwrap();
        let brute: Vec<(usize, usize)> = (0..faces.len())
            .flat_map(|i| (i + 1..faces.len()).map(move |j| (i, j)))
            .filter(|&(i, j)| improper(faces[i], faces[j]))
            .collect();
        assert_eq!(found, brute);
        assert!(brute.len() > 100, "{}", brute.len());
    }

    #[test]
    fn the_census_splits_present_and_new_pairs_and_lists_the_worst_tiles() {
        let effort = super::super::tests::effort_for_tests();
        let floor = floor();
        let chord = CHORD.map(key);
        let sorted = |f: [CanonicalPoint; 3]| {
            let mut k = f;
            k.sort_unstable();
            k
        };
        let mut faces = floor.clone();
        faces.push(chord);
        let brute = (0..floor.len())
            .filter(|&i| improper(floor[i], chord))
            .count() as u64;
        let mut census = CoarseCrossings::default();
        // The children hold the floor only: every pair is new.
        let children: HashSet<_> = floor.iter().copied().map(sorted).collect();
        census
            .tile((7, 1, 0.5), &faces, &children, usize::MAX, &effort)
            .unwrap();
        // The children hold the chord too: every pair is present.
        let mut all = children.clone();
        all.insert(sorted(chord));
        census
            .tile((9, 2, 0.25), &faces, &all, usize::MAX, &effort)
            .unwrap();
        // A tile without crossings.
        census
            .tile((11, 2, 0.25), &floor, &children, usize::MAX, &effort)
            .unwrap();
        assert_eq!(
            (census.tiles, census.crossing_tiles, census.new_tiles),
            (3, 2, 1)
        );
        assert_eq!(census.new_by_depth, vec![0, brute, 0]);
        assert_eq!(census.present_by_depth, vec![0, 0, brute]);
        assert_eq!(census.listed[0].tile, 7);
        assert_eq!(census.listed[1].tile, 9);
        assert_eq!(census.listed.len(), 2);
        assert_eq!(census.incomplete_tiles, 0);
        // Repeated entries (a second placement at the same position, a reversed copy) are
        // one face each: the same pairs and faces.
        let mut twice = faces.clone();
        twice.extend(faces.iter().map(|f| [f[0], f[2], f[1]]));
        let mut again = CoarseCrossings::default();
        again
            .tile((7, 1, 0.5), &twice, &children, usize::MAX, &effort)
            .unwrap();
        assert_eq!(
            (again.faces, again.new_by_depth[1]),
            (faces.len() as u64, brute)
        );
        // 2,345 bytes per entry: about 28,600 entries within half the 128 MiB allowance.
        assert_eq!(workspace_bytes(1), 2345);
        // A tile whose workspace would exceed its bound is incomplete, never refused.
        let mut bounded = CoarseCrossings::default();
        let limit = workspace_bytes(faces.len()) - 1;
        bounded
            .tile((7, 1, 0.5), &faces, &children, limit, &effort)
            .unwrap();
        assert_eq!((bounded.tiles, bounded.incomplete_tiles), (0, 1));
    }
}
