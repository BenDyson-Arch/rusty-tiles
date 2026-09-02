//! Axis-aligned bounding volume.box from glTF POSITION data (3D Tiles 1.1).

use std::fs::File;
use std::io::Read;
use std::path::Path;

use gltf::buffer::Data as BufferData;
use gltf::Semantic;

use crate::error::Error;

/// 12-element `boundingVolume.box`: center + half-axes (column-wise x, y, z).
pub type BoundingBox = [f64; 12];

pub fn bounding_box_from_gltf_path(path: &Path) -> Result<BoundingBox, Error> {
    let document = read_gltf_document(path)?;
    match bounding_box_from_gltf(&document, &[]) {
        Ok(b) => Ok(b),
        Err(_) => {
            // Accessors without min/max: load BIN (still skip image decode).
            let gltf = gltf::Gltf::open(path)?;
            let buffers = gltf::import_buffers(&gltf.document, path.parent(), gltf.blob)?;
            bounding_box_from_gltf(&gltf.document, &buffers)
        }
    }
}

/// Parse glTF JSON only. For `.glb` this reads the JSON chunk, not the 1.5 GiB BIN/textures.
fn read_gltf_document(path: &Path) -> Result<gltf::Document, Error> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "glb" {
        let mut f = File::open(path)?;
        let mut header = [0u8; 20];
        f.read_exact(&mut header)?;
        if &header[0..4] != b"glTF" {
            return Err(Error::msg("not a GLB"));
        }
        let json_len = u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize;
        let mut json = vec![0u8; json_len];
        f.read_exact(&mut json)?;
        Ok(gltf::Gltf::from_slice(&json)?.document)
    } else {
        let data = std::fs::read(path)?;
        Ok(gltf::Gltf::from_slice(&data)?.document)
    }
}

pub fn bounding_box_from_gltf(
    gltf: &gltf::Document,
    buffers: &[BufferData],
) -> Result<BoundingBox, Error> {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    let mut any = false;

    if gltf.scenes().len() == 0 {
        collect_nodes(
            gltf.nodes(),
            buffers,
            IDENTITY,
            &mut min,
            &mut max,
            &mut any,
        )?;
    } else {
        for scene in gltf.scenes() {
            collect_nodes(
                scene.nodes(),
                buffers,
                IDENTITY,
                &mut min,
                &mut max,
                &mut any,
            )?;
        }
    }

    if !any {
        return Err(Error::msg("glTF has no POSITION attributes"));
    }

    Ok(aabb_to_box(min, max))
}

const IDENTITY: [[f32; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

fn collect_nodes<'a>(
    nodes: impl Iterator<Item = gltf::Node<'a>>,
    buffers: &[BufferData],
    parent: [[f32; 4]; 4],
    min: &mut [f64; 3],
    max: &mut [f64; 3],
    any: &mut bool,
) -> Result<(), Error> {
    for node in nodes {
        let local = node.transform().matrix();
        let world = mul4(parent, local);
        if let Some(mesh) = node.mesh() {
            for prim in mesh.primitives() {
                if let Some(acc) = prim.get(&Semantic::Positions) {
                    if let (Some(mn), Some(mx)) = (acc.min(), acc.max()) {
                        let mn = json_vec3(&mn)?;
                        let mx = json_vec3(&mx)?;
                        for corner in aabb_corners(mn, mx) {
                            include(min, max, any, y_up_to_z_up(transform_point(world, corner)));
                        }
                        continue;
                    }
                }
                if buffers.is_empty() {
                    continue;
                }
                let reader = prim.reader(|b| buffers.get(b.index()).map(|d| d.0.as_slice()));
                if let Some(iter) = reader.read_positions() {
                    for p in iter {
                        include(min, max, any, y_up_to_z_up(transform_point(world, p)));
                    }
                }
            }
        }
        collect_nodes(node.children(), buffers, world, min, max, any)?;
    }
    Ok(())
}

fn include(min: &mut [f64; 3], max: &mut [f64; 3], any: &mut bool, p: [f32; 3]) {
    for i in 0..3 {
        min[i] = min[i].min(p[i] as f64);
        max[i] = max[i].max(p[i] as f64);
    }
    *any = true;
}

fn json_vec3(v: &gltf::json::Value) -> Result<[f32; 3], Error> {
    let arr = v
        .as_array()
        .ok_or_else(|| Error::msg("POSITION min/max is not an array"))?;
    if arr.len() < 3 {
        return Err(Error::msg("POSITION min/max is not VEC3"));
    }
    let n = |i: usize| {
        arr[i]
            .as_f64()
            .map(|x| x as f32)
            .ok_or_else(|| Error::msg("POSITION min/max is not numeric"))
    };
    Ok([n(0)?, n(1)?, n(2)?])
}

fn aabb_corners(min: [f32; 3], max: [f32; 3]) -> [[f32; 3]; 8] {
    [
        [min[0], min[1], min[2]],
        [max[0], min[1], min[2]],
        [min[0], max[1], min[2]],
        [max[0], max[1], min[2]],
        [min[0], min[1], max[2]],
        [max[0], min[1], max[2]],
        [min[0], max[1], max[2]],
        [max[0], max[1], max[2]],
    ]
}

pub(crate) fn mul4(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut c = [[0.0f32; 4]; 4];
    for col in 0..4 {
        for row in 0..4 {
            c[col][row] = a[0][row] * b[col][0]
                + a[1][row] * b[col][1]
                + a[2][row] * b[col][2]
                + a[3][row] * b[col][3];
        }
    }
    c
}

pub(crate) fn transform_point(m: [[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0],
        m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1],
        m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2],
    ]
}

/// glTF Y-up → 3D Tiles Z-up (Cesium `Axis.Y_UP_TO_Z_UP`).
pub(crate) fn y_up_to_z_up(p: [f32; 3]) -> [f32; 3] {
    [p[0], -p[2], p[1]]
}

pub(crate) fn z_up_to_y_up(p: [f64; 3]) -> [f64; 3] {
    [p[0], p[2], -p[1]]
}

/// Z-up AABB → Y-up AABB (`yup.z = -zup.y` flips that axis).
pub(crate) fn aabb_zup_to_yup(min: [f64; 3], max: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let a = z_up_to_y_up(min);
    let b = z_up_to_y_up(max);
    (
        [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])],
        [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])],
    )
}

pub fn aabb_center(min: [f64; 3], max: [f64; 3]) -> [f64; 3] {
    [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ]
}

pub fn aabb_diagonal(min: [f64; 3], max: [f64; 3]) -> f64 {
    let dx = max[0] - min[0];
    let dy = max[1] - min[1];
    let dz = max[2] - min[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

pub fn aabb_to_box(min: [f64; 3], max: [f64; 3]) -> BoundingBox {
    let cx = (min[0] + max[0]) * 0.5;
    let cy = (min[1] + max[1]) * 0.5;
    let cz = (min[2] + max[2]) * 0.5;
    let hx = (max[0] - min[0]) * 0.5;
    let hy = (max[1] - min[1]) * 0.5;
    let hz = (max[2] - min[2]) * 0.5;
    [cx, cy, cz, hx, 0.0, 0.0, 0.0, hy, 0.0, 0.0, 0.0, hz]
}

pub fn union_aabb(
    a_min: [f64; 3],
    a_max: [f64; 3],
    b_min: [f64; 3],
    b_max: [f64; 3],
) -> ([f64; 3], [f64; 3]) {
    (
        [
            a_min[0].min(b_min[0]),
            a_min[1].min(b_min[1]),
            a_min[2].min(b_min[2]),
        ],
        [
            a_max[0].max(b_max[0]),
            a_max[1].max(b_max[1]),
            a_max[2].max(b_max[2]),
        ],
    )
}

/// Axis-aligned envelope of a (possibly oriented) `boundingVolume.box`.
pub fn box_to_aabb(b: BoundingBox) -> ([f64; 3], [f64; 3]) {
    let c = [b[0], b[1], b[2]];
    let mut h = [0.0; 3];
    for k in 0..3 {
        for i in 0..3 {
            h[i] += b[3 + k * 3 + i].abs();
        }
    }
    (
        [c[0] - h[0], c[1] - h[1], c[2] - h[2]],
        [c[0] + h[0], c[1] + h[1], c[2] + h[2]],
    )
}

/// Oriented bounding box: `center + Σ t_k · half[k] · axes[k]`, |t_k| ≤ 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obb {
    pub center: [f64; 3],
    /// Orthonormal, row k is axis k.
    pub axes: [[f64; 3]; 3],
    pub half: [f64; 3],
}

const AXIS_ALIGNED: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

impl Obb {
    pub fn from_aabb(min: [f64; 3], max: [f64; 3]) -> Obb {
        Obb {
            center: aabb_center(min, max),
            axes: AXIS_ALIGNED,
            half: [
                (max[0] - min[0]) * 0.5,
                (max[1] - min[1]) * 0.5,
                (max[2] - min[2]) * 0.5,
            ],
        }
    }

    pub fn from_box(b: BoundingBox) -> Obb {
        let mut axes = AXIS_ALIGNED;
        let mut half = [0.0; 3];
        for k in 0..3 {
            let v = [b[3 + k * 3], b[4 + k * 3], b[5 + k * 3]];
            let n = norm(v);
            half[k] = n;
            if n > 0.0 {
                axes[k] = [v[0] / n, v[1] / n, v[2] / n];
            }
        }
        Obb {
            center: [b[0], b[1], b[2]],
            axes,
            half,
        }
    }

    /// Tightest of the axis-aligned box and a PCA-oriented box over `points`.
    /// The oriented fit must save ≥ 10 % volume to be worth the rotation.
    /// Every input point lies inside the result (extents come from the points
    /// themselves), so feeding child box corners gives spec-valid nesting.
    pub fn fit(points: &[[f64; 3]]) -> Option<Obb> {
        if points.is_empty() {
            return None;
        }
        let aabb = Obb::extents(points, AXIS_ALIGNED);
        let n = points.len() as f64;
        let mut mean = [0.0; 3];
        for p in points {
            for i in 0..3 {
                mean[i] += p[i] / n;
            }
        }
        let mut cov = [[0.0; 3]; 3];
        for p in points {
            let d = [p[0] - mean[0], p[1] - mean[1], p[2] - mean[2]];
            for i in 0..3 {
                for j in 0..3 {
                    cov[i][j] += d[i] * d[j] / n;
                }
            }
        }
        let axes = jacobi_eigenvectors(cov);
        let pca = Obb::extents(points, axes);
        Some(if pca.volume() < aabb.volume() * 0.9 {
            pca
        } else {
            aabb
        })
    }

    fn extents(points: &[[f64; 3]], axes: [[f64; 3]; 3]) -> Obb {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for p in points {
            for k in 0..3 {
                let t = dot(*p, axes[k]);
                lo[k] = lo[k].min(t);
                hi[k] = hi[k].max(t);
            }
        }
        let mut center = [0.0; 3];
        let mut half = [0.0; 3];
        for k in 0..3 {
            let mid = (lo[k] + hi[k]) * 0.5;
            half[k] = (hi[k] - lo[k]) * 0.5;
            for i in 0..3 {
                center[i] += mid * axes[k][i];
            }
        }
        Obb { center, axes, half }
    }

    pub fn volume(&self) -> f64 {
        8.0 * self.half[0] * self.half[1] * self.half[2]
    }

    #[allow(clippy::needless_range_loop)]
    pub fn corners(&self) -> [[f64; 3]; 8] {
        let mut out = [[0.0; 3]; 8];
        for (n, c) in out.iter_mut().enumerate() {
            let s = [
                if n & 1 == 0 { -1.0 } else { 1.0 },
                if n & 2 == 0 { -1.0 } else { 1.0 },
                if n & 4 == 0 { -1.0 } else { 1.0 },
            ];
            for i in 0..3 {
                c[i] = self.center[i]
                    + s[0] * self.half[0] * self.axes[0][i]
                    + s[1] * self.half[1] * self.axes[1][i]
                    + s[2] * self.half[2] * self.axes[2][i];
            }
        }
        out
    }

    pub fn contains(&self, p: [f64; 3], eps: f64) -> bool {
        let d = [
            p[0] - self.center[0],
            p[1] - self.center[1],
            p[2] - self.center[2],
        ];
        (0..3).all(|k| dot(d, self.axes[k]).abs() <= self.half[k] + eps)
    }

    /// Cesium needs a hair of thickness on every axis.
    pub fn padded_flat(mut self, min_half: f64) -> Obb {
        for h in &mut self.half {
            if *h < min_half {
                *h = min_half;
            }
        }
        self
    }

    pub fn to_box(&self) -> BoundingBox {
        let mut b = [0.0; 12];
        b[..3].copy_from_slice(&self.center);
        for k in 0..3 {
            for i in 0..3 {
                b[3 + k * 3 + i] = self.axes[k][i] * self.half[k];
            }
        }
        b
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

/// Eigenvectors of a symmetric 3×3 (cyclic Jacobi), rows sorted by
/// descending eigenvalue, made right-handed.
#[allow(clippy::needless_range_loop)]
fn jacobi_eigenvectors(mut a: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut v = AXIS_ALIGNED;
    for _ in 0..32 {
        let off = a[0][1].abs() + a[0][2].abs() + a[1][2].abs();
        if off < 1e-18 {
            break;
        }
        for (p, q) in [(0usize, 1usize), (0, 2), (1, 2)] {
            if a[p][q].abs() < 1e-300 {
                continue;
            }
            let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
            let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
            let t = if theta == 0.0 { 1.0 } else { t };
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            for k in 0..3 {
                let (akp, akq) = (a[k][p], a[k][q]);
                a[k][p] = c * akp - s * akq;
                a[k][q] = s * akp + c * akq;
            }
            for k in 0..3 {
                let (apk, aqk) = (a[p][k], a[q][k]);
                a[p][k] = c * apk - s * aqk;
                a[q][k] = s * apk + c * aqk;
            }
            for k in 0..3 {
                let (vkp, vkq) = (v[k][p], v[k][q]);
                v[k][p] = c * vkp - s * vkq;
                v[k][q] = s * vkp + c * vkq;
            }
        }
    }
    // Columns of v are eigenvectors; emit as rows, largest eigenvalue first.
    let mut order = [0usize, 1, 2];
    order.sort_by(|&i, &j| {
        a[j][j]
            .partial_cmp(&a[i][i])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut axes = [[0.0; 3]; 3];
    for (r, &c) in order.iter().enumerate() {
        axes[r] = [v[0][c], v[1][c], v[2][c]];
        let n = norm(axes[r]);
        if n > 0.0 {
            for x in &mut axes[r] {
                *x /= n;
            }
        }
    }
    let cross = [
        axes[0][1] * axes[1][2] - axes[0][2] * axes[1][1],
        axes[0][2] * axes[1][0] - axes[0][0] * axes[1][2],
        axes[0][0] * axes[1][1] - axes[0][1] * axes[1][0],
    ];
    if dot(cross, axes[2]) < 0.0 {
        for x in &mut axes[2] {
            *x = -*x;
        }
    }
    axes
}

#[cfg(test)]
mod obb_tests {
    use super::*;

    #[test]
    fn tilted_sheet_gets_a_much_smaller_oriented_box() {
        // Thin sheet in the plane x = y (45° about z), 10 × 10 × 0.02.
        let mut pts = Vec::new();
        for i in 0..=20 {
            for j in 0..=20 {
                let u = i as f64 * 0.5 - 5.0;
                let w = j as f64 * 0.5 - 5.0;
                let n = if (i + j) % 2 == 0 { 0.01 } else { -0.01 };
                let s = std::f64::consts::FRAC_1_SQRT_2;
                pts.push([u * s - n * s, u * s + n * s, w]);
            }
        }
        let obb = Obb::fit(&pts).unwrap();
        let aabb = Obb::extents(&pts, AXIS_ALIGNED);
        assert!(
            obb.volume() < aabb.volume() * 0.05,
            "{} vs {}",
            obb.volume(),
            aabb.volume()
        );
        for p in &pts {
            assert!(obb.contains(*p, 1e-9));
        }
        let (lo, hi) = box_to_aabb(obb.to_box());
        for p in &pts {
            for ((x, l), h) in p.iter().zip(&lo).zip(&hi) {
                assert!(*x >= l - 1e-9 && *x <= h + 1e-9);
            }
        }
    }

    #[test]
    fn axis_aligned_cloud_keeps_axis_aligned_box() {
        let pts: Vec<[f64; 3]> = (0..64)
            .map(|i| {
                [
                    (i % 4) as f64,
                    ((i / 4) % 4) as f64 * 2.0,
                    (i / 16) as f64 * 3.0,
                ]
            })
            .collect();
        let obb = Obb::fit(&pts).unwrap();
        assert_eq!(obb.axes, AXIS_ALIGNED);
        assert_eq!(obb.half, [1.5, 3.0, 4.5]);
    }

    #[test]
    fn children_corners_nest_inside_parent_fit() {
        let child = Obb {
            center: [3.0, 4.0, 5.0],
            axes: [[0.6, 0.8, 0.0], [-0.8, 0.6, 0.0], [0.0, 0.0, 1.0]],
            half: [2.0, 0.5, 0.1],
        };
        let mut pts: Vec<[f64; 3]> = child.corners().to_vec();
        pts.push([0.0, 0.0, 0.0]);
        let parent = Obb::fit(&pts).unwrap();
        for c in child.corners() {
            assert!(parent.contains(c, 1e-9));
        }
    }
}
