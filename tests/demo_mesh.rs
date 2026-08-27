//! Optional `mesh-to-3tz` on `mgal_detail.glb` (~1.5 GiB, 10M tris, 89×8K JPEGs).
//! Skip if the file is missing. Override with `TINYOWL_DEMO_GLB`.
//! Run with `--release -- --ignored` — debug will thrash.

use std::fs;
use std::io::Read;
use std::path::PathBuf;

use serde_json::Value;
use tinyowl_tiles::pack::list_zip_names;
use tinyowl_tiles::tile::{mesh_to_3tz, MeshTo3tzOptions};
use tinyowl_tiles::validate_3tz;

fn demo_glb() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("TINYOWL_DEMO_GLB") {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let p =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tinyowl-demodata/glb/mgal_detail.glb");
    p.is_file().then_some(p)
}

fn zip_bytes(tz: &std::path::Path, name: &str) -> Vec<u8> {
    let mut z = zip::ZipArchive::new(fs::File::open(tz).unwrap()).unwrap();
    let mut f = z.by_name(name).unwrap();
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).unwrap();
    buf
}

fn collect_uris(tile: &Value, out: &mut Vec<String>) {
    if let Some(uri) = tile.pointer("/content/uri").and_then(|v| v.as_str()) {
        out.push(uri.to_string());
    }
    if let Some(kids) = tile["children"].as_array() {
        for k in kids {
            collect_uris(k, out);
        }
    }
}

#[test]
#[ignore]
fn mesh_to_3tz_mgal_detail() {
    let Some(glb) = demo_glb() else {
        eprintln!("skip: mgal_detail.glb not found");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let tz = tmp.path().join("mgal_detail.3tz");
    mesh_to_3tz(
        &glb,
        &tz,
        &MeshTo3tzOptions {
            force: true,
            ..Default::default()
        },
    )
    .unwrap();
    validate_3tz(&tz).unwrap();

    let names = list_zip_names(&tz).unwrap();
    assert!(
        !names.iter().any(|n| n.ends_with("mgal_detail.glb")),
        "must not wrap the 1.5 GiB source: {names:?}"
    );
    let ts: Value = serde_json::from_slice(&zip_bytes(&tz, "tileset.json")).unwrap();
    assert_eq!(ts["root"]["refine"], "REPLACE");
    let mut uris = Vec::new();
    collect_uris(&ts["root"], &mut uris);
    assert!(
        uris.len() > 1,
        "expected a split tree, got {} leaves",
        uris.len()
    );
    let src_len = fs::metadata(&glb).unwrap().len();
    let out_len = fs::metadata(&tz).unwrap().len();
    assert!(
        out_len < src_len,
        "tiled 3tz {out_len} should be smaller than source {src_len}"
    );
}
