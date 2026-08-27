//! Tiny GLB used in unit/golden tests (one triangle in XY).

/// glTF 2.0 GLB: triangle (0,0,0)–(1,0,0)–(0,1,0).
pub fn triangle_glb() -> Vec<u8> {
    let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let mut bin = Vec::with_capacity(36);
    for f in positions {
        bin.extend_from_slice(&f.to_le_bytes());
    }

    let json = serde_json::json!({
        "asset": { "version": "2.0" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "mesh": 0 }],
        "meshes": [{
            "primitives": [{ "attributes": { "POSITION": 0 } }]
        }],
        "accessors": [{
            "bufferView": 0,
            "componentType": 5126,
            "count": 3,
            "type": "VEC3",
            "max": [1.0, 1.0, 0.0],
            "min": [0.0, 0.0, 0.0]
        }],
        "bufferViews": [{
            "buffer": 0,
            "byteOffset": 0,
            "byteLength": 36,
            "target": 34962
        }],
        "buffers": [{ "byteLength": 36 }]
    });
    let mut json_bytes = serde_json::to_vec(&json).expect("json");
    while json_bytes.len() % 4 != 0 {
        json_bytes.push(b' ');
    }
    while bin.len() % 4 != 0 {
        bin.push(0);
    }

    let json_len = json_bytes.len() as u32;
    let bin_len = bin.len() as u32;
    let total = 12 + 8 + json_len + 8 + bin_len;

    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&json_len.to_le_bytes());
    out.extend_from_slice(&0x4E4F534Au32.to_le_bytes()); // JSON
    out.extend_from_slice(&json_bytes);
    out.extend_from_slice(&bin_len.to_le_bytes());
    out.extend_from_slice(&0x004E4942u32.to_le_bytes()); // BIN
    out.extend_from_slice(&bin);
    out
}
