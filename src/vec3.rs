//! Minimal `[f64; 3]` helpers shared by the tilers and validator.
//!
//! Evaluation order is part of the contract: published bounds and errors are
//! computed with exactly these roundings, so do not "simplify" the formulas.
#![cfg_attr(not(feature = "native-geospatial"), allow(dead_code))]

pub(crate) type Vec3 = [f64; 3];

pub(crate) fn add(a: Vec3, b: Vec3) -> Vec3 {
    std::array::from_fn(|i| a[i] + b[i])
}

pub(crate) fn sub(a: Vec3, b: Vec3) -> Vec3 {
    std::array::from_fn(|i| a[i] - b[i])
}

pub(crate) fn mul(a: Vec3, s: f64) -> Vec3 {
    a.map(|v| v * s)
}

/// Left-to-right sum of products (`Iterator::sum`), as every caller used.
pub(crate) fn dot(a: Vec3, b: Vec3) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

/// `sqrt(dot(a, a))`; overflows to infinity for huge components.
pub(crate) fn norm(a: Vec3) -> f64 {
    dot(a, a).sqrt()
}

/// Overflow-safe length (`hypot` chain). Not bitwise equal to [`norm`]; the
/// point-cloud tiler's published bounds depend on this form.
pub(crate) fn norm_hypot(a: Vec3) -> f64 {
    a[0].hypot(a[1]).hypot(a[2])
}

/// glTF Y-up → 3D Tiles Z-up, at the caller's precision.
pub(crate) fn y_up_to_z_up<T: Copy + std::ops::Neg<Output = T>>(p: [T; 3]) -> [T; 3] {
    [p[0], -p[2], p[1]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_keep_their_rounding_and_precision() {
        assert_eq!(dot([1., 2., 3.], [4., 5., 6.]), 32.);
        assert_eq!(norm([3., 4., 12.]), 13.);
        assert!(norm([1e200, 0., 0.]).is_infinite());
        assert_eq!(norm_hypot([1e200, 0., 0.]), 1e200);
        assert_eq!(
            add(sub([1., 2., 3.], [1., 1., 1.]), mul([1., 1., 1.], 2.)),
            [2., 3., 4.]
        );
        assert_eq!(y_up_to_z_up([1., 2., 3.]), [1., -3., 2.]);
        assert_eq!(
            y_up_to_z_up([1_f32, 2., 3.]),
            crate::bbox::y_up_to_z_up([1., 2., 3.])
        );
    }
}
