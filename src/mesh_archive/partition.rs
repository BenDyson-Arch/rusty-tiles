//! Serial deterministic spatial membership; no output or publication ownership.
use super::source::Triangle;
use crate::{JobError, JobErrorKind};

pub(super) struct Leaf {
    pub triangles: Vec<usize>,
    pub bounds: Bounds,
}
#[derive(Clone, Copy)]
pub(super) struct Bounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
}
impl Bounds {
    pub fn empty() -> Self {
        Self {
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
        }
    }
    pub fn include(&mut self, triangle: &Triangle) {
        for [x, y, z] in triangle.positions {
            let point = [f64::from(x), -f64::from(z), f64::from(y)];
            for (axis, value) in point.into_iter().enumerate() {
                self.min[axis] = self.min[axis].min(value);
                self.max[axis] = self.max[axis].max(value);
            }
        }
    }
    pub fn box_values(self) -> [f64; 12] {
        let center = std::array::from_fn::<_, 3, _>(|i| (self.min[i] + self.max[i]) / 2.0);
        let half = std::array::from_fn::<_, 3, _>(|i| {
            if self.min[i] == self.max[i] {
                return 0.0;
            }
            // Round outward: midpoint/subtraction rounding must not put a
            // stored vertex just outside its emitted box at mixed magnitudes.
            (center[i] - self.min[i])
                .max(self.max[i] - center[i])
                .next_up()
        });
        [
            center[0], center[1], center[2], half[0], 0.0, 0.0, 0.0, half[1], 0.0, 0.0, 0.0,
            half[2],
        ]
    }
    pub fn diagonal(self) -> f64 {
        let values = self.box_values();
        [values[3], values[7], values[11]]
            .into_iter()
            .map(|h| (2.0 * h).powi(2))
            .sum::<f64>()
            .sqrt()
    }
}
fn centroid(triangle: &Triangle, axis: usize) -> f64 {
    triangle
        .positions
        .iter()
        .map(|p| f64::from(p[axis]))
        .sum::<f64>()
        / 3.0
}
pub(super) fn plan(
    triangles: &[Triangle],
    limit: usize,
    max_leaves: usize,
    mut check: impl FnMut() -> Result<(), JobError>,
) -> Result<(Vec<Leaf>, Bounds), JobError> {
    let mut indices: Vec<usize> = (0..triangles.len()).collect();
    let mut leaves = Vec::new();
    split(
        &mut indices,
        triangles,
        limit,
        max_leaves,
        &mut leaves,
        &mut check,
    )?;
    let mut bounds = Bounds::empty();
    for leaf in &leaves {
        check()?;
        let values = leaf.bounds.box_values();
        for (axis, half) in [values[3], values[7], values[11]].into_iter().enumerate() {
            bounds.min[axis] = bounds.min[axis].min(values[axis] - half);
            bounds.max[axis] = bounds.max[axis].max(values[axis] + half);
        }
    }
    Ok((leaves, bounds))
}
fn split(
    indices: &mut [usize],
    triangles: &[Triangle],
    limit: usize,
    max_leaves: usize,
    leaves: &mut Vec<Leaf>,
    check: &mut impl FnMut() -> Result<(), JobError>,
) -> Result<(), JobError> {
    check()?;
    if indices.len() <= limit {
        if leaves.len() == max_leaves {
            return Err(JobError::new(
                JobErrorKind::Unsupported,
                "mesh partition exceeds F1a leaf admission limit",
            ));
        }
        let mut bounds = Bounds::empty();
        for &index in indices.iter() {
            check()?;
            bounds.include(&triangles[index]);
        }
        leaves.push(Leaf {
            triangles: indices.to_vec(),
            bounds,
        });
        return Ok(());
    }
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for &index in indices.iter() {
        check()?;
        for axis in 0..3 {
            let value = centroid(&triangles[index], axis);
            low[axis] = low[axis].min(value);
            high[axis] = high[axis].max(value);
        }
    }
    let mut axis = 0;
    for candidate in 1..3 {
        if high[candidate] - low[candidate] > high[axis] - low[axis] {
            axis = candidate;
        }
    }
    indices.sort_unstable_by(|a, b| {
        centroid(&triangles[*a], axis)
            .total_cmp(&centroid(&triangles[*b], axis))
            .then_with(|| a.cmp(b))
    });
    let midpoint = indices.len() / 2;
    let (left, right) = indices.split_at_mut(midpoint);
    split(left, triangles, limit, max_leaves, leaves, check)?;
    split(right, triangles, limit, max_leaves, leaves, check)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_magnitude_boxes_enclose_stored_positions_without_inward_rounding() {
        let triangle = Triangle {
            positions: [[-1e-38, 0.0, 0.0], [1e6, 0.0, 0.0], [1.0, 0.0, 0.0]],
            normals: None,
            tangents: None,
            texcoords: [None; 2],
            colors: None,
            material: None,
        };
        let mut bounds = Bounds::empty();
        bounds.include(&triangle);
        let b = bounds.box_values();
        assert!(b[0] - b[3] <= f64::from(triangle.positions[0][0]));
        assert!(b[0] + b[3] >= 1e6);
        assert_eq!(b[7], 0.0);
        assert_eq!(b[11], 0.0);
    }
    #[test]
    fn coincident_triangles_keep_multiplicity_and_obey_actual_leaf_cap() {
        let triangle = Triangle {
            positions: [[0.0; 3]; 3],
            normals: None,
            tangents: None,
            texcoords: [None; 2],
            colors: None,
            material: None,
        };
        let triangles = vec![triangle; 10];
        let (leaves, _) = plan(&triangles, 3, 4, || Ok(())).unwrap();
        assert_eq!(leaves.len(), 4);
        assert!(leaves.iter().all(|leaf| leaf.triangles.len() <= 3));
        let mut indices: Vec<_> = leaves
            .iter()
            .flat_map(|leaf| leaf.triangles.iter().copied())
            .collect();
        indices.sort_unstable();
        assert_eq!(indices, (0..10).collect::<Vec<_>>());
        assert_eq!(
            plan(&triangles, 3, 3, || Ok(())).err().unwrap().kind(),
            JobErrorKind::Unsupported
        );
    }
}
