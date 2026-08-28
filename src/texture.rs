//! Decode source images (JPEG at IDCT scale when huge); crop + resize + JPEG per leaf.

use std::collections::{HashMap, VecDeque};
use std::io::Cursor;

use image::imageops::{self, FilterType};
use image::{DynamicImage, RgbaImage};

use crate::error::Error;
use crate::mesh::EncodedImage;

const UV_PAD_PX: u32 = 8;
const JPEG_QUALITY: u8 = 92;

/// Decode, downsampling so the long edge is at most `8 * tile_size`.
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
    tile_size.max(1).saturating_mul(8).max(512)
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

/// Leaf: tight UV-AABB crop of this primitive’s tris, flood-fill holes, remap UVs.
pub fn crop_leaf(
    image: &RgbaImage,
    uvs: &[[f32; 2]],
    indices: &[u32],
    tile_size: u32,
) -> Result<(Vec<u8>, Vec<[f32; 2]>), Error> {
    if indices.len() < 3 {
        return Err(Error::msg("no triangles for crop"));
    }
    let (w, h) = (image.width().max(1), image.height().max(1));
    let size = tile_size.max(64);

    let mut u0 = f32::INFINITY;
    let mut v0 = f32::INFINITY;
    let mut u1 = f32::NEG_INFINITY;
    let mut v1 = f32::NEG_INFINITY;
    for &vi in indices {
        let uv = uvs.get(vi as usize).copied().unwrap_or([0.0, 0.0]);
        let u = uv[0].clamp(0.0, 1.0);
        let v = uv[1].clamp(0.0, 1.0);
        u0 = u0.min(u);
        v0 = v0.min(v);
        u1 = u1.max(u);
        v1 = v1.max(v);
    }
    if !u0.is_finite() {
        return Err(Error::msg("crop produced no UVs"));
    }
    u1 = u1.max(u0 + 1e-6);
    v1 = v1.max(v0 + 1e-6);
    let mut x0 = ((u0 * w as f32).floor() as i32) - UV_PAD_PX as i32;
    let mut y0 = ((v0 * h as f32).floor() as i32) - UV_PAD_PX as i32;
    let mut x1 = ((u1 * w as f32).ceil() as i32) + UV_PAD_PX as i32;
    let mut y1 = ((v1 * h as f32).ceil() as i32) + UV_PAD_PX as i32;
    x0 = x0.clamp(0, w as i32 - 1);
    y0 = y0.clamp(0, h as i32 - 1);
    x1 = x1.clamp(x0 + 1, w as i32);
    y1 = y1.clamp(y0 + 1, h as i32);
    let cropped = imageops::crop_imm(
        image,
        x0 as u32,
        y0 as u32,
        (x1 - x0) as u32,
        (y1 - y0) as u32,
    )
    .to_image();
    let u_org = x0 as f32 / w as f32;
    let v_org = y0 as f32 / h as f32;
    let u_span = (x1 - x0) as f32 / w as f32;
    let v_span = (y1 - y0) as f32 / h as f32;

    let (rw, rh) = fit(cropped.width(), cropped.height(), size);
    let mut resized = if rw == cropped.width() && rh == cropped.height() {
        cropped
    } else {
        imageops::resize(&cropped, rw, rh, FilterType::Triangle)
    };
    let remapped: Vec<[f32; 2]> = uvs
        .iter()
        .map(|uv| {
            [
                if u_span > 1e-8 {
                    ((uv[0] - u_org) / u_span).clamp(0.0, 1.0)
                } else {
                    0.5
                },
                if v_span > 1e-8 {
                    ((uv[1] - v_org) / v_span).clamp(0.0, 1.0)
                } else {
                    0.5
                },
            ]
        })
        .collect();
    fill_uncovered(&mut resized, &remapped, indices);
    Ok((encode_jpeg(&resized)?, remapped))
}

/// Parent: downscale the source image; UVs stay 0–1 of that image.
pub fn downscale_jpeg(image: &RgbaImage, tile_size: u32) -> Result<Vec<u8>, Error> {
    let size = tile_size.max(64);
    let (rw, rh) = fit(image.width().max(1), image.height().max(1), size);
    let resized = if rw == image.width() && rh == image.height() {
        image.clone()
    } else {
        imageops::resize(image, rw, rh, FilterType::Triangle)
    };
    encode_jpeg(&resized)
}

/// Unique-UV **chart** unwrap of the simplified mesh, then per-texel sample of
/// the high-res source. Adjacent proxy faces that share a plane share UVs;
/// colour is looked up on the source surface, never by stretching source UVs
/// across a simplified triangle.
pub fn bake_simplified(
    simplified: &crate::glb_write::TilePrimitive,
    sampler: &SceneSampler<'_>,
    atlas_size: u32,
) -> Result<crate::glb_write::TilePrimitive, Error> {
    let size = atlas_size.max(64);
    let unwrapped = chart_unwrap(simplified, size)?;
    let mut atlas = RgbaImage::from_pixel(size, size, image::Rgba([128, 128, 128, 255]));
    let denom = (size - 1) as f32;
    let has_n = unwrapped.normals.len() == unwrapped.positions.len();
    let mut candidates = Vec::new();
    let mut seen: HashMap<u32, ()> = HashMap::new();

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
        let na = if has_n {
            unwrapped.normals[ia]
        } else {
            [0.0, 1.0, 0.0]
        };
        let nb = if has_n {
            unwrapped.normals[ib]
        } else {
            [0.0, 1.0, 0.0]
        };
        let nc = if has_n {
            unwrapped.normals[ic]
        } else {
            [0.0, 1.0, 0.0]
        };
        sampler.gather_surface(pa, pb, pc, &mut candidates, &mut seen);
        let local_cell = sampler.cell * 0.5;
        let local = if candidates.len() > 64 {
            sampler.local_index(&candidates, local_cell)
        } else {
            HashMap::new()
        };
        let mut nearby = Vec::new();

        let px = |t: [f32; 2]| -> (i32, i32) {
            ((t[0] * denom).floor() as i32, (t[1] * denom).floor() as i32)
        };
        let (x0, y0) = px(ua);
        let (x1, y1) = px(ub);
        let (x2, y2) = px(uc);
        let min_x = x0.min(x1).min(x2).max(0);
        let max_x = x0.max(x1).max(x2).min(size as i32 - 1);
        let min_y = y0.min(y1).min(y2).max(0);
        let max_y = y0.max(y1).max(y2).min(size as i32 - 1);
        if min_x > max_x || min_y > max_y {
            continue;
        }
        for py in min_y..=max_y {
            for px in min_x..=max_x {
                let q = [px as f32 / denom, py as f32 / denom];
                let (b0, b1, b2) = bary_uv(ua, ub, uc, q);
                if b0 < -1e-3 || b1 < -1e-3 || b2 < -1e-3 {
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
                let rgb = if local.is_empty() {
                    sampler.sample_among(pos, nrm, &candidates)
                } else {
                    sampler.sample_local(pos, nrm, &local, local_cell, &mut nearby)
                };
                atlas.put_pixel(
                    px as u32,
                    py as u32,
                    image::Rgba([rgb[0], rgb[1], rgb[2], 255]),
                );
            }
        }
    }

    fill_uncovered(&mut atlas, &unwrapped.uvs, &unwrapped.indices);
    Ok(crate::glb_write::TilePrimitive {
        positions: unwrapped.positions,
        normals: unwrapped.normals,
        uvs: unwrapped.uvs,
        indices: unwrapped.indices,
        jpeg: Some(encode_jpeg(&atlas)?),
    })
}

/// Grow charts by connectivity + normal, project each to its plane, pack.
/// Vertices are duplicated only on chart seams, not per triangle.
fn chart_unwrap(
    mesh: &crate::glb_write::TilePrimitive,
    atlas_size: u32,
) -> Result<crate::glb_write::TilePrimitive, Error> {
    let ntri = mesh.indices.len() / 3;
    if ntri == 0 {
        return Err(Error::msg("empty simplified mesh"));
    }
    let has_n = mesh.normals.len() == mesh.positions.len();
    let mut face_n = vec![[0.0f32; 3]; ntri];
    let mut face_area = vec![0.0f32; ntri];
    for (fi, tri) in mesh.indices.chunks_exact(3).enumerate() {
        let (n, a) = face_normal_area(
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
    let mut used = vec![false; ntri];
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
    let mut charts = Vec::new();
    for seed in order {
        if used[seed] {
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
        used[seed] = true;
        let mut faces = vec![seed];
        let mut q = VecDeque::new();
        q.push_back(seed);
        while let Some(fi) = q.pop_front() {
            for &nb in &adj[fi] {
                if used[nb] {
                    continue;
                }
                if dot3(face_n[nb], n0) < MIN_DOT {
                    continue;
                }
                let t = &mesh.indices[nb * 3..nb * 3 + 3];
                let a = project2(mesh.positions[t[0] as usize], origin, tangent, bitangent);
                let b = project2(mesh.positions[t[1] as usize], origin, tangent, bitangent);
                let c = project2(mesh.positions[t[2] as usize], origin, tangent, bitangent);
                if (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]) < 0.0 {
                    continue;
                }
                used[nb] = true;
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

    struct Layout {
        faces: Vec<usize>,
        xy: HashMap<u32, [f32; 2]>,
        wm: f32,
        hm: f32,
    }
    let mut layouts = Vec::with_capacity(charts.len());
    for ch in &charts {
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

    let pad = 2u32;
    let total_area: f32 = layouts.iter().map(|l| l.wm * l.hm).sum();
    let mut scale = ((atlas_size * atlas_size) as f32 * 0.82 / total_area.max(1e-8))
        .sqrt()
        .min(64.0);
    let mut rects = vec![(0u32, 0u32, 0u32, 0u32); layouts.len()];
    for _ in 0..40 {
        let mut order: Vec<usize> = (0..layouts.len()).collect();
        order.sort_by(|&a, &b| {
            let ha = (layouts[a].hm * scale).ceil() as u32;
            let hb = (layouts[b].hm * scale).ceil() as u32;
            hb.cmp(&ha)
        });
        let mut shelf_y = 0u32;
        let mut shelf_h = 0u32;
        let mut shelf_x = 0u32;
        let mut ok = true;
        for &i in &order {
            let bw = ((layouts[i].wm * scale).ceil() as u32)
                .max(pad * 2 + 4)
                .min(atlas_size);
            let bh = ((layouts[i].hm * scale).ceil() as u32)
                .max(pad * 2 + 4)
                .min(atlas_size);
            if shelf_x + bw > atlas_size {
                shelf_y = shelf_y.saturating_add(shelf_h);
                shelf_x = 0;
                shelf_h = 0;
            }
            if shelf_y + bh > atlas_size {
                ok = false;
                break;
            }
            rects[i] = (shelf_x, shelf_y, bw, bh);
            shelf_x += bw;
            shelf_h = shelf_h.max(bh);
        }
        if ok {
            break;
        }
        scale *= 0.85;
    }

    let denom = (atlas_size - 1) as f32;
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
        for (&vi, &pxy) in &layout.xy {
            let id = positions.len() as u32;
            remap.insert(vi, id);
            positions.push(mesh.positions[vi as usize]);
            normals.push(if has_n {
                mesh.normals[vi as usize]
            } else {
                [0.0, 1.0, 0.0]
            });
            uvs.push([
                (ox as f32 + (pxy[0] / layout.wm) * (iw - 1) as f32) / denom,
                (oy as f32 + (pxy[1] / layout.hm) * (ih - 1) as f32) / denom,
            ]);
        }
        for &fi in &layout.faces {
            let t = &mesh.indices[fi * 3..fi * 3 + 3];
            indices.push(remap[&t[0]]);
            indices.push(remap[&t[1]]);
            indices.push(remap[&t[2]]);
        }
    }

    Ok(crate::glb_write::TilePrimitive {
        positions,
        normals: if has_n { normals } else { Vec::new() },
        uvs,
        indices,
        jpeg: None,
    })
}

fn face_normal_area(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> ([f32; 3], f32) {
    let e0 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let e1 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        e0[1] * e1[2] - e0[2] * e1[1],
        e0[2] * e1[0] - e0[0] * e1[2],
        e0[0] * e1[1] - e0[1] * e1[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len < 1e-20 {
        ([0.0, 1.0, 0.0], 0.0)
    } else {
        ([n[0] / len, n[1] / len, n[2] / len], len * 0.5)
    }
}

fn plane_basis(n: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let up = if n[1].abs() < 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let t = [
        up[1] * n[2] - up[2] * n[1],
        up[2] * n[0] - up[0] * n[2],
        up[0] * n[1] - up[1] * n[0],
    ];
    let tl = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt().max(1e-20);
    let t = [t[0] / tl, t[1] / tl, t[2] / tl];
    let b = [
        n[1] * t[2] - n[2] * t[1],
        n[2] * t[0] - n[0] * t[2],
        n[0] * t[1] - n[1] * t[0],
    ];
    (t, b)
}

fn project2(p: [f32; 3], origin: [f32; 3], t: [f32; 3], b: [f32; 3]) -> [f32; 2] {
    let d = [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]];
    [
        d[0] * t[0] + d[1] * t[1] + d[2] * t[2],
        d[0] * b[0] + d[1] * b[1] + d[2] * b[2],
    ]
}

fn centroid3(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    [
        (a[0] + b[0] + c[0]) / 3.0,
        (a[1] + b[1] + c[1]) / 3.0,
        (a[2] + b[2] + c[2]) / 3.0,
    ]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

fn tri_aabb(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    (
        [
            a[0].min(b[0]).min(c[0]),
            a[1].min(b[1]).min(c[1]),
            a[2].min(b[2]).min(c[2]),
        ],
        [
            a[0].max(b[0]).max(c[0]),
            a[1].max(b[1]).max(c[1]),
            a[2].max(b[2]).max(c[2]),
        ],
    )
}

fn fill_uncovered(img: &mut RgbaImage, uvs: &[[f32; 2]], indices: &[u32]) {
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
    let mut q = VecDeque::new();
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if cover[i] {
                continue;
            }
            let mut has_n = false;
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
                    if cover[(ny as u32 * w + nx as u32) as usize] {
                        has_n = true;
                    }
                }
            }
            if has_n {
                q.push_back((x, y));
            }
        }
    }
    while let Some((x, y)) = q.pop_front() {
        let i = (y * w + x) as usize;
        if cover[i] {
            continue;
        }
        let mut rgb = [0u32; 3];
        let mut n = 0u32;
        let mut nbrs = Vec::new();
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
                if cover[j] {
                    let p = img.get_pixel(nx as u32, ny as u32).0;
                    rgb[0] += p[0] as u32;
                    rgb[1] += p[1] as u32;
                    rgb[2] += p[2] as u32;
                    n += 1;
                } else {
                    nbrs.push((nx as u32, ny as u32));
                }
            }
        }
        if n == 0 {
            continue;
        }
        img.put_pixel(
            x,
            y,
            image::Rgba([
                (rgb[0] / n) as u8,
                (rgb[1] / n) as u8,
                (rgb[2] / n) as u8,
                255,
            ]),
        );
        cover[i] = true;
        for (nx, ny) in nbrs {
            if !cover[(ny * w + nx) as usize] {
                q.push_back((nx, ny));
            }
        }
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

/// Pack each material’s JPEG into one atlas and concatenate geometry with remapped UVs.
/// Does not weld — the caller welds by position so 3D-adjacent materials join.
pub fn pack_parent_charts(
    prims: &[crate::glb_write::TilePrimitive],
    atlas_size: u32,
) -> Result<(Vec<u8>, crate::glb_write::TilePrimitive), Error> {
    let size = atlas_size.max(64);
    let mut charts = Vec::new();
    for p in prims {
        let Some(jpeg) = &p.jpeg else { continue };
        if p.uvs.len() != p.positions.len() || p.indices.len() < 3 {
            continue;
        }
        let rgba = image::load_from_memory(jpeg)
            .map_err(|e| Error::msg(format!("parent chart jpeg: {e}")))?
            .to_rgba8();
        charts.push((p, rgba));
    }
    if charts.is_empty() {
        return Err(Error::msg("no textured charts to pack"));
    }

    let pad = 2u32;
    let mut order: Vec<usize> = (0..charts.len()).collect();
    order.sort_by(|&a, &b| charts[b].1.height().cmp(&charts[a].1.height()));

    let mut scale = 1.0f32;
    let mut rects = Vec::new();
    for _ in 0..24 {
        rects = vec![(0u32, 0u32, 0u32, 0u32); charts.len()];
        let mut shelf_y = 0u32;
        let mut shelf_h = 0u32;
        let mut shelf_x = 0u32;
        let mut ok = true;
        for &i in &order {
            let (w0, h0) = (charts[i].1.width(), charts[i].1.height());
            let w = ((w0 as f32 * scale).ceil() as u32)
                .max(pad * 2 + 1)
                .min(size);
            let h = ((h0 as f32 * scale).ceil() as u32)
                .max(pad * 2 + 1)
                .min(size);
            if shelf_x + w > size {
                shelf_y = shelf_y.saturating_add(shelf_h);
                shelf_x = 0;
                shelf_h = 0;
            }
            if shelf_y + h > size {
                ok = false;
                break;
            }
            rects[i] = (shelf_x, shelf_y, w, h);
            shelf_x += w;
            shelf_h = shelf_h.max(h);
        }
        if ok {
            break;
        }
        scale *= 0.85;
    }

    let mut atlas = RgbaImage::from_pixel(size, size, image::Rgba([128, 128, 128, 255]));
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    let mut any_n = false;

    for (i, (p, rgba)) in charts.iter().enumerate() {
        let (x, y, w, h) = rects[i];
        let iw = w.saturating_sub(pad * 2).max(1);
        let ih = h.saturating_sub(pad * 2).max(1);
        let resized = if rgba.width() == iw && rgba.height() == ih {
            rgba.clone()
        } else {
            imageops::resize(rgba, iw, ih, FilterType::Triangle)
        };
        let ox = x + pad;
        let oy = y + pad;
        for py in 0..ih {
            for px in 0..iw {
                atlas.put_pixel(ox + px, oy + py, *resized.get_pixel(px, py));
            }
        }
        let base = positions.len() as u32;
        let has_n = p.normals.len() == p.positions.len();
        any_n |= has_n;
        for vi in 0..p.positions.len() {
            positions.push(p.positions[vi]);
            normals.push(if has_n {
                p.normals[vi]
            } else {
                [0.0, 1.0, 0.0]
            });
            let u = p.uvs[vi][0].clamp(0.0, 1.0);
            let v = p.uvs[vi][1].clamp(0.0, 1.0);
            uvs.push([
                (ox as f32 + u * (iw - 1) as f32) / (size - 1) as f32,
                (oy as f32 + v * (ih - 1) as f32) / (size - 1) as f32,
            ]);
        }
        indices.extend(p.indices.iter().map(|i| i + base));
    }

    Ok((
        encode_jpeg(&atlas)?,
        crate::glb_write::TilePrimitive {
            positions,
            normals: if any_n { normals } else { Vec::new() },
            uvs,
            indices,
            jpeg: None,
        },
    ))
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

pub struct SceneSampler<'a> {
    kind: SamplerKind<'a>,
    bins: HashMap<(i32, i32, i32), Vec<u32>>,
    cell: f32,
}

enum SamplerKind<'a> {
    Scene {
        scene: &'a crate::mesh::Scene,
        images: &'a HashMap<u32, RgbaImage>,
    },
    Packed {
        images: Vec<RgbaImage>,
        tris: Vec<PackedTri>,
    },
}

struct PackedTri {
    img: u32,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    ua: [f32; 2],
    ub: [f32; 2],
    uc: [f32; 2],
}

struct SampleView<'b> {
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    ua: [f32; 2],
    ub: [f32; 2],
    uc: [f32; 2],
    image: &'b RgbaImage,
}

impl<'a> SceneSampler<'a> {
    pub fn from_scene(
        scene: &'a crate::mesh::Scene,
        images: &'a HashMap<u32, RgbaImage>,
    ) -> Result<Self, Error> {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        let mut ids = Vec::new();
        for (i, t) in scene.triangles.iter().enumerate() {
            let Some(img) = t.image else { continue };
            if !images.contains_key(&img) {
                continue;
            }
            ids.push(i as u32);
            for &vi in &t.verts {
                let p = scene.vertices[vi as usize].pos;
                for k in 0..3 {
                    min[k] = min[k].min(p[k]);
                    max[k] = max[k].max(p[k]);
                }
            }
        }
        if ids.is_empty() {
            return Err(Error::msg("no textured triangles to bake from"));
        }
        let extent = (max[0] - min[0])
            .max(max[1] - min[1])
            .max(max[2] - min[2])
            .max(1e-3);
        let cell = extent / 256.0;
        let mut bins: HashMap<(i32, i32, i32), Vec<u32>> = HashMap::new();
        for &i in &ids {
            let t = &scene.triangles[i as usize];
            let a = scene.vertices[t.verts[0] as usize].pos;
            let b = scene.vertices[t.verts[1] as usize].pos;
            let c = scene.vertices[t.verts[2] as usize].pos;
            insert_tri_bins(&mut bins, cell, i, a, b, c);
        }
        Ok(Self {
            kind: SamplerKind::Scene { scene, images },
            bins,
            cell,
        })
    }

    pub fn from_prims(
        prims: &[crate::glb_write::TilePrimitive],
    ) -> Result<SceneSampler<'static>, Error> {
        let mut images = Vec::new();
        let mut tris = Vec::new();
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
            for tri in p.indices.chunks_exact(3) {
                let ia = tri[0] as usize;
                let ib = tri[1] as usize;
                let ic = tri[2] as usize;
                tris.push(PackedTri {
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
        let cell = extent / 256.0;
        let mut bins: HashMap<(i32, i32, i32), Vec<u32>> = HashMap::new();
        for (i, t) in tris.iter().enumerate() {
            insert_tri_bins(&mut bins, cell, i as u32, t.a, t.b, t.c);
        }
        Ok(SceneSampler {
            kind: SamplerKind::Packed { images, tris },
            bins,
            cell,
        })
    }

    fn local_index(&self, candidates: &[u32], cell: f32) -> HashMap<(i32, i32, i32), Vec<u32>> {
        let mut local: HashMap<(i32, i32, i32), Vec<u32>> = HashMap::new();
        for &ti in candidates {
            let Some(t) = self.view(ti) else { continue };
            let c = centroid3(t.a, t.b, t.c);
            local
                .entry((
                    (c[0] / cell).floor() as i32,
                    (c[1] / cell).floor() as i32,
                    (c[2] / cell).floor() as i32,
                ))
                .or_default()
                .push(ti);
        }
        local
    }

    fn sample_local(
        &self,
        p: [f32; 3],
        nrm: [f32; 3],
        local: &HashMap<(i32, i32, i32), Vec<u32>>,
        cell: f32,
        scratch: &mut Vec<u32>,
    ) -> [u8; 3] {
        let cx = (p[0] / cell).floor() as i32;
        let cy = (p[1] / cell).floor() as i32;
        let cz = (p[2] / cell).floor() as i32;
        for r in [1, 2, 3, 5] {
            scratch.clear();
            for dz in -r..=r {
                for dy in -r..=r {
                    for dx in -r..=r {
                        if let Some(bin) = local.get(&(cx + dx, cy + dy, cz + dz)) {
                            scratch.extend_from_slice(bin);
                        }
                    }
                }
            }
            if !scratch.is_empty() {
                return self.sample_among(p, nrm, scratch);
            }
        }
        [128, 128, 128]
    }

    /// Source triangles whose cells overlap this proxy face (not its AABB
    /// volume — that pulls in the opposite cave wall).
    fn gather_surface(
        &self,
        pa: [f32; 3],
        pb: [f32; 3],
        pc: [f32; 3],
        out: &mut Vec<u32>,
        seen: &mut HashMap<u32, ()>,
    ) {
        out.clear();
        seen.clear();
        let e0 = dist3(pa, pb);
        let e1 = dist3(pb, pc);
        let e2 = dist3(pc, pa);
        let longest = e0.max(e1).max(e2).max(self.cell);
        let n = ((longest / self.cell).ceil() as i32).clamp(1, 48);
        let nf = n as f32;
        for i in 0..=n {
            for j in 0..=(n - i) {
                let b1 = i as f32 / nf;
                let b2 = j as f32 / nf;
                let b0 = 1.0 - b1 - b2;
                let p = [
                    pa[0] * b0 + pb[0] * b1 + pc[0] * b2,
                    pa[1] * b0 + pb[1] * b1 + pc[1] * b2,
                    pa[2] * b0 + pb[2] * b1 + pc[2] * b2,
                ];
                let cx = (p[0] / self.cell).floor() as i32;
                let cy = (p[1] / self.cell).floor() as i32;
                let cz = (p[2] / self.cell).floor() as i32;
                for dz in -1..=1 {
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            let Some(bin) = self.bins.get(&(cx + dx, cy + dy, cz + dz)) else {
                                continue;
                            };
                            for &ti in bin {
                                if seen.insert(ti, ()).is_none() {
                                    out.push(ti);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn view(&self, i: u32) -> Option<SampleView<'_>> {
        match &self.kind {
            SamplerKind::Scene { scene, images } => {
                let t = scene.triangles.get(i as usize)?;
                let img = images.get(&t.image?)?;
                let va = scene.vertices[t.verts[0] as usize];
                let vb = scene.vertices[t.verts[1] as usize];
                let vc = scene.vertices[t.verts[2] as usize];
                Some(SampleView {
                    a: va.pos,
                    b: vb.pos,
                    c: vc.pos,
                    ua: va.uv,
                    ub: vb.uv,
                    uc: vc.uv,
                    image: img,
                })
            }
            SamplerKind::Packed { images, tris } => {
                let t = tris.get(i as usize)?;
                let img = images.get(t.img as usize)?;
                Some(SampleView {
                    a: t.a,
                    b: t.b,
                    c: t.c,
                    ua: t.ua,
                    ub: t.ub,
                    uc: t.uc,
                    image: img,
                })
            }
        }
    }

    fn sample_among(&self, p: [f32; 3], nrm: [f32; 3], candidates: &[u32]) -> [u8; 3] {
        let mut best_d2 = f32::INFINITY;
        let mut best_i = None;
        let mut best_bary = [0.0f32; 3];
        let mut best_facing_d2 = f32::INFINITY;
        let mut best_facing_i = None;
        let mut best_facing_bary = [0.0f32; 3];
        let nl = (nrm[0] * nrm[0] + nrm[1] * nrm[1] + nrm[2] * nrm[2])
            .sqrt()
            .max(1e-20);
        let n = [nrm[0] / nl, nrm[1] / nl, nrm[2] / nl];
        for &ti in candidates {
            let Some(t) = self.view(ti) else { continue };
            let (d2, bary) = point_tri_dist2_bary(p, t.a, t.b, t.c);
            let (tn, _) = face_normal_area(t.a, t.b, t.c);
            if d2 < best_d2 {
                best_d2 = d2;
                best_i = Some(ti);
                best_bary = bary;
            }
            if dot3(n, tn) > 0.15 && d2 < best_facing_d2 {
                best_facing_d2 = d2;
                best_facing_i = Some(ti);
                best_facing_bary = bary;
            }
        }
        let (ti, bary) = if let Some(i) = best_facing_i {
            if best_facing_d2 <= best_d2 * 8.0 + self.cell * self.cell {
                (i, best_facing_bary)
            } else if let Some(j) = best_i {
                (j, best_bary)
            } else {
                return [128, 128, 128];
            }
        } else if let Some(j) = best_i {
            (j, best_bary)
        } else {
            return [128, 128, 128];
        };
        let Some(t) = self.view(ti) else {
            return [128, 128, 128];
        };
        let u = t.ua[0] * bary[0] + t.ub[0] * bary[1] + t.uc[0] * bary[2];
        let v = t.ua[1] * bary[0] + t.ub[1] * bary[1] + t.uc[1] * bary[2];
        sample_rgba(t.image, u, v)
    }
}

fn insert_tri_bins(
    bins: &mut HashMap<(i32, i32, i32), Vec<u32>>,
    cell: f32,
    i: u32,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
) {
    let (mn, mx) = tri_aabb(a, b, c);
    let x0 = (mn[0] / cell).floor() as i32;
    let y0 = (mn[1] / cell).floor() as i32;
    let z0 = (mn[2] / cell).floor() as i32;
    let x1 = (mx[0] / cell).floor() as i32;
    let y1 = (mx[1] / cell).floor() as i32;
    let z1 = (mx[2] / cell).floor() as i32;
    for z in z0..=z1 {
        for y in y0..=y1 {
            for x in x0..=x1 {
                bins.entry((x, y, z)).or_default().push(i);
            }
        }
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

fn point_tri_dist2_bary(p: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> (f32, [f32; 3]) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glb_write::TilePrimitive;

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
        let out = chart_unwrap(&mesh, 128).unwrap();
        assert_eq!(out.indices.len() / 3, src_tris);
        assert!(
            out.positions.len() <= src_verts + 4,
            "coplanar grid should stay one chart with shared verts, got {} (src {src_verts})",
            out.positions.len()
        );
    }
}
