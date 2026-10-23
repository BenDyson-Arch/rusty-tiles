//! Cheap coarse appearance bake (D5): every covered texel (and, on the vertex-colour path,
//! every coarse vertex) takes the nearest point of the CAPTURED SOURCE surface within the
//! tile's descendant range (S2's bounded [`SourceQuery`]: distance then source-rank ties,
//! no facing or material restriction) and samples the source there. No per-texel ledger,
//! drift gate, facing or donor search.
//!
//! Hits record the donor's interpolants; images are then sampled grouped by image through
//! a bounded, shared cache of decoded colour-correct mip chains ([`ImageCache`]), so a
//! tile decodes each image at most once and the run never holds every image.
use super::source_index::SourceQuery;
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

/// Resolve every query point of the tile against the captured source surface of its
/// descendant range through S2's bounded [`SourceQuery`] (nearest point, distance then
/// source-rank ties), reading each donor's full record once per run of equal donors.
/// Returns the number of donor records read.
pub(crate) fn resolve(
    query: &mut SourceQuery<'_>,
    queries: &mut [Queries],
    check: &dyn Fn() -> Result<(), JobError>,
) -> Result<u64, JobError> {
    let mut donor: Option<FullTriangle> = None;
    let mut donor_rank = u64::MAX;
    let mut reads = 0u64;
    for q in queries.iter_mut() {
        for i in 0..q.points.len() {
            if i % 4096 == 0 {
                check()?;
            }
            let p = q.points[i].map(f64::from);
            let hit = query.nearest(p)?;
            if hit.rank != donor_rank {
                donor = Some(query.triangle(hit)?);
                donor_rank = hit.rank;
                reads += 1;
            }
            let t = donor.as_ref().ok_or_else(|| state("coarse bake donor"))?;
            let w = hit.bary.map(|v| v as f32);
            let lerp2 = |v: [[f32; 2]; 3]| -> [f32; 2] {
                std::array::from_fn(|a| v[0][a] * w[0] + v[1][a] * w[1] + v[2][a] * w[2])
            };
            let color = if t.flags & COLOR != 0 {
                std::array::from_fn(|a| {
                    (0..3).map(|k| t.corners[k].color[a] * w[k]).sum::<f32>()
                })
            } else {
                [1.; 4]
            };
            q.hits[i] = Hit {
                d2: hit.distance2 as f32,
                rank: hit.rank,
                material: t.material,
                flags: t.flags,
                uv: [0, 1].map(|s| lerp2(t.corners.map(|c| c.texcoords[s]))),
                gradient: [gradient(t, 0), gradient(t, 1)],
                color,
            };
            if q.want_frames {
                q.frames[i] = Frame {
                    normal: std::array::from_fn(|a| (0..3).map(|k| t.corners[k].normal[a] * w[k]).sum()),
                    tangent: {
                        let mut v: [f32; 4] = std::array::from_fn(|a| {
                            (0..3).map(|k| t.corners[k].tangent[a] * w[k]).sum()
                        });
                        v[3] = if t.corners[0].tangent[3] < 0. { -1. } else { 1. };
                        v
                    },
                };
            }
        }
    }
    Ok(reads)
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
