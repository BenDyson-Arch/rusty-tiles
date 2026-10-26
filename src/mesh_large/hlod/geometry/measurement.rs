//! Tile-bounded topology eligibility on the candidate's final decoded surface.
//! Changed faces conservatively cover a closure patch; no planarity/diameter proxy.
use super::topology::Change;
use crate::{
    mesh_large::hlod::{
        error::Nearest,
        source_index::{self, Surface, Triangle, V3},
    },
    JobError,
};

#[derive(Debug, Default)]
pub(crate) struct Controls {
    pub source: Vec<f64>,
    pub proxy: Vec<f64>,
    pub closure_faces: Vec<bool>,
    pub closure_max: f64,
}

/// Quarter-grid, including every corner, edge midpoint, interior point and centroid.
/// Fixed controls ensure even a small closure gets sampled independently of area strata.
fn samples(p: [V3; 3]) -> impl Iterator<Item = V3> {
    (0..=4)
        .flat_map(move |a| {
            (0..=4 - a).map(move |b| {
                source_index::bary(p, [a as f64 / 4., b as f64 / 4., (4 - a - b) as f64 / 4.])
            })
        })
        .chain(std::iter::once(source_index::bary(p, [1. / 3.; 3])))
}

/// `source` returns an actual captured-source witness and its distance. Budget exhaustion
/// gives an upper distance, hence cannot make an over-budget closure eligible.
pub(crate) fn assess(
    change: &mut Change,
    decoded: &[[f32; 3]],
    triangles: &[[u32; 3]],
    e: f64,
    g_floor: f64,
    source: &mut dyn FnMut(V3, bool) -> Result<(V3, f64), JobError>,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<Option<Controls>, JobError> {
    let mut controls = Controls::default();
    if change.receipt.closed_holes == 0 && change.receipt.dropped_components == 0 {
        return Ok(Some(controls));
    }
    let mut nearest = Nearest::with_steps(super::super::error::NEAREST_STEPS);
    let mut hint = None;
    if change.receipt.closed_holes > 0 {
        if change.patch_faces.is_empty() || change.boundary.is_empty() {
            return Ok(None);
        }
        controls.closure_faces.resize(triangles.len(), false);
        let mut patch = Vec::with_capacity(change.patch_faces.len());
        for &i in &change.patch_faces {
            check()?;
            controls.closure_faces[i] = true;
            let t = Triangle::new(triangles[i].map(|v| decoded[v as usize]), i as u64);
            for p in samples(t.positions()) {
                let (_, d) = source(p, false)?;
                change.receipt.hole_patch_to_source = change.receipt.hole_patch_to_source.max(d);
                if d * 1.05 > e {
                    return Ok(None);
                }
                controls.proxy.push(d);
            }
            patch.push(t);
        }
        let patch = Surface::new(patch)?;
        for edge in &change.boundary {
            check()?;
            let [a, b] = edge.map(|p| p.map(f64::from));
            for k in 0..=4 {
                let p = std::array::from_fn(|i| a[i] + (b[i] - a[i]) * k as f64 / 4.);
                let (p, _) = source(p, true)?;
                let d = nearest.surface(&patch, p, &mut hint).distance2.sqrt();
                change.receipt.hole_boundary_to_patch =
                    change.receipt.hole_boundary_to_patch.max(d);
                if d * 1.05 > e {
                    return Ok(None);
                }
                controls.source.push(d);
            }
        }
        controls.closure_max = change
            .receipt
            .hole_patch_to_source
            .max(change.receipt.hole_boundary_to_patch);
    }
    if !change.dropped.is_empty() {
        let proxy = Surface::new(
            triangles
                .iter()
                .enumerate()
                .map(|(i, t)| Triangle::new(t.map(|v| decoded[v as usize]), i as u64))
                .collect(),
        )?;
        hint = None;
        for t in &change.dropped {
            check()?;
            for p in samples(t.map(|p| p.map(f64::from))) {
                let (p, _) = source(p, true)?;
                let d = nearest.surface(&proxy, p, &mut hint).distance2.sqrt();
                change.receipt.dropped_component_to_proxy =
                    change.receipt.dropped_component_to_proxy.max(d);
                controls.source.push(d);
            }
        }
    }
    change.receipt.hole_g_increase = (1.05 * controls.closure_max - g_floor).max(0.);
    change.receipt.dropped_component_g_increase =
        (1.05 * change.receipt.dropped_component_to_proxy - g_floor).max(0.);
    Ok(Some(controls))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh_large::hlod::geometry::topology::{self, tests::ring};

    #[test]
    fn courtyard_inradius_controls_closure_not_planarity_or_boundary_diameter() {
        for (half_width, budget, eligible) in [(1., 0.2, false), (0.1, 0.2, true)] {
            let mut m = ring();
            for p in &mut m.positions[4..] {
                p[0] = half_width * p[0].signum();
                p[1] = half_width * p[1].signum();
            }
            m.keys = m.positions.iter().map(|p| p.map(f32::to_bits)).collect();
            let mut filled = m.triangles.clone();
            filled.extend([[4, 5, 6], [4, 6, 7]]);
            let mut change = topology::changes(&m, &filled, &[], budget).unwrap();
            // Independent analytic distance to the square courtyard's rim, z = 0.
            let mut source = |p: V3, _| {
                let mut q = p;
                if p[0].abs() < f64::from(half_width) && p[1].abs() < f64::from(half_width) {
                    let axis = usize::from(p[1].abs() > p[0].abs());
                    q[axis] = if p[axis] < 0. {
                        -f64::from(half_width)
                    } else {
                        f64::from(half_width)
                    };
                }
                q[2] = 0.;
                Ok((q, source_index::distance2(p, q).sqrt()))
            };
            let result = assess(
                &mut change,
                &m.positions,
                &filled,
                budget,
                budget,
                &mut source,
                &|| Ok(()),
            )
            .unwrap();
            assert_eq!(result.is_some(), eligible);
            if let Some(c) = result {
                assert!((c.closure_max - f64::from(half_width)).abs() < 1e-8);
                assert!(
                    c.proxy.iter().any(|&d| d > 0.09),
                    "closure must enter S2's controls"
                );
                assert_eq!(change.receipt.hole_g_increase, 0.);
                assert_eq!(change.receipt.hole_boundary_to_patch, 0.);
            }
        }
    }
}
