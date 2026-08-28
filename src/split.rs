//! k-d split by triangle centroid until the leaf triangle budget.
//!
//! The final cut into two leaves duplicates a thin band around the split
//! plane so adjacent leaves share a strip of triangles (hides hairline seams).
//! Deeper splits stay a hard median partition so the tree does not explode.

use crate::bbox::y_up_to_z_up;
use crate::mesh::{centroid, triangle_aabb_yup, Scene};

pub const MAX_SPLIT_DEPTH: u32 = 12;
const PARALLEL_AFTER: usize = 32_768;
/// Fraction of the split-axis extent duplicated into both final leaves.
const LEAF_OVERLAP_FRAC: f32 = 0.03;

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

pub fn split(scene: &Scene, opts: &SplitOpts) -> SplitNode {
    let centroids: Vec<[f32; 3]> = scene.triangles.iter().map(|t| centroid(scene, t)).collect();
    let mut ids: Vec<usize> = (0..scene.triangles.len()).collect();
    split_ids(scene, &centroids, &mut ids, 0, opts).unwrap_or_else(|| leaf(scene, ids))
}

fn split_ids(
    scene: &Scene,
    centroids: &[[f32; 3]],
    ids: &mut [usize],
    depth: u32,
    opts: &SplitOpts,
) -> Option<SplitNode> {
    if ids.is_empty() {
        return None;
    }
    if ids.len() <= opts.max_triangles || depth >= MAX_SPLIT_DEPTH || ids.len() < 2 {
        return Some(leaf(scene, ids.to_vec()));
    }

    let (ymin, ymax) = triangle_aabb_yup(scene, ids);
    let axis = longest_axis(ymin, ymax);
    let mid = ids.len() / 2;
    ids.select_nth_unstable_by(mid, |&a, &b| {
        centroids[a][axis]
            .partial_cmp(&centroids[b][axis])
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Final cut: both children fit the leaf budget — overlap the split band.
    if mid <= opts.max_triangles && ids.len() - mid <= opts.max_triangles {
        let split_at = centroids[ids[mid]][axis];
        let extent = (ymax[axis] - ymin[axis]).max(1e-6);
        let margin = extent * LEAF_OVERLAP_FRAC;
        let mut left = Vec::with_capacity(mid + mid / 16);
        let mut right = Vec::with_capacity(ids.len() - mid + mid / 16);
        for &id in ids.iter() {
            let c = centroids[id][axis];
            if c < split_at - margin {
                left.push(id);
            } else if c > split_at + margin {
                right.push(id);
            } else {
                left.push(id);
                right.push(id);
            }
        }
        if left.is_empty()
            || right.is_empty()
            || left.len() >= ids.len()
            || right.len() >= ids.len()
        {
            return Some(branch_hard(scene, ids, mid));
        }
        // Cap overlap so a leaf cannot grow without bound.
        let cap = opts.max_triangles + opts.max_triangles / 8;
        if left.len() > cap || right.len() > cap {
            return Some(branch_hard(scene, ids, mid));
        }
        let children = vec![leaf(scene, left), leaf(scene, right)];
        let (min, max) = union_zup(&children);
        return Some(SplitNode::Branch { children, min, max });
    }

    let (left, right) = ids.split_at_mut(mid);
    let (l, r) = if left.len() >= PARALLEL_AFTER && right.len() >= PARALLEL_AFTER {
        rayon::join(
            || split_ids(scene, centroids, left, depth + 1, opts),
            || split_ids(scene, centroids, right, depth + 1, opts),
        )
    } else {
        (
            split_ids(scene, centroids, left, depth + 1, opts),
            split_ids(scene, centroids, right, depth + 1, opts),
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
        _ => {
            let (min, max) = union_zup(&children);
            Some(SplitNode::Branch { children, min, max })
        }
    }
}

fn branch_hard(scene: &Scene, ids: &[usize], mid: usize) -> SplitNode {
    let children = vec![
        leaf(scene, ids[..mid].to_vec()),
        leaf(scene, ids[mid..].to_vec()),
    ];
    let (min, max) = union_zup(&children);
    SplitNode::Branch { children, min, max }
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
