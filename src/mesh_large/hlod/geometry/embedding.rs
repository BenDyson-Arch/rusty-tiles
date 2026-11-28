//! Exact spatial predicates for P1's generated (Steiner) faces.
//!
//! A face that a Steiner rescue creates or moves may meet another face only in the simplex
//! of their common vertices: nowhere with no common vertex, only that vertex with one, only
//! their shared edge with two; a repeated triangle is refused. Every test here is exact on
//! the f64 values of f32 positions (`robust`'s adaptive orientation predicates); a
//! degenerate face that touches a changed face through a common vertex is refused as
//! unresolved.
use robust::{orient2d, orient3d, Coord, Coord3D};

pub(super) type Point = [f64; 3];

fn c3(p: Point) -> Coord3D<f64> {
    Coord3D {
        x: p[0],
        y: p[1],
        z: p[2],
    }
}

/// `p` without coordinate `drop`.
fn c2(p: Point, drop: usize) -> Coord<f64> {
    let (i, j) = match drop {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    Coord { x: p[i], y: p[j] }
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

fn o3(a: Point, b: Point, c: Point, d: Point) -> i8 {
    sign(orient3d(c3(a), c3(b), c3(c), c3(d)))
}

fn o2(a: Point, b: Point, c: Point, drop: usize) -> i8 {
    sign(orient2d(c2(a, drop), c2(b, drop), c2(c, drop)))
}

/// A coordinate whose removal keeps triangle `t` nondegenerate; none when `t` has zero
/// area (its normal is zero exactly when every coordinate projection is degenerate).
fn projection(t: [Point; 3]) -> Option<usize> {
    (0..3).find(|&d| o2(t[0], t[1], t[2], d) != 0)
}

/// Whether `t` has nonzero area.
pub(super) fn nondegenerate(t: [Point; 3]) -> bool {
    projection(t).is_some()
}

/// Whether the closed segments `ab` and `cd` meet, all four points on one plane that
/// projection `d` keeps nondegenerate.
fn segments_meet_2d(a: Point, b: Point, c: Point, e: Point, d: usize) -> bool {
    let within = |p: Point, q: Point, r: Point| {
        (0..3).all(|k| p[k].min(q[k]) <= r[k] && r[k] <= p[k].max(q[k]))
    };
    let (d1, d2) = (o2(a, b, c, d), o2(a, b, e, d));
    let (d3, d4) = (o2(c, e, a, d), o2(c, e, b, d));
    if d1 * d2 < 0 && d3 * d4 < 0 {
        return true;
    }
    (d1 == 0 && within(a, b, c))
        || (d2 == 0 && within(a, b, e))
        || (d3 == 0 && within(c, e, a))
        || (d4 == 0 && within(c, e, b))
}

/// Whether point `p` lies in closed triangle `t`, all on one plane that projection `d`
/// keeps nondegenerate.
fn inside_2d(p: Point, t: [Point; 3], d: usize) -> bool {
    let s = [
        o2(t[0], t[1], p, d),
        o2(t[1], t[2], p, d),
        o2(t[2], t[0], p, d),
    ];
    !(s.contains(&1) && s.contains(&-1))
}

/// Whether the closed segment `pq` meets closed triangle `t` (a degenerate `t` counts as
/// met: unresolved).
fn segment_meets(p: Point, q: Point, t: [Point; 3]) -> bool {
    let Some(d) = projection(t) else {
        return true;
    };
    let (s1, s2) = (o3(t[0], t[1], t[2], p), o3(t[0], t[1], t[2], q));
    if s1 == s2 && s1 != 0 {
        return false;
    }
    if s1 == 0 && s2 == 0 {
        return inside_2d(p, t, d)
            || inside_2d(q, t, d)
            || (0..3).any(|i| segments_meet_2d(p, q, t[i], t[(i + 1) % 3], d));
    }
    // The segment reaches the plane at one point, inside the closed triangle unless the
    // line pq passes two edges on opposite sides.
    let e = [
        o3(p, q, t[0], t[1]),
        o3(p, q, t[1], t[2]),
        o3(p, q, t[2], t[0]),
    ];
    !(e.contains(&1) && e.contains(&-1))
}

/// Whether two faces meet only in the simplex of their common vertex ids. `a` must have
/// nonzero area; the caller checks it.
pub(super) fn permitted(a: [u32; 3], b: [u32; 3], at: impl Fn(u32) -> Point) -> bool {
    let common: Vec<u32> = a.iter().copied().filter(|v| b.contains(v)).collect();
    let (pa, pb) = (a.map(&at), b.map(&at));
    let b_flat = !nondegenerate(pb);
    match common.as_slice() {
        [] => {
            let b_edges = (0..3).any(|i| segment_meets(pb[i], pb[(i + 1) % 3], pa));
            if b_flat {
                // a zero-area face is the union of its edges
                return !b_edges;
            }
            !b_edges && !(0..3).any(|i| segment_meets(pa[i], pa[(i + 1) % 3], pb))
        }
        // The intersection of two triangles through v is convex; any other point puts the
        // edge opposite v of one triangle in contact with the other.
        [v] => {
            if b_flat {
                return false;
            }
            let other = |t: [u32; 3], p: [Point; 3]| {
                let k = t.iter().position(|w| w == v).unwrap_or(0);
                (p[(k + 1) % 3], p[(k + 2) % 3])
            };
            let (a1, a2) = other(a, pa);
            let (b1, b2) = other(b, pb);
            !segment_meets(a1, a2, pb) && !segment_meets(b1, b2, pa)
        }
        // Distinct planes through the shared edge meet only on its line; on one plane the
        // two must lie on opposite sides of it.
        [u, v] => {
            if b_flat {
                return false;
            }
            let apex = |t: [u32; 3], p: [Point; 3]| {
                let k = t.iter().position(|w| w != u && w != v).unwrap_or(0);
                p[k]
            };
            let (pu, pv) = (at(*u), at(*v));
            let (x, y) = (apex(a, pa), apex(b, pb));
            if o3(pu, pv, x, y) != 0 {
                return true;
            }
            let Some(d) = projection(pa) else {
                return false;
            };
            o2(pu, pv, x, d) * o2(pu, pv, y, d) < 0
        }
        _ => false,
    }
}

/// Axis-aligned bounds of a face.
pub(super) fn bounds(t: [Point; 3]) -> [Point; 2] {
    let mut lo = t[0];
    let mut hi = t[0];
    for p in &t[1..] {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    [lo, hi]
}

pub(super) fn overlap(a: [Point; 2], b: [Point; 2]) -> bool {
    (0..3).all(|k| a[0][k] <= b[1][k] && b[0][k] <= a[1][k])
}

/// Whether `n · ((a − q) × (b − q))` is certified strictly positive by its f64 value: the
/// value exceeds a forward bound on its rounding error. `n` is a reference vector computed
/// with `depth` roundings on each component, `n_abs` the same expression on absolute
/// values. An unresolved sign is not positive.
pub(super) fn positive(n: Point, n_abs: Point, depth: u32, a: Point, b: Point, q: Point) -> bool {
    let u = [a[0] - q[0], a[1] - q[1], a[2] - q[2]];
    let v = [b[0] - q[0], b[1] - q[1], b[2] - q[2]];
    let c = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let (ua, va) = (u.map(f64::abs), v.map(f64::abs));
    let ca = [
        ua[1] * va[2] + ua[2] * va[1],
        ua[2] * va[0] + ua[0] * va[2],
        ua[0] * va[1] + ua[1] * va[0],
    ];
    let value = n[0] * c[0] + n[1] * c[1] + n[2] * c[2];
    let perm = n_abs[0] * ca[0] + n_abs[1] * ca[1] + n_abs[2] * ca[2];
    // c: subtraction, product, difference (4 roundings); the dot: product, two sums.
    let k = f64::from(depth.saturating_add(7));
    let bound = 2. * (k + 1.) * f64::EPSILON * perm;
    // Below this the relative model fails (subnormal intermediates): unresolved.
    const TINY: f64 = 1e-250;
    value.is_finite() && perm.is_finite() && perm > TINY && value > bound
}

/// A face's normal `(b − a) × (c − a)` with its absolute-value expression (4 roundings).
pub(super) fn normal(t: [Point; 3]) -> (Point, Point) {
    let u = [t[1][0] - t[0][0], t[1][1] - t[0][1], t[1][2] - t[0][2]];
    let v = [t[2][0] - t[0][0], t[2][1] - t[0][1], t[2][2] - t[0][2]];
    let (ua, va) = (u.map(f64::abs), v.map(f64::abs));
    (
        [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ],
        [
            ua[1] * va[2] + ua[2] * va[1],
            ua[2] * va[0] + ua[0] * va[2],
            ua[0] * va[1] + ua[1] * va[0],
        ],
    )
}

/// Final check of a P1 proposal's generated vertices on its delivered positions: each
/// keeps a key no other vertex decodes to, and each face on one has nonzero area and meets
/// every other face only in their common simplex.
pub(crate) fn generated_embedded(
    positions: &[[f32; 3]],
    triangles: &[[u32; 3]],
    generated: &[u32],
) -> bool {
    if generated.is_empty() {
        return true;
    }
    let key = |p: [f32; 3]| p.map(|x| if x == 0. { 0f32 } else { x }.to_bits());
    let mut is_generated = vec![false; positions.len()];
    for &g in generated {
        match is_generated.get_mut(g as usize) {
            Some(slot) => *slot = true,
            None => return false,
        }
    }
    let mut keys: Vec<([u32; 3], bool)> = positions
        .iter()
        .zip(&is_generated)
        .map(|(&p, &g)| (key(p), g))
        .collect();
    keys.sort_unstable();
    if keys
        .windows(2)
        .any(|w| w[0].0 == w[1].0 && (w[0].1 || w[1].1))
    {
        return false;
    }
    let at = |v: u32| {
        positions
            .get(v as usize)
            .map_or([f64::NAN; 3], |p| p.map(f64::from))
    };
    if triangles
        .iter()
        .flatten()
        .any(|&v| v as usize >= positions.len())
    {
        return false;
    }
    let boxes: Vec<[Point; 2]> = triangles.iter().map(|t| bounds(t.map(at))).collect();
    for (i, t) in triangles.iter().enumerate() {
        if !t.iter().any(|&v| is_generated[v as usize]) {
            continue;
        }
        if !nondegenerate(t.map(at)) {
            return false;
        }
        for (j, u) in triangles.iter().enumerate() {
            if i != j && overlap(boxes[i], boxes[j]) && !permitted(*t, *u, at) {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(p: &[Point]) -> impl Fn(u32) -> Point + '_ {
        move |v| p[v as usize]
    }

    #[test]
    fn faces_meet_only_in_their_common_simplex() {
        let p = [
            [0., 0., 0.],
            [2., 0., 0.],
            [0., 2., 0.],
            [2., 2., 0.],
            [0.5, 0.5, -1.],
            [0.5, 0.5, 1.],
            [5., 5., 5.],
            [-1., -1., 0.],
            [1., 3., 0.],
            [3., 1., 0.],
        ];
        // No common vertex: apart, then piercing the interior.
        assert!(permitted([0, 1, 2], [3, 6, 5], at(&p)));
        assert!(!permitted([0, 1, 2], [4, 5, 6], at(&p)));
        // One common vertex: apart, then overlapping on one plane.
        assert!(permitted([0, 1, 2], [0, 7, 4], at(&p)));
        assert!(!permitted([0, 1, 2], [0, 8, 9], at(&p)));
        // A shared edge: opposite sides of it, then folded onto the same side.
        assert!(permitted([0, 1, 2], [1, 3, 2], at(&p)));
        assert!(!permitted([0, 1, 2], [2, 1, 7], at(&p)));
        // A repeated triangle.
        assert!(!permitted([0, 1, 2], [2, 0, 1], at(&p)));
        // Touching an edge at a point that is not a common vertex is contact.
        let q = [
            [0., 0., 0.],
            [2., 0., 0.],
            [0., 2., 0.],
            [1., 1., 0.],
            [1., 1., 1.],
            [3., 1., 1.],
        ];
        assert!(!permitted([0, 1, 2], [3, 4, 5], at(&q)));
        // A zero-area face apart from the changed one, then crossing it.
        let r = [
            [0., 0., 0.],
            [2., 0., 0.],
            [0., 2., 0.],
            [3., 3., 1.],
            [4., 4., 1.],
            [5., 5., 1.],
            [0.5, 0.5, -1.],
            [0.5, 0.5, 1.],
            [0.5, 0.5, 2.],
        ];
        assert!(permitted([0, 1, 2], [3, 4, 5], at(&r)));
        assert!(!permitted([0, 1, 2], [6, 7, 8], at(&r)));
    }

    #[test]
    fn generated_faces_are_checked_on_delivered_positions() {
        // s = 3 closes (0, 1) with the bridge; vertex 4..6 is a spectator.
        let mut positions = vec![
            [0f32, 0., 0.],
            [64., 0., 0.],
            [32., 64., 0.],
            [32., 3., 0.],
            [32., 1., -1.],
            [32., 1., 1.],
            [32., 2., 0.],
        ];
        let faces = [[0, 3, 2], [2, 3, 1], [3, 0, 1], [4, 5, 6]];
        assert!(!generated_embedded(&positions, &faces, &[3]));
        assert!(generated_embedded(&positions, &faces[..3], &[3]));
        assert!(generated_embedded(&positions, &faces, &[]));
        // A decode that puts another vertex on the generated key.
        positions[6] = [32., 3., 0.];
        assert!(!generated_embedded(&positions, &faces[..3], &[3]));
    }

    #[test]
    fn certified_signs_refuse_an_unresolved_zero() {
        let (n, n_abs) = normal([[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]);
        let (a, b) = ([1., 0., 0.], [0., 1., 0.]);
        assert!(positive(n, n_abs, 4, a, b, [0.2, 0.2, 0.]));
        assert!(!positive(n, n_abs, 4, a, b, [0.6, 0.6, 0.]));
        // On the line ab: zero, refused.
        assert!(!positive(n, n_abs, 4, a, b, [0.5, 0.5, 0.]));
    }
}
