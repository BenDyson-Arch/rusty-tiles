//! ID / class texture pages: `EXT_mesh_features` feature ID textures and
//! categorical `EXT_structural_metadata` property textures. A future annotate
//! pass requests one with [`bake`] and attaches a level to a leaf
//! (or a coarse tile) with [`IdTexture::append_level`].
//!
//! Contract:
//! - Any number (>= 1) of ID sets. Each set is 1..=4 little-endian bytes and is
//!   packed first-fit, in request order, into contiguous channels of RGBA8
//!   pages; a set never straddles two pages and pages are added as needed.
//! - Gutters and padding (zero support) hold the no-feature value: every
//!   channel 255, so each set reads `nullFeatureId = 2^(8*bytes) - 1` there.
//!   Supported texels must carry IDs below that value.
//! - Mips: support-weighted mode of the whole RGBA tuple over each 2x2
//!   footprint (`sparse_mips`, `Class::Id`), ties to the first row-major
//!   contributor. A level never holds a value absent from its source region,
//!   and sets packed on one page stay jointly consistent per texel (pages are
//!   reduced independently, so sets on different pages may not).
//! - A 4-byte set, or a fourth packed byte, lives in alpha. glTF images are
//!   not premultiplied, and consumers must read ID texels without
//!   premultiplying alpha.
//! - Encoding: PNG for every level in every profile. glTF carries KTX2 only
//!   through `KHR_texture_basisu`, whose ETC1S and UASTC are lossy block codecs
//!   that blend neighbouring IDs; uncompressed KTX2 is not a glTF image.
//! - Sampler: NEAREST magnification and NEAREST minification (no mipmapping,
//!   so a viewer never generates averaged mips), clamp to edge. Coarser levels
//!   are separate exact pages for coarse proxies, never runtime mips.
//! - Independent checks on every decoded level: level 0 equals the request
//!   exactly; zero-support regions read no-feature; every other texel equals a
//!   supported level-0 texel of its own source region; the emitted sampler is
//!   re-read and its filters emulated at off-centre UVs.
use super::{
    delivery,
    mips::{self, Class},
    prepared::{PreparedAtlas, PreparedPage},
    sparse_mips::{self, ID_NO_FEATURE},
};
use crate::{JobError, JobErrorKind};
use serde_json::{json, Value};

pub(crate) const NEAREST: u64 = 9728;
const CLAMP_TO_EDGE: u64 = 33071;

fn error(kind: JobErrorKind, message: impl Into<String>) -> JobError {
    JobError::new(kind, message)
}
fn invalid(message: impl Into<String>) -> JobError {
    error(JobErrorKind::InvalidRequest, message)
}
fn broken(message: impl Into<String>) -> JobError {
    error(JobErrorKind::InvalidState, message)
}
fn resource() -> JobError {
    error(
        JobErrorKind::ResourceLimit,
        "ID texture size/allocation exceeds resource limit",
    )
}

/// One feature-ID set (or categorical property) of `bytes` little-endian bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IdSet {
    pub bytes: u8,
}
impl IdSet {
    /// The set's `nullFeatureId`: all channel bits set.
    pub(crate) fn null(self) -> u32 {
        (((1u64) << (8 * u32::from(self.bytes))) - 1) as u32
    }
}

/// Where one set lives: page index and its contiguous channels, low byte first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SetLayout {
    pub page: usize,
    pub channels: Vec<u8>,
}

/// First-fit channel packing in request order; returns layouts and page count.
pub(crate) fn layout(sets: &[IdSet]) -> Result<(Vec<SetLayout>, usize), JobError> {
    if sets.is_empty() {
        return Err(invalid("ID texture needs at least one set"));
    }
    let mut used: Vec<u8> = Vec::new();
    let mut out = Vec::new();
    out.try_reserve_exact(sets.len()).map_err(|_| resource())?;
    for set in sets {
        if !(1..=4).contains(&set.bytes) {
            return Err(invalid("ID set width must be 1..=4 bytes"));
        }
        let page = match used.iter().position(|&u| u + set.bytes <= 4) {
            Some(page) => page,
            None => {
                used.try_reserve(1).map_err(|_| resource())?;
                used.push(0);
                used.len() - 1
            }
        };
        let first = used[page];
        used[page] += set.bytes;
        out.push(SetLayout {
            page,
            channels: (first..first + set.bytes).collect(),
        });
    }
    Ok((out, used.len()))
}

/// The sampler every ID texture uses.
pub(crate) fn sampler() -> Value {
    json!({"magFilter":NEAREST,"minFilter":NEAREST,"wrapS":CLAMP_TO_EDGE,"wrapT":CLAMP_TO_EDGE})
}

/// Refuses any sampler that could blend IDs: filters must be explicit NEAREST
/// (glTF leaves absent filters to the viewer, which usually picks LINEAR), and
/// mipmapped minification would sample viewer-generated, averaged mips.
pub(crate) fn check_sampler(sampler: &Value) -> Result<(), JobError> {
    for field in ["magFilter", "minFilter"] {
        if sampler.get(field).and_then(Value::as_u64) != Some(NEAREST) {
            return Err(invalid(format!(
                "ID texture sampler {field} must be NEAREST (9728), got {}",
                sampler.get(field).unwrap_or(&Value::Null)
            )));
        }
    }
    Ok(())
}

/// Reads `bytes` little-endian channels of one RGBA8 texel.
fn read(pixel: &[u8], channels: &[u8]) -> u32 {
    channels
        .iter()
        .rev()
        .fold(0u32, |v, &c| (v << 8) | u32::from(pixel[usize::from(c)]))
}

/// Request: `ids` holds `sets.len()` IDs per texel, texel-major, row-major.
/// Pages are power-of-two, at least 4 texels per edge (as every H8 page).
/// IDs of zero-support texels are ignored (written as no-feature).
pub(crate) struct Request<'a> {
    pub width: u32,
    pub height: u32,
    pub sets: &'a [IdSet],
    pub ids: &'a [u32],
    pub support: &'a [u32],
    pub memory_limit: u64,
}

#[derive(Debug)]
pub(crate) struct IdPage {
    /// Exact PNG per authored level, level 0 first.
    pub levels: Vec<Vec<u8>>,
    /// Level dimensions, level 0 first.
    pub sizes: Vec<[u32; 2]>,
}
#[derive(Debug)]
pub(crate) struct IdTexture {
    pub sets: Vec<IdSet>,
    pub layouts: Vec<SetLayout>,
    pub pages: Vec<IdPage>,
    /// Distinct non-null IDs per set per level (`featureCount`).
    pub feature_counts: Vec<Vec<u32>>,
    pub receipts: Vec<Value>,
}

/// One attached set: glTF texture index plus its read rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TextureRef {
    pub texture: usize,
    pub channels: Vec<u8>,
    pub null_feature_id: u32,
    pub feature_count: u32,
}
impl TextureRef {
    /// `EXT_mesh_features.featureIds[]` entry of a feature ID texture; `None`
    /// when the level holds no feature (the spec requires `featureCount >= 1`).
    pub(crate) fn feature_id(&self, tex_coord: usize, label: &str) -> Option<Value> {
        (self.feature_count > 0).then(|| {
            json!({"featureCount":self.feature_count,"nullFeatureId":self.null_feature_id,"label":label,
                "texture":{"index":self.texture,"texCoord":tex_coord,"channels":self.channels}})
        })
    }
    /// `EXT_structural_metadata` property texture property (categorical).
    pub(crate) fn property(&self, tex_coord: usize) -> Value {
        json!({"index":self.texture,"texCoord":tex_coord,"channels":self.channels})
    }
}

impl IdTexture {
    /// Attaches `level` of every page to `atlas` with the NEAREST sampler and
    /// returns one reference per set, in request order. Leaves take level 0;
    /// coarse proxies may take a coarser exact level.
    pub(super) fn append_level(
        &self,
        atlas: &mut PreparedAtlas,
        level: usize,
    ) -> Result<Vec<TextureRef>, JobError> {
        let wanted = sampler();
        let sampler = match atlas.samplers.iter().position(|s| *s == wanted) {
            Some(i) => i,
            None => {
                atlas.samplers.push(wanted);
                atlas.samplers.len() - 1
            }
        };
        let mut textures = Vec::new();
        for page in &self.pages {
            let bytes = page
                .levels
                .get(level)
                .ok_or_else(|| invalid("ID texture level out of range"))?;
            let mut copy = Vec::new();
            copy.try_reserve_exact(bytes.len())
                .map_err(|_| resource())?;
            copy.extend_from_slice(bytes);
            atlas.pages.push(PreparedPage {
                bytes: copy,
                mime: "image/png",
            });
            atlas
                .textures
                .push(json!({"source":atlas.pages.len() - 1,"sampler":sampler}));
            textures.push(atlas.textures.len() - 1);
        }
        Ok(self
            .layouts
            .iter()
            .zip(&self.sets)
            .zip(&self.feature_counts)
            .map(|((l, s), counts)| TextureRef {
                texture: textures[l.page],
                channels: l.channels.clone(),
                null_feature_id: s.null(),
                feature_count: counts[level],
            })
            .collect())
    }
}

/// Independent check of one decoded level against the level-0 request page:
/// zero-support source regions must read no-feature; any other texel must
/// equal a supported level-0 texel inside its own source region.
pub(crate) fn check_level(
    base: &[u8],
    support: &[u32],
    base_size: [u32; 2],
    level: &mips::Level,
) -> Result<(), JobError> {
    let [bw, bh] = base_size.map(|v| v as usize);
    let (w, h) = (level.width as usize, level.height as usize);
    if w == 0 || h == 0 || bw % w != 0 || bh % h != 0 || level.rgba.len() != w * h * 4 {
        return Err(broken("ID level shape"));
    }
    let (fx, fy) = (bw / w, bh / h);
    for y in 0..h {
        for x in 0..w {
            let got = &level.rgba[(y * w + x) * 4..][..4];
            let mut supported = false;
            let mut present = false;
            for sy in y * fy..(y + 1) * fy {
                for sx in x * fx..(x + 1) * fx {
                    let i = sy * bw + sx;
                    if support[i] > 0 {
                        supported = true;
                        present |= &base[i * 4..i * 4 + 4] == got;
                    }
                }
            }
            if !supported && got != ID_NO_FEATURE {
                return Err(broken(format!(
                    "ID level {w}x{h} texel ({x},{y}) gutter is {got:?}, not no-feature"
                )));
            }
            if supported && !present {
                return Err(broken(format!(
                    "ID level {w}x{h} texel ({x},{y}) value {got:?} absent from its source region"
                )));
            }
        }
    }
    Ok(())
}

/// Emulates `sampler` on `level` (no mips: both filters read this level) at
/// three-quarter texel offsets, where LINEAR blends 3:1 with a neighbour.
/// Every sample must be one of its own 2x2 tap texels, unchanged.
pub(crate) fn check_sampled(level: &mips::Level, sampler: &Value) -> Result<(), JobError> {
    let (w, h) = (level.width as i64, level.height as i64);
    let texel = |x: i64, y: i64| -> &[u8] {
        let (x, y) = (x.clamp(0, w - 1), y.clamp(0, h - 1));
        &level.rgba[((y * w + x) * 4) as usize..][..4]
    };
    for field in ["magFilter", "minFilter"] {
        let linear = match sampler.get(field).and_then(Value::as_u64) {
            Some(NEAREST) => false,
            Some(9729) => true,
            other => {
                return Err(invalid(format!(
                    "ID sampler {field} {other:?} cannot be emulated without mips"
                )))
            }
        };
        for y in 0..h {
            for x in 0..w {
                let (u, v) = ((x as f64 + 0.75) / w as f64, (y as f64 + 0.75) / h as f64);
                let (px, py) = (u * w as f64, v * h as f64);
                let sample: [u8; 4] = if linear {
                    let (x0, y0) = ((px - 0.5).floor(), (py - 0.5).floor());
                    let (ax, ay) = (px - 0.5 - x0, py - 0.5 - y0);
                    let (x0, y0) = (x0 as i64, y0 as i64);
                    std::array::from_fn(|c| {
                        let t = |x, y| f64::from(texel(x, y)[c]);
                        let top = t(x0, y0) * (1. - ax) + t(x0 + 1, y0) * ax;
                        let bottom = t(x0, y0 + 1) * (1. - ax) + t(x0 + 1, y0 + 1) * ax;
                        (top * (1. - ay) + bottom * ay).round() as u8
                    })
                } else {
                    texel(px.floor() as i64, py.floor() as i64)
                        .try_into()
                        .expect("RGBA8")
                };
                let (tx, ty) = ((px - 0.5).floor() as i64, (py - 0.5).floor() as i64);
                let taps = [(tx, ty), (tx + 1, ty), (tx, ty + 1), (tx + 1, ty + 1)];
                if !taps.iter().any(|&(a, b)| texel(a, b) == sample) {
                    return Err(broken(format!(
                        "ID sampler {field} blends texel ({x},{y}) into {sample:?}"
                    )));
                }
            }
        }
    }
    Ok(())
}

fn distinct(level: &sparse_mips::Level, channels: &[u8], null: u32) -> Result<u32, JobError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(level.support.len())
        .map_err(|_| resource())?;
    for (p, &s) in level.rgba.as_chunks::<4>().0.iter().zip(&level.support) {
        let v = read(p, channels);
        if s > 0 && v != null {
            values.push(v);
        }
    }
    values.sort_unstable();
    values.dedup();
    u32::try_from(values.len()).map_err(|_| resource())
}

/// Bakes, mips, encodes and independently checks every page of one request.
pub(crate) fn bake(request: &Request<'_>) -> Result<IdTexture, JobError> {
    let (layouts, page_count) = layout(request.sets)?;
    let texels = (request.width as usize)
        .checked_mul(request.height as usize)
        .ok_or_else(resource)?;
    if request.support.len() != texels
        || Some(request.ids.len()) != texels.checked_mul(request.sets.len())
    {
        return Err(invalid("ID texture ids/support shape mismatch"));
    }
    let sampler_json = sampler();
    check_sampler(&sampler_json)?;
    let mut pages = Vec::new();
    let mut feature_counts = vec![Vec::new(); request.sets.len()];
    let mut receipts = Vec::new();
    for page in 0..page_count {
        let mut rgba = Vec::new();
        rgba.try_reserve_exact(texels.checked_mul(4).ok_or_else(resource)?)
            .map_err(|_| resource())?;
        rgba.resize(texels * 4, 255);
        for (t, &s) in request.support.iter().enumerate() {
            if s == 0 {
                continue;
            }
            for (k, (set, l)) in request.sets.iter().zip(&layouts).enumerate() {
                if l.page != page {
                    continue;
                }
                let id = request.ids[t * request.sets.len() + k];
                if id >= set.null() {
                    return Err(invalid(format!(
                        "ID set {k} value {id} at texel {t} is not below its null {}",
                        set.null()
                    )));
                }
                for (b, &c) in l.channels.iter().enumerate() {
                    rgba[t * 4 + usize::from(c)] = (id >> (8 * b)) as u8;
                }
            }
        }
        let chain = sparse_mips::build(
            request.width,
            request.height,
            &rgba,
            request.support,
            Class::Id,
            request.memory_limit,
        )?;
        for (k, l) in layouts.iter().enumerate() {
            if l.page == page {
                for level in &chain.levels {
                    let null = request.sets[k].null();
                    feature_counts[k].push(distinct(level, &l.channels, null)?);
                }
            }
        }
        let base_size = [request.width, request.height];
        let delivered = delivery::encode_id_chain(
            &chain,
            request.memory_limit,
            |index, decoded| {
                if index == 0 && decoded.rgba != rgba {
                    return Err(broken("decoded ID level 0 differs from the request"));
                }
                check_level(&rgba, request.support, base_size, decoded)?;
                if index == 0 {
                    check_sampled(decoded, &sampler_json)?;
                }
                Ok(
                    json!({"page":page,"mip":index,"exact":index == 0,"idsInSourceRegion":true,"gutterNoFeature":true}),
                )
            },
        )?;
        receipts.extend(delivered.receipts);
        pages.push(IdPage {
            levels: delivered.levels,
            sizes: chain.levels.iter().map(|l| [l.width, l.height]).collect(),
        });
    }
    receipts.push(json!({"class":"id","sets":request.sets.len(),"pages":page_count,"sampler":sampler_json,"mips":"support-weighted mode, separate exact PNG pages","encoding":"png"}));
    Ok(IdTexture {
        sets: request.sets.to_vec(),
        layouts,
        pages,
        feature_counts,
        receipts,
    })
}

#[cfg(test)]
#[path = "id_texture_tests.rs"]
mod tests;
