//! k-d split by triangle centroid until the leaf triangle budget.
//!
//! The final cut into two leaves duplicates a thin band around the split
//! plane so adjacent leaves share a strip of triangles (hides hairline seams).
//! Deeper splits stay a hard median partition so the tree does not explode.
//! After the tree is built, [`seal_leaf_borders`] grows every leaf by a world
//! margin so seams from *earlier* split axes are covered too.

use std::collections::{HashMap, HashSet};

use crate::bbox::y_up_to_z_up;
use crate::mesh::{centroid, triangle_aabb_yup, Scene};

pub const MAX_SPLIT_DEPTH: u32 = 12;
const PARALLEL_AFTER: usize = 32_768;
/// Fraction of the split-axis extent duplicated into both final leaves.
const LEAF_OVERLAP_FRAC: f32 = 0.05;
/// Expand each leaf AABB by this fraction of its max half-extent, then pull in
/// any intersecting triangles (covers seams from non-final split axes).
const BORDER_SEAL_FRAC: f32 = 0.06;

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

/// Grow every leaf so triangles near its boundary are duplicated into the leaf.
///
/// Centroid k-d only overlaps the *last* split; earlier planes still crack.
/// This post-pass pulls in any triangle whose AABB intersects an expanded leaf
/// box, capped so leaves stay near the triangle budget.
pub fn seal_leaf_borders(scene: &Scene, root: &mut SplitNode, max_triangles: usize) {
    if scene.triangles.is_empty() {
        return;
    }
    let mut scene_min = [f32::INFINITY; 3];
    let mut scene_max = [f32::NEG_INFINITY; 3];
    let mut tri_aabb = Vec::with_capacity(scene.triangles.len());
    let mut tri_cent = Vec::with_capacity(scene.triangles.len());
    for t in &scene.triangles {
        let mut mn = [f32::INFINITY; 3];
        let mut mx = [f32::NEG_INFINITY; 3];
        let mut c = [0.0f32; 3];
        for &v in &t.verts {
            let p = y_up_to_z_up(scene.vertices[v as usize].pos);
            for i in 0..3 {
                mn[i] = mn[i].min(p[i]);
                mx[i] = mx[i].max(p[i]);
                scene_min[i] = scene_min[i].min(p[i]);
                scene_max[i] = scene_max[i].max(p[i]);
                c[i] += p[i];
            }
        }
        c = [c[0] / 3.0, c[1] / 3.0, c[2] / 3.0];
        tri_aabb.push((mn, mx));
        tri_cent.push(c);
    }
    let extent = (scene_max[0] - scene_min[0])
        .max(scene_max[1] - scene_min[1])
        .max(scene_max[2] - scene_min[2])
        .max(1e-3);
    let cell = extent / 64.0;
    let mut bins: HashMap<(i32, i32, i32), Vec<usize>> = HashMap::new();
    for (i, c) in tri_cent.iter().enumerate() {
        let key = (
            ((c[0] - scene_min[0]) / cell).floor() as i32,
            ((c[1] - scene_min[1]) / cell).floor() as i32,
            ((c[2] - scene_min[2]) / cell).floor() as i32,
        );
        bins.entry(key).or_default().push(i);
    }
    let cap = max_triangles + max_triangles / 4;
    seal_node(
        root, &tri_aabb, &tri_cent, &bins, scene_min, cell, cap, scene,
    );
}

fn seal_node(
    node: &mut SplitNode,
    tri_aabb: &[([f32; 3], [f32; 3])],
    tri_cent: &[[f32; 3]],
    bins: &HashMap<(i32, i32, i32), Vec<usize>>,
    scene_min: [f32; 3],
    cell: f32,
    cap: usize,
    scene: &Scene,
) {
    match node {
        SplitNode::Leaf {
            triangle_ids,
            min,
            max,
        } => {
            let hx = ((max[0] - min[0]) * 0.5).max(0.0);
            let hy = ((max[1] - min[1]) * 0.5).max(0.0);
            let hz = ((max[2] - min[2]) * 0.5).max(0.0);
            let margin = hx.max(hy).max(hz) * BORDER_SEAL_FRAC as f64;
            if margin <= 0.0 {
                return;
            }
            let emin = [
                (min[0] - margin) as f32,
                (min[1] - margin) as f32,
                (min[2] - margin) as f32,
            ];
            let emax = [
                (max[0] + margin) as f32,
                (max[1] + margin) as f32,
                (max[2] + margin) as f32,
            ];
            let mut have: HashSet<usize> = triangle_ids.iter().copied().collect();
            let mut extras: Vec<(f32, usize)> = Vec::new();
            let i0 = (((emin[0] - scene_min[0]) / cell).floor() as i32 - 1).max(0);
            let j0 = (((emin[1] - scene_min[1]) / cell).floor() as i32 - 1).max(0);
            let k0 = (((emin[2] - scene_min[2]) / cell).floor() as i32 - 1).max(0);
            let i1 = ((emax[0] - scene_min[0]) / cell).floor() as i32 + 1;
            let j1 = ((emax[1] - scene_min[1]) / cell).floor() as i32 + 1;
            let k1 = ((emax[2] - scene_min[2]) / cell).floor() as i32 + 1;
            for ix in i0..=i1 {
                for iy in j0..=j1 {
                    for iz in k0..=k1 {
                        let Some(bin) = bins.get(&(ix, iy, iz)) else {
                            continue;
                        };
                        for &id in bin {
                            if have.contains(&id) {
                                continue;
                            }
                            let (tn, tx) = tri_aabb[id];
                            if !aabb_overlap(tn, tx, emin, emax) {
                                continue;
                            }
                            // Prefer triangles near the original (unexpanded) surface.
                            let c = tri_cent[id];
                            let d = dist_point_aabb(
                                c,
                                [min[0] as f32, min[1] as f32, min[2] as f32],
                                [max[0] as f32, max[1] as f32, max[2] as f32],
                            );
                            extras.push((d, id));
                        }
                    }
                }
            }
            extras.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            for (_, id) in extras {
                if triangle_ids.len() >= cap {
                    break;
                }
                if have.insert(id) {
                    triangle_ids.push(id);
                }
            }
            let (nmin, nmax) = zup_aabb(scene, triangle_ids);
            *min = nmin;
            *max = nmax;
        }
        SplitNode::Branch { children, min, max } => {
            for c in children.iter_mut() {
                seal_node(c, tri_aabb, tri_cent, bins, scene_min, cell, cap, scene);
            }
            let (nmin, nmax) = union_zup(children);
            *min = nmin;
            *max = nmax;
        }
    }
}

fn aabb_overlap(a0: [f32; 3], a1: [f32; 3], b0: [f32; 3], b1: [f32; 3]) -> bool {
    a0[0] <= b1[0]
        && a1[0] >= b0[0]
        && a0[1] <= b1[1]
        && a1[1] >= b0[1]
        && a0[2] <= b1[2]
        && a1[2] >= b0[2]
}

fn dist_point_aabb(p: [f32; 3], mn: [f32; 3], mx: [f32; 3]) -> f32 {
    let mut d2 = 0.0f32;
    for i in 0..3 {
        let v = if p[i] < mn[i] {
            mn[i] - p[i]
        } else if p[i] > mx[i] {
            p[i] - mx[i]
        } else {
            0.0
        };
        d2 += v * v;
    }
    d2
}
