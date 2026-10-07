//! Parent proxies, built bottom-up: weld + meshopt-simplify the **children's
//! proxies** (never the raw scene), bake a new atlas from the children's
//! atlases, and measure a two-sided sampled surface error for `geometricError`.

use std::io::Cursor;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use meshopt::optimize::optimize_vertex_fetch;
use meshopt::simplify::{simplify, simplify_with_attributes_and_locks, SimplifyOptions};
use meshopt::utilities::VertexDataAdapter;
use rayon::prelude::*;

use crate::error::Error;
use crate::glb_write::TilePrimitive;
use crate::grid::{IdMap, TriGrid};

const ERROR_SAMPLE_CAP: usize = 4_096;
const MIN_PARENT_ATLAS: u32 = 256;

/// CPU-microseconds per stage across all parents (for the stage log).
pub struct Timing {
    pub simplify: AtomicU64,
    pub bake: AtomicU64,
    pub error: AtomicU64,
    /// Inside `bake`: chart unwrap, sampler build (child decode + grid),
    /// texel rasterisation, fill + encode.
    pub unwrap: AtomicU64,
    pub sampler: AtomicU64,
    pub raster: AtomicU64,
    pub encode: AtomicU64,
}

pub static TIMING: Timing = Timing {
    simplify: AtomicU64::new(0),
    bake: AtomicU64::new(0),
    error: AtomicU64::new(0),
    unwrap: AtomicU64::new(0),
    sampler: AtomicU64::new(0),
    raster: AtomicU64::new(0),
    encode: AtomicU64::new(0),
};

impl Timing {
    pub fn add(counter: &AtomicU64, since: Instant) {
        counter.fetch_add(since.elapsed().as_micros() as u64, Ordering::Relaxed);
    }

    /// CPU seconds summed over threads.
    pub fn summary(&self) -> String {
        let s = |a: &AtomicU64| a.load(Ordering::Relaxed) as f64 / 1e6;
        format!(
            "cpu simplify={:.1}s bake={:.1}s(unwrap={:.1} sampler={:.1} raster={:.1} encode={:.1}) error={:.1}s",
            s(&self.simplify),
            s(&self.bake),
            s(&self.unwrap),
            s(&self.sampler),
            s(&self.raster),
            s(&self.encode),
            s(&self.error)
        )
    }
}

/// One REPLACE level halves linear resolution: a quarter of the children's
/// triangles, capped at the leaf budget.
pub fn parent_triangle_budget(children_tris: usize, max_triangles: usize) -> usize {
    let cap = max_triangles.max(1);
    (children_tris / 4).clamp(256.min(cap), cap)
}

/// Parent atlas edge: a quarter of the children's texels, rounded up to a
/// power of two, clamped to `[256, tile_size]`.
pub fn parent_atlas_size(children: &[TilePrimitive], tile_size: u32) -> u32 {
    let texels: u64 = children
        .iter()
        .filter_map(|p| p.jpeg.as_ref())
        .filter_map(|j| image_dims(j))
        .map(|(w, h)| w as u64 * h as u64)
        .sum();
    let want = ((texels as f64 / 4.0).sqrt().ceil() as u32).max(1);
    let mut px = MIN_PARENT_ATLAS;
    while px < want && px < tile_size {
        px *= 2;
    }
    px.clamp(MIN_PARENT_ATLAS.min(tile_size.max(64)), tile_size.max(64))
}

fn image_dims(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

pub struct ParentResult {
    pub prims: Vec<TilePrimitive>,
    /// Two-sided sampled surface distance parent ↔ children, metres.
    pub error_m: f64,
    /// Metres per parent texel (0 when untextured).
    pub texel_m: f64,
}

/// Build one parent from its children's proxies.
pub fn build_parent(
    children: &[TilePrimitive],
    max_triangles: usize,
    tile_size: u32,
) -> Result<ParentResult, Error> {
    if children.is_empty() {
        return Err(Error::msg("parent with no children"));
    }
    let mut merged = TilePrimitive::default();
    let mut any_n = false;
    for p in children {
        let base = merged.positions.len() as u32;
        let has_n = p.normals.len() == p.positions.len();
        any_n |= has_n;
        for i in 0..p.positions.len() {
            merged.positions.push(p.positions[i]);
            merged
                .normals
                .push(if has_n { p.normals[i] } else { [0.0, 1.0, 0.0] });
        }
        merged.indices.extend(p.indices.iter().map(|i| i + base));
    }
    if !any_n {
        merged.normals.clear();
    }
    let budget = parent_triangle_budget(merged.indices.len() / 3, max_triangles);
    let t0 = Instant::now();
    let mut simplified = simplify_primitive(&merged, budget)?;
    simplified.uvs.clear();
    simplified.jpeg = None;
    Timing::add(&TIMING.simplify, t0);

    let textured = children
        .iter()
        .any(|p| p.jpeg.is_some() && p.uvs.len() == p.positions.len());
    let t0 = Instant::now();
    let (prims, texel_m) = if textured {
        let atlas = parent_atlas_size(children, tile_size);
        match crate::texture::bake_simplified(&simplified, children, atlas) {
            Ok((p, texel_m)) => (vec![p], texel_m),
            Err(e @ Error::TextureProjection { .. }) => {
                // Some non-manifold inputs cannot be safely reprojected after
                // simplification. Retain the textured child surfaces rather
                // than paint the opposite wall or discard their appearance.
                eprintln!("mesh-to-3tz: retaining child surfaces: {e}");
                let texel_m = children
                    .iter()
                    .filter_map(|p| {
                        p.jpeg
                            .as_ref()
                            .and_then(|j| image_dims(j))
                            .map(|(w, h)| crate::tile::max_texel_size(p, w, h))
                    })
                    .fold(0.0f64, f64::max);
                return Ok(ParentResult {
                    prims: children.to_vec(),
                    error_m: 0.0,
                    texel_m,
                });
            }
            Err(e) => return Err(e),
        }
    } else {
        (vec![simplified], 0.0)
    };
    Timing::add(&TIMING.bake, t0);
    let t0 = Instant::now();
    let error_m = two_sided_error(&prims, children);
    Timing::add(&TIMING.error, t0);
    Ok(ParentResult {
        prims,
        error_m,
        texel_m,
    })
}

#[derive(Clone, Copy, Default)]
#[repr(C)]
struct Vtx {
    p: [f32; 3],
    n: [f32; 3],
    t: [f32; 2],
}

/// Weld by position, then simplify toward `target_tris` preferring a
/// watertight result over hitting the budget. Never `simplify_sloppy`
/// (it punches holes that show up as shattered parent tiles).
pub fn simplify_primitive(
    prim: &TilePrimitive,
    target_tris: usize,
) -> Result<TilePrimitive, Error> {
    if prim.indices.len() < 3 || prim.positions.is_empty() {
        return Ok(prim.clone());
    }
    let welded = weld_by_position(prim);
    if welded.indices.len() < 3 {
        return Ok(prim.clone());
    }
    let target_indices = (target_tris.max(1) * 3).min(welded.indices.len());
    if target_indices >= welded.indices.len() {
        return Ok(welded);
    }
    let pos_bytes: &[u8] = bytemuck::cast_slice(&welded.positions);
    let adapter = VertexDataAdapter::new(pos_bytes, 12, 0)
        .map_err(|e| Error::msg(format!("meshopt adapter: {e}")))?;
    let simplified = reduce_indices(&welded.indices, &adapter, target_indices);
    if simplified.len() < 3 {
        return Ok(welded);
    }

    let has_n = welded.normals.len() == welded.positions.len();
    let has_uv = welded.uvs.len() == welded.positions.len();
    let packed: Vec<Vtx> = (0..welded.positions.len())
        .map(|i| Vtx {
            p: welded.positions[i],
            n: if has_n {
                welded.normals[i]
            } else {
                [0.0, 1.0, 0.0]
            },
            t: if has_uv { welded.uvs[i] } else { [0.0, 0.0] },
        })
        .collect();
    let mut indices = simplified;
    let compacted = optimize_vertex_fetch(&mut indices, &packed);
    Ok(TilePrimitive {
        positions: compacted.iter().map(|v| v.p).collect(),
        normals: if has_n {
            compacted.iter().map(|v| v.n).collect()
        } else {
            Vec::new()
        },
        uvs: if has_uv {
            compacted.iter().map(|v| v.t).collect()
        } else {
            Vec::new()
        },
        indices,
        jpeg: prim.jpeg.clone(),
    })
}

/// Weld vertices that share a position (manifold proxies; normals averaged,
/// first UV kept).
fn weld_by_position(prim: &TilePrimitive) -> TilePrimitive {
    let mut extent = 0.0f32;
    for p in &prim.positions {
        extent = extent.max(p[0].abs()).max(p[1].abs()).max(p[2].abs());
    }
    let cell = (extent * 1e-6).max(1e-5);
    let has_n = prim.normals.len() == prim.positions.len();
    let has_uv = prim.uvs.len() == prim.positions.len();

    let mut map: IdMap<[i64; 3], u32> = IdMap::default();
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut remap = vec![0u32; prim.positions.len()];
    for (i, &p) in prim.positions.iter().enumerate() {
        let k = [
            (p[0] / cell).round() as i64,
            (p[1] / cell).round() as i64,
            (p[2] / cell).round() as i64,
        ];
        if let Some(&id) = map.get(&k) {
            if has_n {
                let n = &mut normals[id as usize];
                n[0] += prim.normals[i][0];
                n[1] += prim.normals[i][1];
                n[2] += prim.normals[i][2];
            }
            remap[i] = id;
        } else {
            let id = positions.len() as u32;
            map.insert(k, id);
            positions.push(p);
            normals.push(if has_n {
                prim.normals[i]
            } else {
                [0.0, 1.0, 0.0]
            });
            uvs.push(if has_uv { prim.uvs[i] } else { [0.0, 0.0] });
            remap[i] = id;
        }
    }
    for n in &mut normals {
        *n = crate::grid::normalize(*n);
    }
    let mut out_idx = Vec::with_capacity(prim.indices.len());
    for tri in prim.indices.as_chunks::<3>().0 {
        let t = [
            remap[tri[0] as usize],
            remap[tri[1] as usize],
            remap[tri[2] as usize],
        ];
        if t[0] != t[1] && t[1] != t[2] && t[2] != t[0] {
            out_idx.extend_from_slice(&t);
        }
    }
    TilePrimitive {
        positions,
        normals: if has_n { normals } else { Vec::new() },
        uvs: if has_uv { uvs } else { Vec::new() },
        indices: out_idx,
        jpeg: None,
    }
}

/// Keep tile boundaries fixed so independently refined neighbours cannot open
/// cracks. A parent may exceed its triangle budget rather than move a wall by
/// a large fraction of the scene extent to force an arbitrary count.
fn reduce_indices(indices: &[u32], adapter: &VertexDataAdapter<'_>, target: usize) -> Vec<u32> {
    let mut best = indices.to_vec();
    for error in [0.005, 0.01, 0.03] {
        let (out, _) =
            simplify_border_locked(indices, adapter, target, error, SimplifyOptions::None, None);
        if out.len() >= 3 && out.len() < best.len() {
            best = out;
        }
        if best.len() <= target.saturating_mul(2) {
            break;
        }
    }
    best
}

/// Optional metric attributes and explicit vertex locks for simplification.
pub(crate) struct SimplificationAttributes<'a> {
    pub values: &'a [f32],
    pub weights: &'a [f32],
    pub stride: usize,
    pub locks: &'a [bool],
}

/// Shared mesh/terrain simplification kernel. Original vertex indices are
/// retained and every topological boundary is fixed. Terrain supplies
/// ErrorAbsolute so both tolerance and the returned estimate are in metres.
pub(crate) fn simplify_border_locked(
    indices: &[u32],
    adapter: &VertexDataAdapter<'_>,
    target: usize,
    error: f32,
    additional_options: SimplifyOptions,
    attributes: Option<SimplificationAttributes<'_>>,
) -> (Vec<u32>, f32) {
    let mut measured = 0.;
    let options = SimplifyOptions::LockBorder | SimplifyOptions::Permissive | additional_options;
    let indices = if let Some(attributes) = attributes {
        simplify_with_attributes_and_locks(
            indices,
            adapter,
            attributes.values,
            attributes.weights,
            attributes.stride,
            attributes.locks,
            target,
            error,
            options,
            Some(&mut measured),
        )
    } else {
        simplify(
            indices,
            adapter,
            target,
            error,
            options,
            Some(&mut measured),
        )
    };
    (indices, measured)
}

/// Two-sided sampled surface distance in model metres: child vertices → parent
/// surface, and parent vertices + face centroids → child surface.
pub fn two_sided_error(parent: &[TilePrimitive], children: &[TilePrimitive]) -> f64 {
    let p_tris = soup(parent);
    let c_tris = soup(children);
    if p_tris.is_empty() || c_tris.is_empty() {
        return 0.0;
    }
    // `f32::max` is order-independent, so both directions and their samples
    // can be measured in parallel with the same result.
    let directed = |samples: Vec<[f32; 3]>, tris| {
        let grid = TriGrid::new(tris);
        samples
            .par_iter()
            .filter_map(|&q| grid.nearest_dist2(q))
            .reduce(|| 0.0f32, f32::max)
    };
    let (to_parent, to_children) = rayon::join(
        || directed(sample_points(children), p_tris),
        || directed(sample_points(parent), c_tris),
    );
    to_parent.max(to_children).sqrt() as f64
}

fn soup(prims: &[TilePrimitive]) -> Vec<[[f32; 3]; 3]> {
    let mut out = Vec::new();
    for p in prims {
        for t in p.indices.as_chunks::<3>().0 {
            out.push([
                p.positions[t[0] as usize],
                p.positions[t[1] as usize],
                p.positions[t[2] as usize],
            ]);
        }
    }
    out
}

/// Vertices plus face centroids, strided to `ERROR_SAMPLE_CAP`.
fn sample_points(prims: &[TilePrimitive]) -> Vec<[f32; 3]> {
    let mut pts = Vec::new();
    for p in prims {
        pts.extend_from_slice(&p.positions);
        for t in p.indices.as_chunks::<3>().0 {
            let a = p.positions[t[0] as usize];
            let b = p.positions[t[1] as usize];
            let c = p.positions[t[2] as usize];
            pts.push([
                (a[0] + b[0] + c[0]) / 3.0,
                (a[1] + b[1] + c[1]) / 3.0,
                (a[2] + b[2] + c[2]) / 3.0,
            ]);
        }
    }
    let step = pts.len().div_ceil(ERROR_SAMPLE_CAP).max(1);
    pts.into_iter().step_by(step).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn soup_grid(nx: u32, ny: u32) -> TilePrimitive {
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        for y in 0..ny {
            for x in 0..nx {
                let i = positions.len() as u32;
                let fx = x as f32;
                let fy = y as f32;
                positions.push([fx, fy, 0.0]);
                positions.push([fx + 1.0, fy, 0.0]);
                positions.push([fx + 1.0, fy + 1.0, 0.0]);
                positions.push([fx, fy + 1.0, 0.0]);
                indices.extend_from_slice(&[i, i + 1, i + 2, i, i + 2, i + 3]);
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

    fn connected_grid(nx: u32, ny: u32, z: impl Fn(u32, u32) -> f32) -> TilePrimitive {
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        for y in 0..=ny {
            for x in 0..=nx {
                positions.push([x as f32, y as f32, z(x, y)]);
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
    fn parent_budget_is_quarter_of_children_capped() {
        assert_eq!(parent_triangle_budget(160_000, 20_000), 20_000);
        assert_eq!(parent_triangle_budget(40_000, 20_000), 10_000);
        assert_eq!(parent_triangle_budget(100, 20_000), 256);
    }

    #[test]
    fn simplify_welds_soup_and_preserves_boundary_edges() {
        let prim = soup_grid(40, 40);
        assert_eq!(prim.indices.len() / 3, 3200);
        let out = simplify_primitive(&prim, 64).unwrap();
        assert!(
            out.indices.len() / 3 <= 200,
            "expected substantial reduction while retaining the 160 boundary edges, got {} tris",
            out.indices.len() / 3
        );
        let boundary = |mesh: &TilePrimitive| {
            let mut counts = std::collections::BTreeMap::new();
            for t in mesh.indices.as_chunks::<3>().0 {
                for i in 0..3 {
                    let a = mesh.positions[t[i] as usize].map(f32::to_bits);
                    let b = mesh.positions[t[(i + 1) % 3] as usize].map(f32::to_bits);
                    *counts
                        .entry(if a < b { (a, b) } else { (b, a) })
                        .or_insert(0) += 1;
                }
            }
            counts
                .into_iter()
                .filter_map(|(edge, n)| (n == 1).then_some(edge))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            boundary(&prim),
            boundary(&out),
            "simplification changed a boundary edge"
        );
    }

    #[test]
    fn simplify_never_punches_holes_on_connected_grid() {
        let prim = connected_grid(30, 30, |_, _| 0.0);
        let src_tris = prim.indices.len() / 3;
        let out = simplify_primitive(&prim, 100).unwrap();
        let out_tris = out.indices.len() / 3;
        assert!(
            out_tris < src_tris,
            "should reduce {src_tris} → got {out_tris}"
        );
        assert!(out_tris >= 3);
        for t in out.indices.as_chunks::<3>().0 {
            assert!(t[0] != t[1] && t[1] != t[2] && t[2] != t[0]);
            assert!((t[0] as usize) < out.positions.len());
        }
    }

    #[test]
    fn two_sided_error_sees_bumps_the_parent_flattened() {
        let bumpy = connected_grid(20, 20, |x, y| if (x + y) % 2 == 0 { 2.0 } else { 0.0 });
        let flat = connected_grid(2, 2, |_, _| 1.0);
        let flat = TilePrimitive {
            positions: flat
                .positions
                .iter()
                .map(|p| [p[0] * 10.0, p[1] * 10.0, p[2]])
                .collect(),
            ..flat
        };
        let e = two_sided_error(std::slice::from_ref(&flat), &[bumpy]);
        assert!(
            (0.9..=1.1).contains(&e),
            "bumps of ±1 around the plane, got {e}"
        );
        let zero = two_sided_error(std::slice::from_ref(&flat), std::slice::from_ref(&flat));
        assert!(zero < 1e-5);
    }

    #[test]
    fn build_parent_packs_two_textured_children_into_one_mesh() {
        fn textured_quad(origin: [f32; 3], rgb: [u8; 3]) -> TilePrimitive {
            let mut img = image::RgbImage::new(16, 16);
            for p in img.pixels_mut() {
                *p = image::Rgb(rgb);
            }
            let mut jpeg = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90)
                .encode(img.as_raw(), 16, 16, image::ExtendedColorType::Rgb8)
                .unwrap();
            let o = origin;
            TilePrimitive {
                positions: vec![
                    [o[0], o[1], o[2]],
                    [o[0] + 2.0, o[1], o[2]],
                    [o[0] + 2.0, o[1], o[2] + 2.0],
                    [o[0], o[1], o[2] + 2.0],
                ],
                normals: vec![[0.0, 1.0, 0.0]; 4],
                uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                indices: vec![0, 1, 2, 0, 2, 3],
                jpeg: Some(jpeg),
            }
        }
        let a = textured_quad([0.0, 0.0, 0.0], [200, 40, 40]);
        let b = textured_quad([2.0, 0.0, 0.0], [40, 200, 40]);
        let out = build_parent(&[a, b], 8, 64).unwrap();
        assert_eq!(out.prims.len(), 1, "packed parent is one mesh");
        let p = &out.prims[0];
        assert!(p.jpeg.as_ref().unwrap().len() > 32);
        assert_eq!(p.uvs.len(), p.positions.len());
        assert!(
            out.error_m < 1e-3,
            "coplanar quads → ~0 error, got {}",
            out.error_m
        );
        assert!(out.texel_m > 0.0);
        let img = image::load_from_memory(p.jpeg.as_ref().unwrap())
            .unwrap()
            .to_rgba8();
        let mut red = 0u32;
        let mut green = 0u32;
        for px in img.pixels() {
            if px[0] > 140 && px[1] < 90 {
                red += 1;
            }
            if px[1] > 140 && px[0] < 90 {
                green += 1;
            }
        }
        assert!(
            red > 10 && green > 10,
            "baked atlas should keep both source colours, red={red} green={green}"
        );
    }
}
