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
use crate::mesh_large::mask_pack::{self, Mask};
use crate::{JobError, JobErrorKind};

type V3 = [f64; 3];
type P2 = [f64; 2];

/// Least cosine between a chart face's normal and its seed's: bounds a face's texel-area
/// stretch on the chart plane at `1 / 0.8²` (about 1.56×).
const CONE_COS: f64 = 0.5;
/// Charts of at most this many faces offer their faces to an adjacent larger chart.
const SMALL_CHART: usize = 16;
/// Least cosine between a merged face's normal and the receiving chart's normal (its
/// texel-area stretch stays below 1 / 0.2 on that one face).
const MERGE_COS: f64 = 0.2;
/// Most faces one chart may grow to (bounds the overlap test; a 2D box prefilter keeps
/// its quadratic part cheap).
const CHART_FACES: usize = 192;
/// Texels of gutter on every side of a chart; the sparse-mip encoder fills them from the
/// nearest covered texel at every level.
pub(crate) const GUTTER: u32 = 2;
/// Mip levels through which a coarse page keeps its charts isolated: chart masks are whole
/// cells of the `2^K` grid with gutters of `2^K` (at least [`GUTTER`]). A scale-gate tuning
/// constant. One level suffices in normal traversal: a coarse texel is at least `G/16`
/// (the requested spacing is `1.25·G/16`; the source-density floor and the page cap only
/// coarsen it), so
/// when Cesium refines at SSE 16 the tile's `G` spans 16 px and a texel at least 1 px.
/// The tile stays shown only until its parent's SSE drops below 16; with the parent's
/// error about `2G` the tile's `G` then spans 8 px, a texel at least 0.5 px, so the tile is
/// minified at most about 2x (one level) before its parent replaces it. The exceptions
/// (the root, which nothing replaces; a parent whose error is well above `2G`; parents
/// shown while children load) are seen from distances where bleed below level 1 is
/// invisible.
pub(crate) const COARSE_ISLAND_ISOLATED_LEVELS: u32 = 1;
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
    /// Per face: its projected 2D box (overlap prefilter during growth).
    boxes: Vec<(P2, P2)>,
    lo: P2,
    hi: P2,
    origin: [u32; 2],
    size: [u32; 2],
    /// Packed cells on the `2^isolated` grid.
    mask: Mask,
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
    /// Mip levels through which the charts stay isolated (`2^isolated` alignment and
    /// gutters): [`COARSE_ISLAND_ISOLATED_LEVELS`], lower only when the charts cannot fit the
    /// page at that alignment even at zero scale.
    pub isolated: u32,
    /// The requested spacing could not fit `max_edge²`: `scale < 1 / spacing`.
    pub capped: bool,
}

impl Layout {
    /// Per page texel: 1 + the chart whose packed mask (gutters included) holds it, 0
    /// outside every chart. Masks are whole `2^isolated` cells and never share one, which
    /// keeps charts apart in reduced levels and gutter fill.
    pub(crate) fn labels(&self) -> Vec<u32> {
        let [w, h] = self.size;
        let a = 1u32 << self.isolated;
        let mut labels = vec![0u32; w as usize * h as usize];
        for (i, c) in self.charts.iter().enumerate() {
            for (cy, c0, c1) in c.mask.runs() {
                for cx in c0..c1 {
                    let (x0, y0) = (c.origin[0] + cx * a, c.origin[1] + cy * a);
                    for y in y0..(y0 + a).min(h) {
                        let row = (y * w) as usize;
                        labels[row + x0 as usize..row + (x0 + a).min(w) as usize]
                            .fill(i as u32 + 1);
                    }
                }
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

/// Face adjacency over manifold (welded, exactly two distinct faces) edges in CSR form:
/// at most two directed pairs per edge, so at most `6 F` pairs and O(F log F) time.
/// Charts never grow across a non-manifold edge (a fan of F faces on one edge stays
/// linear instead of creating every face pair).
fn adjacency(faces: &[[u32; 3]]) -> (Vec<u32>, Vec<(u32, u32)>) {
    let n = faces.len();
    let mut edges: Vec<(u32, u32, u32)> = Vec::with_capacity(n * 3);
    for (i, f) in faces.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (f[k], f[(k + 1) % 3]);
            edges.push((a.min(b), a.max(b), i as u32));
        }
    }
    edges.sort_unstable();
    let mut pairs: Vec<(u32, u32)> = Vec::with_capacity(edges.len());
    let mut s = 0;
    while s < edges.len() {
        let mut e = s + 1;
        while e < edges.len() && edges[e].0 == edges[s].0 && edges[e].1 == edges[s].1 {
            e += 1;
        }
        if e - s == 2 && edges[s].2 != edges[s + 1].2 {
            pairs.push((edges[s].2, edges[s + 1].2));
            pairs.push((edges[s + 1].2, edges[s].2));
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
    (start, pairs)
}

/// Grow charts over `faces` (corner indices into `positions`): seeds by descending area
/// (then face order), breadth-first over shared edges within the normal cone, rejecting a
/// face whose projection overlaps the chart's interior. Zero-area faces get a point chart.
pub(crate) fn grow(positions: &[[f32; 3]], faces: &[[u32; 3]]) -> (Vec<Chart>, Vec<u32>) {
    let p = |v: u32| positions[v as usize].map(f64::from);
    let n = faces.len();
    let mut normals = Vec::with_capacity(n);
    let mut areas = Vec::with_capacity(n);
    for f in faces {
        let c = cross(sub(p(f[1]), p(f[0])), sub(p(f[2]), p(f[0])));
        areas.push(dot(c, c).sqrt());
        normals.push(unit(c));
    }
    let (start, pairs) = adjacency(faces);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| areas[b].total_cmp(&areas[a]).then(a.cmp(&b)));
    let mut chart_of = vec![u32::MAX; n];
    let mut charts: Vec<Chart> = Vec::new();
    // Per chart: its plane origin and normal (the seed's).
    let mut frames: Vec<(V3, V3)> = Vec::new();
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
                boxes: vec![([0.; 2], [0.; 2])],
                lo: [0.; 2],
                hi: [0.; 2],
                origin: [0; 2],
                size: [0; 2],
                mask: Mask::default(),
            });
            frames.push((p(faces[seed][0]), [0., 1., 0.]));
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
            boxes: Vec::new(),
            lo: [f64::MAX; 2],
            hi: [f64::MIN; 2],
            origin: [0; 2],
            size: [0; 2],
            mask: Mask::default(),
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
            let boxed = |t: &[P2; 3]| {
                let lo = [0, 1].map(|a| t[0][a].min(t[1][a]).min(t[2][a]));
                let hi = [0, 1].map(|a| t[0][a].max(t[1][a]).max(t[2][a]));
                (lo, hi)
            };
            let (plo, phi) = boxed(&projected);
            if chart.points.iter().zip(&chart.boxes).any(|(t, (lo, hi))| {
                lo[0] < phi[0]
                    && plo[0] < hi[0]
                    && lo[1] < phi[1]
                    && plo[1] < hi[1]
                    && overlap(&projected, t)
            }) {
                continue;
            }
            chart_of[f] = id;
            for c in &projected {
                chart.lo = std::array::from_fn(|a| chart.lo[a].min(c[a]));
                chart.hi = std::array::from_fn(|a| chart.hi[a].max(c[a]));
            }
            chart.faces.push(f);
            chart.points.push(projected);
            chart.boxes.push((plo, phi));
            for k in start[f]..start[f + 1] {
                let next = pairs[k as usize].1 as usize;
                if chart_of[next] == u32::MAX {
                    queue.push(next);
                }
            }
        }
        charts.push(chart);
        frames.push((origin, normal));
    }
    merge_small(
        &mut charts,
        &mut chart_of,
        &frames,
        (&start, &pairs),
        &normals,
        &p,
        faces,
    );
    for chart in &mut charts {
        orient(chart);
    }
    (charts, chart_of)
}

/// Offer each face of a small chart (at most [`SMALL_CHART`] faces) to an adjacent chart
/// with more faces whose plane it faces within [`MERGE_COS`] and whose projection it does
/// not overlap; drop charts left empty and renumber. Fewer, larger charts spend fewer
/// page texels on gutters.
#[allow(clippy::too_many_arguments)]
fn merge_small(
    charts: &mut Vec<Chart>,
    chart_of: &mut [u32],
    frames: &[(V3, V3)],
    (start, pairs): (&[u32], &[(u32, u32)]),
    normals: &[Option<V3>],
    p: &dyn Fn(u32) -> V3,
    faces: &[[u32; 3]],
) {
    for small in 0..charts.len() {
        if charts[small].faces.len() > SMALL_CHART || charts[small].faces.is_empty() {
            continue;
        }
        let members = charts[small].faces.clone();
        for f in members {
            let Some(m) = normals[f] else { continue };
            let target = (start[f]..start[f + 1])
                .map(|k| chart_of[pairs[k as usize].1 as usize] as usize)
                .filter(|&c| {
                    c != small
                        && charts[c].faces.len() > charts[small].faces.len()
                        // Merges never grow a chart past the growth cap: overlap tests stay
                        // bounded per face.
                        && charts[c].faces.len() < CHART_FACES
                })
                .find(|&c| {
                    let (origin, normal) = frames[c];
                    if dot(m, normal) < MERGE_COS {
                        return false;
                    }
                    let (x, y) = (charts[c].x, charts[c].y);
                    let projected: [P2; 3] = std::array::from_fn(|k| {
                        let v = sub(p(faces[f][k]), origin);
                        [dot(v, x), dot(v, y)]
                    });
                    let lo =
                        [0, 1].map(|a| projected[0][a].min(projected[1][a]).min(projected[2][a]));
                    let hi =
                        [0, 1].map(|a| projected[0][a].max(projected[1][a]).max(projected[2][a]));
                    !charts[c]
                        .points
                        .iter()
                        .zip(&charts[c].boxes)
                        .any(|(t, (blo, bhi))| {
                            blo[0] < hi[0]
                                && lo[0] < bhi[0]
                                && blo[1] < hi[1]
                                && lo[1] < bhi[1]
                                && overlap(&projected, t)
                        })
                });
            let Some(c) = target else { continue };
            let (origin, _) = frames[c];
            let (x, y) = (charts[c].x, charts[c].y);
            let projected: [P2; 3] = std::array::from_fn(|k| {
                let v = sub(p(faces[f][k]), origin);
                [dot(v, x), dot(v, y)]
            });
            let lo = [0, 1].map(|a| projected[0][a].min(projected[1][a]).min(projected[2][a]));
            let hi = [0, 1].map(|a| projected[0][a].max(projected[1][a]).max(projected[2][a]));
            let s = &mut charts[small];
            let at = s.faces.iter().position(|&g| g == f).expect("member face");
            s.faces.remove(at);
            s.points.remove(at);
            s.boxes.remove(at);
            let d = &mut charts[c];
            d.faces.push(f);
            d.points.push(projected);
            d.boxes.push((lo, hi));
            d.lo = std::array::from_fn(|a| d.lo[a].min(lo[a]));
            d.hi = std::array::from_fn(|a| d.hi[a].max(hi[a]));
            chart_of[f] = c as u32;
        }
    }
    // Drop emptied charts; recompute the bounds of shrunk ones and renumber.
    let mut renumber = vec![u32::MAX; charts.len()];
    let mut kept = 0u32;
    for (i, c) in charts.iter().enumerate() {
        if !c.faces.is_empty() {
            renumber[i] = kept;
            kept += 1;
        }
    }
    charts.retain(|c| !c.faces.is_empty());
    for c in charts.iter_mut() {
        c.lo = [f64::MAX; 2];
        c.hi = [f64::MIN; 2];
        for t in &c.points {
            for q in t {
                c.lo = std::array::from_fn(|a| c.lo[a].min(q[a]));
                c.hi = std::array::from_fn(|a| c.hi[a].max(q[a]));
            }
        }
    }
    for c in chart_of.iter_mut() {
        *c = renumber[*c as usize];
    }
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
            let d = if hi[1] - lo[1] > hi[0] - lo[0] {
                [-d[1], d[0]]
            } else {
                d
            };
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

/// Each chart's mask at `scale` on the `a` grid: every cell holding a texel a face
/// rasterises to, or within `g` texels of one (the gutter). Word-parallel: a texel
/// bitmap, dilated by `g` along rows, then each cell ORs its `a + 2g` dilated rows.
fn masks(charts: &mut [Chart], scale: f64, a: u32, g: u32) {
    masks_with_usage(charts, scale, a, g, |_, _, _| {});
}

fn masks_with_usage(
    charts: &mut [Chart],
    scale: f64,
    a: u32,
    g: u32,
    mut usage: impl FnMut(u64, u64, u64),
) {
    assert!(a.is_power_of_two() && a <= 64 && g < 64);
    let (mut bits, mut wide, mut block) = (Vec::new(), Vec::new(), Vec::new());
    for c in charts.iter_mut() {
        let [w, h] = c.size;
        let tw = w.div_ceil(64) as usize;
        bits.clear();
        bits.resize(tw * h as usize, 0u64);
        for t in &c.points {
            let pixels =
                t.map(|q| std::array::from_fn(|k| f64::from(g) + (q[k] - c.lo[k]) * scale));
            raster_spans(&pixels, c.size, |y, x0, x1| {
                let row = &mut bits[y as usize * tw..(y as usize + 1) * tw];
                for x in x0..=x1 {
                    row[x as usize / 64] |= 1 << (x % 64);
                }
            });
        }
        // Row dilation: every bit spreads `g` texels left and right.
        wide.clear();
        wide.resize(bits.len(), 0u64);
        for (row, out) in bits.chunks_exact(tw).zip(wide.chunks_exact_mut(tw)) {
            for (i, o) in out.iter_mut().enumerate() {
                let mut v = row[i];
                for s in 1..=g {
                    v |= row[i] << s | row[i] >> s;
                    if i > 0 {
                        v |= row[i - 1] >> (64 - s);
                    }
                    if i + 1 < tw {
                        v |= row[i + 1] << (64 - s);
                    }
                }
                *o = v;
            }
        }
        let cells = c.size.map(|e| e.div_ceil(a));
        let mut mask = Mask::new(cells);
        let low = if a == 64 { u64::MAX } else { (1u64 << a) - 1 };
        for cy in 0..cells[1] {
            let y0 = (cy * a).saturating_sub(g) as usize;
            let y1 = ((cy * a + a + g) as usize).min(h as usize);
            block.clear();
            block.resize(tw, 0u64);
            for y in y0..y1 {
                for (b, &v) in block.iter_mut().zip(&wide[y * tw..(y + 1) * tw]) {
                    *b |= v;
                }
            }
            for cx in 0..cells[0] {
                let x = (cx * a) as usize;
                if block[x / 64] >> (x % 64) & low != 0 {
                    mask.set(cx, cy);
                }
            }
        }
        usage(
            (c.mask.bits.capacity() * 8) as u64,
            (mask.bits.capacity() * 8) as u64,
            ((bits.capacity() + wide.capacity() + block.capacity()) * 8) as u64,
        );
        c.mask = mask;
    }
}

/// Peak heap for one group's mask construction or packing, which run sequentially.
/// `texels` is the group's allowance, `faces` bounds its chart count. All fits have
/// at least one isolated mip (a >= 2), edges <= edge and summed rectangles <= 64*T.
/// For a mask w/a by h/a, bytes = 8*ceil(w/(64*a))*h/a, bounded by
/// area/(8*a²) + 8*h/a. Also cap by `faces` full-edge masks, useful for small groups.
/// Replacement keeps at most faces+1 masks live. Their rectangle area is bounded
/// by the previous and current fits together (2*A), including isolation changes.
pub(crate) fn workspace_bytes(edge: u32, texels: u64, faces: u64) -> u64 {
    let a = 2u32; // layout's minimum isolation, even if the preferred K increases.
    let cells = edge.div_ceil(a);
    let mask = u64::from(cells.div_ceil(64)) * u64::from(cells) * 8;
    let rectangles = texels
        .saturating_mul(64)
        .min(u64::from(edge).pow(2).saturating_mul(faces));
    let retained = (rectangles
        .div_ceil(8 * u64::from(a).pow(2))
        .saturating_add(8 * u64::from(cells) * faces))
    .min(mask.saturating_mul(faces));
    let row_words = u64::from(edge.div_ceil(64));
    // Vec<u64> starts at four words even for a one-word row.
    let row = row_words.max(4);
    let raster = (row_words * u64::from(edge)).max(4);
    // Two reusable raster Vecs: <=2*raster capacity each. During growth one
    // old allocation (<raster) can coexist with its new allocation. Row scratch
    // has <=2*row capacity; growing it instead needs less than this raster peak.
    let scratch = 8 * (5 * raster + 2 * row);
    let containers = faces.saturating_mul(256);
    let live_masks = faces.saturating_add(1);
    let replacing = (rectangles
        .saturating_mul(2)
        .div_ceil(8 * u64::from(a).pow(2))
        .saturating_add(8 * u64::from(cells) * live_masks))
    .min(mask.saturating_mul(live_masks));
    let construction = replacing.saturating_add(scratch);
    // The shared pack bound covers doubled retained page capacity. Also cover
    // an old page allocation overlapping realloc, just as for raster scratch.
    let old_page = 8 * (u64::from(cells.div_ceil(64)) + 1) * u64::from(cells);
    let packing = retained
        .saturating_add(mask_pack::workspace_bytes(cells, faces as usize))
        .saturating_add(old_page);
    containers.saturating_add(construction.max(packing))
}

/// Page widths tried, as factors of the masked area's square root (plus the widest chart).
const WIDTHS: [f64; 4] = [1., 1.15, 1.35, 1.7];

fn sizes(charts: &mut [Chart], scale: f64, a: u32) -> Option<()> {
    for c in charts.iter_mut() {
        let extent = |a: usize| ((c.hi[a] - c.lo[a]).max(0.) * scale).ceil();
        let (w, h) = (extent(0), extent(1));
        if !(w.is_finite() && h.is_finite()) || w > 1e6 || h > 1e6 {
            return None;
        }
        // Gutters of `a` and rectangles on the `a` grid (isolation through log2 a).
        let g = a.max(GUTTER);
        c.size = [w.max(1.) as u32, h.max(1.) as u32].map(|e| (e + 2 * g).div_ceil(a) * a);
    }
    Some(())
}

/// The least-area block-aligned page in `MIN_PAGE..=max_edge` per side holding every
/// chart's mask at `scale` (bottom-left by descending mask height at a few widths around
/// the square root of the masked area; ties to the narrower); packs the charts into it.
fn fit(
    charts: &mut [Chart],
    scale: f64,
    max_edge: u32,
    max_texels: u64,
    a: u32,
) -> Option<[u32; 2]> {
    sizes(charts, scale, a)?;
    // Reject impossible rectangles before raster scratch/masks are allocated. The
    // total bounding area also bounds retained masks for arbitrarily sparse charts.
    let area: u64 = charts
        .iter()
        .map(|c| u64::from(c.size[0]) * u64::from(c.size[1]))
        .sum();
    if charts.iter().any(|c| c.size.iter().any(|&e| e > max_edge))
        || area > max_texels.saturating_mul(64)
    {
        return None;
    }
    masks(charts, scale, a, a.max(GUTTER));
    let unit = a.max(BLOCK);
    // Small allowances may use pages below MIN_PAGE (whole blocks).
    let min_edge =
        ((max_texels as f64).sqrt() as u32 / unit * unit).clamp(unit, MIN_PAGE.max(unit));
    let mut order: Vec<usize> = (0..charts.len()).collect();
    order.sort_by(|&x, &y| {
        charts[y].mask.cells[1]
            .cmp(&charts[x].mask.cells[1])
            .then(x.cmp(&y))
    });
    let round = |v: u32| v.div_ceil(unit) * unit;
    let cells: u64 = charts.iter().map(|c| c.mask.count()).sum();
    let widest = charts.iter().map(|c| c.mask.cells[0]).max().unwrap_or(1) * a;
    let side = ((cells * u64::from(a * a)) as f64).sqrt();
    let mut widths: Vec<u32> = WIDTHS
        .iter()
        .map(|f| round((side * f).ceil() as u32))
        .chain([round(widest)])
        .map(|w| w.clamp(min_edge, max_edge).max(round(widest)))
        .filter(|&w| w <= max_edge)
        .collect();
    widths.sort_unstable();
    widths.dedup();
    let masks: Vec<&Mask> = charts.iter().map(|c| &c.mask).collect();
    // (page area, longer side, page, chart origins in cells)
    type Fit = (u64, u32, [u32; 2], Vec<[u32; 2]>);
    let mut best: Option<Fit> = None;
    for &w in &widths {
        let Some((at, used, h)) = mask_pack::pack(&masks, &order, w / a, max_edge / a) else {
            continue;
        };
        let page = [round(used * a).max(min_edge), round(h * a).max(min_edge)];
        if page[1] > max_edge || u64::from(page[0]) * u64::from(page[1]) > max_texels {
            continue;
        }
        let key = (
            u64::from(page[0]) * u64::from(page[1]),
            page[0].max(page[1]),
        );
        if best.as_ref().is_none_or(|(x, m, ..)| key < (*x, *m)) {
            best = Some((key.0, key.1, page, at));
        }
    }
    let (_, _, page, origins) = best?;
    // The chosen width's own packing (cells to texels).
    for (c, o) in charts.iter_mut().zip(&origins) {
        c.origin = o.map(|v| v * a);
    }
    Some(page)
}

/// Chart and pack one group at `spacing` metres per texel, capped at `max_edge` per side
/// and `max_texels` per page (the group's share of its tile's texel budget).
pub(crate) fn layout(
    positions: &[[f32; 3]],
    faces: &[[u32; 3]],
    spacing: f64,
    max_edge: u32,
    max_texels: u64,
) -> Result<Layout, JobError> {
    if faces.is_empty() || !(spacing.is_finite() && spacing > 0.) {
        return Err(JobError::new(
            JobErrorKind::InvalidState,
            "coarse chart group is empty or its spacing is not positive",
        ));
    }
    let (mut charts, chart_of) = grow(positions, faces);
    let target = 1. / spacing;
    // The deepest isolation whose alignment and gutters still fit at zero scale.
    let Some(isolated) = (1..=COARSE_ISLAND_ISOLATED_LEVELS.max(1))
        .rev()
        .find(|&k| fit(&mut charts, 0., max_edge, max_texels, 1 << k).is_some())
    else {
        return Err(JobError::new(
            JobErrorKind::Unsupported,
            format!(
                "{} coarse charts cannot fit one {max_edge}² page of {max_texels} texels even at zero scale",
                charts.len()
            ),
        ));
    };
    let a = 1u32 << isolated;
    let gutter = a.max(GUTTER);
    let (scale, size, capped) =
        if let Some(size) = fit(&mut charts, target, max_edge, max_texels, a) {
            (target, size, false)
        } else {
            // The finest scale that fits the cap, by bisection: `lo` always fits (zero scale
            // fits at this isolation), and a fit is deterministic, so the final fit holds.
            let (mut lo, mut hi) = (0., target);
            for _ in 0..40 {
                let mid = 0.5 * (lo + hi);
                if fit(&mut charts, mid, max_edge, max_texels, a).is_some() {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let size = fit(&mut charts, lo, max_edge, max_texels, a)
                .expect("bisection keeps a fitting scale");
            (lo, size, true)
        };
    let mut pixels = vec![[[0.; 2]; 3]; faces.len()];
    for c in &charts {
        for (k, &f) in c.faces.iter().enumerate() {
            pixels[f] = c.points[k].map(|q| {
                std::array::from_fn(|a| {
                    f64::from(c.origin[a]) + f64::from(gutter) + (q[a] - c.lo[a]) * scale
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
        isolated,
        capped,
    })
}

/// Texels whose centres lie inside a face, with their barycentric weights (corner order).
/// A face covering no texel centre (sliver, point chart) gets the texel holding its
/// pixel-space centroid, so every face has appearance. Row spans: O(rows + covered) per
/// face, never its bounding box (a long sliver costs its length, not its box area).
pub(crate) fn raster(pixels: &[[f64; 2]; 3], size: [u32; 2], out: &mut Vec<(u32, [f64; 3])>) {
    out.clear();
    raster_each(pixels, size, |texel, w| out.push((texel, w)));
}

/// [`raster`] without collecting: `f(texel, weights)` per covered texel, in the same order.
fn raster_each(pixels: &[[f64; 2]; 3], size: [u32; 2], mut f: impl FnMut(u32, [f64; 3])) {
    raster_rows(pixels, size, |y, x0, x1, bary| {
        let mut any = false;
        for x in x0..=x1 {
            let w = bary(f64::from(x) + 0.5);
            if w.iter().all(|&v| v >= -1e-9) {
                any = true;
                f(y * size[0] + x, w.map(|v| v.max(0.)));
            }
        }
        any
    });
}

/// The covered texels of each row as one span `[x0, x1]`: the weights are affine along a
/// row, so only the ends are tested. Used for coverage masks, where weights are unused.
fn raster_spans(pixels: &[[f64; 2]; 3], size: [u32; 2], mut f: impl FnMut(u32, u32, u32)) {
    raster_rows(pixels, size, |y, x0, x1, bary| {
        let inside = |x: u32| bary(f64::from(x) + 0.5).iter().all(|&v| v >= -1e-9);
        let Some(a) = (x0..=x1).find(|&x| inside(x)) else {
            return false;
        };
        let b = (a..=x1).rev().find(|&x| inside(x)).unwrap_or(a);
        f(y, a, b);
        true
    });
}

/// Rows of [`raster`]: per row `y`, the candidate texel range `[x0, x1]` and the weights at
/// a texel centre `x`; `row` returns whether it covered a texel. When no row did, the
/// centroid texel is reported with equal weights.
fn raster_rows(
    pixels: &[[f64; 2]; 3],
    size: [u32; 2],
    mut row: impl FnMut(u32, u32, u32, &dyn Fn(f64) -> [f64; 3]) -> bool,
) {
    let mut any = false;
    let p = pixels;
    let den = (p[1][1] - p[2][1]) * (p[0][0] - p[2][0]) + (p[2][0] - p[1][0]) * (p[0][1] - p[2][1]);
    if den.abs() > 1e-12 {
        let ylo = (p[0][1].min(p[1][1]).min(p[2][1]) - 0.5).ceil().max(0.) as u32;
        let yhi = ((p[0][1].max(p[1][1]).max(p[2][1]) - 0.5).floor().max(-1.) as i64)
            .min(i64::from(size[1]) - 1);
        let bary = |cx: f64, cy: f64| {
            let u =
                ((p[1][1] - p[2][1]) * (cx - p[2][0]) + (p[2][0] - p[1][0]) * (cy - p[2][1])) / den;
            let v =
                ((p[2][1] - p[0][1]) * (cx - p[2][0]) + (p[0][0] - p[2][0]) * (cy - p[2][1])) / den;
            [u, v, 1. - u - v]
        };
        for y in i64::from(ylo)..=yhi {
            let cy = y as f64 + 0.5;
            // The span of the row's centre line inside the triangle (edge crossings).
            let (mut xl, mut xr) = (f64::INFINITY, f64::NEG_INFINITY);
            for k in 0..3 {
                let (a, b) = (p[k], p[(k + 1) % 3]);
                if (a[1] - cy) * (b[1] - cy) <= 0. {
                    let x = if a[1] == b[1] {
                        xl = xl.min(a[0].min(b[0]));
                        xr = xr.max(a[0].max(b[0]));
                        continue;
                    } else {
                        a[0] + (cy - a[1]) / (b[1] - a[1]) * (b[0] - a[0])
                    };
                    xl = xl.min(x);
                    xr = xr.max(x);
                }
            }
            if xl > xr {
                continue;
            }
            let x0 = ((xl - 0.5).ceil() - 1.).max(0.) as u32;
            let x1 = ((xr - 0.5).floor() + 1.).min(f64::from(size[0]) - 1.);
            if x1 < 0. {
                continue;
            }
            any |= row(y as u32, x0, x1 as u32, &|cx| bary(cx, cy));
        }
    }
    if !any {
        let c: [f64; 2] = std::array::from_fn(|a| (p[0][a] + p[1][a] + p[2][a]) / 3.);
        let x = (c[0].floor().max(0.) as u32).min(size[0] - 1);
        let y = (c[1].floor().max(0.) as u32).min(size[1] - 1);
        row(y, x, x, &|_| [1. / 3.; 3]);
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
    fn mask_charge_covers_measured_aspect_changes_and_word_rounding() {
        let (p, f) = fold();
        let (mut charts, _) = grow(&p, &f);
        // Reuse buffers across narrow/tall, wide/short and non-word-aligned masks,
        // then replace the retained set at another scale/aspect arrangement.
        for dims in [
            [[6, 1024], [1024, 6], [130, 130]],
            [[1024, 1024], [6, 1024], [1024, 6]],
            [[130, 130], [1024, 1024], [6, 6]],
        ] {
            for (c, size) in charts.iter_mut().zip(dims) {
                c.size = size;
            }
            let mut retained = charts
                .iter()
                .map(|c| (c.mask.bits.capacity() * 8) as u64)
                .sum::<u64>();
            let mut peak = retained;
            masks_with_usage(&mut charts, 1., 2, 2, |old, new, scratch| {
                peak = peak.max(retained + new + scratch);
                retained = retained - old + new;
            });
            let charge = workspace_bytes(1024, 1 << 20, f.len() as u64);
            assert!(peak <= charge, "measured {peak}, charge {charge}");
            for c in &charts {
                assert_eq!(
                    c.mask.bits.capacity() as u64 * 8,
                    u64::from(c.size[0].div_ceil(2).div_ceil(64))
                        * u64::from(c.size[1].div_ceil(2))
                        * 8
                );
            }
        }
        assert_eq!(workspace_bytes(1024, 1 << 20, 8192), 40_505_600);
        assert!(workspace_bytes(512, 1 << 18, 4096) < workspace_bytes(1024, 1 << 20, 8192));
    }

    #[test]
    fn oversized_mask_workspace_is_rejected_before_raster_allocation() {
        let (p, f) = fold();
        let (mut charts, _) = grow(&p, &f);
        assert!(fit(&mut charts, 1000., 1024, 1 << 20, 2).is_none());
        assert!(charts.iter().all(|c| c.mask.bits.capacity() == 0));
        assert!(fit(&mut charts, 10., 1024, 1, 2).is_none());
        assert!(charts.iter().all(|c| c.mask.bits.capacity() == 0));
    }

    #[test]
    fn charts_respect_the_cone_and_never_overlap() {
        let (p, f) = fold();
        let l = layout(&p, &f, 0.1, 1024, 1 << 20).unwrap();
        // Two planes plus the degenerate point chart.
        assert_eq!(l.charts.len(), 3);
        assert_eq!(l.chart_of[0], l.chart_of[1]);
        assert_ne!(l.chart_of[0], l.chart_of[2]);
        assert!(!l.capped);
        assert!((l.scale - 10.).abs() < 1e-12);
        // Content sized on the 2^K grid with 2^K gutters (K = 1: 2-texel grid and gutters):
        // two 104-texel charts stacked, the point chart beside, block-aligned page.
        assert_eq!(l.isolated, COARSE_ISLAND_ISOLATED_LEVELS);
        let a = 1u32 << l.isolated;
        assert_eq!(l.size, [104, 216]);
        assert!(l.size.iter().all(|e| e % 4 == 0));
        assert!(l
            .charts
            .iter()
            .all(|c| c.origin.iter().chain(&c.size).all(|v| v % a == 0)));
        let r = rects(&l);
        for (i, a) in r.iter().enumerate() {
            assert!(a[0] + a[2] <= l.size[0] && a[1] + a[3] <= l.size[1]);
            for b in &r[i + 1..] {
                let apart = a[0] + a[2] <= b[0]
                    || b[0] + b[2] <= a[0]
                    || a[1] + a[3] <= b[1]
                    || b[1] + b[3] <= a[1];
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
                assert!(
                    owner[*t as usize] == usize::MAX
                        || l.chart_of[owner[*t as usize]] as usize == c
                );
                owner[*t as usize] = face;
            }
        }
        let covered = owner.iter().filter(|o| **o != usize::MAX).count();
        assert!(
            (2 * 100 * 100..=2 * 101 * 101).contains(&covered),
            "{covered}"
        );
        raster(&l.pixels[4], l.size, &mut out);
        assert_eq!(out.len(), 1, "a degenerate face still gets one texel");
    }

    #[test]
    fn requested_density_beyond_the_cap_is_reported_and_reduced() {
        let (p, f) = fold();
        let l = layout(&p, &f, 0.001, 1024, 1 << 20).unwrap();
        assert!(l.capped);
        assert_eq!(l.size[0].max(l.size[1]), 1024);
        assert!(l.scale < 1000. && l.scale > 40.);
        // Smallest page at the minimum edge for tiny content.
        let l = layout(&p, &f, 10., 1024, 1 << 20).unwrap();
        assert_eq!(
            l.size,
            [MIN_PAGE, MIN_PAGE],
            "tiny content: the minimum page"
        );
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
        let l = layout(&positions, &faces, 0.1, 1024, 1 << 20).unwrap();
        assert_eq!(l.charts.len(), 1);
        let area: u64 = l
            .charts
            .iter()
            .map(|c| u64::from(c.size[0]) * u64::from(c.size[1]))
            .sum();
        assert!(
            area < 160 * 20,
            "the rotated strip stays thin: {area} texels, unrotated ~111^2"
        );
        assert!(
            l.charts.iter().all(|c| c.size[0] >= c.size[1]),
            "long side along u"
        );
    }

    /// A long sliver across a 1024² page rasterises by row spans (exactly the texel
    /// centres inside it, the same set a bounding-box scan finds) and a 50,000-face
    /// bumpy grid's charts never exceed the growth cap after merging.
    #[test]
    fn slivers_raster_by_spans_and_merges_respect_the_cap() {
        let sliver = [[1.3, 2.1], [1020.7, 1019.2], [1021.1, 1018.6]];
        let size = [1024, 1024];
        let mut spans = Vec::new();
        raster(&sliver, size, &mut spans);
        let mut boxed = Vec::new();
        let den = (sliver[1][1] - sliver[2][1]) * (sliver[0][0] - sliver[2][0])
            + (sliver[2][0] - sliver[1][0]) * (sliver[0][1] - sliver[2][1]);
        for y in 0..1024u32 {
            for x in 0..1024u32 {
                let (cx, cy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                let p = sliver;
                let u = ((p[1][1] - p[2][1]) * (cx - p[2][0])
                    + (p[2][0] - p[1][0]) * (cy - p[2][1]))
                    / den;
                let v = ((p[2][1] - p[0][1]) * (cx - p[2][0])
                    + (p[0][0] - p[2][0]) * (cy - p[2][1]))
                    / den;
                if u >= -1e-9 && v >= -1e-9 && 1. - u - v >= -1e-9 {
                    boxed.push(y * 1024 + x);
                }
            }
        }
        assert!(!boxed.is_empty());
        assert_eq!(spans.iter().map(|t| t.0).collect::<Vec<_>>(), boxed);
        // Bumpy grid: alternating tilts keep charts small, so merging is exercised.
        let n = 160u32;
        let positions: Vec<[f32; 3]> = (0..=n)
            .flat_map(|i| {
                (0..=n).map(move |j| [i as f32, ((i * 7 + j * 13) % 5) as f32 * 0.4, j as f32])
            })
            .collect();
        let faces: Vec<[u32; 3]> = (0..n)
            .flat_map(|i| {
                (0..n).flat_map(move |j| {
                    let v = |a: u32, b: u32| a * (n + 1) + b;
                    [
                        [v(i, j), v(i, j + 1), v(i + 1, j)],
                        [v(i + 1, j), v(i, j + 1), v(i + 1, j + 1)],
                    ]
                })
            })
            .collect();
        let (charts, chart_of) = grow(&positions, &faces);
        assert!(charts.iter().all(|c| c.faces.len() <= CHART_FACES));
        assert_eq!(
            charts.iter().map(|c| c.faces.len()).sum::<usize>(),
            faces.len()
        );
        for (i, c) in charts.iter().enumerate() {
            assert!(c.faces.iter().all(|&f| chart_of[f] as usize == i));
        }
    }

    /// Review adversarial (round 2): a fan of 8,192 faces on one non-manifold edge.
    /// Adjacency stays linear (no face pairs across the edge), charting and packing
    /// finish quickly, and every face keeps a chart.
    #[test]
    fn non_manifold_fans_stay_linear() {
        let n = 8192u32;
        let mut positions = vec![[0f32, 0., 0.], [1., 0., 0.]];
        let faces: Vec<[u32; 3]> = (0..n)
            .map(|k| {
                let a = std::f32::consts::TAU * k as f32 / n as f32;
                positions.push([0.5, a.cos(), a.sin()]);
                [0, 1, 2 + k]
            })
            .collect();
        let started = std::time::Instant::now();
        let (_, pairs) = adjacency(&faces);
        assert!(
            pairs.len() <= 6 * faces.len(),
            "adjacency memory is O(F): {}",
            pairs.len()
        );
        let l = layout(&positions, &faces, 0.05, 1024, 1 << 20).unwrap();
        assert!(
            started.elapsed().as_secs_f64() < 20.,
            "{:?}",
            started.elapsed()
        );
        assert_eq!(l.chart_of.len(), faces.len());
        assert!(
            l.charts.iter().all(|c| c.faces.len() == 1),
            "no growth across the fan's edge"
        );
    }
}
