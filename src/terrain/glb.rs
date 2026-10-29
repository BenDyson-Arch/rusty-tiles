//! Terrain tile GLBs: one opaque primitive holding the surface triangles and
//! then the skirt triangles, in one shared local frame. Positions are uint16
//! `KHR_mesh_quantization` integers on the level's power-of-two grid, or
//! Float32 where a full-detail grid would exceed the storage limit. The vertex
//! and index views are `EXT_meshopt_compression` streams.
use crate::glb::{self as container, MeshoptLayout, MeshoptStream};
use crate::{JobError, JobErrorKind};
use serde_json::json;
/// A tile's surface lattice. Positions are decoded glTF (y-up) coordinates,
/// exactly representable by the level's [`Storage`]. Only `used` nodes, those
/// of a triangle whose three nodes are valid, are encoded; the others hold
/// their zero-height lattice position.
pub(super) struct Grid {
    pub positions: Vec<[f64; 3]>,
    pub used: Vec<bool>,
    pub columns: usize,
    pub rows: usize,
    pub bounds: [[f64; 3]; 2],
    pub max_position_error: f64,
    pub height_range: [f64; 2],
}
/// Vertical curtains below a tile's interior edges. Their tops are the surface
/// vertices themselves; `indices` address the surface lattice followed by
/// `bottoms`.
pub(super) struct Skirt {
    pub bottoms: Vec<[f64; 3]>,
    pub indices: Vec<u32>,
}
/// Largest quantized coordinate: uint16 positions.
pub(super) const QUANTIZED_STEPS: f64 = 65535.;
/// How one level stores positions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Storage {
    /// Unrounded frame positions: planning estimates only, never encoded.
    Exact,
    Float,
    /// Unnormalized uint16 multiples of a power-of-two step per glTF axis (the
    /// planner uses one step for all three). The
    /// node translation is a multiple of the step, so decoding is exact and
    /// tiles of one level agree on every shared vertex.
    Quantized([f64; 3]),
}
impl Storage {
    /// The decoded glTF position storing `p`.
    pub(super) fn store(self, p: [f64; 3]) -> [f64; 3] {
        match self {
            Storage::Exact => p,
            Storage::Float => p.map(|v| f64::from(v as f32)),
            // Division and multiplication by a power of two are exact.
            Storage::Quantized(step) => {
                std::array::from_fn(|i| (p[i] / step[i]).round_ties_even() * step[i])
            }
        }
    }
    /// An upper bound on twice the distance between a decoded position and the
    /// position it stores, for coordinates of at most `magnitude`.
    pub(super) fn margin(self, magnitude: f64) -> f64 {
        match self {
            Storage::Exact => 0.,
            // Rounding to Float32 moves each coordinate by at most |v| 2^-24.
            Storage::Float => (2. * 3f64.sqrt() * magnitude * 2f64.powi(-24)).next_up(),
            Storage::Quantized(step) => step.iter().map(|s| s * s).sum::<f64>().sqrt().next_up(),
        }
    }
}
fn failure(message: impl std::fmt::Display) -> JobError {
    JobError::new(
        JobErrorKind::InvalidState,
        format!("encode terrain GLB: {message}"),
    )
}
/// Every view is a meshopt stream on a four-byte boundary.
const LAYOUT: MeshoptLayout = MeshoptLayout {
    align: 4,
    explicit_filter: false,
};
pub(super) fn encode(
    grid: &Grid,
    skirt: &Skirt,
    storage: Storage,
    mut checkpoint: impl FnMut() -> Result<(), JobError>,
) -> Result<Vec<u8>, JobError> {
    // Lattice index to encoded index; unused nodes are dropped.
    let mut remap = vec![u32::MAX; grid.positions.len()];
    let mut used = 0;
    for (i, _) in grid.used.iter().enumerate().filter(|(_, u)| **u) {
        remap[i] = used;
        used += 1;
    }
    let count = used as usize + skirt.bottoms.len();
    if count > 1 << 16 {
        return Err(JobError::new(
            JobErrorKind::Unsupported,
            "terrain tile exceeds 65536 vertices",
        ));
    }
    let positions = || {
        (grid.positions.iter().zip(&grid.used))
            .filter_map(|(p, u)| u.then_some(p))
            .chain(&skirt.bottoms)
    };
    let mut binary =
        Vec::with_capacity(count * 12 + (grid.columns * grid.rows + skirt.indices.len()) * 12);
    let mut node = json!({"mesh":0});
    let (component, stride, minimum, maximum) = match storage {
        Storage::Exact => return Err(failure("unrounded positions are not encodable")),
        Storage::Float => {
            let mut minimum = [f32::INFINITY; 3];
            let mut maximum = [f32::NEG_INFINITY; 3];
            for (i, p) in positions().enumerate() {
                if i % 4096 == 0 {
                    checkpoint()?
                }
                for (axis, &v) in p.iter().enumerate() {
                    let stored = v as f32;
                    debug_assert_eq!(f64::from(stored), v);
                    minimum[axis] = minimum[axis].min(stored);
                    maximum[axis] = maximum[axis].max(stored);
                    binary.extend(stored.to_le_bytes())
                }
            }
            (5126, 12, json!(minimum), json!(maximum))
        }
        Storage::Quantized(step) => {
            // Integer grid coordinates; the tile origin is their minimum.
            let mut origin = [f64::INFINITY; 3];
            for p in positions() {
                for axis in 0..3 {
                    origin[axis] = origin[axis].min(p[axis] / step[axis]);
                }
            }
            let mut maximum = [0u16; 3];
            for (i, p) in positions().enumerate() {
                if i % 4096 == 0 {
                    checkpoint()?
                }
                for axis in 0..3 {
                    let q = p[axis] / step[axis] - origin[axis];
                    if !(0. ..=QUANTIZED_STEPS).contains(&q) || q.fract() != 0. {
                        return Err(failure("position off its level's quantization grid"));
                    }
                    let q = q as u16;
                    maximum[axis] = maximum[axis].max(q);
                    binary.extend(q.to_le_bytes())
                }
                binary.extend([0, 0]);
            }
            node["translation"] = json!(std::array::from_fn::<f64, 3, _>(|i| origin[i] * step[i]));
            node["scale"] = json!(step);
            (5123, 8, json!([0, 0, 0]), json!(maximum))
        }
    };
    let position_bytes = binary.len();
    checkpoint()?;
    let mut indices = 0;
    for y in 0..grid.rows - 1 {
        for x in 0..grid.columns - 1 {
            let a = y * grid.columns + x;
            let c = grid.columns;
            for t in [[a, a + 1, a + c], [a + 1, a + c + 1, a + c]] {
                if t.iter().all(|&i| grid.used[i]) {
                    for i in t {
                        binary.extend((remap[i] as u16).to_le_bytes())
                    }
                    indices += 3;
                }
            }
        }
    }
    // Skirt triangles follow the surface in the same primitive and material.
    let surface = grid.positions.len() as u32;
    for &i in &skirt.indices {
        let i = if i < surface {
            remap[i as usize]
        } else {
            used + i - surface
        };
        binary.extend(u16::try_from(i).map_err(failure)?.to_le_bytes())
    }
    indices += skirt.indices.len();
    let index_bytes = binary.len() - position_bytes;
    let mut document = json!({
      "asset":{"version":"2.0","generator":"rusty-tiles T1 terrain mesh"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[node],
      "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1,"material":0,"mode":4}]}],
      "materials":[{"doubleSided":false,"alphaMode":"OPAQUE","pbrMetallicRoughness":{"baseColorFactor":[1.0,1.0,1.0,1.0],"metallicFactor":0.0,"roughnessFactor":1.0}}],
      "buffers":[{"byteLength":binary.len()}],
      "bufferViews":[
        {"buffer":0,"byteOffset":0,"byteLength":position_bytes,"byteStride":stride,"target":34962},
        {"buffer":0,"byteOffset":position_bytes,"byteLength":index_bytes,"target":34963}],
      "accessors":[
        {"bufferView":0,"componentType":component,"count":count,"type":"VEC3","min":minimum,"max":maximum},
        {"bufferView":1,"componentType":5123,"count":indices,"type":"SCALAR"}]
    });
    checkpoint()?;
    let packed = container::meshopt_compress(&mut document, &binary, LAYOUT, |view, _| {
        Ok(Some(if view == 0 {
            MeshoptStream::Attributes { count, stride }
        } else {
            MeshoptStream::TriangleIndices {
                count: indices,
                width: 2,
                vertex_count: count,
            }
        }))
    })
    .map_err(failure)?;
    if matches!(storage, Storage::Quantized(_)) {
        container::add_extension(&mut document, "KHR_mesh_quantization", true).map_err(failure)?;
    }
    container::encode_glb(&document, &packed).map_err(|e| match e {
        crate::Error::Data(_) => JobError::new(
            JobErrorKind::Unsupported,
            "terrain GLB exceeds uint32 length",
        ),
        e => failure(e),
    })
}
