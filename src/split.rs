//! k-d split by triangle centroid until leaf triangle and byte budgets.

use std::collections::HashSet;

use crate::bbox::y_up_to_z_up;
use crate::mesh::{centroid, triangle_aabb_yup, Scene};

pub const MAX_SPLIT_DEPTH: u32 = 12;

pub struct SplitOpts {
    pub max_triangles: usize,
    pub max_bytes: u64,
    pub tile_size: u32,
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

pub fn split(scene: &Scene, opts: &SplitOpts) -> SplitNode {
    let ids: Vec<usize> = (0..scene.triangles.len()).collect();
    split_ids(scene, &ids, 0, opts).unwrap_or_else(|| leaf(scene, ids))
}

fn split_ids(scene: &Scene, ids: &[usize], depth: u32, opts: &SplitOpts) -> Option<SplitNode> {
    if ids.is_empty() {
        return None;
    }
    if is_leaf(scene, ids, opts) || depth >= MAX_SPLIT_DEPTH {
        return Some(leaf(scene, ids.to_vec()));
    }

    let (ymin, ymax) = triangle_aabb_yup(scene, ids);
    let axis = longest_axis(ymin, ymax);

    let mut ordered = ids.to_vec();
    ordered.sort_by(|&a, &b| {
        let ca = centroid(scene, &scene.triangles[a])[axis];
        let cb = centroid(scene, &scene.triangles[b])[axis];
        ca.partial_cmp(&cb).unwrap_or(std::cmp::Ordering::Equal)
    });
    let median = centroid(scene, &scene.triangles[ordered[ordered.len() / 2]])[axis];
    let (mut left, mut right) = partition(scene, ids, axis, median, true);

    if left.is_empty() || right.is_empty() {
        let mid = (ymin[axis] + ymax[axis]) * 0.5;
        let parts = partition(scene, ids, axis, mid, false);
        left = parts.0;
        right = parts.1;
    }
    if left.is_empty() || right.is_empty() {
        return Some(leaf(scene, ids.to_vec()));
    }

    let mut children = Vec::new();
    if let Some(n) = split_ids(scene, &left, depth + 1, opts) {
        children.push(n);
    }
    if let Some(n) = split_ids(scene, &right, depth + 1, opts) {
        children.push(n);
    }
    match children.len() {
        0 => None,
        1 => children.pop(),
        _ => {
            let (min, max) = union_zup(&children);
            Some(SplitNode::Branch { children, min, max })
        }
    }
}

fn partition(
    scene: &Scene,
    ids: &[usize],
    axis: usize,
    split: f32,
    inclusive_left: bool,
) -> (Vec<usize>, Vec<usize>) {
    let mut left = Vec::new();
    let mut right = Vec::new();
    for &id in ids {
        let c = centroid(scene, &scene.triangles[id])[axis];
        if if inclusive_left {
            c <= split
        } else {
            c < split
        } {
            left.push(id);
        } else {
            right.push(id);
        }
    }
    (left, right)
}

fn is_leaf(scene: &Scene, ids: &[usize], opts: &SplitOpts) -> bool {
    ids.len() <= opts.max_triangles && estimate_bytes(scene, ids, opts.tile_size) <= opts.max_bytes
}

pub fn estimate_bytes(scene: &Scene, ids: &[usize], tile_size: u32) -> u64 {
    let mut verts = HashSet::new();
    let mut textured = false;
    for &id in ids {
        let t = &scene.triangles[id];
        verts.insert(t.verts[0]);
        verts.insert(t.verts[1]);
        verts.insert(t.verts[2]);
        if t.image.is_some() {
            textured = true;
        }
    }
    let geom = verts.len() as u64 * 32 + ids.len() as u64 * 12;
    let tex = if textured {
        (tile_size as u64 * tile_size as u64) / 4
    } else {
        0
    };
    geom + tex
}

fn longest_axis(min: [f32; 3], max: [f32; 3]) -> usize {
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

fn leaf(scene: &Scene, triangle_ids: Vec<usize>) -> SplitNode {
    let (min, max) = zup_aabb(scene, &triangle_ids);
    SplitNode::Leaf {
        triangle_ids,
        min,
        max,
    }
}

fn zup_aabb(scene: &Scene, ids: &[usize]) -> ([f64; 3], [f64; 3]) {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for &id in ids {
        for v in scene.triangles[id].verts {
            let p = y_up_to_z_up(scene.vertices[v as usize].pos);
            for i in 0..3 {
                min[i] = min[i].min(p[i] as f64);
                max[i] = max[i].max(p[i] as f64);
            }
        }
    }
    if !min[0].is_finite() {
        ([0.0; 3], [0.0; 3])
    } else {
        (min, max)
    }
}

fn union_zup(nodes: &[SplitNode]) -> ([f64; 3], [f64; 3]) {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for n in nodes {
        let (mn, mx) = n.aabb();
        for i in 0..3 {
            min[i] = min[i].min(mn[i]);
            max[i] = max[i].max(mx[i]);
        }
    }
    (min, max)
}

impl SplitNode {
    pub fn aabb(&self) -> ([f64; 3], [f64; 3]) {
        match self {
            SplitNode::Leaf { min, max, .. } | SplitNode::Branch { min, max, .. } => (*min, *max),
        }
    }
}
