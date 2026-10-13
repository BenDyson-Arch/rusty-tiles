//! KHR_texture_basisu / KTX2 container controls. The independent stdlib oracle,
//! its 46 archive controls and the mutant records are in
//! bench/architecture_audit/c1_payload_integrity/production_controls/ktx2_basisu_amendment/2026-10-13.
//! These Rust controls cover hostile headers that must be refused before any
//! level is touched, and the texture/image rules at the payload owner.
use super::*;
use serde_json::{json, Value};

/// An unsupercompressed UASTC RGB container: header, level index, one basic
/// DFD block, no key/value data, levels smallest first, each 16-aligned.
fn uastc(width: u32, height: u32, levels: u32) -> Vec<u8> {
    let dfd_offset = 80 + 24 * levels;
    let mut dfd = vec![];
    for word in [44u32, 0, 2 | (40 << 16)] {
        dfd.extend(word.to_le_bytes());
    }
    dfd.extend([166, 1, 2, 0, 3, 3, 0, 0, 16, 0, 0, 0, 0, 0, 0, 0]);
    for word in [127 << 16, 0, 0, u32::MAX] {
        dfd.extend(u32::to_le_bytes(word));
    }
    let mut index = vec![(0u64, 0u64); levels as usize];
    let mut body = vec![];
    let mut cursor = u64::from(dfd_offset) + dfd.len() as u64;
    for p in (0..levels).rev() {
        let start = cursor.next_multiple_of(16);
        let blocks = (width >> p).max(1).div_ceil(4) * (height >> p).max(1).div_ceil(4);
        body.resize(body.len() + (start - cursor) as usize, 0);
        body.extend((0..16 * blocks).map(|i| i as u8));
        index[p as usize] = (start, u64::from(16 * blocks));
        cursor = start + u64::from(16 * blocks);
    }
    let mut out = vec![
        0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
    ];
    for word in [
        0, 1, width, height, 0, 0, 1, levels, 0, dfd_offset, 44, 0, 0,
    ] {
        out.extend(u32::to_le_bytes(word));
    }
    out.extend([0u8; 16]);
    for (offset, length) in index {
        for word in [offset, length, length] {
            out.extend(word.to_le_bytes());
        }
    }
    out.extend(dfd);
    out.extend(body);
    out
}
fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn put64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn invalid_input(result: FormatResult<()>) -> bool {
    matches!(result, Err(FormatError::InvalidInput(_)))
}

#[test]
fn packed_uastc_with_a_full_chain_is_admitted() {
    for (w, h, levels) in [(4, 4, 1), (12, 8, 4), (256, 1024, 11), (8, 4, 4)] {
        let bytes = uastc(w, h, levels);
        assert!(ktx2::inspect(&bytes, true).is_ok(), "{w}x{h}x{levels}");
    }
}

#[test]
fn hostile_headers_are_refused_before_the_level_index() {
    let valid = uastc(64, 64, 7);
    // A huge levelCount fails the dimension rule before any index entry is read,
    // even when the file is far too short to hold that index.
    let mut huge = valid[..80].to_vec();
    put32(&mut huge, 40, u32::MAX);
    assert!(invalid_input(ktx2::inspect(&huge, true)));
    // An index entry past EOF and an offset near u64::MAX do not wrap.
    for (field, value) in [(80, u64::MAX - 4), (88, u64::MAX), (80, 1 << 63)] {
        let mut bytes = valid.clone();
        put64(&mut bytes, field, value);
        assert!(
            invalid_input(ktx2::inspect(&bytes, true)),
            "{field} {value}"
        );
    }
    // Overlapping levels: level 1 placed at level 2's offset.
    let mut overlap = valid.clone();
    let level2 = u64::from_le_bytes(overlap[80 + 48..80 + 56].try_into().unwrap());
    put64(&mut overlap, 80 + 24, level2);
    assert!(invalid_input(ktx2::inspect(&overlap, true)));
    // DFD and key/value sections outside the file.
    for (field, value) in [(52, u32::MAX), (48, u32::MAX - 2), (60, 4), (56, 9)] {
        let mut bytes = valid.clone();
        put32(&mut bytes, field, value);
        assert!(invalid_input(ktx2::inspect(&bytes, true)), "{field}");
    }
    // Every truncation is a typed failure, never a panic.
    for n in 0..valid.len() {
        assert!(invalid_input(ktx2::inspect(&valid[..n], true)), "{n}");
    }
}

#[test]
fn texture_type_and_dimension_rules() {
    let valid = uastc(64, 64, 7);
    let field = |at: usize, value: u32| {
        let mut bytes = valid.clone();
        put32(&mut bytes, at, value);
        bytes
    };
    // Valid non-2D types: invalid for material maps, outside the profile otherwise.
    let array = field(32, 1);
    assert!(invalid_input(ktx2::inspect(&array, true)));
    assert!(matches!(
        ktx2::inspect(&array, false),
        Err(FormatError::Unsupported(_))
    ));
    // faceCount 2 is no texture type at all; height 0 is not block-compressed.
    assert!(invalid_input(ktx2::inspect(&field(36, 2), false)));
    assert!(invalid_input(ktx2::inspect(&field(24, 0), false)));
    assert!(invalid_input(ktx2::inspect(&uastc(6, 8, 1), true)));
    assert!(invalid_input(ktx2::inspect(&field(40, 0), true)));
    assert!(invalid_input(ktx2::inspect(&field(40, 8), true)));
    // Schemes outside ETC1S/BasisLZ and UASTC/none/Zstd.
    for scheme in [1, 3, 0x10000] {
        assert!(
            invalid_input(ktx2::inspect(&field(44, scheme), true)),
            "{scheme}"
        );
    }
}

fn limits() -> PayloadLimits {
    PayloadLimits {
        member_bytes: 1 << 20,
        decoded_bytes: 1 << 20,
        accessor_components: 1000,
        json: super::super::JsonLimits {
            bytes: 1 << 16,
            depth: 64,
            value_nodes: 4096,
        },
    }
}
/// A one-triangle GLB whose material maps texture 0; `ktx2` is image 0 in a
/// bufferView, `png` an optional external image 1.
fn textured(
    texture: Value,
    image: Value,
    used: &[&str],
    required: &[&str],
    ktx2: &[u8],
) -> Vec<u8> {
    let mut bin = [0f32, 0., 0., 1., 0., 0., 0., 1., 0.]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<u8>>();
    bin.extend(ktx2);
    let mut image = image;
    if image.get("uri").is_none() {
        image["bufferView"] = json!(1);
    }
    let mut doc = json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":bin.len()}],
        "bufferViews":[{"buffer":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":ktx2.len()}],
        "accessors":[{"bufferView":0,"componentType":5126,"type":"VEC3","count":3,"min":[0,0,0],"max":[1,1,0]}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0},"material":0}]}],
        "materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}}],
        "textures":[texture],"images":[image,{"uri":"fallback.png"}]});
    if !used.is_empty() {
        doc["extensionsUsed"] = json!(used);
    }
    if !required.is_empty() {
        doc["extensionsRequired"] = json!(required);
    }
    let mut json = serde_json::to_vec(&doc).unwrap();
    json.resize(json.len().next_multiple_of(4), b' ');
    bin.resize(bin.len().next_multiple_of(4), 0);
    let mut out = b"glTF".to_vec();
    for word in [2, 28 + json.len() + bin.len(), json.len()] {
        out.extend((word as u32).to_le_bytes());
    }
    out.extend(b"JSON");
    out.extend(json);
    out.extend((bin.len() as u32).to_le_bytes());
    out.extend(b"BIN\0");
    out.extend(bin);
    out
}
fn payload(glb: &[u8]) -> FormatResult<PayloadInspection> {
    payload_with(glb, b"")
}
/// External `fallback.png` resolves to PNG bytes; every other URI to `external`.
fn payload_with(glb: &[u8], external: &[u8]) -> FormatResult<PayloadInspection> {
    inspect(PayloadKind::Glb, glb, limits(), 1000, |uri, _| {
        Ok::<_, FormatError>(if uri == "fallback.png" {
            b"\x89PNG".to_vec()
        } else {
            external.to_vec()
        })
    })
    .map_err(|e| match e {
        PayloadError::Format(e) | PayloadError::Resolver(e) => e,
    })
}

#[test]
fn texture_and_image_rules_follow_the_extension() {
    const EXT: &str = "KHR_texture_basisu";
    let ktx2 = uastc(8, 8, 4);
    let basisu = json!({"extensions":{EXT:{"source":0}}});
    let mime = json!({"mimeType":"image/ktx2"});
    // Required, no fallback: admitted; the report names the uninspected level data
    // and no longer lists the extension as unknown.
    let report = payload(&textured(
        basisu.clone(),
        mime.clone(),
        &[EXT],
        &[EXT],
        &ktx2,
    ))
    .unwrap();
    assert!(report
        .not_inspected
        .contains("KTX2 level data and transcoding"));
    assert!(!report.not_inspected.contains(EXT));
    // A PNG fallback makes extensionsRequired optional; without one it is required.
    let fallback = json!({"source":1,"extensions":{EXT:{"source":0}}});
    assert!(payload(&textured(fallback, mime.clone(), &[EXT], &[], &ktx2)).is_ok());
    let missing = payload(&textured(basisu.clone(), mime.clone(), &[EXT], &[], &ktx2));
    assert!(matches!(missing, Err(FormatError::InvalidInput(_))));
    // The fallback may not be the KTX2 image itself.
    let itself = json!({"source":0,"extensions":{EXT:{"source":0}}});
    let itself = payload(&textured(itself, mime.clone(), &[EXT], &[], &ktx2));
    assert!(matches!(itself, Err(FormatError::InvalidInput(_))));
    // The source image must be declared KTX2; its bytes must be a valid container.
    for image in [json!({"mimeType":"image/png"}), json!({})] {
        let declared = payload(&textured(basisu.clone(), image, &[EXT], &[EXT], &ktx2));
        assert!(matches!(declared, Err(FormatError::InvalidInput(_))));
    }
    let broken = payload(&textured(
        basisu.clone(),
        mime,
        &[EXT],
        &[EXT],
        &ktx2[..ktx2.len() - 1],
    ));
    assert!(matches!(broken, Err(FormatError::InvalidInput(_))));
    // Undeclared extension object.
    let undeclared = payload(&textured(
        json!({"source":1,"extensions":{EXT:{"source":0}}}),
        json!({"mimeType":"image/ktx2"}),
        &[],
        &[],
        &ktx2,
    ));
    assert!(matches!(undeclared, Err(FormatError::InvalidInput(_))));
}

#[test]
fn external_and_data_uri_sources_follow_gltf_not_file_names() {
    const EXT: &str = "KHR_texture_basisu";
    let ktx2 = uastc(8, 8, 4);
    let basisu = json!({"extensions":{EXT:{"source":0}}});
    // No mimeType and no `.ktx2` suffix: KTX2 by its identifier, so admitted.
    let glb = textured(
        basisu.clone(),
        json!({"uri":"tex_0"}),
        &[EXT],
        &[EXT],
        &ktx2,
    );
    assert!(payload_with(&glb, &ktx2).is_ok());
    // The same URI holding PNG bytes is not a KTX2 container.
    let png = payload_with(&glb, b"\x89PNG\r\n\x1a\n0000");
    assert!(matches!(png, Err(FormatError::InvalidInput(_))));
    // A data URI is Unsupported whether or not it declares a mimeType.
    for image in [
        json!({"uri":"data:image/ktx2;base64,AAAA"}),
        json!({"uri":"data:image/ktx2;base64,AAAA","mimeType":"image/ktx2"}),
    ] {
        let glb = textured(basisu.clone(), image, &[EXT], &[EXT], &ktx2);
        assert!(matches!(payload(&glb), Err(FormatError::Unsupported(_))));
    }
    // A fallback without mimeType is KTX2 by its identifier too.
    let fallback = json!({"source":1,"extensions":{EXT:{"source":0}}});
    let mut glb = textured(
        fallback,
        json!({"mimeType":"image/ktx2"}),
        &[EXT],
        &[],
        &ktx2,
    );
    let at = glb.windows(12).position(|w| w == b"fallback.png").unwrap();
    glb[at..at + 12].copy_from_slice(b"fallback.bin");
    let itself = payload_with(&glb, &ktx2);
    assert!(matches!(itself, Err(FormatError::InvalidInput(_))));
}

#[test]
fn images_sharing_one_view_inspect_its_container_once() {
    const EXT: &str = "KHR_texture_basisu";
    let ktx2 = uastc(8, 8, 4);
    let mut glb = textured(
        json!({"extensions":{EXT:{"source":0}}}),
        json!({"mimeType":"image/ktx2"}),
        &[EXT],
        &[EXT],
        &ktx2,
    );
    // Rebuild the document with 300 images on view 1, each its own texture.
    let json_length = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let mut doc: Value = serde_json::from_slice(&glb[20..20 + json_length]).unwrap();
    doc["images"] = json!(vec![json!({"bufferView":1,"mimeType":"image/ktx2"}); 300]);
    doc["textures"] = json!((0..300)
        .map(|i| json!({"extensions":{EXT:{"source":i}}}))
        .collect::<Vec<_>>());
    let bin = glb[20 + json_length + 8..].to_vec();
    let mut json = serde_json::to_vec(&doc).unwrap();
    json.resize(json.len().next_multiple_of(4), b' ');
    glb = b"glTF".to_vec();
    for word in [2, 28 + json.len() + bin.len(), json.len()] {
        glb.extend((word as u32).to_le_bytes());
    }
    glb.extend(b"JSON");
    glb.extend(json);
    glb.extend((bin.len() as u32).to_le_bytes());
    glb.extend(b"BIN\0");
    glb.extend(bin);
    ktx2::INSPECTIONS.with(|n| n.set(0));
    let report = payload(&glb);
    assert!(report.is_ok(), "{:?}", report.err());
    assert_eq!(ktx2::INSPECTIONS.with(|n| n.get()), 1);
}

#[test]
fn red_srgb_and_premultiplied_material_textures_are_refused() {
    let valid = uastc(8, 8, 1);
    let dfd = 80 + 24;
    let patched = |at: usize, value: u8| {
        let mut bytes = valid.clone();
        bytes[at] = value;
        bytes
    };
    // UASTC_RRR (4) and UASTC_RG (6) with sRGB are refused; linear is admitted.
    for channel in [4, 6] {
        let mut red = patched(dfd + 31, channel);
        assert!(invalid_input(ktx2::inspect(&red, true)), "{channel}");
        red[dfd + 14] = 1;
        assert!(ktx2::inspect(&red, true).is_ok(), "{channel}");
    }
    // Premultiplied alpha: refused for a material texture only.
    let premultiplied = patched(dfd + 15, 1);
    assert!(invalid_input(ktx2::inspect(&premultiplied, true)));
    assert!(ktx2::inspect(&premultiplied, false).is_ok());
}
