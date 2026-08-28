//! Decode source images (JPEG at IDCT 1/8 when huge); crop + resize + JPEG per leaf.

use std::collections::HashMap;
use std::io::Cursor;

use image::imageops::{self, FilterType};
use image::{DynamicImage, RgbaImage};

use crate::error::Error;
use crate::mesh::EncodedImage;

const UV_PAD_PX: u32 = 4;
const JPEG_QUALITY: u8 = 85;

/// Decode, downsampling so the long edge is at most `4 * tile_size` (JPEG uses IDCT 1/8/4/2).
pub fn decode_rgba(encoded: &EncodedImage, tile_size: u32) -> Result<RgbaImage, Error> {
    let bytes = encoded.load()?;
    if let Ok(img) = decode_jpeg_scaled(&bytes, tile_size) {
        return Ok(img);
    }
    let dynimg = image::load_from_memory(&bytes)?;
    let mut rgba = dynimg.to_rgba8();
    let cap = decode_cap(tile_size);
    let long = rgba.width().max(rgba.height());
    if long > cap {
        let s = cap as f32 / long as f32;
        let nw = ((rgba.width() as f32) * s).round().max(1.0) as u32;
        let nh = ((rgba.height() as f32) * s).round().max(1.0) as u32;
        rgba = imageops::resize(&rgba, nw, nh, FilterType::Triangle);
    }
    Ok(rgba)
}

fn decode_cap(tile_size: u32) -> u32 {
    tile_size.max(1).saturating_mul(4).max(256)
}

fn decode_jpeg_scaled(bytes: &[u8], tile_size: u32) -> Result<RgbaImage, Error> {
    let mut dec = jpeg_decoder::Decoder::new(Cursor::new(bytes));
    dec.read_info()
        .map_err(|e| Error::msg(format!("jpeg: {e}")))?;
    let cap = decode_cap(tile_size).min(u16::MAX as u32) as u16;
    let (w, h) = dec
        .scale(cap, cap)
        .map_err(|e| Error::msg(format!("jpeg scale: {e}")))?;
    let pixels = dec
        .decode()
        .map_err(|e| Error::msg(format!("jpeg decode: {e}")))?;
    let info = dec.info().ok_or_else(|| Error::msg("jpeg lost header"))?;
    let (w, h) = (w as u32, h as u32);
    match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => {
            let rgb = image::RgbImage::from_raw(w, h, pixels)
                .ok_or_else(|| Error::msg("jpeg RGB buffer size"))?;
            Ok(DynamicImage::ImageRgb8(rgb).to_rgba8())
        }
        jpeg_decoder::PixelFormat::L8 => {
            let gray = image::GrayImage::from_raw(w, h, pixels)
                .ok_or_else(|| Error::msg("jpeg L buffer size"))?;
            Ok(DynamicImage::ImageLuma8(gray).to_rgba8())
        }
        _ => Err(Error::msg("jpeg pixel format not RGB/L")),
    }
}

/// Crop the UV AABB (plus padding), fit to `tile_size`, encode JPEG, remap UVs to 0–1.
pub fn crop_leaf(
    image: &RgbaImage,
    uvs: &[[f32; 2]],
    tile_size: u32,
) -> Result<(Vec<u8>, Vec<[f32; 2]>), Error> {
    let (w, h) = (image.width().max(1), image.height().max(1));
    let (u0, v0, u1, v1) = uv_bounds(uvs);
    let mut x0 = ((u0 * w as f32).floor() as i32) - UV_PAD_PX as i32;
    let mut y0 = ((v0 * h as f32).floor() as i32) - UV_PAD_PX as i32;
    let mut x1 = ((u1 * w as f32).ceil() as i32) + UV_PAD_PX as i32;
    let mut y1 = ((v1 * h as f32).ceil() as i32) + UV_PAD_PX as i32;
    x0 = x0.clamp(0, w as i32 - 1);
    y0 = y0.clamp(0, h as i32 - 1);
    x1 = x1.clamp(x0 + 1, w as i32);
    y1 = y1.clamp(y0 + 1, h as i32);

    let crop_w = (x1 - x0) as u32;
    let crop_h = (y1 - y0) as u32;
    let cropped = imageops::crop_imm(image, x0 as u32, y0 as u32, crop_w, crop_h).to_image();

    let (rw, rh) = fit(crop_w, crop_h, tile_size.max(1));
    let resized = if rw == crop_w && rh == crop_h {
        cropped
    } else {
        imageops::resize(&cropped, rw, rh, FilterType::Triangle)
    };

    let jpeg = encode_jpeg(&resized)?;

    let u_span = (x1 - x0) as f32 / w as f32;
    let v_span = (y1 - y0) as f32 / h as f32;
    let u_org = x0 as f32 / w as f32;
    let v_org = y0 as f32 / h as f32;
    let remapped = uvs
        .iter()
        .map(|uv| {
            let u = if u_span > 1e-8 {
                ((uv[0] - u_org) / u_span).clamp(0.0, 1.0)
            } else {
                0.5
            };
            let v = if v_span > 1e-8 {
                ((uv[1] - v_org) / v_span).clamp(0.0, 1.0)
            } else {
                0.5
            };
            [u, v]
        })
        .collect();

    Ok((jpeg, remapped))
}

fn uv_bounds(uvs: &[[f32; 2]]) -> (f32, f32, f32, f32) {
    let mut u0 = f32::INFINITY;
    let mut v0 = f32::INFINITY;
    let mut u1 = f32::NEG_INFINITY;
    let mut v1 = f32::NEG_INFINITY;
    for uv in uvs {
        let u = uv[0].clamp(0.0, 1.0);
        let v = uv[1].clamp(0.0, 1.0);
        u0 = u0.min(u);
        v0 = v0.min(v);
        u1 = u1.max(u);
        v1 = v1.max(v);
    }
    if !u0.is_finite() {
        (0.0, 0.0, 1.0, 1.0)
    } else {
        (u0, v0, u1.max(u0 + 1e-6), v1.max(v0 + 1e-6))
    }
}

fn fit(w: u32, h: u32, tile_size: u32) -> (u32, u32) {
    if w <= tile_size && h <= tile_size {
        return (w.max(1), h.max(1));
    }
    let long = w.max(h) as f32;
    let s = tile_size as f32 / long;
    (
        ((w as f32) * s).round().max(1.0) as u32,
        ((h as f32) * s).round().max(1.0) as u32,
    )
}

fn encode_jpeg(img: &RgbaImage) -> Result<Vec<u8>, Error> {
    let rgb = DynamicImage::ImageRgba8(img.clone()).to_rgb8();
    let mut buf = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, JPEG_QUALITY);
    enc.encode(
        rgb.as_raw(),
        rgb.width(),
        rgb.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(buf)
}

/// Source charts for closest-point albedo transfer onto a simplified LOD.
pub struct SourceSampler {
    images: Vec<RgbaImage>,
    tris: Vec<SampleTri>,
    bins: HashMap<(i32, i32, i32), Vec<u32>>,
    cell: f32,
}

struct SampleTri {
    img: u32,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    ua: [f32; 2],
    ub: [f32; 2],
    uc: [f32; 2],
}

impl SourceSampler {
    /// Build from parent bake charts. Large meshes are stride-sampled so the
    /// hash stays bounded (distant LOD does not need every source triangle).
    pub fn from_prims(prims: &[crate::glb_write::TilePrimitive]) -> Result<Self, Error> {
        let mut images = Vec::new();
        let mut tris = Vec::new();
        let mut total_src = 0usize;
        for p in prims {
            total_src += p.indices.len() / 3;
        }
        let stride = (total_src / 400_000).max(1);

        for p in prims {
            let Some(jpeg) = &p.jpeg else { continue };
            if p.uvs.len() != p.positions.len() || p.indices.len() < 3 {
                continue;
            }
            let img_id = images.len() as u32;
            let rgba = image::load_from_memory(jpeg)
                .map_err(|e| Error::msg(format!("parent chart jpeg: {e}")))?
                .to_rgba8();
            images.push(rgba);
            for (ti, tri) in p.indices.chunks_exact(3).enumerate() {
                if ti % stride != 0 {
                    continue;
                }
                let ia = tri[0] as usize;
                let ib = tri[1] as usize;
                let ic = tri[2] as usize;
                tris.push(SampleTri {
                    img: img_id,
                    a: p.positions[ia],
                    b: p.positions[ib],
                    c: p.positions[ic],
                    ua: p.uvs[ia],
                    ub: p.uvs[ib],
                    uc: p.uvs[ic],
                });
            }
        }
        if images.is_empty() || tris.is_empty() {
            return Err(Error::msg("no textured charts to bake from"));
        }

        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for t in &tris {
            for p in [t.a, t.b, t.c] {
                for i in 0..3 {
                    min[i] = min[i].min(p[i]);
                    max[i] = max[i].max(p[i]);
                }
            }
        }
        let extent = (max[0] - min[0])
            .max(max[1] - min[1])
            .max(max[2] - min[2])
            .max(1e-3);
        let cell = extent / 64.0;
        let mut bins: HashMap<(i32, i32, i32), Vec<u32>> = HashMap::new();
        for (i, t) in tris.iter().enumerate() {
            let c = centroid(t.a, t.b, t.c);
            let key = (
                (c[0] / cell).floor() as i32,
                (c[1] / cell).floor() as i32,
                (c[2] / cell).floor() as i32,
            );
            bins.entry(key).or_default().push(i as u32);
        }
        Ok(Self {
            images,
            tris,
            bins,
            cell,
        })
    }

    fn sample(&self, p: [f32; 3]) -> [u8; 3] {
        let cx = (p[0] / self.cell).floor() as i32;
        let cy = (p[1] / self.cell).floor() as i32;
        let cz = (p[2] / self.cell).floor() as i32;
        let mut best_d2 = f32::INFINITY;
        let mut best_rgb = [128u8, 128, 128];
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let Some(bin) = self.bins.get(&(cx + dx, cy + dy, cz + dz)) else {
                        continue;
                    };
                    for &ti in bin {
                        let t = &self.tris[ti as usize];
                        let (d2, bary) = point_tri_dist2_bary(p, t.a, t.b, t.c);
                        if d2 < best_d2 {
                            best_d2 = d2;
                            let u = t.ua[0] * bary[0] + t.ub[0] * bary[1] + t.uc[0] * bary[2];
                            let v = t.ua[1] * bary[0] + t.ub[1] * bary[1] + t.uc[1] * bary[2];
                            best_rgb = sample_rgba(&self.images[t.img as usize], u, v);
                        }
                    }
                }
            }
        }
        best_rgb
    }
}

fn centroid(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    [
        (a[0] + b[0] + c[0]) / 3.0,
        (a[1] + b[1] + c[1]) / 3.0,
        (a[2] + b[2] + c[2]) / 3.0,
    ]
}

fn sample_rgba(img: &RgbaImage, u: f32, v: f32) -> [u8; 3] {
    let w = img.width().max(1);
    let h = img.height().max(1);
    let x = ((u.clamp(0.0, 1.0) * (w - 1) as f32).round() as u32).min(w - 1);
    // glTF / image V is top-origin in most engines for 2D images; our crops use
    // standard image coords (v=0 top after crop_leaf remap). Sample with v from top.
    let y = ((v.clamp(0.0, 1.0) * (h - 1) as f32).round() as u32).min(h - 1);
    let px = img.get_pixel(x, y).0;
    [px[0], px[1], px[2]]
}

/// Closest-point LOD atlas bake with normal-clustered charts.
///
/// A single top-down ortho stretch-maps cliffs; triangles are binned by
/// dominant face normal (±X/±Y/±Z), each chart projected along that axis and
/// packed into one atlas. Vertices are split on chart seams.
pub struct BakedLod {
    pub jpeg: Vec<u8>,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

pub fn bake_lod_atlas(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    indices: &[u32],
    sampler: &SourceSampler,
    atlas_size: u32,
) -> Result<BakedLod, Error> {
    let size = atlas_size.max(32);
    let has_n = normals.len() == positions.len();
    let mut buckets: [Vec<[u32; 3]>; 6] = Default::default();
    for tri in indices.chunks_exact(3) {
        let i0 = tri[0];
        let i1 = tri[1];
        let i2 = tri[2];
        let (axis, sign) = face_axis(
            positions[i0 as usize],
            positions[i1 as usize],
            positions[i2 as usize],
        );
        let bi = axis * 2 + if sign >= 0.0 { 0 } else { 1 };
        buckets[bi].push([i0, i1, i2]);
    }

    let mut charts = Vec::new();
    for (bi, tris) in buckets.into_iter().enumerate() {
        if tris.is_empty() {
            continue;
        }
        let axis = bi / 2;
        let sign = if bi % 2 == 0 { 1.0f32 } else { -1.0 };
        let u_ax = (axis + 1) % 3;
        let v_ax = (axis + 2) % 3;
        let mut umin = f32::INFINITY;
        let mut umax = f32::NEG_INFINITY;
        let mut vmin = f32::INFINITY;
        let mut vmax = f32::NEG_INFINITY;
        for t in &tris {
            for &i in t {
                let p = positions[i as usize];
                umin = umin.min(p[u_ax]);
                umax = umax.max(p[u_ax]);
                vmin = vmin.min(p[v_ax]);
                vmax = vmax.max(p[v_ax]);
            }
        }
        let eu = (umax - umin).max(1e-4);
        let ev = (vmax - vmin).max(1e-4);
        charts.push(ChartBuild {
            axis,
            sign,
            u_ax,
            v_ax,
            umin,
            vmin,
            eu,
            ev,
            tris,
        });
    }
    if charts.is_empty() {
        return Err(Error::msg("no triangles to bake"));
    }

    let pad = 2u32;
    let packs = pack_charts(&charts, size, pad)?;
    let mut atlas = RgbaImage::from_pixel(size, size, image::Rgba([128, 128, 128, 255]));
    let mut zbuf = vec![f32::INFINITY; (size * size) as usize];

    let mut out_pos = Vec::new();
    let mut out_nrm = Vec::new();
    let mut out_uv = Vec::new();
    let mut out_idx = Vec::new();

    for (chart, rect) in charts.iter().zip(packs.iter()) {
        let inner_w = rect.w.saturating_sub(pad * 2).max(1) as f32;
        let inner_h = rect.h.saturating_sub(pad * 2).max(1) as f32;
        let ox = (rect.x + pad) as f32;
        let oy = (rect.y + pad) as f32;
        let mut weld: HashMap<(i32, i32, i32), u32> = HashMap::new();

        for t in &chart.tris {
            let mut emit = |pi: u32, weld: &mut HashMap<(i32, i32, i32), u32>| -> (u32, [f32; 2]) {
                let p = positions[pi as usize];
                let key = (
                    (p[0] * 1e4).round() as i32,
                    (p[1] * 1e4).round() as i32,
                    (p[2] * 1e4).round() as i32,
                );
                if let Some(&id) = weld.get(&key) {
                    return (id, out_uv[id as usize]);
                }
                let lu = ((p[chart.u_ax] - chart.umin) / chart.eu).clamp(0.0, 1.0);
                let lv = ((p[chart.v_ax] - chart.vmin) / chart.ev).clamp(0.0, 1.0);
                let u = ((ox + lu * inner_w) / size as f32).clamp(0.0, 1.0);
                let v = ((oy + lv * inner_h) / size as f32).clamp(0.0, 1.0);
                let uv = [u, v];
                let id = out_pos.len() as u32;
                out_pos.push(p);
                out_nrm.push(if has_n {
                    normals[pi as usize]
                } else {
                    [0.0, 1.0, 0.0]
                });
                out_uv.push(uv);
                weld.insert(key, id);
                (id, uv)
            };

            let (a, ua) = emit(t[0], &mut weld);
            let (b, ub) = emit(t[1], &mut weld);
            let (c, uc) = emit(t[2], &mut weld);
            out_idx.extend_from_slice(&[a, b, c]);
            raster_tri(
                &mut atlas,
                &mut zbuf,
                size,
                positions[t[0] as usize],
                positions[t[1] as usize],
                positions[t[2] as usize],
                ua,
                ub,
                uc,
                chart.axis,
                chart.sign,
                sampler,
            );
        }
    }

    for _ in 0..3 {
        dilate_atlas(&mut atlas, &mut zbuf, size);
    }
    Ok(BakedLod {
        jpeg: encode_jpeg(&atlas)?,
        positions: out_pos,
        normals: out_nrm,
        uvs: out_uv,
        indices: out_idx,
    })
}

struct ChartBuild {
    axis: usize,
    sign: f32,
    u_ax: usize,
    v_ax: usize,
    umin: f32,
    vmin: f32,
    eu: f32,
    ev: f32,
    tris: Vec<[u32; 3]>,
}

#[derive(Clone, Copy)]
struct ChartRect {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

fn face_axis(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> (usize, f32) {
    let e0 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let e1 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        e0[1] * e1[2] - e0[2] * e1[1],
        e0[2] * e1[0] - e0[0] * e1[2],
        e0[0] * e1[1] - e0[1] * e1[0],
    ];
    let ax = if n[0].abs() >= n[1].abs() && n[0].abs() >= n[2].abs() {
        0
    } else if n[1].abs() >= n[2].abs() {
        1
    } else {
        2
    };
    let sign = if n[ax] >= 0.0 { 1.0 } else { -1.0 };
    (ax, sign)
}

fn pack_charts(charts: &[ChartBuild], size: u32, pad: u32) -> Result<Vec<ChartRect>, Error> {
    let world_area: f32 = charts.iter().map(|c| c.eu * c.ev).sum();
    let mut scale = ((size * size) as f32 * 0.88 / world_area.max(1e-6)).sqrt();
    for _ in 0..24 {
        let mut rects = Vec::with_capacity(charts.len());
        let mut order: Vec<usize> = (0..charts.len()).collect();
        order.sort_by(|&a, &b| {
            let ha = (charts[a].ev * scale).ceil() as u32;
            let hb = (charts[b].ev * scale).ceil() as u32;
            hb.cmp(&ha)
        });
        let mut shelf_y = 0u32;
        let mut shelf_h = 0u32;
        let mut shelf_x = 0u32;
        let mut placed = vec![
            ChartRect {
                x: 0,
                y: 0,
                w: 0,
                h: 0
            };
            charts.len()
        ];
        let mut ok = true;
        for &i in &order {
            let c = &charts[i];
            let w = ((c.eu * scale).ceil() as u32).max(pad * 2 + 4).min(size);
            let h = ((c.ev * scale).ceil() as u32).max(pad * 2 + 4).min(size);
            if shelf_x + w > size {
                shelf_y = shelf_y.saturating_add(shelf_h);
                shelf_x = 0;
                shelf_h = 0;
            }
            if shelf_y + h > size {
                ok = false;
                break;
            }
            placed[i] = ChartRect {
                x: shelf_x,
                y: shelf_y,
                w,
                h,
            };
            shelf_x += w;
            shelf_h = shelf_h.max(h);
            rects.push(i);
        }
        if ok {
            let _ = rects;
            return Ok(placed);
        }
        scale *= 0.9;
    }
    // Fallback: equal grid cells.
    let n = charts.len().max(1);
    let cols = (n as f32).sqrt().ceil() as u32;
    let rows = ((n as u32) + cols - 1) / cols;
    let cw = (size / cols).max(1);
    let ch = (size / rows).max(1);
    Ok(charts
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let col = (i as u32) % cols;
            let row = (i as u32) / cols;
            ChartRect {
                x: col * cw,
                y: row * ch,
                w: cw,
                h: ch,
            }
        })
        .collect())
}

#[allow(clippy::too_many_arguments)]
fn raster_tri(
    atlas: &mut RgbaImage,
    zbuf: &mut [f32],
    size: u32,
    p0: [f32; 3],
    p1: [f32; 3],
    p2: [f32; 3],
    t0: [f32; 2],
    t1: [f32; 2],
    t2: [f32; 2],
    axis: usize,
    sign: f32,
    sampler: &SourceSampler,
) {
    let px = |t: [f32; 2]| -> (i32, i32) {
        (
            (t[0] * (size - 1) as f32).round() as i32,
            (t[1] * (size - 1) as f32).round() as i32,
        )
    };
    let (x0, y0) = px(t0);
    let (x1, y1) = px(t1);
    let (x2, y2) = px(t2);
    let min_x = x0.min(x1).min(x2).max(0);
    let max_x = x0.max(x1).max(x2).min(size as i32 - 1);
    let min_y = y0.min(y1).min(y2).max(0);
    let max_y = y0.max(y1).max(y2).min(size as i32 - 1);
    if min_x > max_x || min_y > max_y {
        return;
    }
    let area = (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0);
    if area == 0 {
        return;
    }
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let (b0, b1, b2) = bary_uv(
                t0,
                t1,
                t2,
                [x as f32 / (size - 1) as f32, y as f32 / (size - 1) as f32],
            );
            if b0 < -1e-4 || b1 < -1e-4 || b2 < -1e-4 {
                continue;
            }
            let world = [
                p0[0] * b0 + p1[0] * b1 + p2[0] * b2,
                p0[1] * b0 + p1[1] * b1 + p2[1] * b2,
                p0[2] * b0 + p1[2] * b1 + p2[2] * b2,
            ];
            // Prefer the front face along the chart normal.
            let z = -sign * world[axis];
            let zi = (y as u32 * size + x as u32) as usize;
            if z >= zbuf[zi] {
                continue;
            }
            zbuf[zi] = z;
            let rgb = sampler.sample(world);
            atlas.put_pixel(
                x as u32,
                y as u32,
                image::Rgba([rgb[0], rgb[1], rgb[2], 255]),
            );
        }
    }
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

fn dilate_atlas(atlas: &mut RgbaImage, zbuf: &mut [f32], size: u32) {
    let mut extra = Vec::new();
    for y in 0..size {
        for x in 0..size {
            let i = (y * size + x) as usize;
            if zbuf[i].is_finite() {
                continue;
            }
            let mut rgb = [0u32; 3];
            let mut n = 0u32;
            let mut z = f32::INFINITY;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx < 0 || ny < 0 || nx >= size as i32 || ny >= size as i32 {
                        continue;
                    }
                    let j = (ny as u32 * size + nx as u32) as usize;
                    if !zbuf[j].is_finite() {
                        continue;
                    }
                    let p = atlas.get_pixel(nx as u32, ny as u32).0;
                    rgb[0] += p[0] as u32;
                    rgb[1] += p[1] as u32;
                    rgb[2] += p[2] as u32;
                    z = z.min(zbuf[j]);
                    n += 1;
                }
            }
            if n > 0 {
                extra.push((
                    x,
                    y,
                    [(rgb[0] / n) as u8, (rgb[1] / n) as u8, (rgb[2] / n) as u8],
                    z,
                ));
            }
        }
    }
    for (x, y, rgb, z) in extra {
        atlas.put_pixel(x, y, image::Rgba([rgb[0], rgb[1], rgb[2], 255]));
        zbuf[(y * size + x) as usize] = z;
    }
}

fn point_tri_dist2_bary(p: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> (f32, [f32; 3]) {
    // Ericson closest-point; return (dist², barycentric of closest)
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let ap = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
    let d1 = ab[0] * ap[0] + ab[1] * ap[1] + ab[2] * ap[2];
    let d2 = ac[0] * ap[0] + ac[1] * ap[1] + ac[2] * ap[2];
    if d1 <= 0.0 && d2 <= 0.0 {
        let d = ap[0] * ap[0] + ap[1] * ap[1] + ap[2] * ap[2];
        return (d, [1.0, 0.0, 0.0]);
    }
    let bp = [p[0] - b[0], p[1] - b[1], p[2] - b[2]];
    let d3 = ab[0] * bp[0] + ab[1] * bp[1] + ab[2] * bp[2];
    let d4 = ac[0] * bp[0] + ac[1] * bp[1] + ac[2] * bp[2];
    if d3 >= 0.0 && d4 <= d3 {
        let d = bp[0] * bp[0] + bp[1] * bp[1] + bp[2] * bp[2];
        return (d, [0.0, 1.0, 0.0]);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        let q = [a[0] + ab[0] * v, a[1] + ab[1] * v, a[2] + ab[2] * v];
        let d = (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2);
        return (d, [1.0 - v, v, 0.0]);
    }
    let cp = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
    let d5 = ab[0] * cp[0] + ab[1] * cp[1] + ab[2] * cp[2];
    let d6 = ac[0] * cp[0] + ac[1] * cp[1] + ac[2] * cp[2];
    if d6 >= 0.0 && d5 <= d6 {
        let d = cp[0] * cp[0] + cp[1] * cp[1] + cp[2] * cp[2];
        return (d, [0.0, 0.0, 1.0]);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        let q = [a[0] + ac[0] * w, a[1] + ac[1] * w, a[2] + ac[2] * w];
        let d = (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2);
        return (d, [1.0 - w, 0.0, w]);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        let q = [
            b[0] + (c[0] - b[0]) * w,
            b[1] + (c[1] - b[1]) * w,
            b[2] + (c[2] - b[2]) * w,
        ];
        let d = (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2);
        return (d, [0.0, 1.0 - w, w]);
    }
    let denom = (va + vb + vc).max(1e-20);
    let v = vb / denom;
    let w = vc / denom;
    let u = 1.0 - v - w;
    let q = [
        a[0] * u + b[0] * v + c[0] * w,
        a[1] * u + b[1] * v + c[1] * w,
        a[2] * u + b[2] * v + c[2] * w,
    ];
    let d = (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2);
    (d, [u, v, w])
}
