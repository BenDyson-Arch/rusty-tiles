//! Oriented `boundingVolume.box` geometry (3D Tiles 1.1).

/// 12-element `boundingVolume.box`: center + half-axes (column-wise x, y, z).
pub type BoundingBox = [f64; 12];

/// Oriented bounding box: `center + Σ t_k · half[k] · axes[k]`, |t_k| ≤ 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obb {
    pub center: [f64; 3],
    /// Orthonormal, row k is axis k.
    pub axes: [[f64; 3]; 3],
    pub half: [f64; 3],
}

const AXIS_ALIGNED: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

impl Obb {
    pub fn from_box(b: BoundingBox) -> Obb {
        let mut axes = AXIS_ALIGNED;
        let mut half = [0.0; 3];
        for k in 0..3 {
            let v = [b[3 + k * 3], b[4 + k * 3], b[5 + k * 3]];
            let n = norm(v);
            half[k] = n;
            if n > 0.0 {
                axes[k] = [v[0] / n, v[1] / n, v[2] / n];
            }
        }
        Obb {
            center: [b[0], b[1], b[2]],
            axes,
            half,
        }
    }

    #[allow(clippy::needless_range_loop)]
    pub fn corners(&self) -> [[f64; 3]; 8] {
        let mut out = [[0.0; 3]; 8];
        for (n, c) in out.iter_mut().enumerate() {
            let s = [
                if n & 1 == 0 { -1.0 } else { 1.0 },
                if n & 2 == 0 { -1.0 } else { 1.0 },
                if n & 4 == 0 { -1.0 } else { 1.0 },
            ];
            for i in 0..3 {
                c[i] = self.center[i]
                    + s[0] * self.half[0] * self.axes[0][i]
                    + s[1] * self.half[1] * self.axes[1][i]
                    + s[2] * self.half[2] * self.axes[2][i];
            }
        }
        out
    }

    pub fn contains(&self, p: [f64; 3], eps: f64) -> bool {
        let d = [
            p[0] - self.center[0],
            p[1] - self.center[1],
            p[2] - self.center[2],
        ];
        (0..3).all(|k| dot(d, self.axes[k]).abs() <= self.half[k] + eps)
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

/// Eigenvectors of a symmetric 3×3 (cyclic Jacobi), rows sorted by
/// descending eigenvalue, made right-handed.
#[allow(clippy::needless_range_loop)]
pub(crate) fn jacobi_eigenvectors(mut a: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut v = AXIS_ALIGNED;
    for _ in 0..32 {
        let off = a[0][1].abs() + a[0][2].abs() + a[1][2].abs();
        if off < 1e-18 {
            break;
        }
        for (p, q) in [(0usize, 1usize), (0, 2), (1, 2)] {
            if a[p][q].abs() < 1e-300 {
                continue;
            }
            let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
            let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
            let t = if theta == 0.0 { 1.0 } else { t };
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            for k in 0..3 {
                let (akp, akq) = (a[k][p], a[k][q]);
                a[k][p] = c * akp - s * akq;
                a[k][q] = s * akp + c * akq;
            }
            for k in 0..3 {
                let (apk, aqk) = (a[p][k], a[q][k]);
                a[p][k] = c * apk - s * aqk;
                a[q][k] = s * apk + c * aqk;
            }
            for k in 0..3 {
                let (vkp, vkq) = (v[k][p], v[k][q]);
                v[k][p] = c * vkp - s * vkq;
                v[k][q] = s * vkp + c * vkq;
            }
        }
    }
    // Columns of v are eigenvectors; emit as rows, largest eigenvalue first.
    let mut order = [0usize, 1, 2];
    order.sort_by(|&i, &j| {
        a[j][j]
            .partial_cmp(&a[i][i])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut axes = [[0.0; 3]; 3];
    for (r, &c) in order.iter().enumerate() {
        axes[r] = [v[0][c], v[1][c], v[2][c]];
        let n = norm(axes[r]);
        if n > 0.0 {
            for x in &mut axes[r] {
                *x /= n;
            }
        }
    }
    let cross = [
        axes[0][1] * axes[1][2] - axes[0][2] * axes[1][1],
        axes[0][2] * axes[1][0] - axes[0][0] * axes[1][2],
        axes[0][0] * axes[1][1] - axes[0][1] * axes[1][0],
    ];
    if dot(cross, axes[2]) < 0.0 {
        for x in &mut axes[2] {
            *x = -*x;
        }
    }
    axes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jacobi_axes_sort_by_eigenvalue_and_are_right_handed() {
        // Eigenvalues 1, 9 and 4 along x, y and z.
        let axes = jacobi_eigenvectors([[1., 0., 0.], [0., 9., 0.], [0., 0., 4.]]);
        assert_eq!(axes[0].map(f64::abs), [0., 1., 0.]);
        assert_eq!(axes[1].map(f64::abs), [0., 0., 1.]);
        let cross = [
            axes[0][1] * axes[1][2] - axes[0][2] * axes[1][1],
            axes[0][2] * axes[1][0] - axes[0][0] * axes[1][2],
            axes[0][0] * axes[1][1] - axes[0][1] * axes[1][0],
        ];
        assert_eq!(dot(cross, axes[2]), 1.);
        // A 45° rotation about z of eigenvalues 9, 1, 4.
        let axes = jacobi_eigenvectors([[5., 4., 0.], [4., 5., 0.], [0., 0., 4.]]);
        let s = std::f64::consts::FRAC_1_SQRT_2;
        assert!((axes[0][0].abs() - s).abs() < 1e-12 && (axes[0][1].abs() - s).abs() < 1e-12);
        assert!((axes[1][2].abs() - 1.).abs() < 1e-12);
    }

    #[test]
    fn box_corners_are_contained() {
        let obb = Obb::from_box([1., 2., 3., 2., 0., 0., 0., 0., 3., 0., -1., 0.]);
        for corner in obb.corners() {
            assert!(obb.contains(corner, 1e-12));
        }
        assert!(!obb.contains([1., 2., 7.], 1e-12));
    }
}
