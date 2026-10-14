//! Deterministic same-primitive, fixed-seed-cone charts and translation packing.
use super::{
    invalid,
    math::{self, V3},
    resource,
    types::{BakeLimits, FrozenNode},
    unsupported,
};
use crate::{mesh_large::config::Profile, JobError};
use std::collections::{BTreeMap, VecDeque};
type P2 = [f64; 2];
pub(super) struct Chart {
    pub faces: Vec<usize>,
    pub material: u32,
    pub normal: V3,
    pub x: V3,
    pub y: V3,
    pub lo: P2,
    pub hi: P2,
    pub sigma: f64,
    pub pixels_per_metre: f64,
    pub page: usize,
    pub origin: [u32; 2],
    pub size: [u32; 2],
}
pub(super) struct Page {
    pub material: u32,
    pub size: [u32; 2],
    pub charts: Vec<usize>,
}
pub(super) struct Layout {
    pub charts: Vec<Chart>,
    pub pages: Vec<Page>,
    pub uv: Vec<[[f32; 2]; 3]>,
    /// Source binding/ID-set participation resolved by the owning bake boundary.
    /// False faces keep normals/COLOR/corners without UV sampling obligations.
    pub raster_faces: Vec<bool>,
    /// Bound faces whose exact page footprint is under one texel and whose 16-bit UVs
    /// collapse: demoted to no-raster (geometry, corners and features kept).
    pub subtexel_no_raster: usize,
    pub texel_size: f64,
    pub gutter: u32,
}
fn project(p: V3, x: V3, y: V3) -> P2 {
    [math::dot(p, x), math::dot(p, y)]
}
fn orient(a: P2, b: P2, c: P2) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
/// A separating axis proves no strict interior overlap. Near-zero arithmetic
/// has no such proof and conservatively splits the chart.
fn overlap(a: [P2; 3], b: [P2; 3]) -> bool {
    for t in [a, b] {
        for i in 0..3 {
            let edge = [t[(i + 1) % 3][0] - t[i][0], t[(i + 1) % 3][1] - t[i][1]];
            let axis = [-edge[1], edge[0]];
            let dot = |p: P2| axis[0] * p[0] + axis[1] * p[1];
            let av = a.map(dot);
            let bv = b.map(dot);
            let amin = av.into_iter().fold(f64::INFINITY, f64::min);
            let amax = av.into_iter().fold(f64::NEG_INFINITY, f64::max);
            let bmin = bv.into_iter().fold(f64::INFINITY, f64::min);
            let bmax = bv.into_iter().fold(f64::NEG_INFINITY, f64::max);
            let guard =
                32. * f64::EPSILON * amin.abs().max(amax.abs()).max(bmin.abs()).max(bmax.abs());
            if amax < bmin - guard || bmax < amin - guard {
                return false;
            }
            // Exact common edge, with the remaining corners strictly on opposite sides.
            let shared: Vec<P2> = a.into_iter().filter(|p| b.contains(p)).collect();
            if shared.len() == 2 {
                let ac = a.into_iter().find(|p| !shared.contains(p)).unwrap();
                let bc = b.into_iter().find(|p| !shared.contains(p)).unwrap();
                let aa = orient(shared[0], shared[1], ac);
                let bb = orient(shared[0], shared[1], bc);
                if aa * bb < 0. {
                    return false;
                }
            }
        }
    }
    true
}
pub(super) fn build(
    f: &FrozenNode,
    raster_faces: &[bool],
    limits: BakeLimits,
    texel: f64,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<Layout, JobError> {
    if raster_faces.len() != f.faces.len()
        || !texel.is_finite()
        || texel <= 0.
        || limits.max_page_edge < 4
        || !limits.max_page_edge.is_power_of_two()
    {
        return Err(invalid("proxy chart density/page policy"));
    }
    let n = f.faces.len();
    if (n as u64)
        .checked_mul(1024)
        .is_none_or(|b| b > limits.memory_bytes)
    {
        return Err(resource("proxy chart/corner memory"));
    }
    let mut edges: BTreeMap<(_, _, _), Vec<usize>> = BTreeMap::new();
    for (i, face) in f.faces.iter().enumerate() {
        check()?;
        for e in 0..3 {
            let mut a = face.welds[e];
            let mut b = face.welds[(e + 1) % 3];
            if b < a {
                std::mem::swap(&mut a, &mut b);
            }
            edges.entry((face.source, a, b)).or_default().push(i);
        }
    }
    let mut adjacent = vec![Vec::new(); n];
    for faces in edges.values() {
        if faces.len() == 2 {
            adjacent[faces[0]].push(faces[1]);
            adjacent[faces[1]].push(faces[0]);
        }
    }
    for a in &mut adjacent {
        a.sort_unstable();
    }
    drop(edges);
    let mut assigned = vec![false; n];
    let mut charts = Vec::new();
    let mut work = 0u64;
    for seed in 0..n {
        if assigned[seed] {
            continue;
        }
        if !raster_faces[seed] {
            assigned[seed] = true;
            continue;
        }
        check()?;
        let face = &f.faces[seed];
        let p = face.positions.map(|p| p.map(f64::from));
        // Zero-area faces retain their immutable corner map and finite default
        // UVs, but own no atlas chart/dimension/fidelity obligation. A very short
        // collinear edge must not force an enormous raster allocation.
        let Some(normal) = math::face_normal(p) else {
            assigned[seed] = true;
            continue;
        };
        let axis = (0..3)
            .min_by(|a, b| normal[*a].abs().total_cmp(&normal[*b].abs()))
            .unwrap();
        let mut canonical = [0.; 3];
        canonical[axis] = 1.;
        let x = math::unit(math::cross(canonical, normal)).unwrap();
        let y = math::cross(normal, x);
        let mut chart = Chart {
            faces: vec![seed],
            material: face.material,
            normal,
            x,
            y,
            lo: [f64::INFINITY; 2],
            hi: [f64::NEG_INFINITY; 2],
            sigma: 1.,
            pixels_per_metre: 0.,
            page: 0,
            origin: [0; 2],
            size: [0; 2],
        };
        assigned[seed] = true;
        let mut queue = VecDeque::from([seed]);
        while let Some(current) = queue.pop_front() {
            for &candidate in &adjacent[current] {
                check()?;
                if assigned[candidate] || !raster_faces[candidate] {
                    continue;
                }
                let c = &f.faces[candidate];
                if c.source != face.source || c.material != face.material {
                    continue;
                }
                let cp = c.positions.map(|p| p.map(f64::from));
                let Some(cn) = math::face_normal(cp) else {
                    continue;
                };
                let sigma = math::dot(normal, cn);
                if sigma < 0.5 {
                    continue;
                }
                let projected = cp.map(|p| project(p, x, y));
                let mut intersects = false;
                for &other in &chart.faces {
                    work = work
                        .checked_add(1)
                        .ok_or_else(|| resource("chart overlap work"))?;
                    if work > limits.query_work {
                        return Err(resource("chart overlap work budget"));
                    }
                    if overlap(
                        projected,
                        f.faces[other]
                            .positions
                            .map(|p| project(p.map(f64::from), x, y)),
                    ) {
                        intersects = true;
                        break;
                    }
                }
                if intersects {
                    continue;
                }
                assigned[candidate] = true;
                chart.sigma = chart.sigma.min(sigma);
                chart.faces.push(candidate);
                queue.push_back(candidate);
            }
        }
        chart.faces.sort_unstable();
        for &i in &chart.faces {
            for p in f.faces[i].positions {
                let p = project(p.map(f64::from), x, y);
                for (a, value) in p.into_iter().enumerate() {
                    chart.lo[a] = chart.lo[a].min(value);
                    chart.hi[a] = chart.hi[a].max(value);
                }
            }
        }
        charts.push(chart);
    }
    let gutter = if limits.profile == Profile::Lossless {
        4
    } else {
        2
    };
    let mut boosts = vec![1.; charts.len()];
    for attempt in 0..4 {
        let mut pages: Vec<Page> = Vec::new();
        // Stable chart order within each material; shelf state resets only at page boundaries.
        let mut order: Vec<usize> = (0..charts.len()).collect();
        order.sort_by_key(|&i| (charts[i].material, charts[i].faces[0]));
        let mut shelf = (0u32, 0u32, 0u32);
        let mut current = None;
        for i in order {
            let c = &mut charts[i];
            c.pixels_per_metre = boosts[i] / (texel * c.sigma);
            for a in 0..2 {
                let interior = ((c.hi[a] - c.lo[a]) * c.pixels_per_metre).ceil().max(1.);
                if !interior.is_finite() || interior > f64::from(limits.max_page_edge) {
                    return Err(resource("proxy chart physical dimension"));
                }
                c.size[a] = (interior as u32)
                    .checked_add(2 * gutter + 2)
                    .ok_or_else(|| resource("proxy chart padding dimension"))?;
            }
            if c.size.iter().any(|s| *s > limits.max_page_edge) {
                return Err(resource("proxy chart exceeds admitted page edge"));
            }
            if current.is_none_or(|p: usize| pages[p].material != c.material) {
                current = Some(pages.len());
                pages.push(Page {
                    material: c.material,
                    size: [4; 2],
                    charts: Vec::new(),
                });
                shelf = (0, 0, 0);
            }
            if shelf.0 + c.size[0] > limits.max_page_edge {
                shelf.0 = 0;
                shelf.1 += shelf.2;
                shelf.2 = 0;
            }
            if shelf.1 + c.size[1] > limits.max_page_edge {
                current = Some(pages.len());
                pages.push(Page {
                    material: c.material,
                    size: [4; 2],
                    charts: Vec::new(),
                });
                shelf = (0, 0, 0);
            }
            let page = current.unwrap();
            c.page = page;
            c.origin = [shelf.0, shelf.1];
            pages[page].charts.push(i);
            pages[page].size[0] = pages[page].size[0].max(shelf.0 + c.size[0]);
            pages[page].size[1] = pages[page].size[1].max(shelf.1 + c.size[1]);
            shelf.0 += c.size[0];
            shelf.2 = shelf.2.max(c.size[1]);
        }
        for p in &mut pages {
            p.size = p.size.map(u32::next_power_of_two);
        }
        let mut uv = vec![[[0.; 2]; 3]; n];
        let mut failed = Vec::new();
        let mut demoted = Vec::new();
        for (ci, c) in charts.iter().enumerate() {
            let size = pages[c.page].size;
            for &i in &c.faces {
                let p = f.faces[i].positions.map(|p| p.map(f64::from));
                let mut texels = [[0.; 2]; 3];
                for (j, point) in p.into_iter().enumerate() {
                    let q = project(point, c.x, c.y);
                    for a in 0..2 {
                        let px = f64::from(c.origin[a] + gutter + 1)
                            + (q[a] - c.lo[a]) * c.pixels_per_metre;
                        texels[j][a] = px;
                        let v = px / f64::from(size[a]);
                        uv[i][j][a] = if limits.profile == Profile::Lossless {
                            v as f32
                        } else {
                            ((v * 65535.).round() as u16) as f32 / 65535.
                        };
                    }
                }
                if math::face_normal(p).is_some() {
                    let decoded = uv[i].map(|p| p.map(f64::from));
                    if orient(decoded[0], decoded[1], decoded[2]) <= 0. {
                        // A sliver whose exact footprint is under one texel covers no
                        // texel centre at any repack worth making: it keeps its geometry
                        // and corner attributes as a no-raster face. A larger collapse
                        // or inversion is a real chart defect and forces a repack.
                        if orient(texels[0], texels[1], texels[2]).abs() < 2. {
                            demoted.push(i);
                        } else {
                            failed.push(ci);
                        }
                    }
                }
            }
        }
        if !failed.is_empty() {
            if attempt < 3 {
                failed.sort_unstable();
                failed.dedup();
                for i in failed {
                    boosts[i] *= 2.;
                }
                continue;
            }
            return Err(unsupported(
                "nondegenerate proxy decoded UV collapsed/inverted after four repacks",
            ));
        }
        let mut raster_faces = raster_faces.to_vec();
        for &i in &demoted {
            raster_faces[i] = false;
        }
        for c in &mut charts {
            c.faces.retain(|&i| raster_faces[i]);
        }
        return Ok(Layout {
            charts,
            pages,
            uv,
            raster_faces,
            subtexel_no_raster: demoted.len(),
            texel_size: texel,
            gutter,
        });
    }
    Err(unsupported("decoded proxy UV repack exhausted"))
}
