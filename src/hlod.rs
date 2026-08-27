//! Parent-mesh simplify (meshopt) and sampled Hausdorff geometricError.

use meshopt::optimize::optimize_vertex_fetch;
use meshopt::simplify::{simplify, simplify_sloppy, SimplifyOptions};
use meshopt::utilities::VertexDataAdapter;

use crate::error::Error;
use crate::glb_write::TilePrimitive;
use crate::mesh::Scene;

const SOURCE_SAMPLE_CAP: usize = 2_048;
pub(crate) const MIN_PARENT_GE: f64 = 1e-3;

#[derive(Clone, Copy, Default)]
#[repr(C)]
struct Vtx {
    p: [f32; 3],
    n: [f32; 3],
    t: [f32; 2],
}

/// Simplify to `target_tris`. Photogrammetry borders/holes make LockBorder a
/// no-op, so fall back until the budget is actually hit.
pub fn simplify_primitive(
    prim: &TilePrimitive,
    target_tris: usize,
) -> Result<TilePrimitive, Error> {
    if prim.indices.len() < 3 || prim.positions.is_empty() {
        return Ok(prim.clone());
    }
    let target_indices = (target_tris.max(1) * 3).min(prim.indices.len());
    if target_indices >= prim.indices.len() {
        return Ok(prim.clone());
    }

    let pos_bytes: &[u8] = bytemuck::cast_slice(&prim.positions);
    let adapter = VertexDataAdapter::new(pos_bytes, 12, 0)
        .map_err(|e| Error::msg(format!("meshopt adapter: {e}")))?;
    let simplified = reduce_indices(&prim.indices, &adapter, target_indices);
    if simplified.len() < 3 {
        return Ok(prim.clone());
    }

    let mut packed = Vec::with_capacity(prim.positions.len());
    let has_n = prim.normals.len() == prim.positions.len();
    let has_uv = prim.uvs.len() == prim.positions.len();
    for i in 0..prim.positions.len() {
        packed.push(Vtx {
            p: prim.positions[i],
            n: if has_n {
                prim.normals[i]
            } else {
                [0.0, 1.0, 0.0]
            },
            t: if has_uv { prim.uvs[i] } else { [0.0, 0.0] },
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

/// Split `budget` triangles across primitives (the parent tile, not each material).
pub fn simplify_tile(
    prims: &[TilePrimitive],
    budget: usize,
) -> Result<Vec<TilePrimitive>, Error> {
    let counts: Vec<usize> = prims.iter().map(|p| p.indices.len() / 3).collect();
    let total = counts.iter().sum::<usize>().max(1);
    prims
        .iter()
        .zip(&counts)
        .map(|(p, &n)| {
            let share = budget.saturating_mul(n).max(1) / total;
            simplify_primitive(p, share.max(1))
        })
        .collect()
}

fn reduce_indices(indices: &[u32], adapter: &VertexDataAdapter<'_>, target: usize) -> Vec<u32> {
    let cap = target.saturating_mul(2).min(indices.len());
    let locked = simplify(
        indices,
        adapter,
        target,
        1.0,
        SimplifyOptions::LockBorder | SimplifyOptions::Prune | SimplifyOptions::Permissive,
        None,
    );
    if locked.len() >= 3 && locked.len() <= cap {
        return locked;
    }
    let open = simplify(
        indices,
        adapter,
        target,
        1.0,
        SimplifyOptions::Prune | SimplifyOptions::Permissive,
        None,
    );
    if open.len() >= 3 && open.len() <= cap {
        return open;
    }
    let sloppy = simplify_sloppy(indices, adapter, target, 1.0, None);
    if sloppy.len() >= 3 {
        sloppy
    } else if open.len() >= 3 {
        open
    } else if locked.len() >= 3 {
        locked
    } else {
        indices.to_vec()
    }
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
    fn simplify_hits_budget_when_lock_border_cannot() {
        let prim = soup_grid(40, 40);
        assert_eq!(prim.indices.len() / 3, 3200);
        let out = simplify_primitive(&prim, 64).unwrap();
        assert!(
            out.indices.len() / 3 <= 128,
            "expected sloppy fallback, got {} tris",
            out.indices.len() / 3
        );
    }

    #[test]
    fn simplify_tile_splits_budget_across_prims() {
        let a = soup_grid(20, 20);
        let b = soup_grid(20, 20);
        let out = simplify_tile(&[a, b], 80).unwrap();
        let tris: usize = out.iter().map(|p| p.indices.len() / 3).sum();
        assert!(tris <= 160, "tile budget 80, got {tris}");
    }
}
