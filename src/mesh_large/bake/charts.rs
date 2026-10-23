//! Deterministic same-material-class, fixed-seed-cone charts and translation packing.
//!
//! Coarse proxies are dense, noisy meshes whose faces are often under a square texel
//! at the planned density, and every chart pays a fixed gutter in texels. Charts
//! therefore grow across source primitives and materials of one material class (the
//! class shares its pages; every face still samples its own source), and shelves are
//! packed tallest first on the power-of-two page width that wastes the fewest texels.
use super::{
    invalid,
    math::{self, V3},
    resource,
    types::{BakeLimits, FrozenNode},
    unsupported,
};
use crate::{
    mesh_large::{batching::MaterialClasses, config::Profile},
    JobError,
};
use std::collections::{BTreeMap, HashMap, VecDeque};
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
    /// Material class representative (H28): charts of every source material of one
    /// class share pages.
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
/// Least cosine between a chart face's normal and its seed's. A chart rasterises at
/// `1 / (texel * sigma)` so every face meets the density, so this bounds the texel
/// area overhead of a whole chart at `1 / CHART_CONE_COS^2` (about 1.23). Noisy proxy
/// surfaces still form large charts through their sub-texel faces, which join under
/// [`SUBTEXEL_CHART_COS`] without raising this overhead.
const CHART_CONE_COS: f64 = 0.9;
/// Least cosine at which a face under one square planned texel joins a chart. Such a
/// face covers at most a texel or two at any orientation, so it neither meets nor
/// misses the density in a way a viewer can see: it joins without lowering the
/// chart's normal agreement, and only needs to face the chart plane so its decoded
/// UVs keep their orientation.
const SUBTEXEL_CHART_COS: f64 = 0.3;
/// Faces meeting at one welded vertex are chart neighbours when at most this many
/// faces meet there (edge neighbours always are).
const VERTEX_NEIGHBOURS: usize = 16;
/// Chart-plane cells, in planned texels, of the index that finds a candidate's
/// possibly overlapping chart members.
const OVERLAP_CELL_TEXELS: f64 = 4.;
/// A member whose box spans more index cells than this is tested against every
/// candidate instead of being indexed.
const OVERLAP_CELL_SPAN: i64 = 64;
fn project(p: V3, x: V3, y: V3) -> P2 {
    [math::dot(p, x), math::dot(p, y)]
}
fn orient(a: P2, b: P2, c: P2) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
/// Appearance-contract bound for sub-texel sliver demotion, in square texels of the
/// face's exact (pre-quantization) page footprint.
///
/// A rasterising face whose decoded UVs collapse or invert is demoted to a no-raster
/// face (geometry, corner attributes and feature IDs kept, no texture sampling
/// obligation) only when that footprint is strictly under this bound. A collapse at
/// or above it is a chart defect and forces a repack, then a refusal.
pub(super) const SUBTEXEL_DEMOTION_TEXELS: f64 = 1.;
/// True when a face whose decoded UVs collapsed may be demoted: its exact footprint
/// `texels` (page pixel coordinates) covers strictly less than one square texel.
fn subtexel_sliver(texels: [P2; 3]) -> bool {
    orient(texels[0], texels[1], texels[2]).abs() / 2. < SUBTEXEL_DEMOTION_TEXELS
}
/// Two faces sharing exactly one corner `v` have disjoint interiors iff their corner
/// wedges at `v` are disjoint, which holds iff each wedge's rays lie strictly outside
/// the other wedge. Every such separation must survive moving each corner by `slack`
/// (UV storage), and near-zero arithmetic has no proof: both report overlap.
fn wedges_disjoint(v: P2, a: [P2; 2], b: [P2; 2], slack: f64) -> bool {
    let ray = |p: P2| [p[0] - v[0], p[1] - v[1]];
    let len = |r: P2| r[0].hypot(r[1]);
    // Signed cross product, or zero when its sign is not proven under `slack`.
    let cross = |u: P2, w: P2| {
        let c = u[0] * w[1] - u[1] * w[0];
        let margin = 2. * slack * (len(u) + len(w))
            + 4. * slack * slack
            + 8. * f64::EPSILON * ((u[0] * w[1]).abs() + (u[1] * w[0]).abs());
        if c.abs() > margin {
            c
        } else {
            0.
        }
    };
    // Counterclockwise rays of a nondegenerate wedge.
    let wedge = |[p, q]: [P2; 2]| {
        let (p, q) = (ray(p), ray(q));
        let c = cross(p, q);
        (c != 0.).then_some(if c > 0. { [p, q] } else { [q, p] })
    };
    let (Some(wa), Some(wb)) = (wedge(a), wedge(b)) else {
        return false;
    };
    let outside = |w: [P2; 2], r: P2| cross(w[0], r) < 0. || cross(w[1], r) > 0.;
    wb.iter().all(|&r| outside(wa, r)) && wa.iter().all(|&r| outside(wb, r))
}
/// A separating axis proves no strict interior overlap. Near-zero arithmetic
/// has no such proof and conservatively splits the chart. Faces touching at one
/// shared corner, which every separating line passes through, use their wedges.
fn overlap(a: [P2; 3], b: [P2; 3], slack: f64) -> bool {
    let in_b = a.map(|p| b.contains(&p));
    let shared = in_b.iter().filter(|s| **s).count();
    if shared == 1 {
        let i = in_b.iter().position(|s| *s).unwrap();
        let j = b.iter().position(|p| *p == a[i]).unwrap();
        let others = |t: [P2; 3], k: usize| [t[(k + 1) % 3], t[(k + 2) % 3]];
        if wedges_disjoint(a[i], others(a, i), others(b, j), slack) {
            return false;
        }
    }
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
            // A gap that UV storage (each corner moved by `slack`) could close is no proof.
            let guard =
                32. * f64::EPSILON * amin.abs().max(amax.abs()).max(bmin.abs()).max(bmax.abs())
                    + 2. * slack * axis[0].hypot(axis[1]);
            if amax < bmin - guard || bmax < amin - guard {
                return false;
            }
            // Exact common edge, with the remaining corners strictly on opposite sides.
            if shared == 2 {
                let k = in_b.iter().position(|s| !*s).unwrap();
                let (s0, s1) = (a[(k + 1) % 3], a[(k + 2) % 3]);
                let ac = a[k];
                let bc = b.into_iter().find(|p| *p != s0 && *p != s1).unwrap();
                let aa = orient(s0, s1, ac);
                let bb = orient(s0, s1, bc);
                if aa * bb < 0. {
                    return false;
                }
            }
        }
    }
    true
}
/// Each packed chart's page (within its class) and origin, and every page's used extent.
type Shelves = (Vec<(usize, [u32; 2])>, Vec<[u32; 2]>);
/// Shelf-packs `run` (chart indices, tallest first) on pages at most `cap` wide and
/// `max_edge` tall; every page extent is at least 4 x 4.
fn shelves(run: &[usize], sizes: &[[u32; 2]], cap: u32, max_edge: u32) -> Shelves {
    let mut placed = Vec::with_capacity(run.len());
    let mut extents = vec![[4u32; 2]];
    let (mut x, mut y, mut row) = (0u32, 0u32, 0u32);
    for &i in run {
        let [w, h] = sizes[i];
        if x + w > cap {
            x = 0;
            y += row;
            row = 0;
        }
        if y + h > max_edge {
            extents.push([4; 2]);
            (x, y, row) = (0, 0, 0);
        }
        let page = extents.len() - 1;
        placed.push((page, [x, y]));
        extents[page] = [extents[page][0].max(x + w), extents[page][1].max(y + h)];
        x += w;
        row = row.max(h);
    }
    (placed, extents)
}
pub(super) fn build(
    f: &FrozenNode,
    raster_faces: &[bool],
    limits: BakeLimits,
    texel: f64,
    classes: &MaterialClasses,
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
            edges
                .entry((classes.of(face.material), a, b))
                .or_default()
                .push(i);
        }
    }
    let mut adjacent = vec![Vec::new(); n];
    for faces in edges.values() {
        if faces.len() == 2 {
            adjacent[faces[0]].push(faces[1]);
            adjacent[faces[1]].push(faces[0]);
        }
    }
    drop(edges);
    // Then faces sharing only a welded vertex: noisy proxies leave many faces whose
    // edge neighbours all fail the cone. Edge neighbours stay first.
    let mut vertices: BTreeMap<(u32, [u32; 3]), Vec<usize>> = BTreeMap::new();
    for (i, face) in f.faces.iter().enumerate() {
        check()?;
        for w in face.welds {
            vertices
                .entry((classes.of(face.material), w))
                .or_default()
                .push(i);
        }
    }
    let mut corner = vec![Vec::new(); n];
    for faces in vertices.values() {
        if faces.len() <= VERTEX_NEIGHBOURS {
            for &a in faces {
                corner[a].extend(faces.iter().copied().filter(|&b| b != a));
            }
        }
    }
    drop(vertices);
    for (a, mut c) in adjacent.iter_mut().zip(corner) {
        a.sort_unstable();
        a.dedup();
        c.sort_unstable();
        c.dedup();
        c.retain(|b| a.binary_search(b).is_err());
        a.extend(c);
    }
    // Chart metres per page texel are at most `texel` (sigma <= 1, boosts >= 1), and
    // storage moves a UV by at most one 16-bit step of the largest admitted page.
    let slack = f64::from(limits.max_page_edge) / 65535. * texel;
    let gutter = if limits.profile == Profile::Lossless {
        4
    } else {
        2
    };
    // Largest chart interior, in texels, that packs on one page (the ceil adds one).
    let max_interior = f64::from(limits.max_page_edge.saturating_sub(2 * gutter + 1));
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
        let bounds = |t: [P2; 3]| {
            let lo = [0, 1].map(|a| t.iter().map(|p| p[a]).fold(f64::INFINITY, f64::min));
            let hi = [0, 1].map(|a| t.iter().map(|p| p[a]).fold(f64::NEG_INFINITY, f64::max));
            [lo, hi]
        };
        let seed_projected = p.map(|p| project(p, x, y));
        let [mut lo, mut hi] = bounds(seed_projected);
        let mut members = vec![(seed_projected, [lo, hi])];
        // Members by chart-plane cell (boxes widened by what storage can move them);
        // the few spanning many cells are tested against every candidate.
        let cell = OVERLAP_CELL_TEXELS * texel;
        let cells = |[lo, hi]: [P2; 2]| {
            let c = |v: f64| (v / cell).floor() as i64;
            let lo = [c(lo[0] - 2. * slack), c(lo[1] - 2. * slack)];
            let hi = [c(hi[0] + 2. * slack), c(hi[1] + 2. * slack)];
            let wide = (hi[0] - lo[0] + 1).saturating_mul(hi[1] - lo[1] + 1) > OVERLAP_CELL_SPAN;
            (lo, hi, wide)
        };
        let mut index: HashMap<[i64; 2], Vec<usize>> = HashMap::new();
        let mut wide_members = Vec::new();
        let insert = |m: usize,
                      b: [P2; 2],
                      index: &mut HashMap<[i64; 2], Vec<usize>>,
                      wide_members: &mut Vec<usize>| {
            let (lo, hi, wide) = cells(b);
            if wide {
                wide_members.push(m);
                return;
            }
            for cy in lo[1]..=hi[1] {
                for cx in lo[0]..=hi[0] {
                    index.entry([cx, cy]).or_default().push(m);
                }
            }
        };
        insert(0, [lo, hi], &mut index, &mut wide_members);
        let mut stamp = vec![0u64];
        let mut tests = 0u64;
        let mut queue = VecDeque::from([seed]);
        while let Some(current) = queue.pop_front() {
            for &candidate in &adjacent[current] {
                check()?;
                if assigned[candidate] || !raster_faces[candidate] {
                    continue;
                }
                let c = &f.faces[candidate];
                if classes.of(c.material) != classes.of(face.material) {
                    continue;
                }
                let cp = c.positions.map(|p| p.map(f64::from));
                let Some(cn) = math::face_normal(cp) else {
                    continue;
                };
                let sigma = math::dot(normal, cn);
                let subtexel = math::triangle_area(cp) < texel * texel;
                if sigma
                    < if subtexel {
                        SUBTEXEL_CHART_COS
                    } else {
                        CHART_CONE_COS
                    }
                {
                    continue;
                }
                let projected = cp.map(|p| project(p, x, y));
                let [clo, chi] = bounds(projected);
                // The chart must still fit one page after every repack boost (at most
                // 2^3) at its lowest normal agreement; a larger region starts new charts.
                let grown_sigma = if subtexel {
                    chart.sigma
                } else {
                    chart.sigma.min(sigma)
                };
                let fits = (0..2).all(|a| {
                    (hi[a].max(chi[a]) - lo[a].min(clo[a])) * 8. / (texel * grown_sigma)
                        <= max_interior
                });
                if !fits {
                    continue;
                }
                let mut intersects = false;
                // Every member sharing a cell with the candidate's widened box, once.
                let (clo_cell, chi_cell, candidate_wide) = cells([clo, chi]);
                let mut near: Vec<usize> = wide_members.clone();
                if candidate_wide {
                    near = (0..members.len()).collect();
                } else {
                    for cy in clo_cell[1]..=chi_cell[1] {
                        for cx in clo_cell[0]..=chi_cell[0] {
                            if let Some(ms) = index.get(&[cx, cy]) {
                                near.extend(ms.iter().copied());
                            }
                        }
                    }
                }
                tests += 1;
                for m in near {
                    if stamp[m] == tests {
                        continue;
                    }
                    stamp[m] = tests;
                    let (other, [olo, ohi]) = members[m];
                    work = work
                        .checked_add(1)
                        .ok_or_else(|| resource("chart overlap work"))?;
                    if work > limits.query_work {
                        return Err(resource("chart overlap work budget"));
                    }
                    // Boxes apart by more than storage can move them cannot overlap.
                    if (0..2).any(|a| clo[a] > ohi[a] + 2. * slack || olo[a] > chi[a] + 2. * slack)
                    {
                        continue;
                    }
                    if overlap(projected, other, slack) {
                        intersects = true;
                        break;
                    }
                }
                if intersects {
                    continue;
                }
                assigned[candidate] = true;
                chart.sigma = grown_sigma;
                chart.faces.push(candidate);
                insert(members.len(), [clo, chi], &mut index, &mut wide_members);
                stamp.push(0);
                members.push((projected, [clo, chi]));
                for a in 0..2 {
                    lo[a] = lo[a].min(clo[a]);
                    hi[a] = hi[a].max(chi[a]);
                }
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
    let mut boosts = vec![1.; charts.len()];
    for attempt in 0..4 {
        // Every chart keeps `gutter` texels on each side, so neighbouring charts are
        // twice the gutter apart: base-level and first-mip taps stay in their chart.
        for (i, c) in charts.iter_mut().enumerate() {
            c.pixels_per_metre = boosts[i] / (texel * c.sigma);
            for a in 0..2 {
                let interior = ((c.hi[a] - c.lo[a]) * c.pixels_per_metre).ceil().max(1.);
                if !interior.is_finite() || interior > f64::from(limits.max_page_edge) {
                    return Err(resource("proxy chart physical dimension"));
                }
                c.size[a] = (interior as u32)
                    .checked_add(2 * gutter)
                    .ok_or_else(|| resource("proxy chart padding dimension"))?;
            }
            if c.size.iter().any(|s| *s > limits.max_page_edge) {
                return Err(resource("proxy chart exceeds admitted page edge"));
            }
        }
        // One page set per material class (H28), tallest charts first so shelves stay
        // full; ties keep a stable chart order.
        let mut order: Vec<usize> = (0..charts.len()).collect();
        order.sort_by_key(|&i| {
            let c = &charts[i];
            (
                classes.of(c.material),
                std::cmp::Reverse(c.size[1]),
                std::cmp::Reverse(c.size[0]),
                c.faces[0],
            )
        });
        let sizes: Vec<[u32; 2]> = charts.iter().map(|c| c.size).collect();
        let chart_class: Vec<u32> = charts.iter().map(|c| classes.of(c.material)).collect();
        let mut pages: Vec<Page> = Vec::new();
        for run in order.chunk_by(|&a, &b| chart_class[a] == chart_class[b]) {
            check()?;
            let class = chart_class[run[0]];
            // Each class takes the power-of-two shelf width whose pages hold the fewest
            // texels once rounded to powers of two; ties keep fewer pages, then the
            // squarer pages (the shorter longest edge), then the narrower width.
            let widest = run.iter().map(|&i| sizes[i][0]).max().unwrap_or(4);
            let mut best: Option<([u64; 3], Shelves)> = None;
            let mut cap = widest.next_power_of_two().max(4);
            while cap <= limits.max_page_edge {
                let (placed, extents) = shelves(run, &sizes, cap, limits.max_page_edge);
                let texels = extents
                    .iter()
                    .map(|e| {
                        u64::from(e[0].next_power_of_two()) * u64::from(e[1].next_power_of_two())
                    })
                    .sum::<u64>();
                let edge = extents
                    .iter()
                    .flat_map(|e| e.map(u32::next_power_of_two))
                    .max()
                    .unwrap_or(4);
                let key = [texels, extents.len() as u64, u64::from(edge)];
                if best.as_ref().is_none_or(|b| key < b.0) {
                    best = Some((key, (placed, extents)));
                }
                cap *= 2;
            }
            let (_, (placed, extents)) =
                best.ok_or_else(|| resource("proxy chart exceeds admitted page edge"))?;
            let first = pages.len();
            pages.extend(extents.iter().map(|e| Page {
                material: class,
                size: e.map(u32::next_power_of_two),
                charts: Vec::new(),
            }));
            for (&i, (page, origin)) in run.iter().zip(placed) {
                charts[i].page = first + page;
                charts[i].origin = origin;
                pages[first + page].charts.push(i);
            }
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
                        let px =
                            f64::from(c.origin[a] + gutter) + (q[a] - c.lo[a]) * c.pixels_per_metre;
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
                        if subtexel_sliver(texels) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh_large::bake::types::{FrozenFace, PositionTransform, PositionWord, SourceKey};

    #[test]
    fn subtexel_demotion_bound_is_strictly_under_one_square_texel() {
        assert_eq!(SUBTEXEL_DEMOTION_TEXELS, 1.);
        // Right triangles with legs 1 x h texels: area h / 2.
        let right = |h: f64| [[0., 0.], [1., 0.], [0., h]];
        assert!(subtexel_sliver(right(0.)));
        assert!(subtexel_sliver(right(1.)));
        assert!(subtexel_sliver(right(2f64.next_down())));
        // Exactly one square texel, and anything larger, is not a sliver.
        assert!(!subtexel_sliver(right(2.)));
        assert!(!subtexel_sliver(right(2f64.next_up())));
        assert!(!subtexel_sliver(right(64.)));
        // Orientation does not matter: an inverted footprint uses its magnitude.
        assert!(subtexel_sliver([[0., 0.], [0., 1.9], [1., 0.]]));
        assert!(!subtexel_sliver([[0., 0.], [0., 2.], [1., 0.]]));
    }

    fn face(ordinal: u64, primitive: u32, positions: [[f32; 3]; 3]) -> FrozenFace {
        FrozenFace {
            ordinal,
            source: SourceKey {
                node: 0,
                mesh: 0,
                primitive,
                instance: 0,
            },
            source_triangle: ordinal,
            material: 0,
            positions,
            words: positions.map(|p| PositionWord::Float(p.map(f32::to_bits))),
            transform: PositionTransform::default(),
            welds: positions.map(|p| p.map(f32::to_bits)),
            features: Vec::new(),
            fill_bound: None,
            donor_ranges: std::iter::once(ordinal..ordinal + 1).collect(),
        }
    }

    #[test]
    fn faces_touching_at_one_corner_overlap_only_when_their_wedges_do() {
        let a = [[0., 0.], [1., 0.], [0., 1.]];
        // Opposite quadrant: collinear opposite rays, disjoint interiors.
        assert!(!overlap(a, [[0., 0.], [-1., 0.], [0., -1.]], 0.));
        assert!(!overlap(a, [[0., -1.], [0., 0.], [-1., 0.]], 0.));
        // Strip neighbour across a shared corner.
        assert!(!overlap(
            [[0., 0.], [1., 0.], [0., 1.]],
            [[1., 0.], [2., 0.], [1., 1.]],
            0.
        ));
        // A ray inside the other wedge, and a face nested in the other's wedge.
        assert!(overlap(a, [[0., 0.], [1., 1.], [-1., 0.]], 0.));
        assert!(overlap(a, [[0., 0.], [0.5, 0.1], [0.1, 0.5]], 0.));
        // Rays in the same direction (T-junction) have no strict separation.
        assert!(overlap(a, [[0., 0.], [0., 2.], [-1., 0.]], 0.));
        // A separation that 16-bit UV storage could close is not proven.
        let b = [[0., 0.], [1., -1e-3], [-1., 0.]];
        assert!(!overlap(a, b, 0.));
        assert!(overlap(a, b, 1e-3));
        // Disjoint and edge-sharing faces keep their existing rules.
        assert!(!overlap(a, [[2., 2.], [3., 2.], [2., 3.]], 0.));
        assert!(!overlap(a, [[1., 0.], [0., 1.], [1., 1.]], 0.));
        assert!(overlap(a, [[1., 0.], [0., 1.], [0.2, 0.2]], 0.));
    }

    fn grid_faces(cells: usize) -> Vec<FrozenFace> {
        let mut faces = Vec::new();
        let p = |i: usize, j: usize| [i as f32, j as f32, 0.];
        for j in 0..cells {
            for i in 0..cells {
                let ordinal = faces.len() as u64;
                faces.push(face(ordinal, 0, [p(i, j), p(i + 1, j), p(i + 1, j + 1)]));
                faces.push(face(
                    ordinal + 1,
                    0,
                    [p(i, j), p(i + 1, j + 1), p(i, j + 1)],
                ));
            }
        }
        faces
    }

    fn fan_faces(sides: usize) -> Vec<FrozenFace> {
        let rim = |k: usize| {
            let a = std::f64::consts::TAU * (k % sides) as f64 / sides as f64;
            [a.cos() as f32, a.sin() as f32, 0.]
        };
        (0..sides)
            .map(|k| face(k as u64, 0, [[0.; 3], rim(k), rim(k + 1)]))
            .collect()
    }

    /// The raster's double-coverage invariant on the decoded UVs of every chart.
    fn assert_single_coverage(f: &FrozenNode, layout: &Layout) {
        for (pi, page) in layout.pages.iter().enumerate() {
            let [w, h] = page.size;
            let mut weights = vec![0.; w as usize * h as usize];
            for c in layout.charts.iter().filter(|c| c.page == pi) {
                for &fi in &c.faces {
                    let p = f.faces[fi].positions.map(|p| p.map(f64::from));
                    assert!(math::face_normal(p).is_some());
                    let t = layout.uv[fi].map(|p| {
                        [
                            f64::from(p[0]) * f64::from(w),
                            f64::from(p[1]) * f64::from(h),
                        ]
                    });
                    let lo = [0, 1].map(|a| t.iter().map(|p| p[a]).fold(f64::INFINITY, f64::min));
                    let hi = [0, 1].map(|a| t.iter().map(|p| p[a]).fold(0., f64::max));
                    for y in lo[1].floor() as u32..(hi[1].ceil() as u32).min(h) {
                        for x in lo[0].floor() as u32..(hi[0].ceil() as u32).min(w) {
                            weights[(y * w + x) as usize] +=
                                super::super::raster::coverage(t, x, y).0;
                        }
                    }
                }
            }
            assert!(
                weights.iter().all(|v| *v <= 1. + 1e-8),
                "double coverage on page {pi}"
            );
        }
    }

    #[test]
    fn connected_planar_grids_and_closed_fans_form_one_chart() {
        let limits = BakeLimits {
            profile: Profile::Balanced,
            memory_bytes: 64 << 20,
            member_bytes: 16 << 20,
            source_cache_bytes: 16 << 20,
            shared_image_bytes: 0,
            child_texel_bytes: 256 << 20,
            query_work: 1_000_000,
            max_page_edge: 2048,
            max_density_drops: 4,
            id_textures: false,
        };
        for faces in [grid_faces(4), fan_faces(6), fan_faces(5)] {
            let mut f = crate::mesh_large::bake::controls::immutable_fixture();
            f.faces = faces;
            let n = f.faces.len();
            let layout = build(
                &f,
                &vec![true; n],
                limits,
                0.125,
                &MaterialClasses::EMPTY,
                &mut || Ok(()),
            )
            .unwrap();
            assert_eq!(layout.charts.len(), 1, "{n} welded coplanar faces");
            assert_eq!(layout.charts[0].faces, (0..n).collect::<Vec<_>>());
            assert_eq!(layout.subtexel_no_raster, 0);
            assert_single_coverage(&f, &layout);
        }
    }

    fn test_limits() -> BakeLimits {
        BakeLimits {
            profile: Profile::Balanced,
            memory_bytes: 64 << 20,
            member_bytes: 16 << 20,
            source_cache_bytes: 16 << 20,
            shared_image_bytes: 0,
            child_texel_bytes: 256 << 20,
            query_work: 1_000_000,
            max_page_edge: 2048,
            max_density_drops: 4,
            id_textures: false,
        }
    }

    #[test]
    fn charts_span_source_primitives_of_one_material_class() {
        let mut f = crate::mesh_large::bake::controls::immutable_fixture();
        f.faces = grid_faces(4);
        for (i, face) in f.faces.iter_mut().enumerate() {
            face.source.primitive = (i % 3) as u32;
        }
        let n = f.faces.len();
        let layout = build(
            &f,
            &vec![true; n],
            test_limits(),
            0.125,
            &MaterialClasses::EMPTY,
            &mut || Ok(()),
        )
        .unwrap();
        assert_eq!(
            layout.charts.len(),
            1,
            "welded coplanar faces of three primitives"
        );
        assert_single_coverage(&f, &layout);
    }

    #[test]
    fn subtexel_faces_join_beyond_the_cone_without_lowering_sigma() {
        // A 1 m face on z = 0 and, across its hypotenuse, a face folded 60 degrees up
        // (cosine 0.5, outside the chart cone) whose width `d` sets its area.
        let folded = |d: f32| {
            let s = d / std::f32::consts::SQRT_2 * 0.5;
            let q = [0.5 + s, 0.5 + s, d * 0.75f32.sqrt()];
            vec![
                face(0, 0, [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]),
                face(1, 0, [[0., 1., 0.], [1., 0., 0.], q]),
            ]
        };
        for (d, charts) in [(0.02, 1), (1., 2)] {
            let mut f = crate::mesh_large::bake::controls::immutable_fixture();
            f.faces = folded(d);
            // At 0.125 m texels one square texel is 0.015625 m2: the narrow fold is
            // 0.0141 m2, the wide one 0.707 m2.
            let layout = build(
                &f,
                &[true, true],
                test_limits(),
                0.125,
                &MaterialClasses::EMPTY,
                &mut || Ok(()),
            )
            .unwrap();
            assert_eq!(layout.charts.len(), charts, "fold width {d}");
            assert!(layout.charts.iter().all(|c| c.sigma == 1.));
            assert_eq!(layout.raster_faces, [true, true]);
            assert_single_coverage(&f, &layout);
        }
    }

    #[test]
    fn shelves_fill_the_squarest_power_of_two_page_of_least_area() {
        // 64 separate 1 m faces at 8 texels per metre: 64 charts of 8 + 2 x 2 = 12
        // texels a side. 128 x 128 holds them (10 a row, 7 rows); 1024 x 16 and
        // 16 x 1024 hold as many texels but are not square.
        let mut f = crate::mesh_large::bake::controls::immutable_fixture();
        f.faces = (0..64)
            .map(|i| {
                let x = 3. * i as f32;
                face(i, 0, [[x, 0., 0.], [x + 1., 0., 0.], [x, 1., 0.]])
            })
            .collect();
        let layout = build(
            &f,
            &[true; 64],
            test_limits(),
            0.125,
            &MaterialClasses::EMPTY,
            &mut || Ok(()),
        )
        .unwrap();
        assert_eq!(layout.charts.len(), 64);
        assert!(layout.charts.iter().all(|c| c.size == [12, 12]));
        assert_eq!(layout.pages.len(), 1);
        assert_eq!(layout.pages[0].size, [128, 128]);
        assert_single_coverage(&f, &layout);
    }

    #[test]
    fn regions_wider_than_a_page_split_into_charts_that_pack() {
        let limits = BakeLimits {
            profile: Profile::Balanced,
            memory_bytes: 64 << 20,
            member_bytes: 16 << 20,
            source_cache_bytes: 16 << 20,
            shared_image_bytes: 0,
            child_texel_bytes: 256 << 20,
            query_work: 1_000_000_000,
            max_page_edge: 2048,
            max_density_drops: 4,
            id_textures: false,
        };
        // 40 m of grid at 8 texels per metre is 320 texels, over the 2043 / 8 = 255
        // texel chart extent that still fits a page after every repack boost.
        let mut f = crate::mesh_large::bake::controls::immutable_fixture();
        f.faces = grid_faces(40);
        let n = f.faces.len();
        let layout = build(
            &f,
            &vec![true; n],
            limits,
            0.125,
            &MaterialClasses::EMPTY,
            &mut || Ok(()),
        )
        .unwrap();
        assert!(layout.charts.len() > 1);
        assert!(layout.charts.len() < 16, "{} charts", layout.charts.len());
        let mut faces: Vec<_> = layout.charts.iter().flat_map(|c| c.faces.clone()).collect();
        faces.sort_unstable();
        assert_eq!(faces, (0..n).collect::<Vec<_>>());
        for c in &layout.charts {
            // Interior within a boost-8 eighth of the page, plus gutters and the ceil.
            assert!(c
                .size
                .iter()
                .all(|s| *s <= limits.max_page_edge / 8 + 2 * 2 + 1));
        }
        assert_single_coverage(&f, &layout);
    }

    #[test]
    fn collapsed_subtexel_sliver_is_demoted_to_no_raster_and_others_keep_charts() {
        let mut f = crate::mesh_large::bake::controls::immutable_fixture();
        // A 1 m face, and a separate 1 m needle 1e-7 m wide: at 8 texels per metre its
        // footprint is 3.2e-6 square texels and its 16-bit decoded UVs are collinear.
        f.faces = vec![
            face(0, 0, [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]),
            face(1, 1, [[3., 0., 0.], [4., 0., 0.], [3.5, 1e-7, 0.]]),
        ];
        let limits = BakeLimits {
            profile: Profile::Balanced,
            memory_bytes: 64 << 20,
            member_bytes: 16 << 20,
            source_cache_bytes: 16 << 20,
            shared_image_bytes: 0,
            child_texel_bytes: 256 << 20,
            query_work: 1_000_000,
            max_page_edge: 2048,
            max_density_drops: 4,
            id_textures: false,
        };
        let layout = build(
            &f,
            &[true, true],
            limits,
            0.125,
            &MaterialClasses::EMPTY,
            &mut || Ok(()),
        )
        .unwrap();
        assert_eq!(layout.subtexel_no_raster, 1);
        assert_eq!(layout.raster_faces, [true, false]);
        assert!(layout.charts.iter().all(|c| !c.faces.contains(&1)));
        assert!(layout.charts.iter().any(|c| c.faces == [0]));
        // The demoted face keeps finite decoded UVs for its retained corners.
        assert!(layout.uv[1].iter().flatten().all(|v| v.is_finite()));
    }
}
