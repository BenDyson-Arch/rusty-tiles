//! Cheap coarse appearance bake (D5): every covered texel (and, on the vertex-colour path,
//! every coarse vertex) takes the nearest point of the CAPTURED SOURCE surface within the
//! tile's descendant range (S2's bounded [`SourceQuery`]: distance then source-rank ties,
//! no facing or material restriction) and samples the source there. No per-texel ledger,
//! drift gate, facing or donor search.
//!
//! Hits record the donor's interpolants; images are then sampled grouped by image through
//! a bounded, shared cache of decoded colour-correct mip chains ([`ImageCache`]), so a
//! tile decodes each image at most once and the run never holds every image.
use super::source_index::{Hit as SourceHit, SourceQuery, Surface};
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

fn state(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.into())
}
fn resource(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::ResourceLimit, message.into())
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

/// Isotropic UV units per metre of a source triangle for texcoord set `set`.
fn gradient(t: &FullTriangle, set: usize) -> f32 {
    let p = t.triangle.positions.map(|p| p.map(f64::from));
    let (e1, e2) = (
        [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]],
        [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]],
    );
    let c = [
        e1[1] * e2[2] - e1[2] * e2[1],
        e1[2] * e2[0] - e1[0] * e2[2],
        e1[0] * e2[1] - e1[1] * e2[0],
    ];
    let area = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
    let uv = t.corners.map(|c| c.texcoords[set].map(f64::from));
    let a = (uv[1][0] - uv[0][0]) * (uv[2][1] - uv[0][1]) - (uv[2][0] - uv[0][0]) * (uv[1][1] - uv[0][1]);
    if area > 0. && area.is_finite() {
        (a.abs() / area).sqrt() as f32
    } else {
        0.
    }
}

/// Query points resolved per chunk: nearest searches first, then the chunk's donors are
/// read in rank order (merged runs), then hits are filled. Bounds the donor set.
pub(crate) const CHUNK: usize = 1 << 16;
/// Ranks closer than this share one positioned read of at most `RUN_SPAN` records.
const RUN_GAP: u64 = 32;
const RUN_SPAN: u64 = 512;

/// What every texel a donor serves needs, derived once per donor.
struct Donor {
    material: u32,
    flags: u8,
    texcoords: [[[f32; 2]; 3]; 2],
    colors: [[f32; 4]; 3],
    normals: [[f32; 3]; 3],
    tangents: [[f32; 4]; 3],
    gradient: [f32; 2],
}
impl Donor {
    fn new(t: &FullTriangle) -> Self {
        Self {
            material: t.material,
            flags: t.flags,
            texcoords: [0, 1].map(|s| t.corners.map(|c| c.texcoords[s])),
            colors: t.corners.map(|c| c.color),
            normals: t.corners.map(|c| c.normal),
            tangents: t.corners.map(|c| c.tangent),
            gradient: [gradient(t, 0), gradient(t, 1)],
        }
    }
}

/// Bytes [`resolve`] holds besides its queries: one chunk's witnesses and donors.
pub(crate) const RESOLVE_BYTES: u64 = (CHUNK as u64)
    * (std::mem::size_of::<(u64, [f32; 3], f32)>() + 8 + std::mem::size_of::<(u64, Donor)>())
        as u64
    + RUN_SPAN * 2 * std::mem::size_of::<FullTriangle>() as u64
    + (1 << 20);

/// Tiles whose descendant source range has at most this many triangles answer the bake's
/// nearest searches from one in-memory surface of that range ([`Searcher::local`]).
pub(crate) const LOCAL_TRIANGLES: u64 = 1 << 17;
/// Bytes of the largest local surface: compact records and their hierarchy.
pub(crate) const LOCAL_BYTES: u64 = LOCAL_TRIANGLES * 160 + (1 << 20);

/// The bake's exact nearest-surface search over a tile's descendant source range: S2's
/// paged [`SourceQuery`], or, for a small range, the same triangles in one resident
/// [`Surface`] (built once per tile) searched with the same distance and tie rule.
pub(crate) struct Searcher<'q, 'a> {
    query: &'q mut SourceQuery<'a>,
    local: Option<(Surface, Option<usize>)>,
}

impl<'q, 'a> Searcher<'q, 'a> {
    /// A searcher of `query`'s range; resident when the range is small enough.
    pub(crate) fn new(query: &'q mut SourceQuery<'a>) -> Result<Self, JobError> {
        let key = query.key;
        let local = if key.end - key.first <= LOCAL_TRIANGLES {
            let triangles = query.surface()?.collect::<Result<Vec<_>, _>>()?;
            let surface = Surface::new(triangles)?;
            if surface.charge() as u64 > LOCAL_BYTES {
                return Err(resource("coarse bake local surface exceeds its charge"));
            }
            Some((surface, None))
        } else {
            None
        };
        Ok(Self { query, local })
    }
    /// Always S2's paged query (tests compare the two paths).
    #[cfg(test)]
    pub(crate) fn paged(query: &'q mut SourceQuery<'a>) -> Self {
        Self { query, local: None }
    }
    pub(crate) fn query(&mut self) -> &mut SourceQuery<'a> {
        self.query
    }
    fn nearest(&mut self, p: [f64; 3]) -> Result<SourceHit, JobError> {
        let key = self.query.key;
        match &mut self.local {
            Some((surface, hint)) => Ok(surface.nearest_hint(p, key, hint)),
            None => self.query.nearest(p),
        }
    }
}

/// Resolve every query point of the tile against the captured source surface of its
/// descendant range: every point gets its exact nearest triangle (distance, then source
/// order and rank on ties; no reuse, facing, material or donor-distance heuristic),
/// on a resident surface for small ranges ([`Searcher`]). Donor records are read
/// per chunk in rank order. Returns (points searched, donor reads).
pub(crate) fn resolve(
    searcher: &mut Searcher<'_, '_>,
    queries: &mut [Queries],
    check: &dyn Fn() -> Result<(), JobError>,
) -> Result<(u64, u64), JobError> {
    let (mut searches, mut reads) = (0u64, 0u64);
    let mut witnesses: Vec<(u64, [f32; 3], f32)> = Vec::with_capacity(CHUNK);
    let mut ranks: Vec<u64> = Vec::with_capacity(CHUNK);
    let mut donors: Vec<(u64, Donor)> = Vec::with_capacity(CHUNK);
    let (mut encoded, mut run) = (Vec::new(), Vec::new());
    for q in queries.iter_mut() {
        let mut start = 0;
        while start < q.points.len() {
            check()?;
            let end = (start + CHUNK).min(q.points.len());
            witnesses.clear();
            for point in &q.points[start..end] {
                searches += 1;
                let hit = searcher.nearest(point.map(f64::from))?;
                witnesses.push((hit.rank, hit.bary.map(|v| v as f32), hit.distance2 as f32));
            }
            let query = searcher.query();
            // The chunk's donors, read in rank order as merged runs.
            ranks.clear();
            ranks.extend(witnesses.iter().map(|w| w.0));
            ranks.sort_unstable();
            ranks.dedup();
            donors.clear();
            let mut i = 0;
            while i < ranks.len() {
                let first = ranks[i];
                let mut j = i + 1;
                while j < ranks.len()
                    && ranks[j] - ranks[j - 1] <= RUN_GAP
                    && ranks[j] - first < RUN_SPAN
                {
                    j += 1;
                }
                let last = ranks[j - 1];
                query.triangles(first, (last - first + 1) as usize, &mut encoded, &mut run)?;
                reads += 1;
                for &rank in &ranks[i..j] {
                    donors.push((rank, Donor::new(&run[(rank - first) as usize])));
                }
                i = j;
            }
            for (k, (rank, w, d2)) in witnesses.iter().enumerate() {
                let i = start + k;
                let at = donors
                    .binary_search_by_key(rank, |d| d.0)
                    .map_err(|_| state("coarse bake donor"))?;
                let t = &donors[at].1;
                let lerp2 = |v: [[f32; 2]; 3]| -> [f32; 2] {
                    std::array::from_fn(|a| v[0][a] * w[0] + v[1][a] * w[1] + v[2][a] * w[2])
                };
                let color = if t.flags & COLOR != 0 {
                    std::array::from_fn(|a| (0..3).map(|c| t.colors[c][a] * w[c]).sum::<f32>())
                } else {
                    [1.; 4]
                };
                q.hits[i] = Hit {
                    d2: *d2,
                    rank: *rank,
                    material: t.material,
                    flags: t.flags,
                    uv: [lerp2(t.texcoords[0]), lerp2(t.texcoords[1])],
                    gradient: t.gradient,
                    color,
                };
                if q.want_frames {
                    q.frames[i] = Frame {
                        normal: std::array::from_fn(|a| (0..3).map(|c| t.normals[c][a] * w[c]).sum()),
                        tangent: {
                            let mut v: [f32; 4] = std::array::from_fn(|a| {
                                (0..3).map(|c| t.tangents[c][a] * w[c]).sum()
                            });
                            v[3] = if t.tangents[0][3] < 0. { -1. } else { 1. };
                            v
                        },
                    };
                }
            }
            start = end;
        }
    }
    Ok((searches, reads))
}

// ----------------------------------------------------------------------------------------
// Decoded source chains and sampling.

/// A decoded source image's chain from level `base` down to 1x1 (RGBA8, colour
/// classes sRGB-encoded): `levels[0]` is source level `base`.
pub(crate) struct Chain {
    pub base: u32,
    pub levels: Vec<mips::Level>,
    pub bytes: u64,
}

type Cell = Arc<OnceLock<Result<Arc<Chain>, JobError>>>;
struct Entry {
    cell: Cell,
    base: u32,
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
    load_seconds: f64,
}

/// Decoded chains keyed by (image, class), shared by the stage's workers. A request
/// names the finest level it samples; a cached chain starting at that level or finer
/// serves it, otherwise the chain is rebuilt from the requested level (coarse tiles
/// keep only small levels resident). Least recently used entries beyond `budget` bytes
/// are evicted (holders keep an evicted chain alive only while they sample it). Never
/// changes output: every chain level is the same function of the source image.
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
    /// Thread CPU seconds of image reconstruction, decode and chain builds.
    pub load_cpu_seconds: f64,
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
            load_cpu_seconds: s.load_seconds,
        }
    }
    /// The chain of `image` as `class`, holding at least levels `need..`.
    pub(crate) fn get(
        &self,
        captured: &FullCaptured,
        image: usize,
        class: mips::Class,
        need: u32,
        check: &dyn Fn() -> Result<(), JobError>,
    ) -> Result<Arc<Chain>, JobError> {
        let key = (image, class_rank(class));
        let (cell, base) = {
            let mut s = lock(&self.state);
            s.clock += 1;
            let clock = s.clock;
            let cached = s.entries.get(&key).filter(|e| e.base <= need).map(|e| e.cell.clone());
            match cached {
                Some(cell) => {
                    s.hits += 1;
                    let e = s.entries.get_mut(&key).expect("cached entry");
                    e.last_use = clock;
                    (cell, e.base)
                }
                None => {
                    s.misses += 1;
                    let cell: Cell = Arc::new(OnceLock::new());
                    let old = s.entries.insert(
                        key,
                        Entry {
                            cell: cell.clone(),
                            base: need,
                            last_use: clock,
                            bytes: 0,
                        },
                    );
                    s.resident -= old.map_or(0, |e| e.bytes);
                    (cell, need)
                }
            }
        };
        let mut seconds = 0.;
        let result = cell
            .get_or_init(|| {
                let started = super::error::thread_cpu();
                let chain = load(captured, image, class, base, check).map(Arc::new);
                seconds = super::error::thread_cpu().zip(started).map_or(0., |(b, a)| b - a);
                chain
            })
            .clone();
        let mut s = lock(&self.state);
        s.load_seconds += seconds;
        let current = s.entries.get(&key).is_some_and(|e| Arc::ptr_eq(&e.cell, &cell));
        match &result {
            Err(_) => {
                if current {
                    let removed = s.entries.remove(&key).map_or(0, |e| e.bytes);
                    s.resident -= removed;
                }
            }
            Ok(chain) => {
                if current {
                    let e = s.entries.get_mut(&key).expect("current entry");
                    if e.bytes == 0 {
                        e.bytes = chain.bytes;
                        s.resident += chain.bytes;
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

/// The next level of a chain: each texel is the mean of its (up to) 2x2 parents,
/// in linear light for colour RGB, raw otherwise (alpha always raw).
fn halve(src: &mips::Level, class: mips::Class) -> mips::Level {
    let (sw, sh) = (src.width as usize, src.height as usize);
    let (w, h) = ((sw / 2).max(1), (sh / 2).max(1));
    let lut = linear_lut();
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        let rows = [(2 * y).min(sh - 1), (2 * y + 1).min(sh - 1)];
        for x in 0..w {
            let cols = [(2 * x).min(sw - 1), (2 * x + 1).min(sw - 1)];
            let mut sum = [0f64; 4];
            for r in rows {
                for c in cols {
                    let i = (r * sw + c) * 4;
                    for (k, v) in sum.iter_mut().enumerate() {
                        let b = src.rgba[i + k];
                        *v += if class == mips::Class::Colour && k < 3 {
                            lut[b as usize]
                        } else {
                            f64::from(b) / 255.
                        };
                    }
                }
            }
            let o = (y * w + x) * 4;
            for (k, v) in sum.iter().enumerate() {
                let mean = v * 0.25;
                rgba[o + k] = if class == mips::Class::Colour && k < 3 {
                    srgb_byte(mean)
                } else {
                    (mean.clamp(0., 1.) * 255.).round() as u8
                };
            }
        }
    }
    mips::Level {
        width: w as u32,
        height: h as u32,
        rgba,
    }
}

fn load(
    captured: &FullCaptured,
    image: usize,
    class: mips::Class,
    base: u32,
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
    let mut level = mips::Level {
        width: info.width,
        height: info.height,
        rgba,
    };
    let mut index = 0;
    while index < base && (level.width > 1 || level.height > 1) {
        level = halve(&level, class);
        index += 1;
    }
    check()?;
    let mut levels = vec![level];
    loop {
        let last = levels.last().expect("chain level");
        if last.width == 1 && last.height == 1 {
            break;
        }
        let next = halve(last, class);
        levels.push(next);
    }
    let bytes = levels.iter().map(|l| l.rgba.len() as u64).sum::<u64>() + 4096;
    Ok(Chain {
        base: index,
        levels,
        bytes,
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

/// sRGB byte to linear light.
fn linear_lut() -> &'static [f64; 256] {
    static LUT: OnceLock<[f64; 256]> = OnceLock::new();
    LUT.get_or_init(|| std::array::from_fn(|i| linear(i as f64 / 255.)))
}
const ENCODE_STEPS: usize = 1 << 14;
/// Linear light in [0, 1] to the nearest sRGB byte, through a 16,384-step table
/// (at most a quarter byte from the exact transfer away from rounding ties).
pub(crate) fn srgb_byte(v: f64) -> u8 {
    static LUT: OnceLock<Vec<u8>> = OnceLock::new();
    let lut = LUT.get_or_init(|| {
        (0..=ENCODE_STEPS)
            .map(|i| (srgb(i as f64 / ENCODE_STEPS as f64) * 255.).round() as u8)
            .collect()
    });
    let x = if v.is_finite() { v.clamp(0., 1.) } else { 0. };
    lut[(x * ENCODE_STEPS as f64).round() as usize]
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

/// Trilinear sample of a decoded chain at `uv` and source level `lod`, in LINEAR value space for
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
    let l = if lod.is_finite() {
        (lod - f64::from(chain.base)).clamp(0., top)
    } else {
        0.
    };
    let a = l.floor() as usize;
    let b = (a + 1).min(levels.len() - 1);
    let f = l - a as f64;
    let nearest = chain.base == 0 && l <= 0. && matches!(sampler.mag_filter, 9728);
    let fetch = |level: &mips::Level| -> [f64; 4] {
        let x = f64::from(uv[0]) * f64::from(level.width) - 0.5;
        let y = f64::from(uv[1]) * f64::from(level.height) - 0.5;
        let lut = linear_lut();
        let get = |x: i64, y: i64| -> [f64; 4] {
            let i = (wrap(y, level.height, sampler.wrap_t) * level.width as usize
                + wrap(x, level.width, sampler.wrap_s))
                * 4;
            std::array::from_fn(|c| {
                let v = level.rgba[i + c];
                if class == mips::Class::Colour && c < 3 {
                    lut[v as usize]
                } else {
                    f64::from(v) / 255.
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
    fn trilinear_sampling_wraps_and_minifies_in_linear_light() {
        // A 2x2 black/white checker: level 1 is the linear-light mean, not sRGB 0.5.
        let rgba = [0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255].to_vec();
        let base = mips::Level {
            width: 2,
            height: 2,
            rgba,
        };
        let next = halve(&base, mips::Class::Colour);
        let chain = Chain {
            base: 0,
            levels: vec![base, next],
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
