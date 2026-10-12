//! Uncompressed glTF 2.0 regular terrain meshes in one shared local frame.
use crate::{JobError, JobErrorKind};
use serde_json::json;
pub(super) struct Grid {
    pub positions: Vec<[f32; 3]>,
    pub columns: usize,
    pub rows: usize,
    pub bounds: [[f64; 3]; 2],
    pub max_position_error: f64,
    pub height_range: [f64; 2],
}
/// Vertical curtains below a tile's interior edges, as a separate primitive.
pub(super) struct Skirt {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u16>,
}
fn accessor_bounds(positions: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    for p in positions {
        for (axis, v) in p.iter().enumerate() {
            minimum[axis] = minimum[axis].min(*v);
            maximum[axis] = maximum[axis].max(*v);
        }
    }
    (minimum, maximum)
}
pub(super) fn encode(
    grid: &Grid,
    skirt: &Skirt,
    mut checkpoint: impl FnMut() -> Result<(), JobError>,
) -> Result<Vec<u8>, JobError> {
    let mut binary = Vec::with_capacity(
        grid.positions.len() * 12
            + (grid.columns - 1) * (grid.rows - 1) * 12
            + skirt.positions.len() * 12
            + skirt.indices.len() * 2,
    );
    for (i, p) in grid.positions.iter().enumerate() {
        if i % 4096 == 0 {
            checkpoint()?
        }
        for v in p {
            binary.extend(v.to_le_bytes())
        }
    }
    let (minimum, maximum) = accessor_bounds(&grid.positions);
    let position_bytes = binary.len();
    for y in 0..grid.rows - 1 {
        checkpoint()?;
        for x in 0..grid.columns - 1 {
            let a = y * grid.columns + x;
            for i in [
                a,
                a + 1,
                a + grid.columns,
                a + 1,
                a + grid.columns + 1,
                a + grid.columns,
            ] {
                binary.extend((i as u16).to_le_bytes())
            }
        }
    }
    let index_bytes = binary.len() - position_bytes;
    let count = grid.positions.len();
    let material = json!({"doubleSided":false,"alphaMode":"OPAQUE","pbrMetallicRoughness":{"baseColorFactor":[1.0,1.0,1.0,1.0],"metallicFactor":0.0,"roughnessFactor":1.0}});
    let mut primitives =
        vec![json!({"attributes":{"POSITION":0},"indices":1,"material":0,"mode":4})];
    let mut materials = vec![material.clone()];
    let mut views = vec![
        json!({"buffer":0,"byteOffset":0,"byteLength":position_bytes,"target":34962}),
        json!({"buffer":0,"byteOffset":position_bytes,"byteLength":index_bytes,"target":34963}),
    ];
    let mut accessors = vec![
        json!({"bufferView":0,"componentType":5126,"count":count,"type":"VEC3","min":minimum,"max":maximum}),
        json!({"bufferView":1,"componentType":5123,"count":index_bytes/2,"type":"SCALAR"}),
    ];
    // The surface primitive above is unchanged by skirts; they are appended after it.
    if !skirt.indices.is_empty() {
        checkpoint()?;
        let start = binary.len();
        for v in skirt.positions.iter().flatten() {
            binary.extend(v.to_le_bytes())
        }
        let middle = binary.len();
        for i in &skirt.indices {
            binary.extend(i.to_le_bytes())
        }
        let (minimum, maximum) = accessor_bounds(&skirt.positions);
        primitives.push(json!({"attributes":{"POSITION":2},"indices":3,"material":1,"mode":4}));
        materials.push(material);
        views.push(json!({"buffer":0,"byteOffset":start,"byteLength":middle-start,"target":34962}));
        views.push(
            json!({"buffer":0,"byteOffset":middle,"byteLength":binary.len()-middle,"target":34963}),
        );
        accessors.push(json!({"bufferView":2,"componentType":5126,"count":skirt.positions.len(),"type":"VEC3","min":minimum,"max":maximum}));
        accessors.push(json!({"bufferView":3,"componentType":5123,"count":skirt.indices.len(),"type":"SCALAR"}));
    }
    let mut document=serde_json::to_vec(&json!({
      "asset":{"version":"2.0","generator":"rusty-tiles T1 terrain mesh"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
      "meshes":[{"primitives":primitives}],
      "materials":materials,
      "buffers":[{"byteLength":binary.len()}],"bufferViews":views,
      "accessors":accessors
    })).map_err(|e|JobError::new(JobErrorKind::InvalidInput,format!("serialize terrain glTF: {e}")))?;
    while document.len() % 4 != 0 {
        document.push(b' ')
    }
    while binary.len() % 4 != 0 {
        binary.push(0)
    }
    let length = 12 + 8 + document.len() + 8 + binary.len();
    let length = u32::try_from(length).map_err(|_| {
        JobError::new(
            JobErrorKind::Unsupported,
            "terrain GLB exceeds uint32 length",
        )
    })?;
    let mut result = Vec::with_capacity(length as usize);
    result.extend(b"glTF");
    result.extend(2u32.to_le_bytes());
    result.extend(length.to_le_bytes());
    result.extend((document.len() as u32).to_le_bytes());
    result.extend(b"JSON");
    result.extend(document);
    result.extend((binary.len() as u32).to_le_bytes());
    result.extend(b"BIN\0");
    result.extend(binary);
    Ok(result)
}
