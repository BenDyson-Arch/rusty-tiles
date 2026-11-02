//! Sufficient, outward-rounded certificates. A rounded projection only chooses
//! a witness: nonnegative binary64 weights, normalized in exact arithmetic,
//! define a point in the closed retained face. Enclosing that point's distance
//! is an upper bound even when the projection itself is inaccurate or singular.
//! No numerical allowance is added to the single caller-supplied reach.

#[derive(Clone, Copy, Debug)]
pub(super) struct Interval {
    lo: f64,
    hi: f64,
}

impl Interval {
    pub(super) fn point(x: f64) -> Self {
        Self { lo: x, hi: x }
    }
    fn bounds(lo: f64, hi: f64) -> Self {
        if lo.is_nan() || hi.is_nan() {
            Self {
                lo: f64::NEG_INFINITY,
                hi: f64::INFINITY,
            }
        } else {
            Self {
                lo: lo.next_down(),
                hi: hi.next_up(),
            }
        }
    }
    pub(super) fn add(self, rhs: Self) -> Self {
        Self::bounds(self.lo + rhs.lo, self.hi + rhs.hi)
    }
    fn sub(self, rhs: Self) -> Self {
        Self::bounds(self.lo - rhs.hi, self.hi - rhs.lo)
    }
    fn mul(self, rhs: Self) -> Self {
        let products = [
            self.lo * rhs.lo,
            self.lo * rhs.hi,
            self.hi * rhs.lo,
            self.hi * rhs.hi,
        ];
        if products.iter().any(|x| x.is_nan()) {
            return Self::bounds(f64::NAN, f64::NAN);
        }
        Self::bounds(
            products.into_iter().fold(f64::INFINITY, f64::min),
            products.into_iter().fold(f64::NEG_INFINITY, f64::max),
        )
    }
    pub(super) fn div(self, rhs: Self) -> Self {
        if rhs.lo <= 0. && rhs.hi >= 0. {
            return Self::bounds(f64::NAN, f64::NAN);
        }
        self.mul(Self::bounds(1. / rhs.hi, 1. / rhs.lo))
    }
    pub(super) fn middle(self) -> f64 {
        self.lo * 0.5 + self.hi * 0.5
    }
}

pub(super) type Point = [Interval; 3];

fn norm2_upper(v: Point) -> f64 {
    v.into_iter().fold(0., |sum, x| {
        let magnitude = x.lo.abs().max(x.hi.abs());
        (sum + (magnitude * magnitude).next_up()).next_up()
    })
}

pub(super) fn radius2(t: [Point; 3], center: Point) -> f64 {
    t.into_iter()
        .map(|p| norm2_upper(std::array::from_fn(|k| p[k].sub(center[k]))))
        .fold(0., f64::max)
}

/// sqrt(d2) + sqrt(r2) <= reach, without rounded square roots. d2 and r2
/// are upper bounds; all arithmetic on the acceptance side is enclosed too.
pub(super) fn ball_fits(d2: f64, r2: f64, reach: f64) -> bool {
    if !d2.is_finite() || !r2.is_finite() || !reach.is_finite() {
        return false;
    }
    let gap = Interval::point(reach)
        .mul(Interval::point(reach))
        .sub(Interval::point(d2))
        .sub(Interval::point(r2));
    let rhs = Interval::point(4.)
        .mul(Interval::point(d2))
        .mul(Interval::point(r2));
    gap.lo >= 0. && Interval::point(gap.lo).mul(Interval::point(gap.lo)).lo >= rhs.hi
}

pub(super) fn within(d2: f64, reach: f64) -> bool {
    d2.is_finite()
        && reach.is_finite()
        && d2 <= Interval::point(reach).mul(Interval::point(reach)).lo
}

/// Upper bound to ONE face for every point in p's box. Four projection proposals
/// (interior and three edges) have constant cost. All are merely witness choices:
/// only their enclosed, normalized convex combinations authorize acceptance.
pub(super) fn distance2_upper(p: Point, t: [[f64; 3]; 3]) -> f64 {
    let sub = |a: [f64; 3], b: [f64; 3]| std::array::from_fn::<_, 3, _>(|k| a[k] - b[k]);
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let sample = p.map(Interval::middle);
    let witness = |weights: [f64; 3]| {
        if weights.iter().any(|w| !w.is_finite() || *w < 0.) {
            return f64::INFINITY;
        }
        let weights = weights.map(Interval::point);
        let sum = weights[0].add(weights[1]).add(weights[2]);
        if sum.lo <= 0. {
            return f64::INFINITY;
        }
        let b = weights[1].div(sum);
        let c = weights[2].div(sum);
        // Translate before combining, reducing cancellation for large coordinates.
        norm2_upper(std::array::from_fn(|k| {
            let a = Interval::point(t[0][k]);
            p[k].sub(a)
                .sub(b.mul(Interval::point(t[1][k]).sub(a)))
                .sub(c.mul(Interval::point(t[2][k]).sub(a)))
        }))
    };
    let mut best = witness([1., 0., 0.]);
    for (i, j) in [(0, 1), (1, 2), (2, 0)] {
        let edge = sub(t[j], t[i]);
        let dd = dot(edge, edge);
        let u = if dd == 0. {
            0.
        } else {
            (dot(sub(sample, t[i]), edge) / dd).clamp(0., 1.)
        };
        let mut weights = [0.; 3];
        weights[i] = 1. - u;
        weights[j] = u;
        best = best.min(witness(weights));
    }
    let (u, v, w) = (sub(t[1], t[0]), sub(t[2], t[0]), sub(sample, t[0]));
    let (uu, uv, vv, wu, wv) = (dot(u, u), dot(u, v), dot(v, v), dot(w, u), dot(w, v));
    let det = uu * vv - uv * uv;
    if det > 0. {
        let (b, c) = ((wu * vv - wv * uv) / det, (wv * uu - wu * uv) / det);
        best = best.min(witness([(1. - b - c).max(0.), b.max(0.), c.max(0.)]));
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skinny_face_never_certifies_the_review_counterexample() {
        let t = [
            [
                -1.4724074926764578e23,
                2.9648180383498556e22,
                -7.774404609302878e22,
            ],
            [
                5.001019053693828e22,
                1.1372596964707108e23,
                -1.23504860697962e23,
            ],
            [
                -3.7499307865693355e22,
                7.642523899974737e22,
                -1.032032911033214e23,
            ],
        ];
        let p = [
            2.9580527309896196e22,
            1.0501786345078302e23,
            -1.1876531748611357e23,
        ]
        .map(Interval::point);
        let upper = distance2_upper(p, t);
        // Exact-rational review witness is strictly above this integer squared.
        let lower = 8277550729000000_f64;
        assert!(upper > lower * lower);
        assert!(!within(upper, 8277550726550266.));
        assert!(within(upper, 2. * 8277550726550266.));
    }

    #[test]
    fn subdivision_keeps_rounding_in_the_geometry() {
        let a = Interval::point(1.);
        let b = Interval::point(1f64.next_up());
        let midpoint = a.add(b).div(Interval::point(2.));
        assert!(midpoint.lo <= 1. && midpoint.hi >= 1f64.next_up());
        let center = a.add(a).add(Interval::point(2.)).div(Interval::point(3.));
        assert!(center.lo < 4. / 3. && center.hi > 4. / 3.);
    }

    #[test]
    fn outward_arithmetic_and_unresolved_bounds_do_not_pass() {
        let x = Interval::point(1.);
        let y = x.add(Interval::point(f64::EPSILON / 2.));
        assert!(y.lo <= 1. && y.hi >= 1. + f64::EPSILON);
        assert!(!within(1., 1f64.next_down()));
        assert!(!within(1., 1.)); // The unresolved band never gets another tolerance.
        assert!(within(1., 1f64.next_up()));
        assert!(!within(f64::INFINITY, 1.));
        assert!(!ball_fits(1., 1., 2f64.next_down()));
        assert!(ball_fits(1., 1., 2.01));
        let t = [[0., 0., 0.], [2., 0., 0.], [0., 2., 0.]];
        let upper = distance2_upper([0.5, 0.5, 1.].map(Interval::point), t);
        assert!(upper >= 1. && upper < 1.000001);
    }
}
