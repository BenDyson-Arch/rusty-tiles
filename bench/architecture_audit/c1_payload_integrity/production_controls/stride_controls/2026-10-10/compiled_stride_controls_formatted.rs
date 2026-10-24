// Independently authored additive controls; no repository fixture or target encoder dependency.
// Literal primary: zeux/meshoptimizer@3d8e9b8a2a2b9a5becfc6fbc512307b04207ab5d,
// js/meshopt_decoder.test.js decodeVertexBuffer expected/encoded arrays (85 -> 48 bytes).
// Integrate as a cfg(test) crate module; generated file is self-contained for CI.
use crate::content_integrity::{
    payload::{self, PayloadInspection, PayloadKind},
    FormatError, JsonLimits, PayloadError, PayloadLimits,
};
fn frame(text: &str) -> Vec<u8> {
    let mut json = text.as_bytes().to_vec();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let mut bin = COMPRESSED.to_vec();
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let mut bytes = b"glTF".to_vec();
    bytes.extend_from_slice(&2u32.to_le_bytes());
    bytes.extend_from_slice(&(28u32 + json.len() as u32 + bin.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
    bytes.extend_from_slice(b"JSON");
    bytes.extend_from_slice(&json);
    bytes.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    bytes.extend_from_slice(b"BIN\0");
    bytes.extend_from_slice(&bin);
    bytes
}
fn inspect(
    text: &str,
    cap: u64,
    remaining: u64,
    decoded: usize,
) -> Result<PayloadInspection, PayloadError<&'static str>> {
    let mut calls = 0;
    let result = payload::inspect(
        PayloadKind::Glb,
        &frame(text),
        PayloadLimits {
            member_bytes: 4096,
            decoded_bytes: decoded,
            accessor_components: cap,
            json: JsonLimits {
                bytes: 4096,
                depth: 64,
                value_nodes: 256,
            },
        },
        remaining,
        |_, _| {
            calls += 1;
            Err("unexpected resolver")
        },
    );
    assert_eq!(
        calls, 0,
        "all backing is embedded or decoded placeholder; resolver must not run"
    );
    result
}
const ABSENT_PARENT_BASELINE: &str = r###"{"asset":{"version":"2.0"},"extensionsUsed":["EXT_meshopt_compression"],"extensionsRequired":["EXT_meshopt_compression"],"buffers":[{"byteLength":85},{"byteLength":48,"extensions":{"EXT_meshopt_compression":{"fallback":true}}}],"bufferViews":[{"buffer":1,"byteOffset":0,"byteLength":48,"extensions":{"EXT_meshopt_compression":{"buffer":0,"byteOffset":0,"byteLength":85,"byteStride":12,"count":4,"mode":"ATTRIBUTES","filter":"NONE"}}}],"accessors":[{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3","min":[0.0,0.0,0.0],"max":[3.159225145742757e-38,0.0,8.963424684712807e-38]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":0}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0}"###;
const UNUSED_PACKED_SCALAR: &str = r###"{"asset":{"version":"2.0"},"extensionsUsed":["EXT_meshopt_compression"],"extensionsRequired":["EXT_meshopt_compression"],"buffers":[{"byteLength":85},{"byteLength":48,"extensions":{"EXT_meshopt_compression":{"fallback":true}}}],"bufferViews":[{"buffer":1,"byteOffset":0,"byteLength":48,"extensions":{"EXT_meshopt_compression":{"buffer":0,"byteOffset":0,"byteLength":85,"byteStride":12,"count":4,"mode":"ATTRIBUTES","filter":"NONE"}}}],"accessors":[{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3","min":[0.0,0.0,0.0],"max":[3.159225145742757e-38,0.0,8.963424684712807e-38]},{"bufferView":0,"componentType":5126,"count":12,"type":"SCALAR","byteOffset":0}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":0}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0}"###;
const UNUSED_PACKED_SCALAR_OFFSET: &str = r###"{"asset":{"version":"2.0"},"extensionsUsed":["EXT_meshopt_compression"],"extensionsRequired":["EXT_meshopt_compression"],"buffers":[{"byteLength":85},{"byteLength":48,"extensions":{"EXT_meshopt_compression":{"fallback":true}}}],"bufferViews":[{"buffer":1,"byteOffset":0,"byteLength":48,"extensions":{"EXT_meshopt_compression":{"buffer":0,"byteOffset":0,"byteLength":85,"byteStride":12,"count":4,"mode":"ATTRIBUTES","filter":"NONE"}}}],"accessors":[{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3","min":[0.0,0.0,0.0],"max":[3.159225145742757e-38,0.0,8.963424684712807e-38]},{"bufferView":0,"componentType":5126,"count":11,"type":"SCALAR","byteOffset":4}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":0}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0}"###;
const EXPLICIT_PARENT_STRIDE: &str = r###"{"asset":{"version":"2.0"},"extensionsUsed":["EXT_meshopt_compression"],"extensionsRequired":["EXT_meshopt_compression"],"buffers":[{"byteLength":85},{"byteLength":48,"extensions":{"EXT_meshopt_compression":{"fallback":true}}}],"bufferViews":[{"buffer":1,"byteOffset":0,"byteLength":48,"extensions":{"EXT_meshopt_compression":{"buffer":0,"byteOffset":0,"byteLength":85,"byteStride":12,"count":4,"mode":"ATTRIBUTES","filter":"NONE"}},"byteStride":12}],"accessors":[{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3","min":[0.0,0.0,0.0],"max":[3.159225145742757e-38,0.0,8.963424684712807e-38]},{"bufferView":0,"componentType":5126,"count":4,"type":"SCALAR","byteOffset":0}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":0}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0}"###;
const SCALAR_REAL_RANGE: &str = r###"{"asset":{"version":"2.0"},"extensionsUsed":["EXT_meshopt_compression"],"extensionsRequired":["EXT_meshopt_compression"],"buffers":[{"byteLength":85},{"byteLength":48,"extensions":{"EXT_meshopt_compression":{"fallback":true}}}],"bufferViews":[{"buffer":1,"byteOffset":0,"byteLength":48,"extensions":{"EXT_meshopt_compression":{"buffer":0,"byteOffset":0,"byteLength":85,"byteStride":12,"count":4,"mode":"ATTRIBUTES","filter":"NONE"}}}],"accessors":[{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3","min":[0.0,0.0,0.0],"max":[3.159225145742757e-38,0.0,8.963424684712807e-38]},{"bufferView":0,"componentType":5126,"count":13,"type":"SCALAR","byteOffset":0}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":0}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0}"###;
const WRONG_PARENT_STRIDE: &str = r###"{"asset":{"version":"2.0"},"extensionsUsed":["EXT_meshopt_compression"],"extensionsRequired":["EXT_meshopt_compression"],"buffers":[{"byteLength":85},{"byteLength":48,"extensions":{"EXT_meshopt_compression":{"fallback":true}}}],"bufferViews":[{"buffer":1,"byteOffset":0,"byteLength":48,"extensions":{"EXT_meshopt_compression":{"buffer":0,"byteOffset":0,"byteLength":85,"byteStride":12,"count":4,"mode":"ATTRIBUTES","filter":"NONE"}},"byteStride":8}],"accessors":[{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3","min":[0.0,0.0,0.0],"max":[3.159225145742757e-38,0.0,8.963424684712807e-38]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":0}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0}"###;
const COMPRESSED: &[u8] = &[
    160, 1, 63, 0, 0, 0, 88, 87, 88, 1, 38, 0, 0, 0, 1, 12, 0, 0, 0, 88, 1, 8, 0, 0, 0, 0, 0, 0, 0,
    1, 63, 0, 0, 0, 23, 24, 23, 1, 38, 0, 0, 0, 1, 12, 0, 0, 0, 23, 1, 8, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

#[test]
fn independent_primary_stride_views_and_accessor_windows() {
    for (text, accessors, components) in [
        (ABSENT_PARENT_BASELINE, 1, 12),
        (UNUSED_PACKED_SCALAR, 2, 24),
        (UNUSED_PACKED_SCALAR_OFFSET, 2, 23),
        (EXPLICIT_PARENT_STRIDE, 2, 16),
    ] {
        let facts = inspect(text, 24, 24, 48)
            .expect("primary-consistent layout must admit with one48-byte physical decode");
        assert_eq!(facts.accessors_checked, accessors);
        assert_eq!(facts.primitives_checked, 1);
        assert_eq!(facts.vertices, 4);
        assert_eq!(facts.elements_checked, components);
    }
    for text in [SCALAR_REAL_RANGE, WRONG_PARENT_STRIDE] {
        assert!(matches!(
            inspect(text, 64, 64, 48),
            Err(PayloadError::Format(FormatError::InvalidInput(_)))
        ));
    }
}
#[test]
fn independent_unused_scalar_exact_component_and_physical_decode_caps() {
    let facts=inspect(UNUSED_PACKED_SCALAR,24,24,48).expect("12 POSITION plus12 unused SCALAR components fit24; shared physical view is charged once48 bytes");
    assert_eq!(facts.elements_checked, 24);
    for (cap, remaining, decoded) in [(23, 24, 48), (24, 23, 48), (24, 24, 47)] {
        assert!(matches!(
            inspect(UNUSED_PACKED_SCALAR, cap, remaining, decoded),
            Err(PayloadError::Format(FormatError::ResourceLimit(_)))
        ));
    }
}
