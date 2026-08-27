//! Axis-aligned bounding volume.box from glTF POSITION data (3D Tiles 1.1).

use std::path::Path;

use gltf::buffer::Data as BufferData;
use gltf::Semantic;

use crate::error::Error;

/// 12-element `boundingVolume.box`: center + half-axes (column-wise x, y, z).
pub type BoundingBox = [f64; 12];

pub fn bounding_box_from_gltf_path(path: &Path) -> Result<BoundingBox, Error> {
    let (gltf, buffers, _) = gltf::import(path)?;
    bounding_box_from_gltf(&gltf, &buffers)
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
                let reader = prim.reader(|b| Some(&buffers[b.index()][..]));
                if let Some(iter) = reader.read_positions() {
                    for p in iter {
                        let tp = y_up_to_z_up(transform_point(world, p));
                        for i in 0..3 {
                            min[i] = min[i].min(tp[i] as f64);
                            max[i] = max[i].max(tp[i] as f64);
                        }
                        *any = true;
                    }
                } else if let Some(acc) = prim.get(&Semantic::Positions) {
                    if let (Some(mn), Some(mx)) = (acc.min(), acc.max()) {
                        let mn = json_vec3(&mn)?;
                        let mx = json_vec3(&mx)?;
                        for corner in aabb_corners(mn, mx) {
                            let tp = y_up_to_z_up(transform_point(world, corner));
                            for i in 0..3 {
                                min[i] = min[i].min(tp[i] as f64);
                                max[i] = max[i].max(tp[i] as f64);
                            }
                            *any = true;
                        }
                    }
                }
            }
        }
        collect_nodes(node.children(), buffers, world, min, max, any)?;
    }
    Ok(())
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

fn mul4(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
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

fn transform_point(m: [[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0],
        m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1],
        m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2],
    ]
}

/// glTF Y-up → 3D Tiles Z-up (Cesium `Axis.Y_UP_TO_Z_UP`).
fn y_up_to_z_up(p: [f32; 3]) -> [f32; 3] {
    [p[0], -p[2], p[1]]
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

/// Inverse of `aabb_to_box` for unioning leaf boxes in a parent tile.
pub fn box_to_aabb(b: BoundingBox) -> ([f64; 3], [f64; 3]) {
    let (cx, cy, cz) = (b[0], b[1], b[2]);
    let hx = (b[3] * b[3] + b[4] * b[4] + b[5] * b[5]).sqrt();
    let hy = (b[6] * b[6] + b[7] * b[7] + b[8] * b[8]).sqrt();
    let hz = (b[9] * b[9] + b[10] * b[10] + b[11] * b[11]).sqrt();
    ([cx - hx, cy - hy, cz - hz], [cx + hx, cy + hy, cz + hz])
}
