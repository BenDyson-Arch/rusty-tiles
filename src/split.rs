//! Spatial k-d: partition the scene AABB, clip triangles to each cell.
//!
//! The node *is* the cell (Z-up, same frame as `boundingVolume.box`). Siblings
//! share a face and do not overlap. Leaves keep source vertices except at the
//! cut, where new verts are snapped onto the shared plane in f64.

use crate::bbox::{aabb_zup_to_yup, y_up_to_z_up};
use crate::mesh::Scene;

pub const MAX_SPLIT_DEPTH: u32 = 20;
const PARALLEL_AFTER: usize = 32_768;
/// Metres: treat a vertex this close to a clip plane as on it.
const SNAP: f64 = 1e-7;
const AREA2_EPS: f64 = 1e-24;

pub struct SplitOpts {
    pub max_triangles: usize,
}

pub enum SplitNode {
    Leaf {
        triangle_ids: Vec<usize>,
        min: [f64; 3],
        max: [f64; 3],
    },
    Branch {
        children: Vec<SplitNode>,
        min: [f64; 3],
        max: [f64; 3],
    },
}

impl SplitNode {
    pub fn aabb(&self) -> ([f64; 3], [f64; 3]) {
        match self {
            SplitNode::Leaf { min, max, .. } | SplitNode::Branch { min, max, .. } => (*min, *max),
        }
    }

    /// 0 = leaf, 1 = children are leaves, 2 = grandchildren are leaves, …
    pub fn subtree_height(&self) -> u32 {
        match self {
            SplitNode::Leaf { .. } => 0,
            SplitNode::Branch { children, .. } => {
                1 + children
                    .iter()
                    .map(SplitNode::subtree_height)
                    .max()
                    .unwrap_or(0)
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ClipVert {
    pub pos: [f32; 3],
    pub nrm: [f32; 3],
    pub uv: [f32; 2],
}

#[derive(Clone, Copy, Debug)]
pub struct ClippedTri {
    pub verts: [ClipVert; 3],
    pub image: Option<u32>,
}

pub fn split(scene: &Scene, opts: &SplitOpts) -> SplitNode {
    let ids: Vec<usize> = (0..scene.triangles.len()).collect();
    let (min, max) = scene_aabb_zup(scene);
    split_cell(scene, ids, min, max, 0, opts).unwrap_or_else(|| leaf(Vec::new(), min, max))
}

fn split_cell(
    scene: &Scene,
    ids: Vec<usize>,
    min: [f64; 3],
    max: [f64; 3],
    depth: u32,
    opts: &SplitOpts,
) -> Option<SplitNode> {
    if ids.is_empty() {
        return None;
    }
    if ids.len() <= opts.max_triangles || depth >= MAX_SPLIT_DEPTH || ids.len() < 2 {
        return Some(leaf(ids, min, max));
    }

    let axis = longest_axis(min, max);
    let span = max[axis] - min[axis];
    if span < 1e-3 {
        return Some(leaf(ids, min, max));
    }
    let split_at = split_plane(scene, &ids, min, max, axis);
    if split_at <= min[axis] + SNAP || split_at >= max[axis] - SNAP {
        return Some(leaf(ids, min, max));
    }

    let mut left_ids = Vec::new();
    let mut right_ids = Vec::new();
    for &id in &ids {
        let (tmin, tmax) = tri_axis_zup(scene, id, axis);
        let on_plane = (tmin - split_at).abs() <= SNAP && (tmax - split_at).abs() <= SNAP;
        if on_plane || tmax < split_at + SNAP {
            left_ids.push(id);
        } else if tmin > split_at - SNAP {
            right_ids.push(id);
        } else {
            left_ids.push(id);
            right_ids.push(id);
        }
    }
    // Empty half: keep splitting the occupied child cell (dense cluster).
    if left_ids.is_empty() {
        let mut rmin = min;
        rmin[axis] = split_at;
        return split_cell(scene, right_ids, rmin, max, depth, opts);
    }
    if right_ids.is_empty() {
        let mut lmax = max;
        lmax[axis] = split_at;
        return split_cell(scene, left_ids, min, lmax, depth, opts);
    }

    let mut lmax = max;
    lmax[axis] = split_at;
    let mut rmin = min;
    rmin[axis] = split_at;

    let (l, r) = if left_ids.len() >= PARALLEL_AFTER && right_ids.len() >= PARALLEL_AFTER {
        rayon::join(
            || split_cell(scene, left_ids, min, lmax, depth + 1, opts),
            || split_cell(scene, right_ids, rmin, max, depth + 1, opts),
        )
    } else {
        (
            split_cell(scene, left_ids, min, lmax, depth + 1, opts),
            split_cell(scene, right_ids, rmin, max, depth + 1, opts),
        )
    };

    let mut children = Vec::new();
    if let Some(n) = l {
        children.push(n);
    }
    if let Some(n) = r {
        children.push(n);
    }
    match children.len() {
        0 => None,
        1 => children.pop(),
        _ => Some(SplitNode::Branch { children, min, max }),
    }
}

fn leaf(triangle_ids: Vec<usize>, min: [f64; 3], max: [f64; 3]) -> SplitNode {
    SplitNode::Leaf {
        triangle_ids,
        min,
        max,
    }
}

fn longest_axis(min: [f64; 3], max: [f64; 3]) -> usize {
    let dx = max[0] - min[0];
    let dy = max[1] - min[1];
    let dz = max[2] - min[2];
    if dx >= dy && dx >= dz {
        0
    } else if dy >= dz {
        1
    } else {
        2
    }
}

/// Spatial midpoint, unless that leaves one side empty — then centroid median.
fn split_plane(scene: &Scene, ids: &[usize], min: [f64; 3], max: [f64; 3], axis: usize) -> f64 {
    let mid = (min[axis] + max[axis]) * 0.5;
    let mut left = 0usize;
    let mut right = 0usize;
    for &id in ids {
        let (tmin, tmax) = tri_axis_zup(scene, id, axis);
        if tmax < mid + SNAP {
            left += 1;
        } else if tmin > mid - SNAP {
            right += 1;
        } else {
            left += 1;
            right += 1;
        }
    }
    if left > 0 && right > 0 && left < ids.len() && right < ids.len() {
        return mid;
    }
    let mut cs: Vec<f64> = ids
        .iter()
        .map(|&id| tri_centroid_zup(scene, id)[axis])
        .collect();
    cs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let med = cs[cs.len() / 2];
    let pad = ((max[axis] - min[axis]) * 0.01).max(SNAP * 10.0);
    med.clamp(min[axis] + pad, max[axis] - pad)
}

fn scene_aabb_zup(scene: &Scene) -> ([f64; 3], [f64; 3]) {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for v in &scene.vertices {
        let p = y_up_to_z_up(v.pos);
        for i in 0..3 {
            min[i] = min[i].min(p[i] as f64);
            max[i] = max[i].max(p[i] as f64);
        }
    }
    let extent = (max[0] - min[0])
        .max(max[1] - min[1])
        .max(max[2] - min[2])
        .max(SNAP);
    let e = (extent * 1e-9).max(SNAP);
    for i in 0..3 {
        min[i] -= e;
        max[i] += e;
    }
    (min, max)
}

fn tri_axis_zup(scene: &Scene, id: usize, axis: usize) -> (f64, f64) {
    let mut mn = f64::INFINITY;
    let mut mx = f64::NEG_INFINITY;
    for v in scene.triangles[id].verts {
        let p = y_up_to_z_up(scene.vertices[v as usize].pos);
        let c = p[axis] as f64;
        mn = mn.min(c);
        mx = mx.max(c);
    }
    (mn, mx)
}

fn tri_centroid_zup(scene: &Scene, id: usize) -> [f64; 3] {
    let mut c = [0.0f64; 3];
    for v in scene.triangles[id].verts {
        let p = y_up_to_z_up(scene.vertices[v as usize].pos);
        for i in 0..3 {
            c[i] += p[i] as f64;
        }
    }
    [c[0] / 3.0, c[1] / 3.0, c[2] / 3.0]
}

/// Clip source triangles to a Z-up cell. Output is glTF Y-up.
pub fn clip_to_cell(scene: &Scene, ids: &[usize], min: [f64; 3], max: [f64; 3]) -> Vec<ClippedTri> {
    let (ymin, ymax) = aabb_zup_to_yup(min, max);
    let planes = [
        Plane::Min(0, ymin[0]),
        Plane::Max(0, ymax[0]),
        Plane::Min(1, ymin[1]),
        Plane::Max(1, ymax[1]),
        Plane::Min(2, ymin[2]),
        Plane::Max(2, ymax[2]),
    ];
    let mut out = Vec::new();
    for &id in ids {
        let t = &scene.triangles[id];
        let mut poly = Vec::with_capacity(8);
        for &vi in &t.verts {
            let v = &scene.vertices[vi as usize];
            poly.push(CVert {
                pos: [v.pos[0] as f64, v.pos[1] as f64, v.pos[2] as f64],
                nrm: v.nrm,
                uv: v.uv,
            });
        }
        for p in &mut poly {
            snap_to_cell(p, ymin, ymax);
        }
        for plane in planes {
            poly = clip_poly(&poly, plane);
            if poly.len() < 3 {
                break;
            }
        }
        if poly.len() < 3 {
            continue;
        }
        dedup_ring(&mut poly);
        if poly.len() < 3 {
            continue;
        }
        for i in 1..poly.len().saturating_sub(1) {
            let a = poly[0];
            let b = poly[i];
            let c = poly[i + 1];
            if area2(a.pos, b.pos, c.pos) < AREA2_EPS {
                continue;
            }
            out.push(ClippedTri {
                verts: [pack_vert(a), pack_vert(b), pack_vert(c)],
                image: t.image,
            });
        }
    }
    out
}

#[derive(Clone, Copy)]
struct CVert {
    pos: [f64; 3],
    nrm: [f32; 3],
    uv: [f32; 2],
}

#[derive(Clone, Copy)]
enum Plane {
    Min(usize, f64),
    Max(usize, f64),
}

fn snap_to_cell(v: &mut CVert, ymin: [f64; 3], ymax: [f64; 3]) {
    for i in 0..3 {
        if (v.pos[i] - ymin[i]).abs() <= SNAP {
            v.pos[i] = ymin[i];
        } else if (v.pos[i] - ymax[i]).abs() <= SNAP {
            v.pos[i] = ymax[i];
        }
    }
}

fn inside(v: CVert, plane: Plane) -> bool {
    match plane {
        Plane::Min(i, b) => v.pos[i] >= b - SNAP,
        Plane::Max(i, b) => v.pos[i] <= b + SNAP,
    }
}

fn intersect(a: CVert, b: CVert, plane: Plane) -> CVert {
    let (i, bound) = match plane {
        Plane::Min(i, b) | Plane::Max(i, b) => (i, b),
    };
    let da = a.pos[i] - bound;
    let db = b.pos[i] - bound;
    let t = if (da - db).abs() < 1e-18 {
        0.5
    } else {
        (da / (da - db)).clamp(0.0, 1.0)
    };
    let mut v = lerp(a, b, t);
    v.pos[i] = bound;
    v
}

fn lerp(a: CVert, b: CVert, t: f64) -> CVert {
    let s = 1.0 - t;
    let mut nrm = [
        (a.nrm[0] as f64 * s + b.nrm[0] as f64 * t) as f32,
        (a.nrm[1] as f64 * s + b.nrm[1] as f64 * t) as f32,
        (a.nrm[2] as f64 * s + b.nrm[2] as f64 * t) as f32,
    ];
    let len = (nrm[0] * nrm[0] + nrm[1] * nrm[1] + nrm[2] * nrm[2]).sqrt();
    if len > 1e-20 {
        nrm = [nrm[0] / len, nrm[1] / len, nrm[2] / len];
    } else {
        nrm = [0.0, 1.0, 0.0];
    }
    CVert {
        pos: [
            a.pos[0] * s + b.pos[0] * t,
            a.pos[1] * s + b.pos[1] * t,
            a.pos[2] * s + b.pos[2] * t,
        ],
        nrm,
        uv: [
            (a.uv[0] as f64 * s + b.uv[0] as f64 * t) as f32,
            (a.uv[1] as f64 * s + b.uv[1] as f64 * t) as f32,
        ],
    }
}

fn clip_poly(poly: &[CVert], plane: Plane) -> Vec<CVert> {
    if poly.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(poly.len() + 1);
    let mut prev = poly[poly.len() - 1];
    let mut prev_in = inside(prev, plane);
    for &cur in poly {
        let cur_in = inside(cur, plane);
        if prev_in && cur_in {
            out.push(cur);
        } else if prev_in && !cur_in {
            out.push(intersect(prev, cur, plane));
        } else if !prev_in && cur_in {
            out.push(intersect(prev, cur, plane));
            out.push(cur);
        }
        prev = cur;
        prev_in = cur_in;
    }
    out
}

fn dedup_ring(poly: &mut Vec<CVert>) {
    if poly.len() < 2 {
        return;
    }
    let mut out: Vec<CVert> = Vec::with_capacity(poly.len());
    for &v in poly.iter() {
        if let Some(last) = out.last() {
            if dist2(last.pos, v.pos) <= SNAP * SNAP {
                continue;
            }
        }
        out.push(v);
    }
    if out.len() >= 2 && dist2(out[0].pos, out[out.len() - 1].pos) <= SNAP * SNAP {
        out.pop();
    }
    *poly = out;
}

fn dist2(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
}

fn area2(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let cr = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    cr[0] * cr[0] + cr[1] * cr[1] + cr[2] * cr[2]
}

fn pack_vert(v: CVert) -> ClipVert {
    ClipVert {
        pos: [v.pos[0] as f32, v.pos[1] as f32, v.pos[2] as f32],
        nrm: v.nrm,
        uv: v.uv,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::{Scene, Triangle, Vertex};

    fn grid_scene(nx: u32, ny: u32) -> Scene {
        let mut vertices = Vec::new();
        let mut triangles = Vec::new();
        let w = nx + 1;
        for y in 0..=ny {
            for x in 0..=nx {
                vertices.push(Vertex {
                    pos: [x as f32, y as f32, 0.0],
                    nrm: [0.0, 0.0, 1.0],
                    uv: [x as f32 / nx as f32, y as f32 / ny as f32],
                });
            }
        }
        for y in 0..ny {
            for x in 0..nx {
                let i = y * w + x;
                triangles.push(Triangle {
                    verts: [i, i + 1, i + w + 1],
                    image: None,
                });
                triangles.push(Triangle {
                    verts: [i, i + w + 1, i + w],
                    image: None,
                });
            }
        }
        Scene {
            vertices,
            triangles,
            images: Vec::new(),
            source_bytes: 0,
        }
    }

    fn leaves_of<'a>(node: &'a SplitNode, out: &mut Vec<&'a SplitNode>) {
        match node {
            SplitNode::Leaf { .. } => out.push(node),
            SplitNode::Branch { children, .. } => {
                for c in children {
                    leaves_of(c, out);
                }
            }
        }
    }

    fn first_branch(node: &SplitNode) -> Option<&SplitNode> {
        match node {
            SplitNode::Branch { .. } => Some(node),
            SplitNode::Leaf { .. } => None,
        }
    }

    fn boxes_only_touch(a: ([f64; 3], [f64; 3]), b: ([f64; 3], [f64; 3])) -> bool {
        for i in 0..3 {
            if a.1[i] <= b.0[i] + 1e-9 || b.1[i] <= a.0[i] + 1e-9 {
                return true;
            }
        }
        false
    }

    #[test]
    fn sibling_cells_disjoint() {
        let scene = grid_scene(8, 8);
        let tree = split(&scene, &SplitOpts { max_triangles: 8 });
        let branch = first_branch(&tree).expect("expected a split");
        let SplitNode::Branch { children, min, max } = branch else {
            panic!("not a branch");
        };
        assert_eq!(children.len(), 2);
        let a = children[0].aabb();
        let b = children[1].aabb();
        assert!(boxes_only_touch(a, b), "siblings overlap {a:?} {b:?}");
        for i in 0..3 {
            assert!((a.0[i].min(b.0[i]) - min[i]).abs() < 1e-9);
            assert!((a.1[i].max(b.1[i]) - max[i]).abs() < 1e-9);
        }
    }

    #[test]
    fn clip_shares_cut_edge() {
        // One quad 0..2 on X, split must cut at x=1 (longest axis, midpoint).
        let scene = grid_scene(2, 1);
        let tree = split(&scene, &SplitOpts { max_triangles: 2 });
        let mut leaves = Vec::new();
        leaves_of(&tree, &mut leaves);
        assert!(leaves.len() >= 2, "expected a split, got {}", leaves.len());

        let mut cuts: Vec<Vec<[i64; 3]>> = Vec::new();
        for leaf in &leaves {
            let SplitNode::Leaf {
                triangle_ids,
                min,
                max,
            } = leaf
            else {
                continue;
            };
            let clipped = clip_to_cell(&scene, triangle_ids, *min, *max);
            let mut keys = Vec::new();
            for t in &clipped {
                for v in &t.verts {
                    let zup = y_up_to_z_up(v.pos);
                    // Cut verts sit on a cell face of a sibling.
                    let on_face = (0..3).any(|i| {
                        (zup[i] as f64 - min[i]).abs() < 1e-5
                            || (zup[i] as f64 - max[i]).abs() < 1e-5
                    });
                    if on_face {
                        keys.push([
                            (v.pos[0] * 1e5).round() as i64,
                            (v.pos[1] * 1e5).round() as i64,
                            (v.pos[2] * 1e5).round() as i64,
                        ]);
                    }
                }
            }
            keys.sort();
            keys.dedup();
            cuts.push(keys);
        }
        assert!(
            !cuts[0].is_empty() && !cuts[1].is_empty(),
            "each leaf should have a cut edge"
        );
        let share: Vec<_> = cuts[0]
            .iter()
            .filter(|k| cuts[1].binary_search(k).is_ok())
            .collect();
        assert!(
            !share.is_empty(),
            "leaves should share cut-edge vertices, left={:?} right={:?}",
            cuts[0],
            cuts[1]
        );
    }

    #[test]
    fn interior_verts_match_source() {
        let scene = grid_scene(4, 4);
        let tree = split(&scene, &SplitOpts { max_triangles: 8 });
        let mut leaves = Vec::new();
        leaves_of(&tree, &mut leaves);
        let mut source_pos = std::collections::HashSet::new();
        for v in &scene.vertices {
            source_pos.insert((
                (v.pos[0] * 1e5).round() as i64,
                (v.pos[1] * 1e5).round() as i64,
                (v.pos[2] * 1e5).round() as i64,
            ));
        }
        for leaf in leaves {
            let SplitNode::Leaf {
                triangle_ids,
                min,
                max,
            } = leaf
            else {
                continue;
            };
            let clipped = clip_to_cell(&scene, triangle_ids, *min, *max);
            let (ymin, ymax) = aabb_zup_to_yup(*min, *max);
            for t in clipped {
                for v in t.verts {
                    let interior = (0..3).all(|i| {
                        v.pos[i] as f64 > ymin[i] + 1e-4 && (v.pos[i] as f64) < ymax[i] - 1e-4
                    });
                    if !interior {
                        continue;
                    }
                    let k = (
                        (v.pos[0] * 1e5).round() as i64,
                        (v.pos[1] * 1e5).round() as i64,
                        (v.pos[2] * 1e5).round() as i64,
                    );
                    assert!(
                        source_pos.contains(&k),
                        "interior clipped vert {k:?} is not a source vertex"
                    );
                }
            }
        }
    }
}
