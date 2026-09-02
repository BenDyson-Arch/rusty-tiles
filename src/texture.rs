//! Textures.
//!
//! Leaves: cut only the **used UV charts** out of each source atlas at source
//! resolution and pack them into one atlas per leaf (one primitive, one
//! texture). Parents: chart-unwrap the simplified proxy and bake colour from
//! the children's atlases through a dense-grid nearest-surface sampler,
//! after downsampling each child atlas to the parent's texel density.

use std::collections::{HashMap, VecDeque};
use std::io::Cursor;
use std::time::Instant;

use image::imageops::{self, FilterType};
use image::{DynamicImage, RgbImage, RgbaImage};

use crate::error::Error;
use crate::glb_write::TilePrimitive;
use crate::grid::{self, TriGrid};
use crate::hlod::{Timing, TIMING};
use crate::mesh::EncodedImage;

/// Gutter around each leaf chart in atlas pixels (clean through ~2 mips).
pub const CHART_GUTTER_PX: u32 = 4;
/// Parent atlas: how far baked colour is dilated into uncovered texels.
const PARENT_DILATE_PX: u32 = 16;
const MIN_ATLAS: u32 = 64;

// ---------------------------------------------------------------------------
// Source image access
// ---------------------------------------------------------------------------

/// Pixel dimensions without decoding pixels.
pub fn image_dimensions(encoded: &EncodedImage) -> Result<(u32, u32), Error> {
    let head = encoded.load_prefix(1 << 20)?;
    if let Some(d) = jpeg_dims(&head) {
        return Ok(d);
    }
    let bytes = encoded.load()?;
    if let Some(d) = jpeg_dims(&bytes) {
        return Ok(d);
    }
    let r = image::ImageReader::new(Cursor::new(&bytes[..])).with_guessed_format()?;
    Ok(r.into_dimensions()?)
}

fn jpeg_dims(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut dec = jpeg_decoder::Decoder::new(Cursor::new(bytes));
    dec.read_info().ok()?;
    let info = dec.info()?;
    Some((info.width as u32, info.height as u32))
}

/// Decode to RGB, letting the JPEG IDCT downscale (1/2, 1/4, 1/8) when the
/// consumer needs at most `max_w × max_h` — an 8K atlas whose charts all end
/// up at ≤ 1/4 scale decodes in 1/16 the memory.
pub fn decode_rgb_max(encoded: &EncodedImage, max_w: u32, max_h: u32) -> Result<RgbImage, Error> {
    let bytes = encoded.load()?;
    if let Ok(img) = decode_jpeg_scaled(&bytes, max_w, max_h) {
        return Ok(img);
    }
    let dynimg = image::load_from_memory(&bytes)?;
    let mut rgb = dynimg.to_rgb8();
    if rgb.width() > max_w.max(1) || rgb.height() > max_h.max(1) {
        let s = (max_w as f32 / rgb.width() as f32).min(max_h as f32 / rgb.height() as f32);
        let nw = ((rgb.width() as f32) * s).round().max(1.0) as u32;
        let nh = ((rgb.height() as f32) * s).round().max(1.0) as u32;
        rgb = imageops::resize(&rgb, nw, nh, FilterType::Triangle);
    }
    Ok(rgb)
}

fn decode_jpeg_scaled(bytes: &[u8], max_w: u32, max_h: u32) -> Result<RgbImage, Error> {
    let mut dec = jpeg_decoder::Decoder::new(Cursor::new(bytes));
    dec.read_info()
        .map_err(|e| Error::msg(format!("jpeg: {e}")))?;
    let (w, h) = dec
        .scale(
            max_w.clamp(1, u16::MAX as u32) as u16,
            max_h.clamp(1, u16::MAX as u32) as u16,
        )
        .map_err(|e| Error::msg(format!("jpeg scale: {e}")))?;
    let pixels = dec
        .decode()
        .map_err(|e| Error::msg(format!("jpeg decode: {e}")))?;
    let info = dec.info().ok_or_else(|| Error::msg("jpeg lost header"))?;
    let (w, h) = (w as u32, h as u32);
    match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => {
            RgbImage::from_raw(w, h, pixels).ok_or_else(|| Error::msg("jpeg RGB buffer size"))
        }
        jpeg_decoder::PixelFormat::L8 => {
            let gray = image::GrayImage::from_raw(w, h, pixels)
                .ok_or_else(|| Error::msg("jpeg L buffer size"))?;
            Ok(DynamicImage::ImageLuma8(gray).to_rgb8())
        }
        _ => Err(Error::msg("jpeg pixel format not RGB/L")),
    }
}

// ---------------------------------------------------------------------------
// Leaf atlas planning
// ---------------------------------------------------------------------------

/// Clipped leaf geometry that shares one source image (UVs in source 0–1).
pub struct LeafGroup {
    pub image: Option<u32>,
    pub prim: TilePrimitive,
}

/// Copy source region `src` (normalized, may exceed 0–1: clamp) into atlas
/// rect `dst` (x, y, w, h in atlas pixels), resampling.
#[derive(Clone, Debug)]
pub struct Blit {
    pub image: u32,
    pub src: [f32; 4],
    pub dst: [u32; 4],
}

pub struct LeafPlan {
    /// All textured groups merged; UVs remapped into the leaf atlas.
    pub textured: Option<TilePrimitive>,
    pub untextured: Option<TilePrimitive>,
    /// Atlas width × height; power-of-two, height is the width or half of it.
    pub atlas_wh: (u32, u32),
    pub blits: Vec<Blit>,
    /// Source pixels → atlas pixels at the chosen scale (≤ 1).
    pub scale: f32,
    /// Charts whose source rect is ≤ 8 px on its long side.
    pub tiny_charts: usize,
    /// Σ chart texels (without gutter) / atlas texels.
    pub fill: f32,
}

struct Chart {
    image: u32,
    /// Source pixel rect without gutter.
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
    /// Per-chart shrink so its texel density does not exceed the cap (≤ 1).
    pre: f32,
    /// Σ 3-D triangle area and Σ source texel area, for the density.
    area_m2: f64,
    texels: f64,
}

impl Chart {
    fn scaled(&self, scale: f32) -> (u32, u32) {
        let s = scale * self.pre;
        let w = (((self.x1 - self.x0) as f32 * s).ceil() as u32).max(1);
        let h = (((self.y1 - self.y0) as f32 * s).ceil() as u32).max(1);
        (w, h)
    }
}

/// Plan one atlas for a leaf. Chart = UV-connected component of the *used*
/// triangles, rect = its UV AABB in source pixels. Overlapping rects on the
/// same image merge so a texel is copied once. Charts denser than
/// `max_density` px/m are pre-shrunk to it (0 = no cap).
pub fn plan_leaf_atlas(
    groups: Vec<LeafGroup>,
    image_dims: &[(u32, u32)],
    tile_size: u32,
    max_density: f64,
) -> Result<LeafPlan, Error> {
    let mut untextured: Vec<TilePrimitive> = Vec::new();
    let mut textured: Vec<(u32, TilePrimitive)> = Vec::new();
    for g in groups {
        match g.image {
            Some(i)
                if (i as usize) < image_dims.len()
                    && g.prim.uvs.len() == g.prim.positions.len() =>
            {
                textured.push((i, g.prim))
            }
            _ => {
                let mut p = g.prim;
                p.uvs.clear();
                p.jpeg = None;
                untextured.push(p);
            }
        }
    }
    let untextured = if untextured.is_empty() {
        None
    } else {
        Some(concat_prims(&untextured))
    };
    if textured.is_empty() {
        return Ok(LeafPlan {
            textured: None,
            untextured,
            atlas_wh: (0, 0),
            blits: Vec::new(),
            scale: 1.0,
            tiny_charts: 0,
            fill: 0.0,
        });
    }

    // Charts per group: union-find over vertex indices through triangles.
    let mut charts: Vec<Chart> = Vec::new();
    let mut chart_of: Vec<Vec<u32>> = Vec::with_capacity(textured.len());
    for (img, prim) in &textured {
        let (w, h) = image_dims[*img as usize];
        let n = prim.positions.len();
        let mut uf = UnionFind::new(n);
        for t in prim.indices.chunks_exact(3) {
            uf.union(t[0] as usize, t[1] as usize);
            uf.union(t[0] as usize, t[2] as usize);
        }
        let mut comp_chart: HashMap<usize, u32> = HashMap::new();
        let mut per_vertex = vec![u32::MAX; n];
        let mut used = vec![false; n];
        for &vi in &prim.indices {
            used[vi as usize] = true;
        }
        for vi in 0..n {
            if !used[vi] {
                continue;
            }
            let root = uf.find(vi);
            let uv = prim.uvs[vi];
            let px = (uv[0].clamp(0.0, 1.0) * w as f32) as f64;
            let py = (uv[1].clamp(0.0, 1.0) * h as f32) as f64;
            let ci = *comp_chart.entry(root).or_insert_with(|| {
                charts.push(Chart {
                    image: *img,
                    x0: px.floor() as i64,
                    y0: py.floor() as i64,
                    x1: px.ceil() as i64,
                    y1: py.ceil() as i64,
                    pre: 1.0,
                    area_m2: 0.0,
                    texels: 0.0,
                });
                (charts.len() - 1) as u32
            });
            let c = &mut charts[ci as usize];
            c.x0 = c.x0.min(px.floor() as i64);
            c.y0 = c.y0.min(py.floor() as i64);
            c.x1 = c.x1.max(px.ceil() as i64);
            c.y1 = c.y1.max(py.ceil() as i64);
            per_vertex[vi] = ci;
        }
        for t in prim.indices.chunks_exact(3) {
            let ci = per_vertex[t[0] as usize];
            if ci == u32::MAX {
                continue;
            }
            let p = [
                prim.positions[t[0] as usize],
                prim.positions[t[1] as usize],
                prim.positions[t[2] as usize],
            ];
            let uv = [
                prim.uvs[t[0] as usize],
                prim.uvs[t[1] as usize],
                prim.uvs[t[2] as usize],
            ];
            let c = &mut charts[ci as usize];
            c.area_m2 += grid::face_normal_area(p[0], p[1], p[2]).1 as f64;
            let (ua, ub, uc) = (
                [uv[0][0] * w as f32, uv[0][1] * h as f32],
                [uv[1][0] * w as f32, uv[1][1] * h as f32],
                [uv[2][0] * w as f32, uv[2][1] * h as f32],
            );
            c.texels += 0.5
                * ((ub[0] - ua[0]) * (uc[1] - ua[1]) - (uc[0] - ua[0]) * (ub[1] - ua[1])).abs()
                    as f64;
        }
        chart_of.push(per_vertex);
    }
    for c in &mut charts {
        c.x1 = c.x1.max(c.x0 + 1);
        c.y1 = c.y1.max(c.y0 + 1);
    }

    // Merge overlapping rects on the same image when the union wastes little.
    let remap = merge_overlapping(&mut charts);
    for pv in &mut chart_of {
        for c in pv.iter_mut() {
            if *c != u32::MAX {
                *c = remap[*c as usize];
            }
        }
    }
    let live: Vec<usize> = (0..charts.len())
        .filter(|&i| remap[i] as usize == i)
        .collect();
    if max_density > 0.0 {
        for &i in &live {
            let c = &mut charts[i];
            if c.area_m2 > 0.0 && c.texels > 0.0 {
                let density = (c.texels / c.area_m2).sqrt();
                c.pre = (max_density / density).min(1.0) as f32;
            }
        }
    }

    // Atlas edge: smallest pow2 that would hold everything at source scale.
    let padded_area = |s: f32| -> f32 {
        live.iter()
            .map(|&i| {
                let (w, h) = charts[i].scaled(s);
                let g = 2.0 * gutter_for(w, h) as f32;
                (w as f32 + g) * (h as f32 + g)
            })
            .sum()
    };
    let area1 = padded_area(1.0);
    // Candidate atlases in ascending area: square, then 2:1, then the next
    // square… A 2:1 atlas halves the dead space for leaves that need just
    // over half a square.
    let max_edge = tile_size.max(MIN_ATLAS);
    let mut sizes: Vec<(u32, u32)> = Vec::new();
    let mut e = MIN_ATLAS;
    loop {
        sizes.push((e, e));
        if e >= max_edge {
            break;
        }
        sizes.push((e * 2, e));
        e *= 2;
    }
    let first = sizes
        .iter()
        .position(|&(w, h)| (w * h) as f32 >= area1 * 1.1)
        .unwrap_or(sizes.len() - 1);
    let mut atlas = sizes[first];
    let mut packed: Option<(Vec<[u32; 4]>, Vec<u32>)> = None;
    let mut scale = 1.0f32;
    // Source scale into the smallest candidate that takes it…
    for &wh in &sizes[first..] {
        if let Some(p) = pack_charts(&charts, &live, 1.0, wh) {
            atlas = wh;
            packed = Some(p);
            break;
        }
        atlas = wh;
    }
    // …else the largest, backing scale off gently so little resolution is
    // left on the table.
    if packed.is_none() {
        let cap = (atlas.0 * atlas.1) as f32;
        scale = (cap * 0.92 / area1).sqrt().min(1.0);
        for _ in 0..96 {
            if let Some(p) = pack_charts(&charts, &live, scale, atlas) {
                packed = Some(p);
                break;
            }
            // Gutters do not shrink with scale, so re-derive from padded area.
            let implied = (cap * 0.92 / padded_area(scale)).sqrt() * scale;
            scale = implied.min(scale * 0.96);
        }
    }
    let cap = (atlas.0 * atlas.1) as f32;
    let Some((rects, gutters)) = packed else {
        return Err(Error::msg("leaf charts do not fit the atlas"));
    };
    let chart_texels: f32 = rects
        .iter()
        .zip(&gutters)
        .map(|(r, &g)| (r[2] - 2 * g) as f32 * (r[3] - 2 * g) as f32)
        .sum();
    let tiny_charts = live
        .iter()
        .filter(|&&i| (charts[i].x1 - charts[i].x0).max(charts[i].y1 - charts[i].y0) <= 8)
        .count();

    // Blits + UV remap.
    let mut blits = Vec::with_capacity(live.len());
    let mut chart_dst: HashMap<u32, (usize, [f32; 4])> = HashMap::new();
    for (k, &ci) in live.iter().enumerate() {
        let c = &charts[ci];
        let (w, h) = image_dims[c.image as usize];
        let gs = (gutters[k] as f32 / (scale * c.pre)).ceil() as i64;
        let src = [
            (c.x0 - gs) as f32 / w as f32,
            (c.y0 - gs) as f32 / h as f32,
            (c.x1 + gs) as f32 / w as f32,
            (c.y1 + gs) as f32 / h as f32,
        ];
        blits.push(Blit {
            image: c.image,
            src,
            dst: rects[k],
        });
        chart_dst.insert(ci as u32, (k, src));
    }
    let (aw, ah) = (atlas.0 as f32, atlas.1 as f32);
    let mut merged = TilePrimitive::default();
    let mut any_n = false;
    for (gi, (_, prim)) in textured.iter().enumerate() {
        let base = merged.positions.len() as u32;
        let has_n = prim.normals.len() == prim.positions.len();
        any_n |= has_n;
        for (vi, (&pos, &uv)) in prim.positions.iter().zip(&prim.uvs).enumerate() {
            merged.positions.push(pos);
            merged.normals.push(if has_n {
                prim.normals[vi]
            } else {
                [0.0, 1.0, 0.0]
            });
            let ci = chart_of[gi][vi];
            let out = if ci == u32::MAX {
                [0.0, 0.0]
            } else {
                let (k, src) = chart_dst[&ci];
                let d = rects[k];
                let fu = ((uv[0].clamp(0.0, 1.0) - src[0]) / (src[2] - src[0]).max(1e-9))
                    .clamp(0.0, 1.0);
                let fv = ((uv[1].clamp(0.0, 1.0) - src[1]) / (src[3] - src[1]).max(1e-9))
                    .clamp(0.0, 1.0);
                [
                    (d[0] as f32 + fu * d[2] as f32) / aw,
                    (d[1] as f32 + fv * d[3] as f32) / ah,
                ]
            };
            merged.uvs.push(out);
        }
        merged.indices.extend(prim.indices.iter().map(|i| i + base));
    }
    if !any_n {
        merged.normals.clear();
    }
    Ok(LeafPlan {
        textured: Some(merged),
        untextured,
        atlas_wh: atlas,
        blits,
        scale,
        tiny_charts,
        fill: chart_texels / cap,
    })
}

fn concat_prims(prims: &[TilePrimitive]) -> TilePrimitive {
    let mut out = TilePrimitive::default();
    let mut any_n = false;
    for p in prims {
        let base = out.positions.len() as u32;
        let has_n = p.normals.len() == p.positions.len();
        any_n |= has_n;
        for i in 0..p.positions.len() {
            out.positions.push(p.positions[i]);
            out.normals
                .push(if has_n { p.normals[i] } else { [0.0, 1.0, 0.0] });
        }
        out.indices.extend(p.indices.iter().map(|i| i + base));
    }
    if !any_n {
        out.normals.clear();
    }
    out
}

struct UnionFind {
    parent: Vec<u32>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n as u32).collect(),
        }
    }
    fn find(&mut self, mut i: usize) -> usize {
        while self.parent[i] as usize != i {
            let p = self.parent[i] as usize;
            self.parent[i] = self.parent[p];
            i = p;
        }
        i
    }
    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent[ra.max(rb)] = ra.min(rb) as u32;
        }
    }
}

/// Merge intersecting rects on the same image while the union AABB does not
/// grow more than 25 % over the pair. Returns old chart → surviving chart.
fn merge_overlapping(charts: &mut [Chart]) -> Vec<u32> {
    let n = charts.len();
    let mut alive: Vec<bool> = vec![true; n];
    let mut remap: Vec<u32> = (0..n as u32).collect();
    let mut changed = true;
    let mut rounds = 0;
    while changed && rounds < 16 {
        changed = false;
        rounds += 1;
        for i in 0..n {
            if !alive[i] {
                continue;
            }
            for j in (i + 1)..n {
                if !alive[j] || charts[i].image != charts[j].image {
                    continue;
                }
                let (a, b) = (&charts[i], &charts[j]);
                let ix = a.x0.max(b.x0)..a.x1.min(b.x1);
                let iy = a.y0.max(b.y0)..a.y1.min(b.y1);
                if ix.is_empty() || iy.is_empty() {
                    continue;
                }
                let inter = (ix.end - ix.start) * (iy.end - iy.start);
                let area_a = (a.x1 - a.x0) * (a.y1 - a.y0);
                let area_b = (b.x1 - b.x0) * (b.y1 - b.y0);
                let ux0 = a.x0.min(b.x0);
                let uy0 = a.y0.min(b.y0);
                let ux1 = a.x1.max(b.x1);
                let uy1 = a.y1.max(b.y1);
                let union_area = (ux1 - ux0) * (uy1 - uy0);
                let covered = area_a + area_b - inter;
                if union_area as f64 <= covered as f64 * 1.25 {
                    let (area_j, texels_j) = (charts[j].area_m2, charts[j].texels);
                    charts[i].x0 = ux0;
                    charts[i].y0 = uy0;
                    charts[i].x1 = ux1;
                    charts[i].y1 = uy1;
                    charts[i].area_m2 += area_j;
                    charts[i].texels += texels_j;
                    alive[j] = false;
                    remap[j] = i as u32;
                    changed = true;
                }
            }
        }
    }
    // Resolve chains j → i → k.
    for k in 0..n {
        let mut r = remap[k];
        while remap[r as usize] != r {
            r = remap[r as usize];
        }
        remap[k] = r;
    }
    remap
}

/// Gutter for a chart of this size in atlas pixels. Tiny charts get 2 px:
/// a 1000-chart photogrammetry leaf would otherwise spend a third of its
/// atlas on gutters (and the fill colour they bleed into is the mean anyway).
fn gutter_for(w: u32, h: u32) -> u32 {
    if w.min(h) >= 32 {
        CHART_GUTTER_PX
    } else {
        2
    }
}

/// Pack gutter-padded chart rects at `scale`; `None` if they do not fit.
/// Returns (x, y, w, h) including gutter, plus each chart's gutter.
fn pack_charts(
    charts: &[Chart],
    live: &[usize],
    scale: f32,
    (aw, ah): (u32, u32),
) -> Option<(Vec<[u32; 4]>, Vec<u32>)> {
    let mut gutters = Vec::with_capacity(live.len());
    let dims: Vec<(u32, u32)> = live
        .iter()
        .map(|&i| {
            let (w, h) = charts[i].scaled(scale);
            let g = gutter_for(w, h);
            gutters.push(g);
            (w + 2 * g, h + 2 * g)
        })
        .collect();
    let placed = pack_rects_wh(&dims, aw, ah)?;
    Some((
        placed
            .iter()
            .zip(&dims)
            .map(|(&(x, y), &(w, h))| [x, y, w, h])
            .collect(),
        gutters,
    ))
}

/// Skyline bottom-left rectangle packing into an `atlas × atlas` square.
pub fn pack_rects(dims: &[(u32, u32)], atlas: u32) -> Option<Vec<(u32, u32)>> {
    pack_rects_wh(dims, atlas, atlas)
}

/// Skyline bottom-left rectangle packing into an `aw × ah` atlas.
/// Rects are placed tallest-first at the lowest (then leftmost) skyline
/// position that fits. ~85–90 % fill on chart-like inputs vs ~60 % for
/// shelf packing.
pub fn pack_rects_wh(dims: &[(u32, u32)], aw: u32, ah: u32) -> Option<Vec<(u32, u32)>> {
    if dims.iter().any(|&(w, h)| w > aw || h > ah) {
        return None;
    }
    let area: u64 = dims.iter().map(|&(w, h)| w as u64 * h as u64).sum();
    if area > aw as u64 * ah as u64 {
        return None;
    }
    let mut order: Vec<usize> = (0..dims.len()).collect();
    order.sort_by(|&a, &b| {
        dims[b]
            .1
            .cmp(&dims[a].1)
            .then(dims[b].0.cmp(&dims[a].0))
            .then(a.cmp(&b))
    });
    // Skyline segments (x, y, w), sorted by x, covering [0, aw).
    let mut sky: Vec<(u32, u32, u32)> = vec![(0, 0, aw)];
    let mut out = vec![(0u32, 0u32); dims.len()];
    for &k in &order {
        let (w, h) = dims[k];
        let mut best: Option<(u32, u32, usize)> = None;
        for i in 0..sky.len() {
            let x = sky[i].0;
            if x + w > aw {
                break;
            }
            // Height needed = max skyline y across [x, x + w).
            let mut y = 0u32;
            let mut span = 0u32;
            let mut j = i;
            while j < sky.len() && span < w {
                y = y.max(sky[j].1);
                span += sky[j].2;
                j += 1;
            }
            if y + h > ah {
                continue;
            }
            let better = match best {
                None => true,
                Some((_, by, bx)) => y < by || (y == by && x < sky[bx].0),
            };
            if better {
                best = Some((x, y, i));
            }
        }
        let (x, y, _) = best?;
        out[k] = (x, y);
        // Raise the skyline over [x, x + w) to y + h.
        let mut new_sky: Vec<(u32, u32, u32)> = Vec::with_capacity(sky.len() + 2);
        for &(sx, sy, sw) in &sky {
            let sx1 = sx + sw;
            if sx1 <= x || sx >= x + w {
                new_sky.push((sx, sy, sw));
                continue;
            }
            if sx < x {
                new_sky.push((sx, sy, x - sx));
            }
            if sx1 > x + w {
                new_sky.push((x + w, sy, sx1 - (x + w)));
            }
        }
        new_sky.push((x, y + h, w));
        new_sky.sort_by_key(|s| s.0);
        // Merge equal-height neighbours.
        let mut merged: Vec<(u32, u32, u32)> = Vec::with_capacity(new_sky.len());
        for s in new_sky {
            if let Some(last) = merged.last_mut() {
                if last.1 == s.1 && last.0 + last.2 == s.0 {
                    last.2 += s.2;
                    continue;
                }
            }
            merged.push(s);
        }
        sky = merged;
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Leaf atlas execution
// ---------------------------------------------------------------------------

/// Longest atlas-pixel edge any blit of `image` needs, in source pixels'
/// terms: the decode may downscale to this and lose nothing.
pub fn needed_decode_size(blits: &[&Blit], image_dims: (u32, u32)) -> (u32, u32) {
    let (w, h) = image_dims;
    let mut sx = 0.0f32;
    let mut sy = 0.0f32;
    for b in blits {
        let sw = ((b.src[2] - b.src[0]) * w as f32).max(1.0);
        let sh = ((b.src[3] - b.src[1]) * h as f32).max(1.0);
        sx = sx.max(b.dst[2] as f32 / sw);
        sy = sy.max(b.dst[3] as f32 / sh);
    }
    let s = sx.max(sy).clamp(0.0, 1.0);
    (
        ((w as f32 * s).ceil() as u32).clamp(1, w),
        ((h as f32 * s).ceil() as u32).clamp(1, h),
    )
}

/// Resample the source region into the atlas rect. Source coordinates outside
/// the image clamp to the edge, which is what a gutter wants.
pub fn blit_chart(atlas: &mut RgbImage, decoded: &RgbImage, b: &Blit) {
    paste_chart(atlas, &resample_chart(decoded, b), b);
}

/// Copy an already-resampled chart (`dst` sized) into the atlas rect.
pub fn paste_chart(atlas: &mut RgbImage, resized: &RgbImage, b: &Blit) {
    let [x, y, w, h] = b.dst;
    for py in 0..h.min(resized.height()) {
        for px in 0..w.min(resized.width()) {
            let ax = x + px;
            let ay = y + py;
            if ax < atlas.width() && ay < atlas.height() {
                atlas.put_pixel(ax, ay, *resized.get_pixel(px, py));
            }
        }
    }
}

/// The source region of `b`, resampled to its `dst` size (edge-clamped).
pub fn resample_chart(decoded: &RgbImage, b: &Blit) -> RgbImage {
    let (dw, dh) = (decoded.width() as f32, decoded.height() as f32);
    let sx0 = b.src[0] * dw;
    let sy0 = b.src[1] * dh;
    let sx1 = b.src[2] * dw;
    let sy1 = b.src[3] * dh;
    let cw = ((sx1 - sx0).round() as i64).max(1);
    let ch = ((sy1 - sy0).round() as i64).max(1);
    let ox = sx0.round() as i64;
    let oy = sy0.round() as i64;
    let maxx = decoded.width() as i64 - 1;
    let maxy = decoded.height() as i64 - 1;
    let mut crop = RgbImage::new(cw as u32, ch as u32);
    for y in 0..ch {
        let sy = (oy + y).clamp(0, maxy) as u32;
        for x in 0..cw {
            let sx = (ox + x).clamp(0, maxx) as u32;
            crop.put_pixel(x as u32, y as u32, *decoded.get_pixel(sx, sy));
        }
    }
    let [_, _, w, h] = b.dst;
    if crop.width() == w && crop.height() == h {
        crop
    } else {
        imageops::resize(&crop, w.max(1), h.max(1), FilterType::Triangle)
    }
}

/// Fill non-chart texels with the mean chart colour (keeps far mips sane) and
/// encode.
pub fn finish_leaf_atlas(mut atlas: RgbImage, blits: &[Blit]) -> Result<Vec<u8>, Error> {
    let (w, h) = (atlas.width(), atlas.height());
    let mut cover = vec![false; (w * h) as usize];
    let mut sum = [0u64; 3];
    let mut n = 0u64;
    for b in blits {
        let [x, y, bw, bh] = b.dst;
        for py in y..(y + bh).min(h) {
            for px in x..(x + bw).min(w) {
                cover[(py * w + px) as usize] = true;
                let p = atlas.get_pixel(px, py).0;
                sum[0] += p[0] as u64;
                sum[1] += p[1] as u64;
                sum[2] += p[2] as u64;
                n += 1;
            }
        }
    }
    let mean = match n {
        0 => image::Rgb([128, 128, 128]),
        n => image::Rgb([(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8]),
    };
    for y in 0..h {
        for x in 0..w {
            if !cover[(y * w + x) as usize] {
                atlas.put_pixel(x, y, mean);
            }
        }
    }
    encode_texture_rgb(&atlas)
}

// ---------------------------------------------------------------------------
// Parent bake
// ---------------------------------------------------------------------------

/// Texels per metre of a textured primitive (sqrt of UV-pixel area over 3D area).
pub fn texel_density(prim: &TilePrimitive, img_w: u32, img_h: u32) -> Option<f32> {
    if prim.uvs.len() != prim.positions.len() {
        return None;
    }
    let mut uv_area = 0.0f64;
    let mut area = 0.0f64;
    for t in prim.indices.chunks_exact(3) {
        let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
        let (_, ar) =
            grid::face_normal_area(prim.positions[a], prim.positions[b], prim.positions[c]);
        let ua = [prim.uvs[a][0] * img_w as f32, prim.uvs[a][1] * img_h as f32];
        let ub = [prim.uvs[b][0] * img_w as f32, prim.uvs[b][1] * img_h as f32];
        let uc = [prim.uvs[c][0] * img_w as f32, prim.uvs[c][1] * img_h as f32];
        let ua2 =
            0.5 * ((ub[0] - ua[0]) * (uc[1] - ua[1]) - (uc[0] - ua[0]) * (ub[1] - ua[1])).abs();
        uv_area += ua2 as f64;
        area += ar as f64;
    }
    if area <= 1e-12 {
        return None;
    }
    Some((uv_area / area).sqrt() as f32)
}

/// Chart-unwrap `simplified`, then bake each texel from the nearest child
/// surface. Returns the textured proxy and its texel size in metres.
pub fn bake_simplified(
    simplified: &TilePrimitive,
    children: &[TilePrimitive],
    atlas_size: u32,
) -> Result<(TilePrimitive, f64), Error> {
    let size = atlas_size.max(MIN_ATLAS);
    let t0 = Instant::now();
    let (unwrapped, (aw, ah)) = chart_unwrap(simplified, size)?;
    let density = texel_density(&unwrapped, aw, ah).unwrap_or(1.0);
    Timing::add(&TIMING.unwrap, t0);
    let t0 = Instant::now();
    let sampler = SceneSampler::from_prims(children, Some(density))?;
    Timing::add(&TIMING.sampler, t0);
    let t0 = Instant::now();
    let mut atlas = RgbaImage::from_pixel(aw, ah, image::Rgba([128, 128, 128, 255]));
    let denom_w = (aw - 1) as f32;
    let denom_h = (ah - 1).max(1) as f32;
    let has_n = unwrapped.normals.len() == unwrapped.positions.len();

    for tri in unwrapped.indices.chunks_exact(3) {
        let ia = tri[0] as usize;
        let ib = tri[1] as usize;
        let ic = tri[2] as usize;
        let pa = unwrapped.positions[ia];
        let pb = unwrapped.positions[ib];
        let pc = unwrapped.positions[ic];
        let ua = unwrapped.uvs[ia];
        let ub = unwrapped.uvs[ib];
        let uc = unwrapped.uvs[ic];
        let (face_n, _) = grid::face_normal_area(pa, pb, pc);
        let n_at = |i: usize| if has_n { unwrapped.normals[i] } else { face_n };
        let (na, nb, nc) = (n_at(ia), n_at(ib), n_at(ic));
        let mut seed: Option<u32> = None;

        let px = |t: [f32; 2]| -> (i32, i32) {
            (
                (t[0] * denom_w).floor() as i32,
                (t[1] * denom_h).floor() as i32,
            )
        };
        let (x0, y0) = px(ua);
        let (x1, y1) = px(ub);
        let (x2, y2) = px(uc);
        let min_x = x0.min(x1).min(x2).max(0);
        let max_x = (x0.max(x1).max(x2) + 1).min(aw as i32 - 1);
        let min_y = y0.min(y1).min(y2).max(0);
        let max_y = (y0.max(y1).max(y2) + 1).min(ah as i32 - 1);
        if min_x > max_x || min_y > max_y {
            continue;
        }
        for py in min_y..=max_y {
            for px in min_x..=max_x {
                let q = [px as f32 / denom_w, py as f32 / denom_h];
                let (b0, b1, b2) = bary_uv(ua, ub, uc, q);
                // Slightly generous so texel centres on shared edges get colour.
                if b0 < -2e-3 || b1 < -2e-3 || b2 < -2e-3 {
                    continue;
                }
                let pos = [
                    pa[0] * b0 + pb[0] * b1 + pc[0] * b2,
                    pa[1] * b0 + pb[1] * b1 + pc[1] * b2,
                    pa[2] * b0 + pb[2] * b1 + pc[2] * b2,
                ];
                let nrm = [
                    na[0] * b0 + nb[0] * b1 + nc[0] * b2,
                    na[1] * b0 + nb[1] * b1 + nc[1] * b2,
                    na[2] * b0 + nb[2] * b1 + nc[2] * b2,
                ];
                let rgb = sampler.sample_seeded(pos, nrm, &mut seed);
                atlas.put_pixel(
                    px as u32,
                    py as u32,
                    image::Rgba([rgb[0], rgb[1], rgb[2], 255]),
                );
            }
        }
    }

    Timing::add(&TIMING.raster, t0);
    let t0 = Instant::now();
    fill_uncovered(
        &mut atlas,
        &unwrapped.uvs,
        &unwrapped.indices,
        PARENT_DILATE_PX,
    );
    let texel_m = if density > 0.0 {
        1.0 / density as f64
    } else {
        0.0
    };
    let jpeg = Some(encode_texture(&atlas)?);
    Timing::add(&TIMING.encode, t0);
    Ok((
        TilePrimitive {
            positions: unwrapped.positions,
            normals: unwrapped.normals,
            uvs: unwrapped.uvs,
            indices: unwrapped.indices,
            jpeg,
        },
        texel_m,
    ))
}

/// Grow charts by connectivity + normal, fold tiny charts into a neighbour,
/// project each to its plane, shelf-pack. Vertices duplicate only on seams.
fn chart_unwrap(
    mesh: &TilePrimitive,
    atlas_size: u32,
) -> Result<(TilePrimitive, (u32, u32)), Error> {
    let ntri = mesh.indices.len() / 3;
    if ntri == 0 {
        return Err(Error::msg("empty simplified mesh"));
    }
    let has_n = mesh.normals.len() == mesh.positions.len();
    let mut face_n = vec![[0.0f32; 3]; ntri];
    let mut face_area = vec![0.0f32; ntri];
    for (fi, tri) in mesh.indices.chunks_exact(3).enumerate() {
        let (n, a) = grid::face_normal_area(
            mesh.positions[tri[0] as usize],
            mesh.positions[tri[1] as usize],
            mesh.positions[tri[2] as usize],
        );
        face_n[fi] = n;
        face_area[fi] = a;
    }

    let mut edge_faces: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (fi, tri) in mesh.indices.chunks_exact(3).enumerate() {
        for k in 0..3 {
            let a = tri[k];
            let b = tri[(k + 1) % 3];
            edge_faces.entry((a.min(b), a.max(b))).or_default().push(fi);
        }
    }
    let mut adj = vec![Vec::new(); ntri];
    for faces in edge_faces.values() {
        for (i, &fa) in faces.iter().enumerate() {
            for &fb in faces.iter().skip(i + 1) {
                adj[fa].push(fb);
                adj[fb].push(fa);
            }
        }
    }
    for a in &mut adj {
        a.sort_unstable();
        a.dedup();
    }

    // 75° — fold over the chart plane starts a new chart (xatlas-style).
    const MIN_DOT: f32 = 0.26;
    const COMPACT_MIN_FACES: usize = 16;
    /// Projected rect may be at most this × the projected area.
    const COMPACT_MAX_SLACK: f32 = 1.5;
    let mut chart_of = vec![usize::MAX; ntri];
    let mut order: Vec<usize> = (0..ntri).collect();
    order.sort_by(|&a, &b| {
        face_area[b]
            .partial_cmp(&face_area[a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    struct Chart {
        faces: Vec<usize>,
        normal: [f32; 3],
        origin: [f32; 3],
    }
    let mut charts: Vec<Chart> = Vec::new();
    for seed in order {
        if chart_of[seed] != usize::MAX {
            continue;
        }
        let n0 = face_n[seed];
        let tri = &mesh.indices[seed * 3..seed * 3 + 3];
        let origin = centroid3(
            mesh.positions[tri[0] as usize],
            mesh.positions[tri[1] as usize],
            mesh.positions[tri[2] as usize],
        );
        let (tangent, bitangent) = plane_basis(n0);
        let id = charts.len();
        chart_of[seed] = id;
        let mut faces = vec![seed];
        let mut q = VecDeque::new();
        q.push_back(seed);
        // Projected AABB vs projected area: refuse growth that turns the
        // chart into an L or a diagonal sliver (its rect would pack mostly
        // dead space). Small charts may grow freely.
        let mut lo = [f32::INFINITY; 2];
        let mut hi = [f32::NEG_INFINITY; 2];
        let mut parea = 0.0f32;
        let extend = |lo: &mut [f32; 2], hi: &mut [f32; 2], pts: [[f32; 2]; 3]| {
            for p in pts {
                lo[0] = lo[0].min(p[0]);
                lo[1] = lo[1].min(p[1]);
                hi[0] = hi[0].max(p[0]);
                hi[1] = hi[1].max(p[1]);
            }
        };
        {
            let t = &mesh.indices[seed * 3..seed * 3 + 3];
            let pts = [
                project2(mesh.positions[t[0] as usize], origin, tangent, bitangent),
                project2(mesh.positions[t[1] as usize], origin, tangent, bitangent),
                project2(mesh.positions[t[2] as usize], origin, tangent, bitangent),
            ];
            extend(&mut lo, &mut hi, pts);
            parea += tri_area2(pts);
        }
        while let Some(fi) = q.pop_front() {
            for &nb in &adj[fi] {
                if chart_of[nb] != usize::MAX {
                    continue;
                }
                if grid::dot(face_n[nb], n0) < MIN_DOT {
                    continue;
                }
                let t = &mesh.indices[nb * 3..nb * 3 + 3];
                let a = project2(mesh.positions[t[0] as usize], origin, tangent, bitangent);
                let b = project2(mesh.positions[t[1] as usize], origin, tangent, bitangent);
                let c = project2(mesh.positions[t[2] as usize], origin, tangent, bitangent);
                if (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]) < 0.0 {
                    continue;
                }
                let (mut nlo, mut nhi) = (lo, hi);
                extend(&mut nlo, &mut nhi, [a, b, c]);
                let narea = parea + tri_area2([a, b, c]);
                if faces.len() >= COMPACT_MIN_FACES
                    && (nhi[0] - nlo[0]) * (nhi[1] - nlo[1]) > COMPACT_MAX_SLACK * narea
                {
                    continue;
                }
                lo = nlo;
                hi = nhi;
                parea = narea;
                chart_of[nb] = id;
                faces.push(nb);
                q.push_back(nb);
            }
        }
        charts.push(Chart {
            faces,
            normal: n0,
            origin,
        });
    }

    // Fold tiny charts into the adjacent chart they share most edges with
    // (front-facing relative to it). Hundreds of 8 px micro-charts otherwise
    // eat a 256² atlas in gutters.
    let total_area: f32 = face_area.iter().sum();
    let tiny_area = total_area * 0.002;
    for ci in 0..charts.len() {
        let area: f32 = charts[ci].faces.iter().map(|&f| face_area[f]).sum();
        if charts[ci].faces.len() > 3 && area > tiny_area {
            continue;
        }
        let mut votes: HashMap<usize, usize> = HashMap::new();
        for &f in &charts[ci].faces {
            for &nb in &adj[f] {
                let other = chart_of[nb];
                if other != ci && grid::dot(charts[other].normal, charts[ci].normal) > 0.0 {
                    *votes.entry(other).or_default() += 1;
                }
            }
        }
        let Some((&target, _)) = votes
            .iter()
            .max_by_key(|(k, v)| (**v, std::cmp::Reverse(**k)))
        else {
            continue;
        };
        let faces = std::mem::take(&mut charts[ci].faces);
        for &f in &faces {
            chart_of[f] = target;
        }
        charts[target].faces.extend(faces);
    }

    struct Layout {
        faces: Vec<usize>,
        xy: HashMap<u32, [f32; 2]>,
        wm: f32,
        hm: f32,
    }
    let mut layouts = Vec::with_capacity(charts.len());
    for ch in &charts {
        if ch.faces.is_empty() {
            continue;
        }
        let (tangent, bitangent) = plane_basis(ch.normal);
        let mut xy: HashMap<u32, [f32; 2]> = HashMap::new();
        let mut min = [f32::INFINITY; 2];
        let mut max = [f32::NEG_INFINITY; 2];
        for &fi in &ch.faces {
            let t = &mesh.indices[fi * 3..fi * 3 + 3];
            for &vi in t {
                xy.entry(vi).or_insert_with(|| {
                    let p = project2(mesh.positions[vi as usize], ch.origin, tangent, bitangent);
                    min[0] = min[0].min(p[0]);
                    min[1] = min[1].min(p[1]);
                    max[0] = max[0].max(p[0]);
                    max[1] = max[1].max(p[1]);
                    p
                });
            }
        }
        if !min[0].is_finite() {
            continue;
        }
        // Rotate to the tightest of 12 bounding rectangles: projected charts
        // are diagonal slivers as often as not, and AABB slack is what keeps
        // parent fill near 60 %.
        let pts: Vec<[f32; 2]> = xy.values().copied().collect();
        let (mut best_a, mut best_area) = (0.0f32, (max[0] - min[0]) * (max[1] - min[1]));
        for k in 1..12 {
            let a = k as f32 * std::f32::consts::PI / 24.0;
            let (s, c) = a.sin_cos();
            let mut lo = [f32::INFINITY; 2];
            let mut hi = [f32::NEG_INFINITY; 2];
            for p in &pts {
                let r = [c * p[0] - s * p[1], s * p[0] + c * p[1]];
                lo[0] = lo[0].min(r[0]);
                lo[1] = lo[1].min(r[1]);
                hi[0] = hi[0].max(r[0]);
                hi[1] = hi[1].max(r[1]);
            }
            let area = (hi[0] - lo[0]) * (hi[1] - lo[1]);
            if area < best_area * 0.97 {
                best_area = area;
                best_a = a;
            }
        }
        if best_a != 0.0 {
            let (s, c) = best_a.sin_cos();
            min = [f32::INFINITY; 2];
            max = [f32::NEG_INFINITY; 2];
            for p in xy.values_mut() {
                *p = [c * p[0] - s * p[1], s * p[0] + c * p[1]];
                min[0] = min[0].min(p[0]);
                min[1] = min[1].min(p[1]);
                max[0] = max[0].max(p[0]);
                max[1] = max[1].max(p[1]);
            }
        }
        for p in xy.values_mut() {
            p[0] -= min[0];
            p[1] -= min[1];
        }
        layouts.push(Layout {
            faces: ch.faces.clone(),
            xy,
            wm: (max[0] - min[0]).max(1e-6),
            hm: (max[1] - min[1]).max(1e-6),
        });
    }
    if layouts.is_empty() {
        return Err(Error::msg("chart unwrap produced no charts"));
    }

    // 4 px: parents are seen at coarse mips, where a 2 px gutter bleeds.
    let pad = 4u32;
    let dims_at = |scale: f32| -> Vec<(u32, u32)> {
        layouts
            .iter()
            .map(|l| {
                (
                    ((l.wm * scale).ceil() as u32 + 2 * pad).max(pad * 2 + 2),
                    ((l.hm * scale).ceil() as u32 + 2 * pad).max(pad * 2 + 2),
                )
            })
            .collect()
    };
    let cap = (atlas_size * atlas_size) as f32;
    let total_area: f32 = layouts.iter().map(|l| l.wm * l.hm).sum();
    let mut scale = (cap * 0.9 / total_area.max(1e-8)).sqrt();
    let mut rects = vec![(0u32, 0u32, 0u32, 0u32); layouts.len()];
    let mut packed = false;
    for _ in 0..96 {
        let dims = dims_at(scale);
        if let Some(pos) = pack_rects(&dims, atlas_size) {
            for (i, (&(x, y), &(w, h))) in pos.iter().zip(&dims).enumerate() {
                rects[i] = (x, y, w, h);
            }
            packed = true;
            break;
        }
        let padded: f32 = dims.iter().map(|&(w, h)| (w * h) as f32).sum();
        let implied = (cap * 0.92 / padded).sqrt() * scale;
        scale = implied.min(scale * 0.96);
    }
    if !packed {
        return Err(Error::msg("chart packing failed"));
    }
    // Then grow while it still fits: texels the parent would otherwise waste.
    for _ in 0..8 {
        let dims = dims_at(scale * 1.04);
        let Some(pos) = pack_rects(&dims, atlas_size) else {
            break;
        };
        scale *= 1.04;
        for (i, (&(x, y), &(w, h))) in pos.iter().zip(&dims).enumerate() {
            rects[i] = (x, y, w, h);
        }
    }

    // A wide sliver or an L-shaped chart caps the scale by atlas width and
    // leaves the top empty: cut the atlas height to the used skyline.
    let used_h = rects.iter().map(|r| r.1 + r.3).max().unwrap_or(1).max(1);
    let mut atlas_h = MIN_ATLAS.min(atlas_size);
    while atlas_h < used_h && atlas_h < atlas_size {
        atlas_h *= 2;
    }
    let denom_w = (atlas_size - 1) as f32;
    let denom_h = (atlas_h - 1).max(1) as f32;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    for (i, layout) in layouts.iter().enumerate() {
        let (x, y, bw, bh) = rects[i];
        let iw = bw.saturating_sub(pad * 2).max(1);
        let ih = bh.saturating_sub(pad * 2).max(1);
        let ox = x + pad;
        let oy = y + pad;
        let mut remap: HashMap<u32, u32> = HashMap::new();
        let mut verts: Vec<(&u32, &[f32; 2])> = layout.xy.iter().collect();
        verts.sort_by_key(|(vi, _)| **vi);
        for (&vi, &pxy) in verts {
            let id = positions.len() as u32;
            remap.insert(vi, id);
            positions.push(mesh.positions[vi as usize]);
            normals.push(if has_n {
                mesh.normals[vi as usize]
            } else {
                [0.0, 1.0, 0.0]
            });
            uvs.push([
                (ox as f32 + (pxy[0] / layout.wm) * (iw - 1) as f32) / denom_w,
                (oy as f32 + (pxy[1] / layout.hm) * (ih - 1) as f32) / denom_h,
            ]);
        }
        for &fi in &layout.faces {
            let t = &mesh.indices[fi * 3..fi * 3 + 3];
            indices.push(remap[&t[0]]);
            indices.push(remap[&t[1]]);
            indices.push(remap[&t[2]]);
        }
    }

    Ok((
        TilePrimitive {
            positions,
            normals: if has_n { normals } else { Vec::new() },
            uvs,
            indices,
            jpeg: None,
        },
        (atlas_size, atlas_h),
    ))
}

fn plane_basis(n: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let up = if n[1].abs() < 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let t = grid::normalize(grid::cross(up, n));
    let b = grid::cross(n, t);
    (t, b)
}

fn tri_area2(p: [[f32; 2]; 3]) -> f32 {
    0.5 * ((p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]))
        .abs()
}

fn project2(p: [f32; 3], origin: [f32; 3], t: [f32; 3], b: [f32; 3]) -> [f32; 2] {
    let d = grid::sub(p, origin);
    [grid::dot(d, t), grid::dot(d, b)]
}

fn centroid3(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    [
        (a[0] + b[0] + c[0]) / 3.0,
        (a[1] + b[1] + c[1]) / 3.0,
        (a[2] + b[2] + c[2]) / 3.0,
    ]
}

/// Dilate covered texels outward by `max_dist`, then fill the rest with the
/// mean covered colour so mip levels do not pull in grey.
fn fill_uncovered(img: &mut RgbaImage, uvs: &[[f32; 2]], indices: &[u32], max_dist: u32) {
    let w = img.width().max(1);
    let h = img.height().max(1);
    let mut cover = vec![false; (w * h) as usize];
    for tri in indices.chunks_exact(3) {
        let t0 = uvs[tri[0] as usize];
        let t1 = uvs[tri[1] as usize];
        let t2 = uvs[tri[2] as usize];
        let px = |t: [f32; 2]| -> (i32, i32) {
            (
                (t[0] * (w - 1) as f32).round() as i32,
                (t[1] * (h - 1) as f32).round() as i32,
            )
        };
        let (x0, y0) = px(t0);
        let (x1, y1) = px(t1);
        let (x2, y2) = px(t2);
        let min_x = x0.min(x1).min(x2).max(0);
        let max_x = x0.max(x1).max(x2).min(w as i32 - 1);
        let min_y = y0.min(y1).min(y2).max(0);
        let max_y = y0.max(y1).max(y2).min(h as i32 - 1);
        if min_x > max_x || min_y > max_y {
            continue;
        }
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let (b0, b1, b2) = bary_uv(
                    t0,
                    t1,
                    t2,
                    [x as f32 / (w - 1) as f32, y as f32 / (h - 1) as f32],
                );
                if b0 >= -1e-4 && b1 >= -1e-4 && b2 >= -1e-4 {
                    cover[(y as u32 * w + x as u32) as usize] = true;
                }
            }
        }
    }

    let mut sum = [0u64; 3];
    let mut n = 0u64;
    let mut dist = vec![u32::MAX; (w * h) as usize];
    let mut q = VecDeque::new();
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if cover[i] {
                let p = img.get_pixel(x, y).0;
                sum[0] += p[0] as u64;
                sum[1] += p[1] as u64;
                sum[2] += p[2] as u64;
                n += 1;
                dist[i] = 0;
                q.push_back((x, y));
            }
        }
    }
    while let Some((x, y)) = q.pop_front() {
        let d = dist[(y * w + x) as usize];
        if d >= max_dist {
            continue;
        }
        let src = *img.get_pixel(x, y);
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                    continue;
                }
                let j = (ny as u32 * w + nx as u32) as usize;
                if dist[j] != u32::MAX {
                    continue;
                }
                dist[j] = d + 1;
                img.put_pixel(nx as u32, ny as u32, src);
                q.push_back((nx as u32, ny as u32));
            }
        }
    }
    if let Some(n) = std::num::NonZeroU64::new(n) {
        let mean = image::Rgba([
            (sum[0] / n) as u8,
            (sum[1] / n) as u8,
            (sum[2] / n) as u8,
            255,
        ]);
        for y in 0..h {
            for x in 0..w {
                if dist[(y * w + x) as usize] == u32::MAX {
                    img.put_pixel(x, y, mean);
                }
            }
        }
    }
}

/// Lossy WebP (q90): full chroma vs JPEG 4:2:0, similar payload. Method 1
/// encodes 2× faster than 3 for ~3 % more bytes (measured on a leaf atlas).
pub fn encode_texture(img: &RgbaImage) -> Result<Vec<u8>, Error> {
    let rgb = DynamicImage::ImageRgba8(img.clone()).to_rgb8();
    encode_texture_rgb(&rgb)
}

pub fn encode_texture_rgb(rgb: &RgbImage) -> Result<Vec<u8>, Error> {
    let cfg = zenwebp::LossyConfig::new()
        .with_quality(90.0)
        .with_method(1);
    zenwebp::EncodeRequest::lossy(
        &cfg,
        rgb.as_raw(),
        zenwebp::PixelLayout::Rgb8,
        rgb.width(),
        rgb.height(),
    )
    .encode()
    .map_err(|e| Error::msg(format!("webp: {e}")))
}

fn bary_uv(a: [f32; 2], b: [f32; 2], c: [f32; 2], p: [f32; 2]) -> (f32, f32, f32) {
    let v0 = [b[0] - a[0], b[1] - a[1]];
    let v1 = [c[0] - a[0], c[1] - a[1]];
    let v2 = [p[0] - a[0], p[1] - a[1]];
    let d00 = v0[0] * v0[0] + v0[1] * v0[1];
    let d01 = v0[0] * v1[0] + v0[1] * v1[1];
    let d11 = v1[0] * v1[0] + v1[1] * v1[1];
    let d20 = v2[0] * v0[0] + v2[1] * v0[1];
    let d21 = v2[0] * v1[0] + v2[1] * v1[1];
    let denom = (d00 * d11 - d01 * d01).max(1e-20);
    let v = (d11 * d20 - d01 * d21) / denom;
    let w = (d00 * d21 - d01 * d20) / denom;
    let u = 1.0 - v - w;
    (u, v, w)
}

// ---------------------------------------------------------------------------
// Sampler
// ---------------------------------------------------------------------------

/// Nearest-surface colour lookup over a set of textured primitives.
pub struct SceneSampler {
    images: Vec<RgbaImage>,
    tri_img: Vec<u32>,
    tri_uv: Vec<[[f32; 2]; 3]>,
    tri_n: Vec<[f32; 3]>,
    grid: TriGrid,
}

impl SceneSampler {
    /// Build from textured primitives. With `target_px_per_m`, each image is
    /// box-downsampled to about that density first so a coarse parent texel
    /// averages the child texels it covers instead of point-sampling one.
    pub fn from_prims(
        prims: &[TilePrimitive],
        target_px_per_m: Option<f32>,
    ) -> Result<Self, Error> {
        let mut images = Vec::new();
        let mut tri_img = Vec::new();
        let mut tri_uv = Vec::new();
        let mut tri_n = Vec::new();
        let mut tris = Vec::new();
        for p in prims {
            let Some(jpeg) = &p.jpeg else { continue };
            if p.uvs.len() != p.positions.len() || p.indices.len() < 3 {
                continue;
            }
            let mut rgba = image::load_from_memory(jpeg)
                .map_err(|e| Error::msg(format!("child atlas: {e}")))?
                .to_rgba8();
            if let Some(target) = target_px_per_m {
                if let Some(d) = texel_density(p, rgba.width(), rgba.height()) {
                    let f = d / target.max(1e-6);
                    if f >= 2.0 {
                        let f = f.floor().min(64.0);
                        let nw = ((rgba.width() as f32 / f).round() as u32).max(1);
                        let nh = ((rgba.height() as f32 / f).round() as u32).max(1);
                        rgba = imageops::resize(&rgba, nw, nh, FilterType::Triangle);
                    }
                }
            }
            let img_id = images.len() as u32;
            images.push(rgba);
            for tri in p.indices.chunks_exact(3) {
                let (ia, ib, ic) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
                let (a, b, c) = (p.positions[ia], p.positions[ib], p.positions[ic]);
                tris.push([a, b, c]);
                tri_img.push(img_id);
                tri_uv.push([p.uvs[ia], p.uvs[ib], p.uvs[ic]]);
                tri_n.push(grid::face_normal_area(a, b, c).0);
            }
        }
        if images.is_empty() || tris.is_empty() {
            return Err(Error::msg("no textured children to bake from"));
        }
        Ok(Self {
            images,
            tri_img,
            tri_uv,
            tri_n,
            grid: TriGrid::new(tris),
        })
    }

    /// Colour at the child surface nearest `p`, preferring triangles that face
    /// the same way (so the far cave wall does not bleed through).
    pub fn sample(&self, p: [f32; 3], nrm: [f32; 3]) -> [u8; 3] {
        let mut seed = None;
        self.sample_seeded(p, nrm, &mut seed)
    }

    /// `sample` carrying the last hit between calls (texels of one triangle).
    pub fn sample_seeded(&self, p: [f32; 3], nrm: [f32; 3], seed: &mut Option<u32>) -> [u8; 3] {
        let n = grid::normalize(nrm);
        let cell2 = self.grid.cell_size() * self.grid.cell_size();
        let hit = self.grid.nearest_by_seeded(p, 6, *seed, |ti, d2, _| {
            let facing = grid::dot(n, self.tri_n[ti as usize]) > 0.15;
            Some(if facing { d2 } else { d2 * 8.0 + cell2 })
        });
        let Some(h) = hit else {
            return [128, 128, 128];
        };
        *seed = Some(h.tri);
        let uv = self.tri_uv[h.tri as usize];
        let u = uv[0][0] * h.bary[0] + uv[1][0] * h.bary[1] + uv[2][0] * h.bary[2];
        let v = uv[0][1] * h.bary[0] + uv[1][1] * h.bary[1] + uv[2][1] * h.bary[2];
        sample_rgba(&self.images[self.tri_img[h.tri as usize] as usize], u, v)
    }
}

fn sample_rgba(img: &RgbaImage, u: f32, v: f32) -> [u8; 3] {
    let w = img.width().max(1);
    let h = img.height().max(1);
    let xf = u.clamp(0.0, 1.0) * (w - 1) as f32;
    let yf = v.clamp(0.0, 1.0) * (h - 1) as f32;
    let x0 = xf.floor() as u32;
    let y0 = yf.floor() as u32;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let tx = xf - x0 as f32;
    let ty = yf - y0 as f32;
    let p00 = img.get_pixel(x0, y0).0;
    let p10 = img.get_pixel(x1, y0).0;
    let p01 = img.get_pixel(x0, y1).0;
    let p11 = img.get_pixel(x1, y1).0;
    let mut out = [0u8; 3];
    for i in 0..3 {
        let a = p00[i] as f32 * (1.0 - tx) + p10[i] as f32 * tx;
        let b = p01[i] as f32 * (1.0 - tx) + p11[i] as f32 * tx;
        out[i] = (a * (1.0 - ty) + b * ty).round().clamp(0.0, 255.0) as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid_plane(nx: u32, ny: u32) -> TilePrimitive {
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        for y in 0..=ny {
            for x in 0..=nx {
                positions.push([x as f32, 0.0, y as f32]);
            }
        }
        let w = nx + 1;
        for y in 0..ny {
            for x in 0..nx {
                let i = y * w + x;
                indices.extend_from_slice(&[i, i + 1, i + w + 1, i, i + w + 1, i + w]);
            }
        }
        TilePrimitive {
            positions,
            normals: Vec::new(),
            uvs: Vec::new(),
            indices,
            jpeg: None,
        }
    }

    #[test]
    fn chart_unwrap_shares_coplanar_vertices() {
        let mesh = grid_plane(4, 4);
        let src_verts = mesh.positions.len();
        let src_tris = mesh.indices.len() / 3;
        let (out, _) = chart_unwrap(&mesh, 128).unwrap();
        assert_eq!(out.indices.len() / 3, src_tris);
        assert!(
            out.positions.len() <= src_verts + 4,
            "coplanar grid should stay one chart with shared verts, got {} (src {src_verts})",
            out.positions.len()
        );
    }

    #[test]
    fn leaf_plan_packs_two_far_islands_tightly() {
        // Two quads whose UVs sit in opposite corners of a 1024² atlas: a
        // UV-AABB crop would take the whole image; charts take 2 small rects.
        let quad = |u0: f32, v0: f32| TilePrimitive {
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            normals: Vec::new(),
            uvs: vec![
                [u0, v0],
                [u0 + 0.05, v0],
                [u0 + 0.05, v0 + 0.05],
                [u0, v0 + 0.05],
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            jpeg: None,
        };
        let mut a = quad(0.0, 0.0);
        let b = quad(0.9, 0.9);
        let base = a.positions.len() as u32;
        a.positions.extend_from_slice(&b.positions);
        a.uvs.extend_from_slice(&b.uvs);
        a.indices.extend(b.indices.iter().map(|i| i + base));
        let plan = plan_leaf_atlas(
            vec![LeafGroup {
                image: Some(0),
                prim: a,
            }],
            &[(1024, 1024)],
            1024,
            0.0,
        )
        .unwrap();
        assert_eq!(plan.blits.len(), 2, "two islands → two blits");
        assert!(
            plan.atlas_wh.0 <= 128,
            "atlas should be small, got {:?}",
            plan.atlas_wh
        );
        assert!((plan.scale - 1.0).abs() < 1e-6, "source resolution kept");
        let t = plan.textured.unwrap();
        for uv in &t.uvs {
            assert!((0.0..=1.0).contains(&uv[0]) && (0.0..=1.0).contains(&uv[1]));
        }
    }

    #[test]
    fn leaf_plan_downscales_when_over_cap() {
        let prim = TilePrimitive {
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            normals: Vec::new(),
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            indices: vec![0, 1, 2, 0, 2, 3],
            jpeg: None,
        };
        let plan = plan_leaf_atlas(
            vec![LeafGroup {
                image: Some(0),
                prim,
            }],
            &[(8192, 8192)],
            512,
            0.0,
        )
        .unwrap();
        assert_eq!(plan.atlas_wh, (512, 512));
        assert!(
            plan.scale < 0.07,
            "8K into 512 needs ~1/16, got {}",
            plan.scale
        );
    }

    #[test]
    fn blit_and_finish_produce_webp() {
        let mut src = RgbImage::new(64, 64);
        for (x, y, p) in src.enumerate_pixels_mut() {
            *p = image::Rgb([x as u8 * 4, y as u8 * 4, 0]);
        }
        let mut atlas = RgbImage::new(64, 64);
        let b = Blit {
            image: 0,
            src: [0.0, 0.0, 0.5, 0.5],
            dst: [0, 0, 32, 32],
        };
        blit_chart(&mut atlas, &src, &b);
        assert!(atlas.get_pixel(31, 31).0[0] > 100);
        let bytes = finish_leaf_atlas(atlas, &[b]).unwrap();
        assert!(bytes.starts_with(b"RIFF"));
    }
}
