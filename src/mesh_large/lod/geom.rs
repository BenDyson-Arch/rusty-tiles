//! Sound one-sided Hausdorff upper bounds between triangle sets (H17 certificates).
//!
//! Every reported distance is an UPPER bound computed with outward rounding: each f64
//! operation's exact result is enclosed with error-free residuals (TwoSum, FMA), so a bound
//! never falls below the real distance it encloses. Plain f64 arithmetic is used only to
//! CHOOSE targets and to prioritise refinement; it never enters a published value.
//!
//! Whole-patch witness: for a patch X (triangle, segment or point, the convex hull of its
//! three corners) and ONE target triangle t, `sup_{p in X} dist(p, t) = max_i dist(x_i, t)`
//! because the distance to a convex set is a convex function. The minimum of that over
//! targets bounds `sup_{p in X} dist(p, Y)`. Patches are refined by midpoint subdivision;
//! rounded midpoints carry an explicit displacement slack (1-Lipschitz distance), so the
//! children cover the parent exactly in the real numbers. A box distance is only ever a
//! lower bound used for pruning.
use std::{cmp::Ordering, collections::BinaryHeap};

pub(crate) type P3 = [f64; 3];
pub(crate) type Tri = [P3; 3];

/// Test-only certificate mutations (controls that must be caught). Production is `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Mutation {
    #[default]
    None,
    /// Round every "upper" operation down instead (control C1).
    InwardRounding,
    /// Drop vanished islands from the child/source -> proxy terms (control C2).
    SkipRemoved,
    /// One side of each seam keeps vertices the other removes: a real crack.
    InjectCrack,
    /// The injected crack, with fills and the seam audit relabelling it a hole (control C3).
    CrackAsHole,
}

/// The rounding direction of upper bounds; inward only under the C1 mutation.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Arith {
    inward: bool,
}

pub(crate) fn sum_error(a: f64, b: f64) -> f64 {
    let sum = a + b;
    let b_part = sum - a;
    (a - (sum - b_part)) + (b - b_part)
}
fn exact_add_dn(a: f64, b: f64) -> f64 {
    let s = a + b;
    if sum_error(a, b) < 0. {
        s.next_down()
    } else {
        s
    }
}
fn exact_add_up(a: f64, b: f64) -> f64 {
    let s = a + b;
    if sum_error(a, b) > 0. {
        s.next_up()
    } else {
        s
    }
}
fn exact_mul_up(a: f64, b: f64) -> f64 {
    let p = a * b;
    if a.mul_add(b, -p) > 0. {
        p.next_up()
    } else {
        p
    }
}
fn exact_mul_dn(a: f64, b: f64) -> f64 {
    let p = a * b;
    if a.mul_add(b, -p) < 0. {
        p.next_down()
    } else {
        p
    }
}

impl Arith {
    pub(crate) const OUTWARD: Self = Self { inward: false };
    pub(crate) fn new(mutation: Mutation) -> Self {
        Self {
            inward: mutation == Mutation::InwardRounding,
        }
    }
    /// `a + b` rounded up (down under the C1 mutation).
    pub(crate) fn add_up(self, a: f64, b: f64) -> f64 {
        if self.inward {
            exact_add_dn(a, b)
        } else {
            exact_add_up(a, b)
        }
    }
    pub(crate) fn add_dn(self, a: f64, b: f64) -> f64 {
        if self.inward {
            exact_add_up(a, b)
        } else {
            exact_add_dn(a, b)
        }
    }
    pub(crate) fn mul_up(self, a: f64, b: f64) -> f64 {
        if self.inward {
            exact_mul_dn(a, b)
        } else {
            exact_mul_up(a, b)
        }
    }
    pub(crate) fn mul_dn(self, a: f64, b: f64) -> f64 {
        if self.inward {
            exact_mul_up(a, b)
        } else {
            exact_mul_dn(a, b)
        }
    }
    /// `sqrt(x)` rounded up, `x >= 0`.
    pub(crate) fn sqrt_up(self, x: f64) -> f64 {
        let r = x.sqrt();
        let residual = r.mul_add(r, -x);
        if self.inward {
            if residual > 0. {
                r.next_down()
            } else {
                r
            }
        } else if residual < 0. {
            r.next_up()
        } else {
            r
        }
    }

    /// An upper bound on `|p - q*|` where `q* = a + v (b - a) + w (c - a)` EXACTLY, for
    /// weights `v, w >= 0` with `v + w <= 1` exactly (`q*` lies in the triangle).
    pub(crate) fn weighted(self, p: P3, t: &Tri, v: f64, w: f64) -> f64 {
        let [a, b, c] = *t;
        let mut squared = 0.;
        for i in 0..3 {
            let (pa_lo, pa_hi) = (self.add_dn(p[i], -a[i]), self.add_up(p[i], -a[i]));
            let (ba_lo, ba_hi) = (self.add_dn(b[i], -a[i]), self.add_up(b[i], -a[i]));
            let (ca_lo, ca_hi) = (self.add_dn(c[i], -a[i]), self.add_up(c[i], -a[i]));
            let (vb_lo, vb_hi) = (self.mul_dn(v, ba_lo), self.mul_up(v, ba_hi));
            let (wc_lo, wc_hi) = (self.mul_dn(w, ca_lo), self.mul_up(w, ca_hi));
            let lo = self.add_dn(self.add_dn(pa_lo, -vb_hi), -wc_hi);
            let hi = self.add_up(self.add_up(pa_hi, -vb_lo), -wc_lo);
            let m = lo.abs().max(hi.abs());
            squared = self.add_up(squared, self.mul_up(m, m));
        }
        self.sqrt_up(squared)
    }

    /// An upper bound on `dist(p, t)` (t a triangle, segment or point).
    pub(crate) fn point_tri(self, p: P3, t: &Tri) -> f64 {
        let (v, w, _) = closest_weights(p, t);
        self.weighted(p, t, v, w)
    }

    /// An upper bound on `|p - q|`.
    pub(crate) fn point_point(self, p: P3, q: P3) -> f64 {
        self.weighted(p, &[q, q, q], 0., 0.)
    }

    /// An upper bound on `sup_{x in X} dist(x, t)` for the patch `x` whose computed corners
    /// are within `slack` of its exact corners.
    pub(crate) fn patch(self, x: &Tri, t: &Tri, slack: f64) -> f64 {
        let mut bound = 0f64;
        for p in x {
            bound = bound.max(self.point_tri(*p, t));
        }
        if slack > 0. {
            self.add_up(bound, slack)
        } else {
            bound
        }
    }

    /// An upper bound on the distance from every point of the closed box to `q` (convexity:
    /// the farthest point of a box from a point is a corner).
    pub(crate) fn box_point(self, lo: P3, hi: P3, q: P3) -> f64 {
        let mut bound = 0f64;
        for corner in 0..8 {
            let p = [
                if corner & 1 == 0 { lo[0] } else { hi[0] },
                if corner & 2 == 0 { lo[1] } else { hi[1] },
                if corner & 4 == 0 { lo[2] } else { hi[2] },
            ];
            bound = bound.max(self.point_point(p, q));
        }
        bound
    }
}

fn sub(a: P3, b: P3) -> P3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: P3, b: P3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The weights `(v, w)` of a point of `t` closest to `p` (plain f64; any weights in the
/// triangle give a valid witness), and the plain squared distance estimate. The weights are
/// clamped so that `v, w >= 0` and `v + w <= 1` hold EXACTLY.
pub(crate) fn closest_weights(p: P3, t: &Tri) -> (f64, f64, f64) {
    let [a, b, c] = *t;
    let (mut v, mut w) = raw_weights(p, a, b, c);
    if !(v.is_finite() && w.is_finite()) {
        (v, w) = degenerate_weights(p, a, b, c);
    }
    let v = if v.is_nan() { 0. } else { v.clamp(0., 1.) };
    let w = if w.is_nan() {
        0.
    } else {
        w.clamp(0., exact_add_dn(1., -v).max(0.))
    };
    let ab = sub(b, a);
    let ac = sub(c, a);
    let q = [
        a[0] + v * ab[0] + w * ac[0],
        a[1] + v * ab[1] + w * ac[1],
        a[2] + v * ab[2] + w * ac[2],
    ];
    let d = sub(p, q);
    (v, w, dot(d, d))
}

/// Ericson, Real-Time Collision Detection 5.1.5, as weights relative to `a`.
fn raw_weights(p: P3, a: P3, b: P3, c: P3) -> (f64, f64) {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0. && d2 <= 0. {
        return (0., 0.);
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0. && d4 <= d3 {
        return (1., 0.);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0. && d1 >= 0. && d3 <= 0. {
        return (d1 / (d1 - d3), 0.);
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0. && d5 <= d6 {
        return (0., 1.);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0. && d2 >= 0. && d6 <= 0. {
        return (0., d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0. && (d4 - d3) >= 0. && (d5 - d6) >= 0. {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return (1. - w, w);
    }
    let denom = va + vb + vc;
    (vb / denom, vc / denom)
}

/// Closest point over the three edges (zero-area triangles: segments or points).
fn degenerate_weights(p: P3, a: P3, b: P3, c: P3) -> (f64, f64) {
    let segment = |x: P3, y: P3| -> (f64, f64) {
        let e = sub(y, x);
        let len = dot(e, e);
        let t = if len > 0. {
            (dot(sub(p, x), e) / len).clamp(0., 1.)
        } else {
            0.
        };
        let q = [x[0] + t * e[0], x[1] + t * e[1], x[2] + t * e[2]];
        let d = sub(p, q);
        (t, dot(d, d))
    };
    let (t0, d0) = segment(a, b);
    let (t1, d1) = segment(a, c);
    let (t2, d2) = segment(b, c);
    if d0 <= d1 && d0 <= d2 {
        (t0, 0.)
    } else if d1 <= d2 {
        (0., t1)
    } else {
        (1. - t2, t2)
    }
}

fn point_box(p: P3, lo: P3, hi: P3) -> f64 {
    let mut s = 0.;
    for i in 0..3 {
        let d = if p[i] < lo[i] {
            lo[i] - p[i]
        } else if p[i] > hi[i] {
            p[i] - hi[i]
        } else {
            0.
        };
        s += d * d;
    }
    s
}

#[derive(Clone, Copy, Debug)]
struct Node {
    lo: P3,
    hi: P3,
    /// Leaf: first triangle and count; internal: `count == 0`, children `first`, `first+1`.
    first: u32,
    count: u32,
}

/// A static bounding volume hierarchy over triangles, built deterministically (median split
/// on the widest centroid axis, ties by original index).
pub(crate) struct Bvh {
    nodes: Vec<Node>,
    tris: Vec<Tri>,
    /// Original index of each stored triangle.
    pub ids: Vec<u32>,
}

const LEAF: usize = 4;

fn tri_box(t: &Tri) -> (P3, P3) {
    let mut lo = t[0];
    let mut hi = t[0];
    for p in &t[1..] {
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    (lo, hi)
}

impl Bvh {
    pub(crate) fn new(tris: &[Tri]) -> Self {
        let mut order: Vec<u32> = (0..tris.len() as u32).collect();
        let centroids: Vec<P3> = tris
            .iter()
            .map(|t| std::array::from_fn(|i| (t[0][i] + t[1][i] + t[2][i]) / 3.))
            .collect();
        let mut nodes = Vec::with_capacity(2 * tris.len() / LEAF + 2);
        nodes.push(Node {
            lo: [0.; 3],
            hi: [0.; 3],
            first: 0,
            count: 0,
        });
        let mut stack = vec![(0usize, 0usize, order.len())];
        while let Some((index, start, end)) = stack.pop() {
            let mut lo = [f64::INFINITY; 3];
            let mut hi = [f64::NEG_INFINITY; 3];
            let mut clo = [f64::INFINITY; 3];
            let mut chi = [f64::NEG_INFINITY; 3];
            for &id in &order[start..end] {
                let (l, h) = tri_box(&tris[id as usize]);
                let c = centroids[id as usize];
                for i in 0..3 {
                    lo[i] = lo[i].min(l[i]);
                    hi[i] = hi[i].max(h[i]);
                    clo[i] = clo[i].min(c[i]);
                    chi[i] = chi[i].max(c[i]);
                }
            }
            if end - start <= LEAF {
                nodes[index] = Node {
                    lo,
                    hi,
                    first: start as u32,
                    count: (end - start) as u32,
                };
                continue;
            }
            let axis = (0..3)
                .max_by(|&a, &b| {
                    (chi[a] - clo[a])
                        .partial_cmp(&(chi[b] - clo[b]))
                        .unwrap_or(Ordering::Equal)
                        .then(b.cmp(&a))
                })
                .unwrap();
            let mid = (start + end) / 2;
            order[start..end].select_nth_unstable_by(mid - start, |&x, &y| {
                centroids[x as usize][axis]
                    .total_cmp(&centroids[y as usize][axis])
                    .then(x.cmp(&y))
            });
            let left = nodes.len();
            nodes.push(Node {
                lo: [0.; 3],
                hi: [0.; 3],
                first: 0,
                count: 0,
            });
            nodes.push(Node {
                lo: [0.; 3],
                hi: [0.; 3],
                first: 0,
                count: 0,
            });
            nodes[index] = Node {
                lo,
                hi,
                first: left as u32,
                count: 0,
            };
            stack.push((left, start, mid));
            stack.push((left + 1, mid, end));
        }
        let stored = order.iter().map(|&i| tris[i as usize]).collect();
        Self {
            nodes,
            tris: stored,
            ids: order,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.tris.len()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }
    pub(crate) fn tri(&self, stored: usize) -> &Tri {
        &self.tris[stored]
    }

    /// The stored index of a target with the least plain whole-patch estimate found within
    /// `cap` triangle evaluations (at least one is always evaluated), and that estimate
    /// (squared). Pruning uses box distances as LOWER bounds only.
    pub(crate) fn best_target(&self, x: &Tri, cap: u32, evals: &mut u64) -> Option<(usize, f64)> {
        self.search(x, cap, evals, None).0
    }

    /// As [`Bvh::best_target`], also returning the best target among the evaluated ones
    /// whose original index has label `want` in `labels` (same evaluations, no extra search).
    pub(crate) fn best_targets(
        &self,
        x: &Tri,
        cap: u32,
        evals: &mut u64,
        labels: &[u32],
        want: u32,
    ) -> (Option<(usize, f64)>, Option<(usize, f64)>) {
        self.search(x, cap, evals, Some((labels, want)))
    }

    fn search(
        &self,
        x: &Tri,
        cap: u32,
        evals: &mut u64,
        labels: Option<(&[u32], u32)>,
    ) -> (Option<(usize, f64)>, Option<(usize, f64)>) {
        if self.tris.is_empty() {
            return (None, None);
        }
        let lower = |n: &Node| {
            x.iter()
                .map(|p| point_box(*p, n.lo, n.hi))
                .fold(0f64, f64::max)
        };
        let mut heap = BinaryHeap::new();
        heap.push(Entry(lower(&self.nodes[0]), 0));
        let mut best: Option<(usize, f64)> = None;
        let mut same: Option<(usize, f64)> = None;
        let mut used = 0u32;
        while let Some(Entry(bound, index)) = heap.pop() {
            if let Some((_, b)) = best {
                if bound >= b || used >= cap {
                    break;
                }
            }
            let node = self.nodes[index];
            if node.count > 0 {
                for k in node.first..node.first + node.count {
                    let t = &self.tris[k as usize];
                    let matches = labels.is_some_and(|(l, w)| l[self.ids[k as usize] as usize] == w);
                    let limit = match (best, same) {
                        (Some((_, b)), Some((_, s))) if matches => b.max(s),
                        (Some((_, b)), None) if matches => f64::INFINITY.max(b),
                        (Some((_, b)), _) => b,
                        (None, _) => f64::INFINITY,
                    };
                    let mut estimate = 0f64;
                    for p in x {
                        estimate = estimate.max(closest_weights(*p, t).2);
                        if estimate >= limit {
                            break;
                        }
                    }
                    used += 1;
                    if best.is_none_or(|(_, b)| estimate < b) {
                        best = Some((k as usize, estimate));
                    }
                    if matches && same.is_none_or(|(_, b)| estimate < b) {
                        same = Some((k as usize, estimate));
                    }
                }
            } else {
                for child in [node.first as usize, node.first as usize + 1] {
                    heap.push(Entry(lower(&self.nodes[child]), child));
                }
            }
        }
        *evals += u64::from(used);
        (best, same)
    }

    /// Plain nearest squared distance from `p` (a refinement threshold, never a bound).
    pub(crate) fn nearest(&self, p: P3, cap: u32, evals: &mut u64) -> f64 {
        let x = [p, p, p];
        self.best_target(&x, cap, evals).map_or(f64::INFINITY, |b| b.1)
    }
}

/// Min-heap entry on a plain lower bound.
struct Entry(f64, usize);
impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Entry {}
impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Entry {
    fn cmp(&self, other: &Self) -> Ordering {
        other.0.total_cmp(&self.0).then(other.1.cmp(&self.1))
    }
}

/// Work limits of one directed bound.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    /// Target evaluations per whole-face query (Phase A).
    pub face_cap: u32,
    /// Target evaluations per refined patch query.
    pub patch_cap: u32,
    /// Refinement depth (each level splits a patch in four).
    pub depth: u8,
    /// Patches per face.
    pub patches: u32,
    /// Total target evaluations, Phase A included.
    pub work: u64,
    /// Refine while a face's bound exceeds `tau` times the lower estimate.
    pub tau: f64,
}

impl Limits {
    pub(crate) fn for_sizes(x: usize, y: usize) -> Self {
        Self {
            face_cap: 64,
            patch_cap: 16,
            depth: 6,
            patches: 4096,
            work: 512 * (x as u64 + y as u64).max(1),
            tau: 1.25,
        }
    }
}

/// A directed bound `sup_{p in X} dist(p, Y)`.
#[derive(Clone, Debug)]
pub(crate) struct Directed {
    /// Upper bound (outward); `INFINITY` if `Y` is empty and `X` is not.
    pub bound: f64,
    /// Upper bound per face of `X`.
    pub faces: Vec<f64>,
    /// Plain estimate of a lower bound (vertex nearest distances); diagnostic only.
    pub lower: f64,
    pub evals: u64,
    pub exhausted: bool,
    pub refined_faces: u64,
    /// With labels: per face of `X`, a bound against the same-label part of `Y` from the
    /// Phase A evaluations (`INFINITY` when none was evaluated; unrefined).
    pub same: Vec<f64>,
}

#[derive(Clone, Copy)]
struct Patch {
    p: Tri,
    slack: f64,
    depth: u8,
    bound: f64,
}

fn split(ar: Arith, patch: &Patch) -> [Patch; 4] {
    let mid = |a: P3, b: P3| -> (P3, f64) {
        let mut m = [0.; 3];
        let mut residual = 0.;
        for i in 0..3 {
            let s = a[i] + b[i];
            m[i] = s * 0.5;
            residual = exact_add_up(residual, sum_error(a[i], b[i]).abs() * 0.5);
            if m[i] != 0. && m[i].abs() < f64::MIN_POSITIVE {
                // A subnormal halving may round; charge one subnormal ulp.
                residual = exact_add_up(residual, f64::from_bits(1));
            }
        }
        (m, residual)
    };
    let [a, b, c] = patch.p;
    let (ab, r0) = mid(a, b);
    let (bc, r1) = mid(b, c);
    let (ca, r2) = mid(c, a);
    let slack = ar.add_up(patch.slack, r0.max(r1).max(r2));
    let child = |p: Tri| Patch {
        p,
        slack,
        depth: patch.depth + 1,
        bound: f64::INFINITY,
    };
    [
        child([a, ab, ca]),
        child([ab, b, bc]),
        child([ca, bc, c]),
        child([ab, bc, ca]),
    ]
}

/// Witness for one patch: query a target, then evaluate it soundly.
fn witness(ar: Arith, y: &Bvh, x: &Tri, slack: f64, cap: u32, evals: &mut u64) -> f64 {
    match y.best_target(x, cap, evals) {
        Some((k, _)) => ar.patch(x, y.tri(k), slack),
        None => f64::INFINITY,
    }
}

/// `sup_{p in X} dist(p, Y)`, bounded soundly within `limits`. Exhausting work keeps every
/// face's last complete bound (a parent bound is replaced only when all four children of
/// a split have bounds).
pub(crate) fn directed(ar: Arith, x: &[Tri], y: &Bvh, limits: Limits) -> Directed {
    directed_labelled(ar, x, y, limits, None)
}

/// [`directed`], also giving same-label witnesses when `labels = (x labels, y labels)`.
pub(crate) fn directed_labelled(
    ar: Arith,
    x: &[Tri],
    y: &Bvh,
    limits: Limits,
    labels: Option<(&[u32], &[u32])>,
) -> Directed {
    let mut evals = 0u64;
    if x.is_empty() {
        return Directed {
            bound: 0.,
            faces: Vec::new(),
            lower: 0.,
            evals,
            exhausted: false,
            refined_faces: 0,
            same: Vec::new(),
        };
    }
    if y.is_empty() {
        return Directed {
            bound: f64::INFINITY,
            faces: vec![f64::INFINITY; x.len()],
            lower: f64::INFINITY,
            evals,
            exhausted: false,
            refined_faces: 0,
            same: vec![f64::INFINITY; x.len()],
        };
    }
    let mut same = Vec::new();
    let mut faces: Vec<f64> = match labels {
        None => x
            .iter()
            .map(|t| witness(ar, y, t, 0., limits.face_cap, &mut evals))
            .collect(),
        Some((xl, yl)) => x
            .iter()
            .zip(xl)
            .map(|(t, &l)| {
                let (best, matched) = y.best_targets(t, limits.face_cap, &mut evals, yl, l);
                same.push(matched.map_or(f64::INFINITY, |(k, _)| ar.patch(t, y.tri(k), 0.)));
                best.map_or(f64::INFINITY, |(k, _)| ar.patch(t, y.tri(k), 0.))
            })
            .collect(),
    };
    // The refinement threshold: nearest distances of the corners of the faces with the
    // largest bounds (refinement only ever matters for those).
    let mut order: Vec<usize> = (0..x.len()).collect();
    let top = order.len().min(64);
    if top < order.len() {
        order.select_nth_unstable_by(top, |&a, &b| faces[b].total_cmp(&faces[a]));
    }
    let mut lower = 0f64;
    for &f in &order[..top] {
        for p in &x[f] {
            lower = lower.max(y.nearest(*p, limits.patch_cap, &mut evals));
        }
    }
    let lower = lower.sqrt();
    let threshold = limits.tau * lower;
    let mut exhausted = evals >= limits.work;
    let mut heap: BinaryHeap<(OrdF, usize)> = faces
        .iter()
        .enumerate()
        .map(|(i, b)| (OrdF(*b), i))
        .collect();
    let mut patches: std::collections::BTreeMap<usize, Vec<Patch>> = Default::default();
    while let Some((OrdF(bound), f)) = heap.pop() {
        if bound <= threshold || evals >= limits.work {
            exhausted |= evals >= limits.work && bound > threshold;
            break;
        }
        let list = patches.entry(f).or_insert_with(|| {
            vec![Patch {
                p: x[f],
                slack: 0.,
                depth: 0,
                bound,
            }]
        });
        let mut next = Vec::with_capacity(list.len() * 4);
        let mut changed = false;
        for patch in list.iter() {
            let room = (next.len() + 4 + list.len()) as u32 <= limits.patches;
            if patch.bound > threshold && patch.depth < limits.depth && room && evals < limits.work
            {
                let mut children = split(ar, patch);
                for child in &mut children {
                    child.bound = witness(ar, y, &child.p, child.slack, limits.patch_cap, &mut evals);
                }
                next.extend(children);
                changed = true;
            } else {
                next.push(*patch);
            }
        }
        if !changed {
            // The largest face cannot be refined further: the maximum cannot drop.
            heap.push((OrdF(bound), f));
            break;
        }
        let updated = next.iter().map(|p| p.bound).fold(0f64, f64::max);
        *list = next;
        faces[f] = updated;
        heap.push((OrdF(updated), f));
    }
    let bound = faces.iter().copied().fold(0f64, f64::max);
    Directed {
        bound,
        faces,
        lower,
        evals,
        exhausted,
        refined_faces: patches.len() as u64,
        same,
    }
}

/// Total order wrapper for max-heaps of bounds.
#[derive(Clone, Copy, PartialEq)]
struct OrdF(f64);
impl Eq for OrdF {}
impl PartialOrd for OrdF {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for OrdF {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny unsigned 256-bit integer for exact rational comparisons.
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
    struct U256([u128; 2]); // [hi, lo]
    fn mul(a: u128, b: u128) -> U256 {
        let (a1, a0) = (a >> 64, a & u128::from(u64::MAX));
        let (b1, b0) = (b >> 64, b & u128::from(u64::MAX));
        let p00 = a0 * b0;
        let p01 = a0 * b1;
        let p10 = a1 * b0;
        let p11 = a1 * b1;
        let mid = (p00 >> 64) + (p01 & u128::from(u64::MAX)) + (p10 & u128::from(u64::MAX));
        let lo = (p00 & u128::from(u64::MAX)) | (mid << 64);
        let hi = p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64);
        U256([hi, lo])
    }
    fn shl(x: U256, n: u32) -> U256 {
        let [hi, lo] = x.0;
        match n {
            0 => x,
            1..=127 => U256([(hi << n) | (lo >> (128 - n)), lo << n]),
            128..=255 => U256([lo << (n - 128), 0]),
            _ => U256([0, 0]),
        }
    }

    /// Exact squared distance `num / den` from integer point `p` to integer triangle `t`.
    fn exact_sq(p: [i64; 3], t: [[i64; 3]; 3]) -> (u128, u128) {
        let s = |a: [i64; 3], b: [i64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        let d = |a: [i64; 3], b: [i64; 3]| -> i128 {
            (0..3).map(|i| i128::from(a[i]) * i128::from(b[i])).sum()
        };
        let cross = |a: [i64; 3], b: [i64; 3]| {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        };
        let point = |a: [i64; 3]| (d(s(p, a), s(p, a)) as u128, 1u128);
        let segment = |a: [i64; 3], b: [i64; 3]| -> (u128, u128) {
            let e = s(b, a);
            let w = s(p, a);
            let ee = d(e, e);
            let we = d(w, e);
            if ee == 0 || we <= 0 {
                return point(a);
            }
            if we >= ee {
                return point(b);
            }
            let num = d(w, w) * ee - we * we;
            (num as u128, ee as u128)
        };
        let [a, b, c] = t;
        let n = cross(s(b, a), s(c, a));
        let nn = d(n, n);
        let mut best = segment(a, b);
        for cand in [segment(b, c), segment(c, a)] {
            if cand.0 * best.1 < best.0 * cand.1 {
                best = cand;
            }
        }
        if nn > 0 {
            // Inside test: projection lies within all three edge half-planes.
            let w = s(p, a);
            let inside = [(a, b), (b, c), (c, a)].iter().all(|&(x, y)| {
                d(cross(s(y, x), s(p, x)), n) >= 0
            });
            if inside {
                let h = d(n, w);
                return ((h * h) as u128, nn as u128);
            }
        }
        best
    }

    /// `bound^2 >= num / den` exactly.
    fn covers(bound: f64, (num, den): (u128, u128)) -> bool {
        assert!(bound.is_finite() && bound >= 0.);
        if num == 0 {
            return true;
        }
        let bits = bound.to_bits();
        let exponent = ((bits >> 52) & 0x7ff) as i32;
        let (m, e) = if exponent == 0 {
            (bits & ((1 << 52) - 1), -1074)
        } else {
            ((bits & ((1 << 52) - 1)) | (1 << 52), exponent - 1075)
        };
        let lhs = mul(u128::from(m) * u128::from(m), den);
        let rhs = U256([0, num]);
        if e >= 0 {
            shl(lhs, (2 * e) as u32) >= rhs
        } else {
            lhs >= shl(rhs, (-2 * e) as u32)
        }
    }

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            self.0 >> 33
        }
        fn int(&mut self, span: i64) -> i64 {
            (self.next() % (2 * span as u64 + 1)) as i64 - span
        }
    }

    fn cases(n: usize) -> Vec<([i64; 3], [[i64; 3]; 3])> {
        let mut rng = Lcg(17);
        (0..n)
            .map(|k| {
                let p = [rng.int(900), rng.int(900), rng.int(900)];
                let mut t = [[0; 3]; 3];
                for v in &mut t {
                    *v = [rng.int(700), rng.int(700), rng.int(700)];
                }
                if k % 7 == 0 {
                    t[2] = t[1]; // segment
                }
                if k % 11 == 0 {
                    t[1] = t[0];
                    t[2] = t[0]; // point
                }
                (p, t)
            })
            .collect()
    }
    fn f(v: [i64; 3]) -> P3 {
        v.map(|x| x as f64)
    }

    /// The witness arithmetic never undercuts the exact distance (exact rational reference),
    /// including segments, points and subnormal-free integer cases at rounding boundaries.
    #[test]
    fn outward_point_triangle_bounds_cover_the_exact_distance() {
        for (p, t) in cases(20_000) {
            let tri = [f(t[0]), f(t[1]), f(t[2])];
            let bound = Arith::OUTWARD.point_tri(f(p), &tri);
            assert!(covers(bound, exact_sq(p, t)), "{p:?} {t:?} {bound}");
        }
    }

    /// Control C1: inward rounding is caught by the same exact reference.
    #[test]
    fn control_inward_rounding_is_caught() {
        let inward = Arith::new(Mutation::InwardRounding);
        let caught = cases(20_000).into_iter().any(|(p, t)| {
            let tri = [f(t[0]), f(t[1]), f(t[2])];
            !covers(inward.point_tri(f(p), &tri), exact_sq(p, t))
        });
        assert!(caught, "inward rounding must undercut some exact distance");
    }

    fn grid(n: usize, z: impl Fn(f64, f64) -> f64) -> Vec<Tri> {
        let mut out = Vec::new();
        let s = 1. / n as f64;
        for i in 0..n {
            for j in 0..n {
                let p = |a: usize, b: usize| {
                    let (x, y) = (a as f64 * s, b as f64 * s);
                    [x, y, z(x, y)]
                };
                out.push([p(i, j), p(i + 1, j), p(i + 1, j + 1)]);
                out.push([p(i, j), p(i + 1, j + 1), p(i, j + 1)]);
            }
        }
        out
    }

    fn sampled(x: &[Tri], y: &[Tri], k: usize) -> f64 {
        let mut worst = 0f64;
        for t in x {
            for i in 0..=k {
                for j in 0..=k - i {
                    let (u, v) = (i as f64 / k as f64, j as f64 / k as f64);
                    let p: P3 = std::array::from_fn(|a| {
                        t[0][a] + u * (t[1][a] - t[0][a]) + v * (t[2][a] - t[0][a])
                    });
                    let d = y
                        .iter()
                        .map(|s| closest_weights(p, s).2)
                        .fold(f64::INFINITY, f64::min)
                        .sqrt();
                    worst = worst.max(d);
                }
            }
        }
        worst
    }

    /// Directed bounds dominate dense sampled distances and refinement tightens a coarse
    /// patch spanning a pit (a vertex-only bound would miss it).
    #[test]
    fn directed_bounds_dominate_samples_and_refine() {
        let fine = grid(16, |x, y| 0.05 * ((6. * x).sin() + (5. * y).cos()));
        let coarse = grid(2, |_, _| 0.);
        let pit = grid(16, |x, y| {
            if (x - 0.5).abs() < 0.1 && (y - 0.5).abs() < 0.1 {
                -0.3
            } else {
                0.
            }
        });
        for (x, y) in [(&coarse, &fine), (&fine, &coarse), (&coarse, &pit), (&pit, &coarse)] {
            let bvh = Bvh::new(y);
            let d = directed(Arith::OUTWARD, x, &bvh, Limits::for_sizes(x.len(), y.len()));
            let s = sampled(x, y, 12);
            assert!(d.bound >= s, "bound {} sample {}", d.bound, s);
            assert!(d.bound <= s * 3. + 1e-9, "refinement too loose: {} vs {}", d.bound, s);
        }
        // The pit's floor is 0.3 below the coarse plane: the plane is within 0.3 of it.
        let d = directed(
            Arith::OUTWARD,
            &pit,
            &Bvh::new(&coarse),
            Limits::for_sizes(pit.len(), coarse.len()),
        );
        assert!(d.bound >= 0.3);
        // Exhausted work still yields a bound at least the true value.
        let mut tight = Limits::for_sizes(coarse.len(), fine.len());
        tight.work = 1;
        let d = directed(Arith::OUTWARD, &coarse, &Bvh::new(&fine), tight);
        assert!(d.exhausted || d.refined_faces == 0);
        assert!(d.bound >= sampled(&coarse, &fine, 12));
    }

    #[test]
    fn split_children_cover_the_parent_with_slack() {
        let ar = Arith::OUTWARD;
        let patch = Patch {
            p: [[0.1, 1e6 + 0.3, -7.7], [1e-9, 3.3, 2.], [1e6, -1e6 + 0.1, 0.]],
            slack: 0.,
            depth: 0,
            bound: 0.,
        };
        for child in split(ar, &patch) {
            assert!(child.slack >= 0.);
            assert!(child.slack < 1e-6);
        }
    }
}
