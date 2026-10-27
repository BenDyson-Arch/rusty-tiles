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
    pub opening_faces: Vec<bool>,
    pub opening_source_max: f64,
    pub opening_proxy_max: f64,
    opening: Option<Opening>,
}

struct Opening {
    reference: Surface,
    nearest: Nearest,
    hint: Option<usize>,
}
impl std::fmt::Debug for Opening {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Opening")
            .field("faces", &self.reference.triangles.len())
            .finish()
    }
}
impl Controls {
    /// Final source strata on the conservative lost-surface domain participate in
    /// eligibility too. If the bounded correspondence search exhausts its budget,
    /// attribute the sample conservatively rather than silently excluding it.
    pub(crate) fn source_sample(&mut self, p: V3, d: f64) {
        if let Some(opening) = &mut self.opening {
            let exhausted = opening.nearest.stats.exhausted;
            let hit = opening
                .nearest
                .surface(&opening.reference, p, &mut opening.hint);
            if hit.rank & 1 != 0 || opening.nearest.stats.exhausted != exhausted {
                self.opening_source_max = self.opening_source_max.max(d);
            }
        }
    }
    pub(crate) fn proxy_sample(&mut self, rank: usize, d: f64) {
        if self.closure_faces.get(rank) == Some(&true) {
            self.closure_max = self.closure_max.max(d);
        }
        if self.opening_faces.get(rank) == Some(&true) {
            self.opening_proxy_max = self.opening_proxy_max.max(d);
        }
    }
    pub(crate) fn eligible(&self, e: f64) -> bool {
        1.05 * self
            .closure_max
            .max(self.opening_source_max)
            .max(self.opening_proxy_max)
            <= e
    }
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

/// Eighth-grid plus centroid (the quarter grid included): a moved boundary's strip is
/// thin, so its faces get the finer fixed density.
fn fine_samples(p: [V3; 3]) -> impl Iterator<Item = V3> {
    (0..=8)
        .flat_map(move |a| {
            (0..=8 - a).map(move |b| {
                source_index::bary(p, [a as f64 / 8., b as f64 / 8., (8 - a - b) as f64 / 8.])
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
    let _cpu = super::super::cpu_profile::start(
        super::super::cpu_profile::Stage::Measurement,
        triangles.len(),
    );
    let mut controls = Controls::default();
    if change.opening_border_contact {
        return Ok(None);
    }
    let strip = &change.strip;
    let moved = !strip.lost_points.is_empty() || !strip.added_edges.is_empty();
    if change.receipt.closed_holes == 0
        && change.receipt.dropped_components == 0
        && change.receipt.opened_holes == 0
        && !moved
    {
        return Ok(Some(controls));
    }
    let mut nearest = Nearest::with_steps(super::super::error::NEAREST_STEPS);
    let mut hint = None;
    // The decoded proxy, for every lost-surface control below.
    let proxy = if moved
        || !change.dropped.is_empty()
        || change.receipt.opened_holes > 0
        || (change.receipt.closed_holes > 0 && !change.closure_centroids.is_empty())
    {
        Some(Surface::new(
            triangles
                .iter()
                .enumerate()
                .map(|(i, t)| Triangle::new(t.map(|v| decoded[v as usize]), i as u64))
                .collect(),
        )?)
    } else {
        None
    };
    if moved {
        // A moved open boundary bounds G, not E: these witnesses only join S2's fixed
        // controls. Changed input surface behind it is measured to the decoded proxy;
        // added proxy surface to the source, at every eighth of an added edge and on
        // the faces along it.
        let proxy = proxy.as_ref().expect("built for a moved boundary");
        let along = |[a, b]: [[f32; 3]; 2]| {
            let [a, b] = [a, b].map(|p| p.map(f64::from));
            (0..=8).map(move |k| std::array::from_fn(|i| a[i] + (b[i] - a[i]) * k as f64 / 8.))
        };
        for (i, &p) in strip.lost_points.iter().enumerate() {
            if i % 256 == 0 {
                check()?;
            }
            controls
                .source
                .push(nearest.surface(proxy, p, &mut hint).distance2.sqrt());
        }
        for &edge in &strip.added_edges {
            check()?;
            for p in along(edge) {
                controls.proxy.push(source(p, false)?.1);
            }
        }
        for &i in &strip.added_faces {
            check()?;
            for p in fine_samples(triangles[i].map(|v| decoded[v as usize].map(f64::from))) {
                controls.proxy.push(source(p, false)?.1);
            }
        }
        hint = None;
    }
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
            // The oracle's grid: the eighth grid holds the quarter grid and centroid.
            for p in fine_samples(t.positions()) {
                let (_, d) = source(p, false)?;
                change.receipt.hole_patch_to_source = change.receipt.hole_patch_to_source.max(d);
                if d * 1.05 > e {
                    return Ok(None);
                }
                controls.proxy.push(d);
            }
            patch.push(t);
        }
        // The oracle's other closure witnesses: each cycle centroid of the closing
        // boundary, projected onto the proxy, against source.
        if let Some(proxy) = &proxy {
            for &c in &change.closure_centroids {
                check()?;
                let hit = nearest.surface(proxy, c, &mut hint);
                let face = triangles[hit.rank as usize].map(|v| decoded[v as usize].map(f64::from));
                let (_, d) = source(source_index::bary(face, hit.bary), false)?;
                change.receipt.hole_patch_to_source = change.receipt.hole_patch_to_source.max(d);
                if d * 1.05 > e {
                    return Ok(None);
                }
                controls.proxy.push(d);
            }
            hint = None;
        }
        // Closure locations conservatively include the entire branching boundary.
        // Its retained parts are covered by unchanged faces, not necessarily by a
        // changed patch face. Keep those real witnesses in the same component;
        // excluding them would measure distance to an artificial hole in the index.
        patch.extend(
            change
                .closure_carriers
                .iter()
                .map(|&i| Triangle::new(triangles[i].map(|v| decoded[v as usize]), i as u64)),
        );
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
    if let Some(proxy) = &proxy {
        hint = None;
        for t in &change.dropped {
            check()?;
            for p in samples(t.map(|p| p.map(f64::from))) {
                let (p, _) = source(p, true)?;
                let d = nearest.surface(proxy, p, &mut hint).distance2.sqrt();
                change.receipt.dropped_component_to_proxy =
                    change.receipt.dropped_component_to_proxy.max(d);
                controls.source.push(d);
            }
        }
        if change.receipt.opened_holes > 0 {
            if change.opening_boundary.is_empty()
                || !change.opening_reference.iter().any(|(_, lost)| *lost)
            {
                return Ok(None);
            }
            for (t, lost) in &change.opening_reference {
                if !lost {
                    continue;
                }
                check()?;
                for p in samples(t.map(|p| p.map(f64::from))) {
                    let (p, _) = source(p, true)?;
                    let d = nearest.surface(proxy, p, &mut hint).distance2.sqrt();
                    change.receipt.opened_hole_to_proxy =
                        change.receipt.opened_hole_to_proxy.max(d);
                    if 1.05 * d > e {
                        return Ok(None);
                    }
                    controls.source.push(d);
                }
            }
            for edge in &change.opening_boundary {
                check()?;
                let [a, b] = edge.map(|p| p.map(f64::from));
                for k in 0..=4 {
                    let p = std::array::from_fn(|i| a[i] + (b[i] - a[i]) * k as f64 / 4.);
                    let (_, d) = source(p, false)?;
                    change.receipt.opened_hole_boundary_to_source =
                        change.receipt.opened_hole_boundary_to_source.max(d);
                    if 1.05 * d > e {
                        return Ok(None);
                    }
                    controls.proxy.push(d);
                }
            }
            controls.opening_faces.clone_from(&change.opening_faces);
            controls.opening_source_max = change.receipt.opened_hole_to_proxy;
            controls.opening_proxy_max = change.receipt.opened_hole_boundary_to_source;
            controls.opening = Some(Opening {
                reference: Surface::new(
                    change
                        .opening_reference
                        .iter()
                        .enumerate()
                        .map(|(i, (t, lost))| Triangle::new(*t, 2 * i as u64 + u64::from(*lost)))
                        .collect(),
                )?,
                nearest: Nearest::with_steps(super::super::error::NEAREST_STEPS),
                hint: None,
            });
        }
    }
    change.receipt.hole_g_increase = (1.05 * controls.closure_max - g_floor).max(0.);
    change.receipt.dropped_component_g_increase =
        (1.05 * change.receipt.dropped_component_to_proxy - g_floor).max(0.);
    change.receipt.opened_hole_g_increase =
        (1.05 * controls.opening_source_max.max(controls.opening_proxy_max) - g_floor).max(0.);
    Ok(Some(controls))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh_large::hlod::geometry::topology::{self, tests::ring};

    #[test]
    fn lost_surface_controls_and_reference_index_fit_the_parent_charge() {
        let n = 16u32;
        let mut m = ring();
        m.positions = (0..=n)
            .flat_map(|y| (0..=n).map(move |x| [x as f32 / 64., y as f32 / 64., 0.]))
            .collect();
        m.keys = m.positions.iter().map(|p| p.map(f32::to_bits)).collect();
        m.triangles.clear();
        let mut output = Vec::new();
        for y in 0..n {
            for x in 0..n {
                let a = y * (n + 1) + x;
                m.triangles
                    .extend([[a, a + 1, a + n + 2], [a, a + n + 2, a + n + 1]]);
                if (x, y) != (n / 2, n / 2) {
                    // Every diagonal changes; the conservative loss controls must cover
                    // the whole changed region while remaining bounded to this parent.
                    output.extend([[a, a + 1, a + n + 1], [a + 1, a + n + 2, a + n + 1]]);
                }
            }
        }
        let (controls, peak) = crate::test_alloc::peak(|| {
            let mut change = topology::changes(&m, &output, &[], 0.2).unwrap();
            assert_eq!(change.receipt.opened_holes, 1);
            assess(
                &mut change,
                &m.positions,
                &output,
                0.2,
                0.2,
                &mut |p, _| Ok((p, 0.)),
                &|| Ok(()),
            )
            .unwrap()
            .unwrap()
        });
        assert!(controls.source.len() >= 16 * m.triangles.len());
        assert!(controls.opening_source_max > 0.);
        assert!(controls.eligible(0.2));
        let charge = m.triangles.len() as u64 * super::super::super::JOB_BYTES_PER_TRIANGLE
            + super::super::super::error::search_bytes(super::super::super::error::NEAREST_STEPS);
        eprintln!(
            "LOST SURFACE: peak {peak} bytes, charge {charge}, {} controls",
            controls.source.len()
        );
        assert!(
            peak as u64 <= charge,
            "lost-surface peak {peak} > parent charge {charge}"
        );
    }

    #[test]
    fn opened_hole_loss_is_budgeted_and_junction_or_edge_contact_is_refused() {
        use crate::mesh_large::hlod::interfaces::InterfaceRow;
        for (half_width, eligible) in [(1., false), (0.1, true)] {
            let mut m = ring();
            for p in &mut m.positions[4..] {
                p[0] = half_width * p[0].signum();
                p[1] = half_width * p[1].signum();
            }
            m.keys = m.positions.iter().map(|p| p.map(f32::to_bits)).collect();
            let opened = m.triangles.clone();
            m.triangles.extend([[4, 5, 6], [4, 6, 7]]);
            let mut change = topology::changes(&m, &opened, &[], 0.2).unwrap();
            assert_eq!(change.receipt.opened_holes, 1);
            assert_eq!(change.receipt.closed_holes, 0);
            assert_eq!(
                change
                    .opening_reference
                    .iter()
                    .filter(|(_, lost)| *lost)
                    .count(),
                2
            );
            let mut source = |p: V3, _| Ok(([p[0], p[1], 0.], p[2].abs()));
            let result = assess(
                &mut change,
                &m.positions,
                &opened,
                0.2,
                0.2,
                &mut source,
                &|| Ok(()),
            )
            .unwrap();
            assert_eq!(result.is_some(), eligible);
            if let Some(mut controls) = result {
                assert!((change.receipt.opened_hole_to_proxy - f64::from(half_width)).abs() < 1e-8);
                assert!(
                    controls.source.iter().any(|&d| d > 0.09),
                    "opening must enter final S2 controls"
                );
                assert_eq!(change.receipt.opened_hole_boundary_to_source, 0.);
                assert_eq!(change.receipt.opened_hole_g_increase, 0.);
                change.receipt.published(0.2);
                assert_eq!(change.receipt.opened_hole_final_g, 0.2);
                assert_eq!(change.receipt.closed_hole_final_g, 0.);
                controls.source_sample([0., 0., 0.], 0.21);
                assert!(
                    !controls.eligible(0.2),
                    "later S2 lost-region strata can reject an opening"
                );
            }
            for b in [m.keys[4], m.keys[5]] {
                let interface = [InterfaceRow::new(0, m.keys[4], b, 0, 1)];
                let mut change = topology::changes(&m, &opened, &interface, 0.2).unwrap();
                assert!(change.opening_border_contact);
                assert!(assess(
                    &mut change,
                    &m.positions,
                    &opened,
                    0.2,
                    0.2,
                    &mut |_, _| panic!("border opening must be rejected before source work"),
                    &|| Ok(())
                )
                .unwrap()
                .is_none());
            }
        }
    }

    #[test]
    fn closure_on_a_branching_boundary_keeps_unchanged_surface_witnesses() {
        let mut m = ring();
        // A retained wing shares the inner loop's vertex. Both boundary cycles
        // belong to one branching boundary, but only the courtyard is closed.
        m.positions.extend([[-0.1, -0.1, 4.], [-0.1, 0.9, 4.]]);
        m.keys = m.positions.iter().map(|p| p.map(f32::to_bits)).collect();
        m.triangles.push([4, 8, 9]);
        let mut filled = m.triangles.clone();
        filled.extend([[4, 5, 6], [4, 6, 7]]);
        let source = Surface::new(
            m.triangles
                .iter()
                .enumerate()
                .map(|(i, t)| Triangle::new(t.map(|v| m.positions[v as usize]), i as u64))
                .collect(),
        )
        .unwrap();
        for retain_carriers in [false, true] {
            let mut change = topology::changes(&m, &filled, &[], 0.2).unwrap();
            assert_eq!(change.receipt.closed_holes, 1);
            assert_eq!(change.receipt.opened_holes, 0);
            assert!(change.closure_carriers.contains(&8));
            if !retain_carriers {
                change.closure_carriers.clear();
            }
            let mut nearest = Nearest::with_steps(super::super::super::error::NEAREST_STEPS);
            let mut hint = None;
            let result = assess(
                &mut change,
                &m.positions,
                &filled,
                0.2,
                0.2,
                &mut |p, _| {
                    let hit = nearest.surface(&source, p, &mut hint);
                    let triangle = m.triangles[hit.rank as usize]
                        .map(|v| m.positions[v as usize].map(f64::from));
                    Ok((source_index::bary(triangle, hit.bary), hit.distance2.sqrt()))
                },
                &|| Ok(()),
            )
            .unwrap();
            assert_eq!(result.is_some(), retain_carriers);
            if let Some(controls) = result {
                assert!(controls.closure_max > 0.09 && controls.closure_max < 0.11);
                assert!(change.receipt.hole_boundary_to_patch < 1e-12);
                assert!(controls.proxy.iter().any(|&d| d > 0.09));
            } else {
                assert!(change.receipt.hole_boundary_to_patch * 1.05 > 0.2);
            }
        }
    }

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
