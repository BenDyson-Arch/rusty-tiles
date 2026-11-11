//! Dev-only actual artifact decoder. No production mip/reference helper is used.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::PathBuf};

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn word(bytes: &[u8], offset: usize) -> usize {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize
}

#[test]
#[ignore = "external published GLB artifact decoder; H8_OUTPUT/H8_DECODED required"]
fn external_driver() {
    let input = PathBuf::from(std::env::var_os("H8_OUTPUT").expect("H8_OUTPUT"));
    let output = PathBuf::from(std::env::var_os("H8_DECODED").expect("H8_DECODED"));
    fs::create_dir_all(&output).unwrap();
    let mut pages = Vec::new();
    let mut unique_levels = BTreeMap::<String, Value>::new();
    for line in fs::read_to_string(input.join("leaves.jsonl"))
        .unwrap()
        .lines()
    {
        let receipt: Value = serde_json::from_str(line).unwrap();
        let uri = receipt["uri"].as_str().unwrap();
        assert!(!uri.starts_with('/') && !uri.split('/').any(|p| p == ".."));
        let glb = fs::read(input.join(uri)).unwrap();
        assert!(glb.len() <= 96 * 1024 * 1024, "physical leaf cap");
        assert_eq!(&glb[..4], b"glTF");
        assert_eq!(word(&glb, 4), 2);
        assert_eq!(word(&glb, 8), glb.len());
        assert_eq!(word(&glb, 16), 0x4e4f534a);
        let json_end = 20 + word(&glb, 12);
        assert_eq!(word(&glb, json_end + 4), 0x004e4942);
        let bin = &glb[json_end + 8..];
        assert_eq!(word(&glb, json_end), bin.len());
        let doc: Value = serde_json::from_slice(&glb[20..json_end]).unwrap();
        let glb_hash = digest(&glb);
        assert_eq!(receipt["sha256"].as_str().unwrap(), glb_hash);
        for (page, image) in doc["images"].as_array().unwrap().iter().enumerate() {
            if image["mimeType"] != "image/ktx2" {
                continue;
            }
            let view = &doc["bufferViews"][image["bufferView"].as_u64().unwrap() as usize];
            assert!(
                view.get("extensions").is_none(),
                "images are physical uncompressed views"
            );
            assert_eq!(view["buffer"], 0);
            let start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
            let len = view["byteLength"].as_u64().unwrap() as usize;
            let encoded = &bin[start..start.checked_add(len).unwrap()];
            assert_eq!(&encoded[..12], b"\xabKTX 20\xbb\r\n\x1a\n");
            let (mut w, mut h) = (word(encoded, 20), word(encoded, 24));
            assert!(w <= 4096 && h <= 4096 && w.is_power_of_two() && h.is_power_of_two());
            let page_hash = digest(encoded);
            if let Some(levels) = unique_levels.get(&page_hash) {
                pages.push(json!({"uri":uri,"page":page,"glbSha256":glb_hash,"pageSha256":page_hash,"levels":levels}));
                continue;
            }
            let transcoder = basisu::Transcoder::new(encoded).unwrap();
            assert_eq!(transcoder.level_count() as usize, word(encoded, 40));
            let mut levels = Vec::new();
            for mip in 0..transcoder.level_count() {
                let bytes = transcoder
                    .transcode(mip, basisu::TargetFormat::Rgba32, basisu::DecodeFlags::NONE)
                    .unwrap();
                assert_eq!(bytes.len(), w * h * 4);
                let name = format!("{page_hash}-{mip}.rgba");
                fs::write(output.join(&name), &bytes).unwrap();
                levels.push(
                    json!({"mip":mip,"width":w,"height":h,"file":name,"sha256":digest(&bytes)}),
                );
                w = (w / 2).max(1);
                h = (h / 2).max(1);
            }
            assert_eq!((w, h), (1, 1));
            unique_levels.insert(page_hash.clone(), json!(levels));
            pages.push(json!({"uri":uri,"page":page,"glbSha256":glb_hash,"pageSha256":page_hash,"levels":levels}));
        }
    }
    assert!(!pages.is_empty(), "no actual KTX pages decoded");
    let driver = fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/bench/architecture_audit/mesh_large_h8/decode_ktx.rs"
    ))
    .unwrap();
    let index = json!({"schemaVersion":1,"driverSha256":digest(&driver),"decoder":"dev basisu::Transcoder RGBA32","pages":pages});
    fs::write(
        output.join("decoded.json"),
        serde_json::to_vec_pretty(&index).unwrap(),
    )
    .unwrap();
}
