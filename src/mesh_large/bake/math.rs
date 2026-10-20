//! Finite geometry used by the immutable proxy appearance owner.
pub(super) type V3 = [f64; 3];
pub(super) fn add(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[i] + b[i])
}
pub(super) fn sub(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[i] - b[i])
}
pub(super) fn mul(a: V3, k: f64) -> V3 {
    a.map(|v| v * k)
}
pub(super) fn dot(a: V3, b: V3) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}
pub(super) fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
/// Rescaling handles every finite magnitude without squaring into overflow.
pub(super) fn unit(a: V3) -> Option<V3> {
    if a.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let scale = a.iter().map(|v| v.abs()).fold(0.0, f64::max);
    if scale == 0.0 {
        return None;
    }
    let q = a.map(|v| v / scale);
    let n = dot(q, q).sqrt();
    (n > 0.0 && n.is_finite()).then(|| q.map(|v| v / n))
}
pub(super) fn bary(p: [V3; 3], w: V3) -> V3 {
    std::array::from_fn(|i| p[0][i] * w[0] + p[1][i] * w[1] + p[2][i] * w[2])
}
/// The smallest value whose cumulative weight reaches half the total weight, over
/// `(value, weight)` samples with finite nonnegative weights; with no positive weight,
/// the plain median. `None` when empty.
pub(super) fn weighted_median(samples: &mut [(f64, f64)]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    samples.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: f64 = samples.iter().map(|s| s.1).sum();
    if !(total.is_finite() && total > 0.) {
        return Some(samples[(samples.len() - 1) / 2].0);
    }
    let mut acc = 0.;
    for &(value, weight) in samples.iter() {
        acc += weight;
        if acc >= total / 2. {
            return Some(value);
        }
    }
    samples.last().map(|s| s.0)
}

/// Half the cross-product length; 0 for a degenerate or non-finite triangle.
pub(super) fn triangle_area(p: [V3; 3]) -> f64 {
    let n = cross(sub(p[1], p[0]), sub(p[2], p[0]));
    let a = 0.5 * dot(n, n).sqrt();
    if a.is_finite() {
        a
    } else {
        0.
    }
}
pub(super) fn face_normal(p: [V3; 3]) -> Option<V3> {
    unit(cross(sub(p[1], p[0]), sub(p[2], p[0])))
}
pub(super) fn distance2(a: V3, b: V3) -> f64 {
    let d = sub(a, b);
    dot(d, d)
}
pub(super) fn distance_upper(a: V3, b: V3) -> f64 {
    let mut sum = 0.;
    for i in 0..3 {
        if a[i] == b[i] {
            continue;
        }
        let d = (a[i] - b[i]).abs().next_up();
        sum = (sum + (d * d).next_up()).next_up();
    }
    if sum == 0. {
        0.
    } else {
        sum.sqrt().next_up()
    }
}
pub(super) fn support_distance_upper(q: V3, lo: V3, hi: V3) -> f64 {
    let mut sum = 0.;
    for i in 0..3 {
        if q[i] == lo[i] && q[i] == hi[i] {
            continue;
        }
        let d = (q[i] - lo[i]).abs().max((q[i] - hi[i]).abs()).next_up();
        sum = (sum + (d * d).next_up()).next_up();
    }
    if sum == 0. {
        0.
    } else {
        sum.sqrt().next_up()
    }
}
/// Largest local-world metres per decoded UV pixel. This includes projection,
/// chart compensation and actual UV quantization; nominal node density does not.
pub(super) fn pixel_size(p: [V3; 3], uv: [[f64; 2]; 3]) -> Option<f64> {
    let a = [uv[1][0] - uv[0][0], uv[1][1] - uv[0][1]];
    let b = [uv[2][0] - uv[0][0], uv[2][1] - uv[0][1]];
    let det = a[0] * b[1] - a[1] * b[0];
    if det == 0. || !det.is_finite() {
        return None;
    }
    let e = sub(p[1], p[0]);
    let f = sub(p[2], p[0]);
    let x = mul(sub(mul(e, b[1]), mul(f, a[1])), 1. / det);
    let y = mul(sub(mul(f, a[0]), mul(e, b[0])), 1. / det);
    let aa = dot(x, x);
    let ab = dot(x, y);
    let bb = dot(y, y);
    let sigma = ((aa + bb + ((aa - bb) * (aa - bb) + 4. * ab * ab).sqrt()) * 0.5).sqrt();
    (sigma.is_finite() && sigma > 0.).then_some(sigma.next_up())
}
/// Largest decoded page pixels per local-world metre, using the stable cross
/// product dual basis rather than cancellation-prone Gram determinants.
pub(super) fn pixel_gradient(p: [V3; 3], uv: [[f64; 2]; 3]) -> Option<f64> {
    let e = sub(p[1], p[0]);
    let f = sub(p[2], p[0]);
    let n = cross(e, f);
    let d = dot(n, n);
    if d <= 0. || !d.is_finite() {
        return None;
    }
    let g: [V3; 2] = std::array::from_fn(|a| {
        mul(
            add(
                mul(cross(f, n), uv[1][a] - uv[0][a]),
                mul(cross(n, e), uv[2][a] - uv[0][a]),
            ),
            1. / d,
        )
    });
    let aa = dot(g[0], g[0]);
    let ab = dot(g[0], g[1]);
    let bb = dot(g[1], g[1]);
    let sigma = ((aa + bb + ((aa - bb) * (aa - bb) + 4. * ab * ab).sqrt()) * 0.5).sqrt();
    (sigma.is_finite() && sigma > 0.).then_some(sigma.next_up())
}
fn segment(q: V3, a: V3, b: V3) -> (V3, f64) {
    let e = sub(b, a);
    let n = dot(e, e);
    let t = if n == 0.0 {
        0.0
    } else {
        (dot(sub(q, a), e) / n).clamp(0.0, 1.0)
    };
    (add(a, mul(e, t)), t)
}
/// Returns an actual barycentric source-surface witness, including degenerates.
/// Its distance is appearance correspondence, never a geometric certificate.
pub(super) fn closest(q: V3, p: [V3; 3]) -> V3 {
    let ab = sub(p[1], p[0]);
    let ac = sub(p[2], p[0]);
    let ap = sub(q, p[0]);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return [1.0, 0.0, 0.0];
    }
    let bp = sub(q, p[1]);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return [0.0, 1.0, 0.0];
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 && d1 != d3 {
        let v = d1 / (d1 - d3);
        return [1.0 - v, v, 0.0];
    }
    let cp = sub(q, p[2]);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return [0.0, 0.0, 1.0];
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 && d2 != d6 {
        let w = d2 / (d2 - d6);
        return [1.0 - w, 0.0, w];
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 >= d3 && d5 >= d6 && (d4 - d3) + (d5 - d6) != 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return [0.0, 1.0 - w, w];
    }
    let den = va + vb + vc;
    if den > 0.0 && den.is_finite() {
        let v = (vb / den).clamp(0.0, 1.0);
        let w = (vc / den).clamp(0.0, 1.0 - v);
        return [1.0 - v - w, v, w];
    }
    let mut best = (f64::INFINITY, [1.0, 0.0, 0.0]);
    for (a, b) in [(0, 1), (1, 2), (2, 0)] {
        let (s, t) = segment(q, p[a], p[b]);
        let d = distance2(q, s);
        if d < best.0 {
            let mut w = [0.0; 3];
            w[a] = 1.0 - t;
            w[b] = t;
            best = (d, w);
        }
    }
    best.1
}
/// Lower bound for pruning. Every operation is rounded towards minus infinity.
pub(super) fn box_distance_lower(q: V3, lo: V3, hi: V3) -> f64 {
    let mut sum = 0.0;
    for i in 0..3 {
        let d = if q[i] < lo[i] {
            (lo[i] - q[i]).next_down().max(0.0)
        } else if q[i] > hi[i] {
            (q[i] - hi[i]).next_down().max(0.0)
        } else {
            0.0
        };
        sum = (sum + (d * d).next_down().max(0.0)).next_down().max(0.0);
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn weighted_median_takes_half_the_weight() {
        assert_eq!(weighted_median(&mut []), None);
        assert_eq!(weighted_median(&mut [(3., 1.)]), Some(3.));
        // Sorted by value: the outlier carries almost no weight.
        assert_eq!(
            weighted_median(&mut [(5000., 1e-6), (2., 1.), (1., 1.), (3., 1.)]),
            Some(2.)
        );
        assert_eq!(weighted_median(&mut [(1., 1.), (9., 3.)]), Some(9.));
        // No positive weight: the plain (lower) median.
        assert_eq!(
            weighted_median(&mut [(4., 0.), (1., 0.), (2., 0.)]),
            Some(2.)
        );
    }
}
