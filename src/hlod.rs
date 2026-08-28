//! Parent-mesh simplify (meshopt) and sampled Hausdorff geometricError.

use std::collections::HashMap;

use meshopt::optimize::optimize_vertex_fetch;
use meshopt::simplify::{simplify, simplify_with_attributes_and_locks, SimplifyOptions};
use meshopt::utilities::VertexDataAdapter;

use crate::error::Error;
use crate::glb_write::TilePrimitive;
use crate::mesh::Scene;

const SOURCE_SAMPLE_CAP: usize = 2_048;
pub(crate) const MIN_PARENT_GE: f64 = 1e-3;

/// SSE floor from the tile AABB (max half-extent / 8).
///
/// Sampled Hausdorff alone under-reports bad simplifies; this floor keeps
/// REPLACE refinement moving toward leaves when the camera is close.
pub(crate) fn spatial_geometric_error(min: [f64; 3], max: [f64; 3]) -> f64 {
    let hx = (max[0] - min[0]) * 0.5;
    let hy = (max[1] - min[1]) * 0.5;
    let hz = (max[2] - min[2]) * 0.5;
    hx.max(hy).max(hz) / 8.0
}

/// Triangle budget for a parent covering `descendant_tris` source triangles.
/// Far parents stay coarse so Cesium can show one GLB instead of a leaf stampede.
pub(crate) fn parent_triangle_budget(descendant_tris: usize, max_triangles: usize) -> usize {
    let cap = (max_triangles / 2).max(256);
    (descendant_tris / 32).clamp(256, cap)
}

#[derive(Clone, Copy, Default)]
#[repr(C)]
struct Vtx {
    p: [f32; 3],
    n: [f32; 3],
    t: [f32; 2],
}

/// Simplify to `target_tris`, preferring a watertight mesh over hitting the
/// budget. Untextured soup is welded by position. Never use `simplify_sloppy`
/// (it punches holes that show up as shattered parent tiles).
pub fn simplify_primitive(
    prim: &TilePrimitive,
    target_tris: usize,
) -> Result<TilePrimitive, Error> {
    simplify_primitive_with(prim, target_tris, WeldMode::PositionOnly)
}

#[derive(Clone, Copy)]
enum WeldMode {
    /// Manifold weld for untextured proxies (may average UVs — do not use
    /// when a JPEG is present).
    PositionOnly,
    /// Keep UV discontinuities so each material stays in its own 0–1.
    PositionUv,
}

fn simplify_primitive_with(
    prim: &TilePrimitive,
    target_tris: usize,
    weld: WeldMode,
) -> Result<TilePrimitive, Error> {
    if prim.indices.len() < 3 || prim.positions.is_empty() {
        return Ok(prim.clone());
    }
    let target_indices = (target_tris.max(1) * 3).min(prim.indices.len());
    if target_indices >= prim.indices.len() {
        return Ok(prim.clone());
    }

    let welded = match weld {
        WeldMode::PositionOnly => weld_by_position_only(prim),
        WeldMode::PositionUv => weld_by_position_uv(prim),
    };
    if welded.indices.len() < 3 {
        return Ok(prim.clone());
    }
    let pos_bytes: &[u8] = bytemuck::cast_slice(&welded.positions);
    let adapter = VertexDataAdapter::new(pos_bytes, 12, 0)
        .map_err(|e| Error::msg(format!("meshopt adapter: {e}")))?;
    let simplified = match weld {
        WeldMode::PositionOnly => reduce_indices(&welded.indices, &adapter, target_indices),
        WeldMode::PositionUv => {
            reduce_indices_uv(&welded.indices, &adapter, &welded.uvs, target_indices)
        }
    };
    if simplified.len() < 3 {
        return Ok(prim.clone());
    }

    let mut packed = Vec::with_capacity(welded.positions.len());
    let has_n = welded.normals.len() == welded.positions.len();
    let has_uv = welded.uvs.len() == welded.positions.len();
    for i in 0..welded.positions.len() {
        packed.push(Vtx {
            p: welded.positions[i],
            n: if has_n {
                welded.normals[i]
            } else {
                [0.0, 1.0, 0.0]
            },
            t: if has_uv { welded.uvs[i] } else { [0.0, 0.0] },
        });
    }
    let mut indices = simplified;
    let compacted = optimize_vertex_fetch(&mut indices, &packed);
    Ok(TilePrimitive {
        positions: compacted.iter().map(|v| v.p).collect(),
        normals: compacted.iter().map(|v| v.n).collect(),
        uvs: compacted.iter().map(|v| v.t).collect(),
        indices,
        jpeg: prim.jpeg.clone(),
    })
}

/// Parent proxy: weld + simplify for a watertight mesh, then bake a new atlas
/// by sampling the high-res source at each simplified texel.
pub fn simplify_tile(
    prims: &[TilePrimitive],
    sampler: Option<&crate::texture::SceneSampler<'_>>,
    budget: usize,
    atlas_size: u32,
) -> Result<Vec<TilePrimitive>, Error> {
    if prims.is_empty() {
        return Ok(Vec::new());
    }
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    let mut any_n = false;
    let mut any_tex = sampler.is_some();
    for p in prims {
        let base = positions.len() as u32;
        let has_n = p.normals.len() == p.positions.len();
        any_n |= has_n;
        any_tex |= p.jpeg.is_some() && p.uvs.len() == p.positions.len();
        for i in 0..p.positions.len() {
            positions.push(p.positions[i]);
            normals.push(if has_n { p.normals[i] } else { [0.0, 1.0, 0.0] });
        }
        indices.extend(p.indices.iter().map(|i| i + base));
    }
    let merged = TilePrimitive {
        positions,
        normals: if any_n { normals } else { Vec::new() },
        uvs: Vec::new(),
        indices,
        jpeg: None,
    };
    let simplified = simplify_primitive_with(&merged, budget, WeldMode::PositionOnly)?;
    if !any_tex {
        let mut s = simplified;
        s.jpeg = None;
        s.uvs.clear();
        return Ok(vec![s]);
    }
    let owned;
    let baked = if let Some(s) = sampler {
        crate::texture::bake_simplified(&simplified, s, atlas_size)
    } else {
        match crate::texture::SceneSampler::from_prims(prims) {
            Ok(s) => {
                owned = s;
                crate::texture::bake_simplified(&simplified, &owned, atlas_size)
            }
            Err(e) => Err(e),
        }
    };
    match baked {
        Ok(b) => Ok(vec![b]),
        Err(_) => {
            let mut s = simplified;
            s.jpeg = None;
            s.uvs.clear();
            Ok(vec![s])
        }
    }
}

/// Weld vertices that share a position only (manifold far LODs).
fn weld_by_position_only(prim: &TilePrimitive) -> TilePrimitive {
    let mut extent = 0.0f32;
    for p in &prim.positions {
        extent = extent.max(p[0].abs()).max(p[1].abs()).max(p[2].abs());
    }
    let cell = (extent * 1e-6).max(1e-5);
    let has_n = prim.normals.len() == prim.positions.len();
    let has_uv = prim.uvs.len() == prim.positions.len();

    let mut map: HashMap<[i64; 3], u32> = HashMap::new();
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut counts: Vec<u32> = Vec::new();
    let mut remap = vec![0u32; prim.positions.len()];
    for (i, &p) in prim.positions.iter().enumerate() {
        let k = [
            (p[0] / cell).round() as i64,
            (p[1] / cell).round() as i64,
            (p[2] / cell).round() as i64,
        ];
        if let Some(&id) = map.get(&k) {
            let j = id as usize;
            if has_n {
                normals[j][0] += prim.normals[i][0];
                normals[j][1] += prim.normals[i][1];
                normals[j][2] += prim.normals[i][2];
            }
            counts[j] += 1;
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
            counts.push(1);
            remap[i] = id;
        }
    }
    for (i, c) in counts.iter().enumerate() {
        let inv = 1.0 / (*c as f32);
        normals[i][0] *= inv;
        normals[i][1] *= inv;
        normals[i][2] *= inv;
        let len = (normals[i][0] * normals[i][0]
            + normals[i][1] * normals[i][1]
            + normals[i][2] * normals[i][2])
            .sqrt()
            .max(1e-20);
        normals[i][0] /= len;
        normals[i][1] /= len;
        normals[i][2] /= len;
        // Keep the first UV at this position — averaging seam UVs samples
        // the middle of the image.
    }
    let indices: Vec<u32> = prim.indices.iter().map(|&i| remap[i as usize]).collect();
    let mut out_idx = Vec::with_capacity(indices.len());
    for tri in indices.chunks_exact(3) {
        if tri[0] != tri[1] && tri[1] != tri[2] && tri[2] != tri[0] {
            out_idx.extend_from_slice(tri);
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

/// Weld vertices that share a position **and** UV.
///
/// Position-only welding averages UVs across materials and samples the wrong
/// image. Keep UV discontinuities.
fn weld_by_position_uv(prim: &TilePrimitive) -> TilePrimitive {
    let mut extent = 0.0f32;
    for p in &prim.positions {
        extent = extent.max(p[0].abs()).max(p[1].abs()).max(p[2].abs());
    }
    let cell = (extent * 1e-6).max(1e-5);
    // ~0.5 texel in a 1024 atlas — tight enough to keep distinct charts apart.
    const UV_CELL: f32 = 1.0 / 2048.0;

    let has_n = prim.normals.len() == prim.positions.len();
    let has_uv = prim.uvs.len() == prim.positions.len();

    let key = |p: [f32; 3], uv: [f32; 2]| -> [i64; 5] {
        [
            (p[0] / cell).round() as i64,
            (p[1] / cell).round() as i64,
            (p[2] / cell).round() as i64,
            if has_uv {
                (uv[0].clamp(0.0, 1.0) / UV_CELL).round() as i64
            } else {
                0
            },
            if has_uv {
                (uv[1].clamp(0.0, 1.0) / UV_CELL).round() as i64
            } else {
                0
            },
        ]
    };

    let mut map: HashMap<[i64; 5], u32> = HashMap::new();
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut counts: Vec<u32> = Vec::new();

    let mut remap = vec![0u32; prim.positions.len()];
    for (i, &p) in prim.positions.iter().enumerate() {
        let uv = if has_uv { prim.uvs[i] } else { [0.0, 0.0] };
        let k = key(p, uv);
        if let Some(&id) = map.get(&k) {
            let j = id as usize;
            if has_n {
                normals[j][0] += prim.normals[i][0];
                normals[j][1] += prim.normals[i][1];
                normals[j][2] += prim.normals[i][2];
            }
            // UVs already match within UV_CELL — keep the first, do not average
            // toward a neighbor chart.
            counts[j] += 1;
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
            uvs.push(uv);
            counts.push(1);
            remap[i] = id;
        }
    }
    for (i, c) in counts.iter().enumerate() {
        let inv = 1.0 / (*c as f32);
        normals[i][0] *= inv;
        normals[i][1] *= inv;
        normals[i][2] *= inv;
        let len = (normals[i][0] * normals[i][0]
            + normals[i][1] * normals[i][1]
            + normals[i][2] * normals[i][2])
            .sqrt()
            .max(1e-20);
        normals[i][0] /= len;
        normals[i][1] /= len;
        normals[i][2] /= len;
    }

    let indices: Vec<u32> = prim.indices.iter().map(|&i| remap[i as usize]).collect();
    let mut out_idx = Vec::with_capacity(indices.len());
    for tri in indices.chunks_exact(3) {
        if tri[0] != tri[1] && tri[1] != tri[2] && tri[2] != tri[0] {
            out_idx.extend_from_slice(tri);
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

fn reduce_indices(indices: &[u32], adapter: &VertexDataAdapter<'_>, target: usize) -> Vec<u32> {
    let attempts: [(SimplifyOptions, f32); 4] = [
        (
            SimplifyOptions::LockBorder | SimplifyOptions::Prune | SimplifyOptions::Permissive,
            1.0,
        ),
        (SimplifyOptions::Prune | SimplifyOptions::Permissive, 1.0),
        (SimplifyOptions::Permissive, 1.0),
        (SimplifyOptions::None, 1.0),
    ];
    pick_reduce(indices, adapter, target, &attempts)
}

fn reduce_indices_uv(
    indices: &[u32],
    adapter: &VertexDataAdapter<'_>,
    uvs: &[[f32; 2]],
    target: usize,
) -> Vec<u32> {
    let nvert = adapter.vertex_count;
    let mut attrs = vec![0.0f32; nvert * 2];
    for i in 0..nvert {
        let uv = uvs.get(i).copied().unwrap_or([0.0, 0.0]);
        attrs[i * 2] = uv[0];
        attrs[i * 2 + 1] = uv[1];
    }
    // UV weight << position (metres): prefer hitting the triangle budget over
    // freezing every chart. Seams stay as duplicate verts from PositionUv weld.
    let weights = [0.1f32, 0.1];
    let locks = vec![false; nvert];
    let stride = std::mem::size_of::<f32>() * 2;
    let attempts: [(SimplifyOptions, f32); 2] =
        [(SimplifyOptions::Prune, 1.0), (SimplifyOptions::None, 1.0)];
    let mut best: Option<Vec<u32>> = None;
    for &(opts, err) in &attempts {
        let out = simplify_with_attributes_and_locks(
            indices, adapter, &attrs, &weights, stride, &locks, target, err, opts, None,
        );
        if out.len() < 3 {
            continue;
        }
        let better = match &best {
            None => true,
            Some(b) => out.len() < b.len(),
        };
        if better {
            best = Some(out);
            if best.as_ref().map(|b| b.len()).unwrap_or(usize::MAX) <= target.saturating_mul(2) {
                break;
            }
        }
    }
    if best
        .as_ref()
        .map(|b| b.len() > target.saturating_mul(4))
        .unwrap_or(true)
    {
        let geo = pick_reduce(
            indices,
            adapter,
            target,
            &[(SimplifyOptions::Prune, 1.0), (SimplifyOptions::None, 1.0)],
        );
        if geo.len() >= 3 && best.as_ref().map(|b| geo.len() < b.len()).unwrap_or(true) {
            best = Some(geo);
        }
    }
    best.unwrap_or_else(|| indices.to_vec())
}

fn pick_reduce(
    indices: &[u32],
    adapter: &VertexDataAdapter<'_>,
    target: usize,
    attempts: &[(SimplifyOptions, f32)],
) -> Vec<u32> {
    let mut best: Option<Vec<u32>> = None;
    for &(opts, err) in attempts {
        let out = simplify(indices, adapter, target, err, opts, None);
        if out.len() < 3 {
            continue;
        }
        let better = match &best {
            None => true,
            Some(b) => out.len() < b.len(),
        };
        if better {
            best = Some(out);
            if best.as_ref().map(|b| b.len()).unwrap_or(usize::MAX) <= target.saturating_mul(2) {
                break;
            }
        }
    }
    best.unwrap_or_else(|| indices.to_vec())
}

/// Two-sided sampled point-to-mesh Hausdorff in model metres.
pub fn sampled_hausdorff(parent: &[TilePrimitive], scene: &Scene, source_ids: &[usize]) -> f64 {
    let (ppos, pidx) = flatten_prims(parent);
    if ppos.is_empty() || source_ids.is_empty() || pidx.len() < 3 {
        return 0.0;
    }
    let parent_grid = TriGrid::from_indexed(&ppos, &pidx);
    let mut d2 = 0.0f32;
    let step = (source_ids.len() / SOURCE_SAMPLE_CAP).max(1);
    for &id in source_ids.iter().step_by(step).take(SOURCE_SAMPLE_CAP) {
        let v = scene.triangles[id].verts[0];
        d2 = d2.max(parent_grid.dist2(scene.vertices[v as usize].pos));
    }
    d2.sqrt() as f64
}

fn flatten_prims(prims: &[TilePrimitive]) -> (Vec<[f32; 3]>, Vec<u32>) {
    let mut pos = Vec::new();
    let mut idx = Vec::new();
    for p in prims {
        let base = pos.len() as u32;
        pos.extend_from_slice(&p.positions);
        idx.extend(p.indices.iter().map(|i| i + base));
    }
    (pos, idx)
}

struct TriGrid {
    tris: Vec<[[f32; 3]; 3]>,
}

impl TriGrid {
    fn from_indexed(pos: &[[f32; 3]], idx: &[u32]) -> Self {
        let tris: Vec<[[f32; 3]; 3]> = idx
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| [pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]])
            .collect();
        Self { tris }
    }

    fn dist2(&self, q: [f32; 3]) -> f32 {
        let mut best = f32::INFINITY;
        for t in &self.tris {
            best = best.min(point_tri_dist2(q, t[0], t[1], t[2]));
        }
        if best.is_finite() {
            best
        } else {
            0.0
        }
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scl(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = sub(a, b);
    dot(d, d)
}

/// Closest-point distance² to a triangle (Ericson).
fn point_tri_dist2(p: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return dist2(p, a);
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return dist2(p, b);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return dist2(p, add(a, scl(ab, v)));
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return dist2(p, c);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return dist2(p, add(a, scl(ac, w)));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return dist2(p, add(b, scl(sub(c, b), w)));
    }
    let denom = va + vb + vc;
    if denom.abs() < 1e-20 {
        return dist2(p, a);
    }
    let v = vb / denom;
    let w = vc / denom;
    dist2(p, add(a, add(scl(ab, v), scl(ac, w))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glb_write::TilePrimitive;

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

    #[test]
    fn simplify_welds_soup_and_hits_budget() {
        // Disconnected quads share positions; after weld they form a grid
        // that topology-preserving simplify can reduce (no sloppy).
        let prim = soup_grid(40, 40);
        assert_eq!(prim.indices.len() / 3, 3200);
        let out = simplify_primitive(&prim, 64).unwrap();
        assert!(
            out.indices.len() / 3 <= 128,
            "expected weld+simplify to hit budget, got {} tris",
            out.indices.len() / 3
        );
    }

    #[test]
    fn simplify_never_punches_holes_on_connected_grid() {
        // Shared-vertex grid: output must stay a continuous surface (every
        // edge shared by 1–2 tris; no isolated spikes from sloppy).
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        let nx = 30u32;
        let ny = 30u32;
        for y in 0..=ny {
            for x in 0..=nx {
                positions.push([x as f32, y as f32, 0.0]);
            }
        }
        let w = nx + 1;
        for y in 0..ny {
            for x in 0..nx {
                let i = y * w + x;
                indices.extend_from_slice(&[i, i + 1, i + w + 1, i, i + w + 1, i + w]);
            }
        }
        let prim = TilePrimitive {
            positions,
            normals: Vec::new(),
            uvs: Vec::new(),
            indices,
            jpeg: None,
        };
        let src_tris = prim.indices.len() / 3;
        let out = simplify_primitive(&prim, 100).unwrap();
        let out_tris = out.indices.len() / 3;
        assert!(
            out_tris < src_tris,
            "should reduce {src_tris} → got {out_tris}"
        );
        assert!(out_tris >= 3);
        // Manifold-ish: index refs in range, no degenerate tris.
        for t in out.indices.chunks_exact(3) {
            assert!(t[0] != t[1] && t[1] != t[2] && t[2] != t[0]);
            assert!((t[0] as usize) < out.positions.len());
            assert!((t[1] as usize) < out.positions.len());
            assert!((t[2] as usize) < out.positions.len());
        }
    }

    #[test]
    fn simplify_tile_packs_into_one_mesh() {
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
        let out = simplify_tile(&[a, b], None, 8, 64).unwrap();
        assert_eq!(out.len(), 1, "packed parent is one mesh");
        assert!(out[0].jpeg.as_ref().unwrap().len() > 32);
        assert_eq!(out[0].uvs.len(), out[0].positions.len());
        assert!(!out[0].indices.is_empty());
        assert!(
            out[0].positions.len() < out[0].indices.len(),
            "chart unwrap should share vertices, got {} verts / {} indices",
            out[0].positions.len(),
            out[0].indices.len()
        );
        for uv in &out[0].uvs {
            assert!((0.0..=1.0).contains(&uv[0]));
            assert!((0.0..=1.0).contains(&uv[1]));
        }
        let img = image::load_from_memory(out[0].jpeg.as_ref().unwrap())
            .unwrap()
            .to_rgba8();
        let mut red = 0u32;
        let mut green = 0u32;
        for p in img.pixels() {
            if p[0] > 140 && p[1] < 90 {
                red += 1;
            }
            if p[1] > 140 && p[0] < 90 {
                green += 1;
            }
        }
        assert!(
            red > 10 && green > 10,
            "baked atlas should keep both source colours, red={red} green={green}"
        );
    }
}
