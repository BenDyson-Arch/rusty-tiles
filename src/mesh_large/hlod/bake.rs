//! Cheap coarse appearance bake (D5): every covered texel (and, on the vertex-colour path,
//! every coarse vertex) takes the nearest point of the CAPTURED SOURCE surface of its
//! material class within the tile's descendant range, and samples the source there.
//! No per-texel ledger, drift gate, facing or donor search.
//!
//! Bounded: the source range is streamed in chunks of at most [`CHUNK_TRIANGLES`]; each
//! chunk gets a dense uniform grid (v0.3.0's `TriGrid`) and every query point keeps the
//! nearest hit over all chunks (ties to the lower source rank, so chunking never changes
//! output). Hits record the source interpolants; images are then sampled grouped by image
//! through a bounded, shared cache of decoded colour-correct mip chains, so a tile decodes
//! each image at most once and the run never holds every image.
use crate::{
    gltf_source::source::{self as gsource, TextureSlot},
    mesh_large::{
        attributes::{FullTriangle, COLOR},
        full::FullCaptured,
        leaves::{crops, mips},
    },
    JobError, JobErrorKind,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, OnceLock},
};

/// Source triangles per bounded query chunk.
pub(crate) const CHUNK_TRIANGLES: u64 = 1 << 18;
/// Charged bytes per chunk triangle: the compact record, optional colour/frame, grid
/// items and bounding sphere, with allocation slack; twice, for a per-class sub-chunk.
pub(crate) const CHUNK_BYTES_PER_TRIANGLE: u64 = 2 * 320;
/// Grid cell cap (4 bytes each).
const MAX_CELLS: f32 = 4.0e6;

fn state(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.into())
}
fn resource(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::ResourceLimit, message.into())
}

// ----------------------------------------------------------------------------------------
// Geometry helpers.

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = sub(a, b);
    dot(d, d)
}

/// Squared distance from `p` to triangle `abc` and the barycentrics of the closest point
/// (Ericson, Real-Time Collision Detection 5.1.5).
pub(crate) fn closest(p: [f32; 3], t: &[[f32; 3]; 3]) -> (f32, [f32; 3]) {
    let [a, b, c] = *t;
    let ab = sub(b, a);
    let ac = sub(c, a);
    let n = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    if dot(n, n) <= 1e-12 * dot(ab, ab) * dot(ac, ac) || dot(n, n) == 0. {
        // Degenerate (collinear or coincident corners): the nearest of its three edges.
        let edge = |i: usize, j: usize| {
            let (u, v) = (t[i], t[j]);
            let e = sub(v, u);
            let l = dot(e, e);
            let s = if l > 0. { (dot(sub(p, u), e) / l).clamp(0., 1.) } else { 0. };
            let q: [f32; 3] = std::array::from_fn(|k| u[k] + e[k] * s);
            let mut w = [0f32; 3];
            w[i] += 1. - s;
            w[j] += s;
            (dist2(p, q), w)
        };
        let mut best = edge(0, 1);
        for (i, j) in [(1, 2), (2, 0)] {
            let r = edge(i, j);
            if r.0 < best.0 {
                best = r;
            }
        }
        return best;
    }
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    let at = |w: [f32; 3]| {
        let q: [f32; 3] = std::array::from_fn(|i| a[i] * w[0] + b[i] * w[1] + c[i] * w[2]);
        (dist2(p, q), w)
    };
    if d1 <= 0. && d2 <= 0. {
        return at([1., 0., 0.]);
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0. && d4 <= d3 {
        return at([0., 1., 0.]);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0. && d1 >= 0. && d3 <= 0. {
        let v = d1 / (d1 - d3);
        return at([1. - v, v, 0.]);
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0. && d5 <= d6 {
        return at([0., 0., 1.]);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0. && d2 >= 0. && d6 <= 0. {
        let w = d2 / (d2 - d6);
        return at([1. - w, 0., w]);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0. && (d4 - d3) >= 0. && (d5 - d6) >= 0. {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return at([0., 1. - w, w]);
    }
    let den = va + vb + vc;
    if !(den.is_finite() && den > 0.) {
        // Degenerate triangle: the nearest of its corners.
        let mut best = at([1., 0., 0.]);
        for w in [[0., 1., 0.], [0., 0., 1.]] {
            let r = at(w);
            if r.0 < best.0 {
                best = r;
            }
        }
        return best;
    }
    let v = vb / den;
    let w = vc / den;
    at([1. - v - w, v, w])
}

// ----------------------------------------------------------------------------------------
// One chunk of source triangles of the queried classes, and its grid.

/// Compact source record of one chunk triangle.
#[derive(Clone, Copy)]
struct Tri {
    p: [[f32; 3]; 3],
    rank: u64,
    material: u32,
    flags: u8,
    uv: [[[f32; 2]; 3]; 2],
}

pub(crate) struct Chunk {
    tris: Vec<Tri>,
    /// Per triangle COLOR_0 corners (empty when the run has no COLOR_0).
    colors: Vec<[[f32; 4]; 3]>,
    /// Per triangle (normal, tangent) corners, when a normal map is baked.
    frames: Vec<[([f32; 3], [f32; 4]); 3]>,
    spheres: Vec<[f32; 4]>,
    min: [f32; 3],
    max: [f32; 3],
    cell: f32,
    dims: [usize; 3],
    offsets: Vec<u32>,
    items: Vec<u32>,
}

impl Chunk {
    pub(crate) fn new() -> Self {
        Self {
            tris: Vec::new(),
            colors: Vec::new(),
            frames: Vec::new(),
            spheres: Vec::new(),
            min: [0.; 3],
            max: [0.; 3],
            cell: 1.,
            dims: [1; 3],
            offsets: Vec::new(),
            items: Vec::new(),
        }
    }
    pub(crate) fn len(&self) -> usize {
        self.tris.len()
    }
    fn clear(&mut self) {
        self.tris.clear();
        self.colors.clear();
        self.frames.clear();
        self.spheres.clear();
        self.offsets.clear();
        self.items.clear();
    }
    fn push(&mut self, rank: u64, t: &FullTriangle, colors: bool, frames: bool) {
        self.tris.push(Tri {
            p: t.triangle.positions,
            rank,
            material: t.material,
            flags: t.flags,
            uv: [0, 1].map(|s| t.corners.map(|c| c.texcoords[s])),
        });
        if colors {
            self.colors.push(if t.flags & COLOR != 0 {
                t.corners.map(|c| c.color)
            } else {
                [[1.; 4]; 3]
            });
        }
        if frames {
            self.frames.push(t.corners.map(|c| (c.normal, c.tangent)));
        }
    }
    fn span(&self, t: &[[f32; 3]; 3]) -> ([usize; 3], [usize; 3]) {
        let mut lo = [usize::MAX; 3];
        let mut hi = [0usize; 3];
        for p in t {
            for i in 0..3 {
                let c = (((p[i] - self.min[i]) / self.cell).floor().max(0.) as usize)
                    .min(self.dims[i] - 1);
                lo[i] = lo[i].min(c);
                hi[i] = hi[i].max(c);
            }
        }
        (lo, hi)
    }
    fn index(&self, x: usize, y: usize, z: usize) -> usize {
        (z * self.dims[1] + y) * self.dims[0] + x
    }
    /// Build the grid over the pushed triangles.
    fn build(&mut self) -> Result<(), JobError> {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        let mut area = 0f64;
        for t in &self.tris {
            for p in &t.p {
                for i in 0..3 {
                    min[i] = min[i].min(p[i]);
                    max[i] = max[i].max(p[i]);
                }
            }
            let (e1, e2) = (sub(t.p[1], t.p[0]), sub(t.p[2], t.p[0]));
            let c = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            area += 0.5 * f64::from(dot(c, c).sqrt());
        }
        if self.tris.is_empty() || !min.iter().chain(&max).all(|v| v.is_finite()) {
            return Err(state("coarse bake chunk is empty or nonfinite"));
        }
        let ext = std::array::from_fn::<f32, 3, _>(|i| (max[i] - min[i]).max(1e-6));
        let longest = ext[0].max(ext[1]).max(ext[2]);
        // About two triangles per occupied surface cell, bounded by the cell cap.
        let mean = (area / self.tris.len() as f64) as f32;
        let by_area = (2. * mean).sqrt();
        let by_volume = (ext[0] * ext[1] * ext[2] / MAX_CELLS).cbrt();
        let cell = by_area.max(by_volume).max(longest / 1024.).max(1e-6);
        let dims = ext.map(|e| ((e / cell).floor() as usize + 1).max(1));
        self.min = min;
        self.max = max;
        self.cell = cell;
        self.dims = dims;
        let cells = dims[0] * dims[1] * dims[2];
        self.offsets.clear();
        self.offsets.resize(cells + 1, 0);
        let mut spans = Vec::with_capacity(self.tris.len());
        for t in &self.tris {
            let (lo, hi) = self.span(&t.p);
            for z in lo[2]..=hi[2] {
                for y in lo[1]..=hi[1] {
                    for x in lo[0]..=hi[0] {
                        let i = self.index(x, y, z);
                        self.offsets[i + 1] += 1;
                    }
                }
            }
            spans.push((lo, hi));
        }
        for i in 1..=cells {
            self.offsets[i] += self.offsets[i - 1];
        }
        let total = self.offsets[cells] as usize;
        if total as u64 > CHUNK_TRIANGLES * 64 {
            return Err(resource("coarse bake grid items exceed their charge"));
        }
        let mut fill = self.offsets.clone();
        self.items.clear();
        self.items.resize(total, 0);
        for (ti, (lo, hi)) in spans.iter().enumerate() {
            for z in lo[2]..=hi[2] {
                for y in lo[1]..=hi[1] {
                    for x in lo[0]..=hi[0] {
                        let c = self.index(x, y, z);
                        self.items[fill[c] as usize] = ti as u32;
                        fill[c] += 1;
                    }
                }
            }
        }
        self.spheres = self
            .tris
            .iter()
            .map(|t| {
                let c: [f32; 3] = std::array::from_fn(|i| (t.p[0][i] + t.p[1][i] + t.p[2][i]) / 3.);
                let r = t.p.iter().map(|p| dist2(*p, c)).fold(0f32, f32::max).sqrt();
                // Slack so the f32 sphere test never rejects a true nearest.
                [c[0], c[1], c[2], r * (1. + 1e-5) + 1e-6]
            })
            .collect();
        Ok(())
    }
    /// Squared distance from `p` to the chunk's box.
    fn box_distance2(&self, p: [f32; 3]) -> f32 {
        (0..3)
            .map(|i| {
                let d = (self.min[i] - p[i]).max(p[i] - self.max[i]).max(0.);
                d * d
            })
            .sum()
    }
    /// The nearest triangle to `p` better than `best` (by squared distance, then rank),
    /// starting from `seed`. Returns `(triangle, d2, bary)` when one improves `best`.
    fn nearest(
        &self,
        p: [f32; 3],
        best: (f32, u64),
        seed: Option<u32>,
    ) -> Option<(u32, f32, [f32; 3])> {
        let mut found: Option<(u32, f32, [f32; 3])> = None;
        let mut bound = best;
        let consider = |ti: u32, found: &mut Option<(u32, f32, [f32; 3])>, bound: &mut (f32, u64)| {
            let t = &self.tris[ti as usize];
            let (d2, w) = closest(p, &t.p);
            if (d2, t.rank) < *bound {
                *bound = (d2, t.rank);
                *found = Some((ti, d2, w));
            }
        };
        if let Some(s) = seed.filter(|s| (*s as usize) < self.tris.len()) {
            consider(s, &mut found, &mut bound);
        }
        let c: [i64; 3] = std::array::from_fn(|i| ((p[i] - self.min[i]) / self.cell).floor() as i64);
        let far = (0..3)
            .map(|i| c[i].abs().max((self.dims[i] as i64 - 1 - c[i]).abs()))
            .max()
            .unwrap_or(0);
        for r in 0..=far {
            for dz in -r..=r {
                let z = c[2] + dz;
                if z < 0 || z >= self.dims[2] as i64 {
                    continue;
                }
                for dy in -r..=r {
                    let y = c[1] + dy;
                    if y < 0 || y >= self.dims[1] as i64 {
                        continue;
                    }
                    let shell = dz.abs() == r || dy.abs() == r;
                    let mut dx = -r;
                    while dx <= r {
                        let x = c[0] + dx;
                        let step = if shell || dx == r { 1 } else { 2 * r };
                        if x >= 0 && x < self.dims[0] as i64 {
                            let ci = self.index(x as usize, y as usize, z as usize);
                            for &ti in &self.items[self.offsets[ci] as usize..self.offsets[ci + 1] as usize] {
                                let s = &self.spheres[ti as usize];
                                let lower = dist2(p, [s[0], s[1], s[2]]).sqrt() - s[3];
                                if lower > 0. && lower * lower > bound.0 {
                                    continue;
                                }
                                consider(ti, &mut found, &mut bound);
                            }
                        }
                        dx += step.max(1);
                    }
                }
            }
            // Every unvisited cell is farther than the searched cube's boundary.
            let wall = (0..3)
                .map(|i| {
                    let lo = self.min[i] + (c[i] - r) as f32 * self.cell;
                    let hi = self.min[i] + (c[i] + r + 1) as f32 * self.cell;
                    (p[i] - lo).min(hi - p[i])
                })
                .fold(f32::INFINITY, f32::min)
                .max(0.);
            if bound.0.is_finite() && bound.0 < wall * wall {
                break;
            }
        }
        found
    }
}

// ----------------------------------------------------------------------------------------
// Query state.

/// The nearest source point of one query, with the interpolants sampling needs.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Hit {
    pub d2: f32,
    pub rank: u64,
    pub material: u32,
    pub flags: u8,
    /// Interpolated TEXCOORD_0/1.
    pub uv: [[f32; 2]; 2],
    /// Isotropic UV-units per metre of the source triangle, per texcoord set.
    pub gradient: [f32; 2],
    /// Interpolated COLOR_0 (white without it).
    pub color: [f32; 4],
}

impl Hit {
    pub(crate) const NONE: Hit = Hit {
        d2: f32::INFINITY,
        rank: u64::MAX,
        material: u32::MAX,
        flags: 0,
        uv: [[0.; 2]; 2],
        gradient: [0.; 2],
        color: [1.; 4],
    };
    pub(crate) fn found(&self) -> bool {
        self.rank != u64::MAX
    }
}

/// Source tangent frame at a hit (normal-map baking only).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Frame {
    pub normal: [f32; 3],
    pub tangent: [f32; 4],
}

/// Query points of one material class and their running nearest hits.
pub(crate) struct Queries {
    pub class: u32,
    pub points: Vec<[f32; 3]>,
    /// Consecutive points likely share a nearest triangle (one coarse face's texels).
    pub hits: Vec<Hit>,
    pub frames: Vec<Frame>,
    pub want_frames: bool,
}

impl Queries {
    pub(crate) fn new(class: u32, points: Vec<[f32; 3]>, want_frames: bool) -> Self {
        let n = points.len();
        Self {
            class,
            points,
            hits: vec![Hit::NONE; n],
            frames: if want_frames { vec![Frame::default(); n] } else { Vec::new() },
            want_frames,
        }
    }
}

fn gradient(t: &Tri, set: usize) -> f32 {
    let (e1, e2) = (sub(t.p[1], t.p[0]), sub(t.p[2], t.p[0]));
    let c = [
        e1[1] * e2[2] - e1[2] * e2[1],
        e1[2] * e2[0] - e1[0] * e2[2],
        e1[0] * e2[1] - e1[1] * e2[0],
    ];
    let area = dot(c, c).sqrt();
    let uv = t.uv[set];
    let a = (uv[1][0] - uv[0][0]) * (uv[2][1] - uv[0][1]) - (uv[2][0] - uv[0][0]) * (uv[1][1] - uv[0][1]);
    if area > 0. && area.is_finite() {
        (a.abs() / area).sqrt()
    } else {
        0.
    }
}

fn run_queries(chunk: &Chunk, q: &mut Queries, check: &dyn Fn() -> Result<(), JobError>) -> Result<(), JobError> {
    let mut seed = None;
    for i in 0..q.points.len() {
        if i % 65536 == 0 {
            check()?;
        }
        let p = q.points[i];
        let current = q.hits[i];
        if chunk.box_distance2(p) >= current.d2 {
            seed = None;
            continue;
        }
        let Some((ti, d2, w)) = chunk.nearest(p, (current.d2, current.rank), seed) else {
            continue;
        };
        seed = Some(ti);
        let t = &chunk.tris[ti as usize];
        let lerp2 = |v: [[f32; 2]; 3]| -> [f32; 2] {
            std::array::from_fn(|a| v[0][a] * w[0] + v[1][a] * w[1] + v[2][a] * w[2])
        };
        let color = if chunk.colors.is_empty() {
            [1.; 4]
        } else {
            let c = chunk.colors[ti as usize];
            std::array::from_fn(|a| c[0][a] * w[0] + c[1][a] * w[1] + c[2][a] * w[2])
        };
        q.hits[i] = Hit {
            d2,
            rank: t.rank,
            material: t.material,
            flags: t.flags,
            uv: [lerp2(t.uv[0]), lerp2(t.uv[1])],
            gradient: [gradient(t, 0), gradient(t, 1)],
            color,
        };
        if q.want_frames && !chunk.frames.is_empty() {
            let f = chunk.frames[ti as usize];
            q.frames[i] = Frame {
                normal: std::array::from_fn(|a| f[0].0[a] * w[0] + f[1].0[a] * w[1] + f[2].0[a] * w[2]),
                tangent: {
                    let mut t: [f32; 4] =
                        std::array::from_fn(|a| f[0].1[a] * w[0] + f[1].1[a] * w[1] + f[2].1[a] * w[2]);
                    t[3] = if f[0].1[3] < 0. { -1. } else { 1. };
                    t
                },
            };
        }
    }
    Ok(())
}

/// Stream the source range `[first, end)` once, in bounded chunks, and resolve every
/// query set against the source triangles of its class. Returns the number of source
/// triangles indexed.
pub(crate) fn resolve(
    captured: &FullCaptured,
    first: u64,
    end: u64,
    queries: &mut [Queries],
    colors: bool,
    check: &dyn Fn() -> Result<(), JobError>,
) -> Result<u64, JobError> {
    if queries.iter().all(|q| q.points.is_empty()) {
        return Ok(0);
    }
    let frames = queries.iter().any(|q| q.want_frames);
    let classes = &captured.material_classes;
    let wanted: Vec<u32> = queries.iter().map(|q| q.class).collect();
    let mut chunk = Chunk::new();
    let mut reader = captured.triangles.range(first, end, 1 << 16)?;
    let mut rank = first;
    let mut indexed = 0u64;
    // One chunk per class set at a time: a chunk holds triangles of every queried class,
    // and each query set considers only its own class (`material` filter below).
    let mut class_of: Vec<u32> = Vec::new();
    loop {
        chunk.clear();
        class_of.clear();
        while (chunk.len() as u64) < CHUNK_TRIANGLES {
            let Some(t) = reader.next().transpose()? else {
                break;
            };
            if (rank - first).is_multiple_of(1 << 16) {
                check()?;
            }
            let class = classes.of(t.material);
            if wanted.contains(&class) {
                chunk.push(rank, &t, colors, frames);
                class_of.push(class);
            }
            rank += 1;
        }
        if chunk.len() == 0 {
            break;
        }
        indexed += chunk.len() as u64;
        // A chunk per queried class keeps the grid exact to the class filter.
        for q in queries.iter_mut() {
            if q.points.is_empty() {
                continue;
            }
            let mut sub = Chunk::new();
            for (i, c) in class_of.iter().enumerate() {
                if *c == q.class {
                    sub.tris.push(chunk.tris[i]);
                    if colors {
                        sub.colors.push(chunk.colors[i]);
                    }
                    if frames {
                        sub.frames.push(chunk.frames[i]);
                    }
                }
            }
            if sub.tris.is_empty() {
                continue;
            }
            if sub.tris.len() == chunk.tris.len() {
                // Every chunk triangle is of this class: index the chunk in place.
                drop(sub);
                chunk.build()?;
                run_queries(&chunk, q, check)?;
            } else {
                sub.build()?;
                run_queries(&sub, q, check)?;
            }
        }
        if rank == end {
            break;
        }
    }
    if rank != end {
        return Err(state("coarse bake source range ended early"));
    }
    for q in queries.iter() {
        if q.hits.iter().any(|h| !h.found()) {
            return Err(state(format!(
                "coarse bake: material class {} has no source triangle in [{first}, {end})",
                q.class
            )));
        }
    }
    Ok(indexed)
}

// ----------------------------------------------------------------------------------------
// Decoded source chains and sampling.

pub(crate) struct Chain {
    pub levels: Vec<mips::Level>,
    pub bytes: u64,
}

type Cell = Arc<OnceLock<Result<Arc<Chain>, JobError>>>;
struct Entry {
    cell: Cell,
    last_use: u64,
    bytes: u64,
}
#[derive(Default)]
struct CacheState {
    entries: HashMap<(usize, u8), Entry>,
    clock: u64,
    resident: u64,
    peak: u64,
    hits: u64,
    misses: u64,
}

/// Decoded colour-correct mip chains keyed by (image, class), shared by the stage's
/// workers; least recently used entries beyond `budget` bytes are evicted (holders keep
/// an evicted chain alive only while they sample it). Never changes output.
pub(crate) struct ImageCache {
    budget: u64,
    state: Mutex<CacheState>,
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStats {
    pub budget_bytes: u64,
    pub peak_resident_bytes: u64,
    pub hits: u64,
    pub misses: u64,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Bytes of a decoded image's complete mip chain (4/3 of RGBA8, rounded up).
pub(crate) fn chain_bytes(width: u32, height: u32) -> u64 {
    (u64::from(width) * u64::from(height) * 4).div_ceil(3) * 4 + 4096
}

impl ImageCache {
    pub(crate) fn new(budget: u64) -> Self {
        Self {
            budget,
            state: Mutex::new(CacheState::default()),
        }
    }
    pub(crate) fn stats(&self) -> CacheStats {
        let s = lock(&self.state);
        CacheStats {
            budget_bytes: self.budget,
            peak_resident_bytes: s.peak,
            hits: s.hits,
            misses: s.misses,
        }
    }
    pub(crate) fn get(
        &self,
        captured: &FullCaptured,
        image: usize,
        class: mips::Class,
        check: &dyn Fn() -> Result<(), JobError>,
    ) -> Result<Arc<Chain>, JobError> {
        let key = (image, class_rank(class));
        let cell = {
            let mut s = lock(&self.state);
            s.clock += 1;
            let clock = s.clock;
            let hit = s.entries.contains_key(&key);
            let entry = s.entries.entry(key).or_insert_with(|| Entry {
                cell: Arc::new(OnceLock::new()),
                last_use: clock,
                bytes: 0,
            });
            entry.last_use = clock;
            let cell = entry.cell.clone();
            if hit {
                s.hits += 1;
            } else {
                s.misses += 1;
            }
            cell
        };
        let result = cell.get_or_init(|| load(captured, image, class, check).map(Arc::new)).clone();
        let mut s = lock(&self.state);
        match &result {
            Err(_) => {
                if s.entries.get(&key).is_some_and(|e| Arc::ptr_eq(&e.cell, &cell)) {
                    let removed = s.entries.remove(&key).map_or(0, |e| e.bytes);
                    s.resident -= removed;
                }
            }
            Ok(chain) => {
                let bytes = chain.bytes;
                if let Some(e) = s.entries.get_mut(&key) {
                    if e.bytes == 0 && Arc::ptr_eq(&e.cell, &cell) {
                        e.bytes = bytes;
                        s.resident += bytes;
                    }
                }
                s.peak = s.peak.max(s.resident);
                while s.resident > self.budget {
                    let Some(oldest) = s
                        .entries
                        .iter()
                        .filter(|(k, e)| **k != key && e.bytes > 0)
                        .min_by_key(|(k, e)| (e.last_use, **k))
                        .map(|(k, _)| *k)
                    else {
                        break;
                    };
                    let removed = s.entries.remove(&oldest).map_or(0, |e| e.bytes);
                    s.resident -= removed;
                }
            }
        }
        result
    }
}

fn class_rank(c: mips::Class) -> u8 {
    match c {
        mips::Class::Colour => 0,
        mips::Class::Normal => 1,
        mips::Class::Data => 2,
        mips::Class::Id => 3,
    }
}

fn load(
    captured: &FullCaptured,
    image: usize,
    class: mips::Class,
    check: &dyn Fn() -> Result<(), JobError>,
) -> Result<Chain, JobError> {
    let snapshot = captured
        .images
        .get(image)
        .ok_or_else(|| state("coarse bake source image"))?;
    let encoded = snapshot.reconstruct(&mut || check())?;
    let (info, rgba) = gsource::decode_image_admitted(
        &encoded,
        snapshot.identity.mime.as_deref(),
        gsource::DecodeAdmission::LARGE_MESH,
        &mut || check(),
    )?;
    drop(encoded);
    let limit = chain_bytes(info.width, info.height).saturating_mul(4);
    let chain = mips::build(
        info.width,
        info.height,
        rgba,
        class,
        None,
        usize::try_from(limit).map_err(|_| resource("coarse bake chain size"))?,
    )?;
    Ok(Chain {
        levels: chain.levels,
        bytes: chain_bytes(info.width, info.height),
    })
}

fn wrap(i: i64, n: u32, mode: u32) -> usize {
    let n = i64::from(n);
    (match mode {
        33071 => i.clamp(0, n - 1),
        33648 => {
            let period = 2 * n;
            let m = i.rem_euclid(period);
            if m < n {
                m
            } else {
                period - 1 - m
            }
        }
        _ => i.rem_euclid(n),
    }) as usize
}

pub(crate) fn linear(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
pub(crate) fn srgb(v: f64) -> f64 {
    let v = v.clamp(0., 1.);
    if v <= 0.0031308 {
        12.92 * v
    } else {
        1.055 * v.powf(1. / 2.4) - 0.055
    }
}

/// Trilinear sample of a decoded chain at `uv` and level `lod`, in LINEAR value space for
/// colour classes (RGB) and raw [0, 1] for data/normal. The source sampler's wrap modes
/// and magnification filter apply; minification always uses the colour-correct chain
/// (the coarse texel represents an area, not a point).
pub(crate) fn sample(
    chain: &Chain,
    class: mips::Class,
    sampler: crops::SourceSampler,
    uv: [f32; 2],
    lod: f64,
) -> [f64; 4] {
    let levels = &chain.levels;
    let top = (levels.len() - 1) as f64;
    let l = if lod.is_finite() { lod.clamp(0., top) } else { 0. };
    let a = l.floor() as usize;
    let b = (a + 1).min(levels.len() - 1);
    let f = l - a as f64;
    let nearest = l <= 0. && matches!(sampler.mag_filter, 9728);
    let fetch = |level: &mips::Level| -> [f64; 4] {
        let x = f64::from(uv[0]) * f64::from(level.width) - 0.5;
        let y = f64::from(uv[1]) * f64::from(level.height) - 0.5;
        let get = |x: i64, y: i64| -> [f64; 4] {
            let i = (wrap(y, level.height, sampler.wrap_t) * level.width as usize
                + wrap(x, level.width, sampler.wrap_s))
                * 4;
            std::array::from_fn(|c| {
                let v = f64::from(level.rgba[i + c]) / 255.;
                if class == mips::Class::Colour && c < 3 {
                    linear(v)
                } else {
                    v
                }
            })
        };
        if !(x.is_finite() && y.is_finite()) {
            return get(0, 0);
        }
        if nearest {
            return get((x + 0.5).floor() as i64, (y + 0.5).floor() as i64);
        }
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let p = [get(x0, y0), get(x0 + 1, y0), get(x0, y0 + 1), get(x0 + 1, y0 + 1)];
        std::array::from_fn(|c| {
            let top = p[0][c] + (p[1][c] - p[0][c]) * fx;
            let bottom = p[2][c] + (p[3][c] - p[2][c]) * fx;
            top + (bottom - top) * fy
        })
    };
    let p = fetch(&levels[a]);
    if b == a || f == 0. {
        return p;
    }
    let q = fetch(&levels[b]);
    std::array::from_fn(|c| p[c] + (q[c] - p[c]) * f)
}

/// The texture of `slot` in source material `material`: (image, sampler, texcoord set).
pub(crate) fn binding(
    captured: &FullCaptured,
    material: u32,
    slot: TextureSlot,
) -> Result<Option<(usize, crops::SourceSampler, usize)>, JobError> {
    let Some(m) = captured.materials.get(material as usize) else {
        return Ok(None);
    };
    let Some(b) = m.bindings().find(|b| b.slot == slot) else {
        return Ok(None);
    };
    let texture = captured
        .textures
        .get(b.texture)
        .ok_or_else(|| state("coarse bake texture"))?;
    let image = texture["source"]
        .as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .ok_or_else(|| state("coarse bake texture image"))?;
    let sampler = crops::resolve(
        texture
            .get("sampler")
            .and_then(serde_json::Value::as_u64)
            .and_then(|i| captured.samplers.get(i as usize)),
    )?;
    Ok(Some((image, sampler, b.texcoord)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closest_point_regions_and_degenerate() {
        let t = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]];
        let (d, w) = closest([0.25, 0.25, 2.], &t);
        assert!((d - 4.).abs() < 1e-6 && (w[0] - 0.5).abs() < 1e-6);
        let (d, w) = closest([-1., -1., 0.], &t);
        assert!((d - 2.).abs() < 1e-6 && w == [1., 0., 0.]);
        let (d, w) = closest([1., 1., 0.], &t);
        assert!((d - 0.5).abs() < 1e-6 && (w[1] - 0.5).abs() < 1e-6 && (w[2] - 0.5).abs() < 1e-6);
        let flat = [[0., 0., 0.], [0., 0., 0.], [2., 0., 0.]];
        let (d, _) = closest([2., 1., 0.], &flat);
        assert!((d - 1.).abs() < 1e-6);
    }

    #[test]
    fn grid_nearest_matches_brute_force_with_rank_ties() {
        // Deterministic pseudo-random triangles, every fifth duplicated (rank tie-break).
        let mut seed = 0x9e3779b97f4a7c15u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 40) as f32 / (1u64 << 24) as f32
        };
        let mut chunk = Chunk::new();
        for i in 0..2000u64 {
            let p = if i % 5 == 4 {
                chunk.tris.last().unwrap().p
            } else {
                let c = [next() * 20., next() * 2., next() * 20.];
                std::array::from_fn(|_| [c[0] + next(), c[1] + next() * 0.1, c[2] + next()])
            };
            chunk.tris.push(Tri {
                p,
                rank: i,
                material: 0,
                flags: 0,
                uv: [[[0.; 2]; 3]; 2],
            });
        }
        chunk.build().unwrap();
        let mut seed_tri = None;
        for _ in 0..500 {
            let q = [next() * 24. - 2., next() * 4. - 1., next() * 24. - 2.];
            let (ti, d2, _) = chunk.nearest(q, (f32::INFINITY, u64::MAX), seed_tri).unwrap();
            seed_tri = Some(ti);
            let brute = chunk
                .tris
                .iter()
                .map(|t| (closest(q, &t.p).0, t.rank))
                .min_by(|a, b| a.partial_cmp(b).unwrap())
                .unwrap();
            assert_eq!((d2, chunk.tris[ti as usize].rank), brute);
        }
    }

    #[test]
    fn trilinear_sampling_wraps_and_minifies_in_linear_light() {
        // A 2x2 black/white checker: level 1 is the linear-light mean, not sRGB 0.5.
        let rgba = [0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255].to_vec();
        let built = mips::build(2, 2, rgba, mips::Class::Colour, None, 1 << 20).unwrap();
        let chain = Chain {
            levels: built.levels,
            bytes: 0,
        };
        let s = crops::SourceSampler {
            wrap_s: 10497,
            wrap_t: 10497,
            min_filter: 9729,
            mag_filter: 9729,
        };
        let v = sample(&chain, mips::Class::Colour, s, [0.25, 0.25], 0.);
        assert!(v[0] < 1e-9);
        // Repeat wrap: u = 1.25 is u = 0.25.
        assert_eq!(sample(&chain, mips::Class::Colour, s, [1.25, 0.25], 0.), v);
        let mean = sample(&chain, mips::Class::Colour, s, [0.3, 0.7], 1.);
        assert!((mean[0] - 0.5).abs() < 0.01, "{mean:?}");
    }
}
