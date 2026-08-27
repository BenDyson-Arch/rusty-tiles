//! Parent-mesh simplify (meshopt LockBorder) and sampled Hausdorff geometricError.

use meshopt::optimize::optimize_vertex_fetch;
use meshopt::simplify::{simplify, SimplifyOptions};
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

/// Simplify to `target_tris` while locking the topological border.
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
    let simplified = simplify(
        &prim.indices,
        &adapter,
        target_indices,
        1.0,
        SimplifyOptions::LockBorder,
        None,
    );
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
