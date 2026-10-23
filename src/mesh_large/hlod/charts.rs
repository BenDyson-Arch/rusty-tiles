//! Fresh coarse charts (D5): bounded planar chart growth over one material group of a
//! final decoded coarse tile, interior-overlap rejection, each chart turned to its
//! minimum-area bounding rectangle (rotating calipers, long side along u), and shelf
//! packing into ONE page sized to its content: the least-area block-aligned rectangle
//! (multiples of four, [`MIN_PAGE`]..=1024 per side; KTX2 needs only whole 4x4 blocks)
//! that holds every chart at the requested texel spacing, or the 1024² cap at the finest
//! spacing that fits. Positions are never moved: charts only assign texture coordinates.
//!
//! Adapted from the research prototype (`exp/lod-cluster-dag` `charts.cpp`), v0.3.0's
//! `chart_unwrap` and #260's tight content-sized packing. UV seams are free: the tile was
//! simplified without them, and every face is charted afresh.
use crate::{JobError, JobErrorKind};

type V3 = [f64; 3];
type P2 = [f64; 2];

/// Least cosine between a chart face's normal and its seed's: bounds a face's texel-area
/// stretch on the chart plane at `1 / 0.8²` (about 1.56×).
const CONE_COS: f64 = 0.8;
/// Most faces one chart may grow to (bounds the overlap test, as the prototype).
const CHART_FACES: usize = 192;
/// Texels of gutter on every side of a chart; the sparse-mip encoder fills them from the
/// nearest covered texel at every level.
pub(crate) const GUTTER: u32 = 2;
pub(crate) const MIN_PAGE: u32 = 32;
/// Page dimensions are whole codec blocks.
const BLOCK: u32 = 4;

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(a: V3) -> Option<V3> {
    let d = dot(a, a).sqrt();
    (d > 0. && d.is_finite()).then(|| a.map(|v| v / d))
}

/// One chart: its faces (group-local), plane axes and projected corners.
pub(crate) struct Chart {
    pub faces: Vec<usize>,
    /// Plane axis that maps to +u (the coarse tangent direction of the chart).
    pub x: V3,
    pub y: V3,
    points: Vec<[P2; 3]>,
    lo: P2,
    hi: P2,
    origin: [u32; 2],
    size: [u32; 2],
}

/// The packed layout of one group.
pub(crate) struct Layout {
    pub charts: Vec<Chart>,
    /// Per face: its chart.
    pub chart_of: Vec<u32>,
    /// Per face corner: page pixel coordinates (texel centres at `i + 0.5`).
    pub pixels: Vec<[[f64; 2]; 3]>,
    pub size: [u32; 2],
    /// Page texels per metre on every chart plane.
    pub scale: f64,
    /// The requested spacing could not fit `max_edge²`: `scale < 1 / spacing`.
    pub capped: bool,
}

impl Layout {
    /// Per page texel: 1 + the chart whose packed rectangle (gutters included) holds it,
    /// 0 outside every chart. Keeps charts apart in reduced levels and gutter fill.
    pub(crate) fn labels(&self) -> Vec<u32> {
        let [w, h] = self.size;
        let mut labels = vec![0u32; w as usize * h as usize];
        for (i, c) in self.charts.iter().enumerate() {
            for y in c.origin[1]..(c.origin[1] + c.size[1]).min(h) {
                let row = (y * w) as usize;
                let (a, b) = (c.origin[0] as usize, ((c.origin[0] + c.size[0]).min(w)) as usize);
                labels[row + a..row + b].fill(i as u32 + 1);
            }
        }
        labels
    }
    /// Normalized UV of a face corner.
    pub(crate) fn uv(&self, face: usize, corner: usize) -> [f32; 2] {
        let p = self.pixels[face][corner];
        [
            (p[0] / f64::from(self.size[0])) as f32,
            (p[1] / f64::from(self.size[1])) as f32,
        ]
    }
}

fn overlap(a: &[P2; 3], b: &[P2; 3]) -> bool {
    for t in [a, b] {
        for k in 0..3 {
            let e = [t[(k + 1) % 3][0] - t[k][0], t[(k + 1) % 3][1] - t[k][1]];
            let axis = [-e[1], e[0]];
            let (mut al, mut ah, mut bl, mut bh) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
            for j in 0..3 {
                let x = a[j][0] * axis[0] + a[j][1] * axis[1];
                let y = b[j][0] * axis[0] + b[j][1] * axis[1];
                al = al.min(x);
                ah = ah.max(x);
                bl = bl.min(y);
                bh = bh.max(y);
            }
            let tolerance = 1e-10 * (axis[0] * axis[0] + axis[1] * axis[1]).sqrt().max(1.);
            if ah.min(bh) - al.max(bl) <= tolerance {
                return false;
            }
        }
    }
    true
}

/// Grow charts over `faces` (corner indices into `positions`): seeds by descending area
/// (then face order), breadth-first over shared edges within the normal cone, rejecting a
/// face whose projection overlaps the chart's interior. Zero-area faces get a point chart.
pub(crate) fn grow(positions: &[[f32; 3]], faces: &[[u32; 3]]) -> (Vec<Chart>, Vec<u32>) {
    let p = |v: u32| positions[v as usize].map(f64::from);
    let n = faces.len();
    let mut normals = Vec::with_capacity(n);
    let mut areas = Vec::with_capacity(n);
    let mut edges: Vec<(u32, u32, u32)> = Vec::with_capacity(n * 3);
    for (i, f) in faces.iter().enumerate() {
        let c = cross(sub(p(f[1]), p(f[0])), sub(p(f[2]), p(f[0])));
        areas.push(dot(c, c).sqrt());
        normals.push(unit(c));
        for k in 0..3 {
            let (a, b) = (f[k], f[(k + 1) % 3]);
            edges.push((a.min(b), a.max(b), i as u32));
        }
    }
    edges.sort_unstable();
    // Adjacency over shared (welded) edges, in CSR form.
    let mut pairs: Vec<(u32, u32)> = Vec::new();
    let mut s = 0;
    while s < edges.len() {
        let mut e = s + 1;
        while e < edges.len() && edges[e].0 == edges[s].0 && edges[e].1 == edges[s].1 {
            e += 1;
        }
        for a in s..e {
            for b in s..e {
                if edges[a].2 != edges[b].2 {
                    pairs.push((edges[a].2, edges[b].2));
                }
            }
        }
        s = e;
    }
    pairs.sort_unstable();
    pairs.dedup();
    let mut start = vec![0u32; n + 1];
    for &(a, _) in &pairs {
        start[a as usize + 1] += 1;
    }
    for i in 0..n {
        start[i + 1] += start[i];
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| areas[b].total_cmp(&areas[a]).then(a.cmp(&b)));
    let mut chart_of = vec![u32::MAX; n];
    let mut charts: Vec<Chart> = Vec::new();
    let mut queue = Vec::new();
    for seed in order {
        if chart_of[seed] != u32::MAX {
            continue;
        }
        let id = charts.len() as u32;
        let Some(normal) = normals[seed] else {
            chart_of[seed] = id;
            charts.push(Chart {
                faces: vec![seed],
                x: [1., 0., 0.],
                y: [0., 0., 1.],
                points: vec![[[0.; 2]; 3]],
                lo: [0.; 2],
                hi: [0.; 2],
                origin: [0; 2],
                size: [0; 2],
            });
            continue;
        };
        let axis = if normal[0].abs() < 0.8 {
            [1., 0., 0.]
        } else {
            [0., 1., 0.]
        };
        let x = unit(cross(axis, normal)).unwrap_or([1., 0., 0.]);
        let y = cross(normal, x);
        let origin = p(faces[seed][0]);
        let mut chart = Chart {
            faces: Vec::new(),
            x,
            y,
            points: Vec::new(),
            lo: [f64::MAX; 2],
            hi: [f64::MIN; 2],
            origin: [0; 2],
            size: [0; 2],
        };
        queue.clear();
        queue.push(seed);
        let mut q = 0;
        while q < queue.len() && chart.faces.len() < CHART_FACES {
            let f = queue[q];
            q += 1;
            if chart_of[f] != u32::MAX || normals[f].is_none_or(|m| dot(m, normal) < CONE_COS) {
                continue;
            }
            let projected: [P2; 3] = std::array::from_fn(|k| {
                let v = sub(p(faces[f][k]), origin);
                [dot(v, x), dot(v, y)]
            });
            if chart.points.iter().any(|t| overlap(&projected, t)) {
                continue;
            }
            chart_of[f] = id;
            for c in &projected {
                chart.lo = std::array::from_fn(|a| chart.lo[a].min(c[a]));
                chart.hi = std::array::from_fn(|a| chart.hi[a].max(c[a]));
            }
            chart.faces.push(f);
            chart.points.push(projected);
            for k in start[f]..start[f + 1] {
                let next = pairs[k as usize].1 as usize;
                if chart_of[next] == u32::MAX {
                    queue.push(next);
                }
            }
        }
        orient(&mut chart);
        charts.push(chart);
    }
    (charts, chart_of)
}

/// Andrew's monotone chain: the convex hull of `points`, counter-clockwise.
fn hull(points: &mut Vec<P2>) -> Vec<P2> {
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    points.dedup();
    if points.len() < 3 {
        return points.clone();
    }
    let turn = |o: P2, a: P2, b: P2| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut out: Vec<P2> = Vec::with_capacity(points.len() * 2);
    for pass in 0..2 {
        let start = out.len();
        let iter: Box<dyn Iterator<Item = &P2>> = if pass == 0 {
            Box::new(points.iter())
        } else {
            Box::new(points.iter().rev())
        };
        for &p in iter {
            while out.len() >= start + 2 && turn(out[out.len() - 2], out[out.len() - 1], p) <= 0. {
                out.pop();
            }
            out.push(p);
        }
        out.pop();
    }
    out
}

/// Turn a chart in its plane to its minimum-area bounding rectangle (rotating calipers
/// over hull edges), long side along u. A proper rotation of the plane axes, so the
/// tangent frame keeps its handedness.
fn orient(chart: &mut Chart) {
    let mut points: Vec<P2> = chart.points.iter().flatten().copied().collect();
    let hull = hull(&mut points);
    if hull.len() < 3 {
        return;
    }
    let mut best = (f64::INFINITY, [1., 0.]);
    for k in 0..hull.len() {
        let (a, b) = (hull[k], hull[(k + 1) % hull.len()]);
        let e = [b[0] - a[0], b[1] - a[1]];
        let l = (e[0] * e[0] + e[1] * e[1]).sqrt();
        if l <= 0. || !l.is_finite() {
            continue;
        }
        let d = [e[0] / l, e[1] / l];
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for q in &hull {
            let r = [q[0] * d[0] + q[1] * d[1], -q[0] * d[1] + q[1] * d[0]];
            for a in 0..2 {
                lo[a] = lo[a].min(r[a]);
                hi[a] = hi[a].max(r[a]);
            }
        }
        let area = (hi[0] - lo[0]) * (hi[1] - lo[1]);
        if area < best.0 * (1. - 1e-9) {
            let d = if hi[1] - lo[1] > hi[0] - lo[0] { [-d[1], d[0]] } else { d };
            best = (area, d);
        }
    }
    let d = best.1;
    let rotate = |q: P2| [q[0] * d[0] + q[1] * d[1], -q[0] * d[1] + q[1] * d[0]];
    let (x, y) = (chart.x, chart.y);
    chart.x = std::array::from_fn(|a| d[0] * x[a] + d[1] * y[a]);
    chart.y = std::array::from_fn(|a| -d[1] * x[a] + d[0] * y[a]);
    chart.lo = [f64::MAX; 2];
    chart.hi = [f64::MIN; 2];
    for t in &mut chart.points {
        for q in t.iter_mut() {
            *q = rotate(*q);
            chart.lo = std::array::from_fn(|a| chart.lo[a].min(q[a]));
            chart.hi = std::array::from_fn(|a| chart.hi[a].max(q[a]));
        }
    }
}

/// Shelf-pack `charts` (sized by [`sizes`]) into width `width`: the (used width, packed
/// height), or `None` when a chart is wider than the page or the height exceeds
/// `max_height`.
fn shelf(charts: &mut [Chart], order: &[usize], width: u32, max_height: u32) -> Option<(u32, u32)> {
    let (mut x, mut y, mut row, mut used) = (0u32, 0u32, 0u32, 0u32);
    for &i in order {
        let c = &mut charts[i];
        let (w, h) = (c.size[0], c.size[1]);
        if w > width {
            return None;
        }
        if x + w > width {
            x = 0;
            y += row;
            row = 0;
        }
        if y + h > max_height {
            return None;
        }
        c.origin = [x, y];
        x += w;
        used = used.max(x);
        row = row.max(h);
    }
    Some((used, y + row))
}

fn sizes(charts: &mut [Chart], scale: f64) -> Option<()> {
    for c in charts.iter_mut() {
        let extent = |a: usize| ((c.hi[a] - c.lo[a]).max(0.) * scale).ceil();
        let (w, h) = (extent(0), extent(1));
        if !(w.is_finite() && h.is_finite()) || w > 1e6 || h > 1e6 {
            return None;
        }
        c.size = [w as u32 + 1 + 2 * GUTTER, h as u32 + 1 + 2 * GUTTER];
    }
    Some(())
}

/// The least-area block-aligned page in `MIN_PAGE..=max_edge` per side holding every
/// chart at `scale` (shelves by descending height at a few widths around the square
/// root of the chart area; ties to the narrower); packs the charts into it.
fn fit(charts: &mut [Chart], scale: f64, max_edge: u32) -> Option<[u32; 2]> {
    sizes(charts, scale)?;
    let mut order: Vec<usize> = (0..charts.len()).collect();
    order.sort_by(|&a, &b| charts[b].size[1].cmp(&charts[a].size[1]).then(a.cmp(&b)));
    let round = |v: f64| ((v.ceil() as u64).div_ceil(u64::from(BLOCK)) * u64::from(BLOCK)) as u32;
    let area: f64 = charts.iter().map(|c| f64::from(c.size[0]) * f64::from(c.size[1])).sum();
    let widest = charts.iter().map(|c| c.size[0]).max().unwrap_or(1);
    let mut widths: Vec<u32> = [1., 1.1, 1.25, 1.4, 1.6, 2., 2.5, 3.2]
        .iter()
        .map(|f| round(area.sqrt() * f))
        .chain([round(f64::from(widest)), max_edge])
        .map(|w| w.clamp(MIN_PAGE, max_edge).max(round(f64::from(widest))))
        .filter(|&w| w <= max_edge)
        .collect();
    widths.sort_unstable();
    widths.dedup();
    let mut best: Option<(u64, u32, [u32; 2])> = None;
    for &w in &widths {
        let Some((used, h)) = shelf(charts, &order, w, max_edge) else {
            continue;
        };
        let page = [round(f64::from(used)).max(MIN_PAGE), round(f64::from(h)).max(MIN_PAGE)];
        if page[1] > max_edge {
            continue;
        }
        let key = (u64::from(page[0]) * u64::from(page[1]), page[0].max(page[1]));
        if best.is_none_or(|(a, m, _)| key < (a, m)) {
            best = Some((key.0, key.1, page));
        }
    }
    let page = best?.2;
    // Re-pack at the chosen width so chart origins match the page.
    shelf(charts, &order, page[0], page[1])?;
    Some(page)
}

/// Chart and pack one group at `spacing` metres per texel (capped at `max_edge` per side).
pub(crate) fn layout(
    positions: &[[f32; 3]],
    faces: &[[u32; 3]],
    spacing: f64,
    max_edge: u32,
) -> Result<Layout, JobError> {
    if faces.is_empty() || !(spacing.is_finite() && spacing > 0.) {
        return Err(JobError::new(
            JobErrorKind::InvalidState,
            "coarse chart group is empty or its spacing is not positive",
        ));
    }
    let (mut charts, chart_of) = grow(positions, faces);
    let target = 1. / spacing;
    let (scale, size, capped) = if let Some(size) = fit(&mut charts, target, max_edge) {
        (target, size, false)
    } else {
        // The finest scale that fits the cap: bisection on a monotone shelf fit.
        let (mut lo, mut hi) = (0., target);
        if fit(&mut charts, lo, max_edge).is_none() {
            return Err(JobError::new(
                JobErrorKind::Unsupported,
                format!(
                    "{} coarse charts cannot fit one {max_edge}² page even at zero scale",
                    charts.len()
                ),
            ));
        }
        for _ in 0..40 {
            let mid = 0.5 * (lo + hi);
            if fit(&mut charts, mid, max_edge).is_some() {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let size = fit(&mut charts, lo, max_edge).expect("bisection keeps a fitting scale");
        (lo, size, true)
    };
    let mut pixels = vec![[[0.; 2]; 3]; faces.len()];
    for c in &charts {
        for (k, &f) in c.faces.iter().enumerate() {
            pixels[f] = c.points[k].map(|q| {
                std::array::from_fn(|a| {
                    f64::from(c.origin[a]) + f64::from(GUTTER) + 0.5 + (q[a] - c.lo[a]) * scale
                })
            });
        }
    }
    Ok(Layout {
        charts,
        chart_of,
        pixels,
        size,
        scale,
        capped,
    })
}

/// Texels whose centres lie inside a face, with their barycentric weights (corner order).
/// A face covering no texel centre (sliver, point chart) gets the texel holding its
/// pixel-space centroid, so every face has appearance.
pub(crate) fn raster(pixels: &[[f64; 2]; 3], size: [u32; 2], out: &mut Vec<(u32, [f64; 3])>) {
    out.clear();
    let p = pixels;
    let den = (p[1][1] - p[2][1]) * (p[0][0] - p[2][0]) + (p[2][0] - p[1][0]) * (p[0][1] - p[2][1]);
    if den.abs() > 1e-12 {
        let lo = |a: usize| p[0][a].min(p[1][a]).min(p[2][a]).floor().max(0.) as u32;
        let hi = |a: usize| {
            (p[0][a].max(p[1][a]).max(p[2][a]).ceil() as u32).min(size[a].saturating_sub(1))
        };
        for y in lo(1)..=hi(1) {
            for x in lo(0)..=hi(0) {
                let (cx, cy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                let u = ((p[1][1] - p[2][1]) * (cx - p[2][0]) + (p[2][0] - p[1][0]) * (cy - p[2][1])) / den;
                let v = ((p[2][1] - p[0][1]) * (cx - p[2][0]) + (p[0][0] - p[2][0]) * (cy - p[2][1])) / den;
                let w = 1. - u - v;
                if u >= -1e-9 && v >= -1e-9 && w >= -1e-9 {
                    out.push((y * size[0] + x, [u.max(0.), v.max(0.), w.max(0.)]));
                }
            }
        }
    }
    if out.is_empty() {
        let c: [f64; 2] = std::array::from_fn(|a| (p[0][a] + p[1][a] + p[2][a]) / 3.);
        let x = (c[0].floor().max(0.) as u32).min(size[0] - 1);
        let y = (c[1].floor().max(0.) as u32).min(size[1] - 1);
        out.push((y * size[0] + x, [1. / 3.; 3]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folded strip: two perpendicular 10 m × 10 m quads and a degenerate face.
    fn fold() -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
        let positions = vec![
            [0., 0., 0.],
            [10., 0., 0.],
            [10., 0., 10.],
            [0., 0., 10.],
            [10., 10., 0.],
            [10., 10., 10.],
        ];
        let faces = vec![[0, 2, 1], [0, 3, 2], [1, 2, 5], [1, 5, 4], [0, 0, 1]];
        (positions, faces)
    }

    fn rects(l: &Layout) -> Vec<[u32; 4]> {
        l.charts
            .iter()
            .map(|c| [c.origin[0], c.origin[1], c.size[0], c.size[1]])
            .collect()
    }

    #[test]
    fn charts_respect_the_cone_and_never_overlap() {
        let (p, f) = fold();
        let l = layout(&p, &f, 0.1, 1024).unwrap();
        // Two planes plus the degenerate point chart.
        assert_eq!(l.charts.len(), 3);
        assert_eq!(l.chart_of[0], l.chart_of[1]);
        assert_ne!(l.chart_of[0], l.chart_of[2]);
        assert!(!l.capped);
        assert!((l.scale - 10.).abs() < 1e-12);
        // Content sized, block aligned: two 105-texel charts stacked, the point chart beside.
        assert_eq!(l.size, [108, 216]);
        assert!(l.size.iter().all(|e| e % 4 == 0));
        let r = rects(&l);
        for (i, a) in r.iter().enumerate() {
            assert!(a[0] + a[2] <= l.size[0] && a[1] + a[3] <= l.size[1]);
            for b in &r[i + 1..] {
                let apart = a[0] + a[2] <= b[0] || b[0] + b[2] <= a[0] || a[1] + a[3] <= b[1] || b[1] + b[3] <= a[1];
                assert!(apart, "chart rectangles overlap");
            }
        }
        // Every corner lies inside its chart's rectangle, at least GUTTER texels in.
        for (face, c) in l.chart_of.iter().enumerate() {
            let rect = r[*c as usize];
            for px in l.pixels[face] {
                for a in 0..2 {
                    assert!(px[a] >= f64::from(rect[a] + GUTTER));
                    assert!(px[a] <= f64::from(rect[a] + rect[a + 2] - GUTTER));
                }
            }
        }
        // Texel coverage of the two charts is disjoint and complete inside each square.
        let mut owner = vec![usize::MAX; (l.size[0] * l.size[1]) as usize];
        let mut out = Vec::new();
        for face in 0..4 {
            raster(&l.pixels[face], l.size, &mut out);
            for (t, _) in &out {
                let c = l.chart_of[face] as usize;
                assert!(owner[*t as usize] == usize::MAX || l.chart_of[owner[*t as usize]] as usize == c);
                owner[*t as usize] = face;
            }
        }
        let covered = owner.iter().filter(|o| **o != usize::MAX).count();
        assert!((2 * 100 * 100..=2 * 101 * 101).contains(&covered), "{covered}");
        raster(&l.pixels[4], l.size, &mut out);
        assert_eq!(out.len(), 1, "a degenerate face still gets one texel");
    }

    #[test]
    fn requested_density_beyond_the_cap_is_reported_and_reduced() {
        let (p, f) = fold();
        let l = layout(&p, &f, 0.001, 1024).unwrap();
        assert!(l.capped);
        assert_eq!(l.size[0].max(l.size[1]), 1024);
        assert!(l.scale < 1000. && l.scale > 40.);
        // Smallest page at the minimum edge for tiny content.
        let l = layout(&p, &f, 10., 1024).unwrap();
        assert_eq!(l.size, [MIN_PAGE, MIN_PAGE]);
        assert!(!l.capped);
    }

    /// A diagonal strip (10 m x 10 m bounding box, 1 m wide) is turned to its
    /// minimum-area rectangle: about 14 m x 1 m of texels, not 10 m x 10 m.
    #[test]
    fn charts_turn_to_their_minimum_area_rectangle() {
        let mut positions = Vec::new();
        for k in 0..=10u32 {
            let t = k as f32;
            positions.extend([[t, 0., t], [t + 0.7, 0., t - 0.7]]);
        }
        let faces: Vec<[u32; 3]> = (0..10u32)
            .flat_map(|k| {
                let (l, r) = (2 * k, 2 * k + 1);
                [[l, l + 2, r], [r, l + 2, r + 2]]
            })
            .collect();
        let l = layout(&positions, &faces, 0.1, 1024).unwrap();
        assert_eq!(l.charts.len(), 1);
        let area: u64 = l.charts.iter().map(|c| u64::from(c.size[0]) * u64::from(c.size[1])).sum();
        assert!(area < 160 * 20, "the rotated strip stays thin: {area} texels, unrotated ~111^2");
        assert!(l.charts.iter().all(|c| c.size[0] >= c.size[1]), "long side along u");
    }
}
