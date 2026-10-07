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
    crate::glb::encode_glb(&json, &bin).expect("fixture GLB")
}

/// Metashape geographic Y-up: X=lon°, Y=height m, Z=−lat°.
pub fn geographic_glb(positions: &[[f32; 3]]) -> Vec<u8> {
    let n = positions.len();
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    let mut bin = Vec::with_capacity(n * 12);
    for p in positions {
        for i in 0..3 {
            bin.extend_from_slice(&p[i].to_le_bytes());
            min[i] = min[i].min(p[i]);
            max[i] = max[i].max(p[i]);
        }
    }
    let json = serde_json::json!({
        "asset": { "version": "2.0", "generator": "Agisoft Metashape" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "mesh": 0 }],
        "meshes": [{
            "primitives": [{ "attributes": { "POSITION": 0 } }]
        }],
        "accessors": [{
            "bufferView": 0,
            "componentType": 5126,
            "count": n,
            "type": "VEC3",
            "max": [max[0], max[1], max[2]],
            "min": [min[0], min[1], min[2]]
        }],
        "bufferViews": [{
            "buffer": 0,
            "byteOffset": 0,
            "byteLength": n * 12
        }],
        "buffers": [{ "byteLength": n * 12 }]
    });
    crate::glb::encode_glb(&json, &bin).expect("fixture GLB")
}
