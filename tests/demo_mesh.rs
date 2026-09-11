//! Opt-in spatial tiling of a user-supplied geographic GLB.
//! Set RUSTY_TILES_DEMO_GLB and run with --release -- --ignored.

use std::fs;
use std::io::Read;
use std::path::PathBuf;

use rusty_tiles::pack::list_zip_names;
use rusty_tiles::tile::{mesh_to_3tz, MeshTo3tzOptions};
use rusty_tiles::validate_3tz;
use serde_json::Value;

fn demo_glb() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("RUSTY_TILES_DEMO_GLB") {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    None
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
fn mesh_to_3tz_user_model() {
    let Some(glb) = demo_glb() else {
        eprintln!("skip: set RUSTY_TILES_DEMO_GLB to your own GLB");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let tz = tmp.path().join("model.3tz");
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
        !names
            .iter()
            .any(|n| n == glb.file_name().unwrap().to_str().unwrap()),
        "supply a model large enough to exercise spatial splitting: {names:?}"
    );
    let ts: Value = serde_json::from_slice(&zip_bytes(&tz, "tileset.json")).unwrap();
    assert_eq!(ts["root"]["refine"], "REPLACE");
    let xf = ts["root"]["transform"]
        .as_array()
        .expect("geog bake sets root.transform");
    let tx = xf[12].as_f64().unwrap();
    let ty = xf[13].as_f64().unwrap();
    let tz_ecef = xf[14].as_f64().unwrap();
    let r = (tx * tx + ty * ty + tz_ecef * tz_ecef).sqrt();
    assert!(
        (6.0e6..6.5e6).contains(&r),
        "root translation should be ECEF metres, got {r}"
    );
    let mut uris = Vec::new();
    collect_uris(&ts["root"], &mut uris);
    assert!(
        uris.len() > 1,
        "expected a split tree, got {} leaves",
        uris.len()
    );
}
