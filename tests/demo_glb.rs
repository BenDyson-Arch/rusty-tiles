//! `mgal_detail.glb` (~1.5 GiB, mostly textures) lives in `tinyowl-demodata`.
//! Tests skip if the file is missing. Override the path with `TINYOWL_DEMO_GLB`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tinyowl_tiles::tileset::{create_tileset_json, CreateTilesetOptions};
use tinyowl_tiles::{Cartographic, ORACLE_NPM};

fn demo_glb() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("TINYOWL_DEMO_GLB") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
        eprintln!("TINYOWL_DEMO_GLB is not a file: {}", p.display());
        return None;
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let p = manifest.join("../tinyowl-demodata/glb/mgal_detail.glb");
    p.is_file().then_some(p)
}

fn npx_ok(input: &Path, output: &Path, extra: &[&str]) -> bool {
    let mut args = vec![
        "--yes",
        ORACLE_NPM,
        "createTilesetJson",
        "-i",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
        "-f",
    ];
    args.extend(extra.iter().copied());
    match Command::new("npx").args(&args).status() {
        Ok(st) if st.success() => true,
        other => {
            eprintln!("skip demo golden: npx {ORACLE_NPM}: {other:?}");
            false
        }
    }
}

fn assert_semantic(oracle: &serde_json::Value, ours: &serde_json::Value) {
    assert_eq!(oracle["asset"]["version"], ours["asset"]["version"]);
    assert_eq!(
        oracle["geometricError"].as_f64(),
        ours["geometricError"].as_f64()
    );
    assert_eq!(oracle["root"]["refine"], ours["root"]["refine"]);
    assert_eq!(
        oracle["root"]["geometricError"].as_f64(),
        ours["root"]["geometricError"].as_f64()
    );
    assert_eq!(
        oracle["root"]["content"]["uri"],
        ours["root"]["content"]["uri"]
    );
    let ob = oracle["root"]["boundingVolume"]["box"]
        .as_array()
        .expect("oracle box");
    let ub = ours["root"]["boundingVolume"]["box"]
        .as_array()
        .expect("ours box");
    assert_eq!(ob.len(), 12);
    assert_eq!(ub.len(), 12);
    for (i, (a, b)) in ob.iter().zip(ub.iter()).enumerate() {
        let a = a.as_f64().unwrap();
        let b = b.as_f64().unwrap();
        assert!(a.is_finite() && b.is_finite(), "box[{i}] non-finite");
    }
    // Centers should agree within a fraction of the model extent. Oracle uses a
    // dito.ts OBB; we use a Z-up AABB — both enclose the same mesh.
    let oc = [
        ob[0].as_f64().unwrap(),
        ob[1].as_f64().unwrap(),
        ob[2].as_f64().unwrap(),
    ];
    let uc = [
        ub[0].as_f64().unwrap(),
        ub[1].as_f64().unwrap(),
        ub[2].as_f64().unwrap(),
    ];
    let extent = (0..3)
        .map(|i| {
            let hx = (ub[3 + i * 3].as_f64().unwrap().powi(2)
                + ub[4 + i * 3].as_f64().unwrap().powi(2)
                + ub[5 + i * 3].as_f64().unwrap().powi(2))
            .sqrt();
            hx
        })
        .fold(0.0_f64, f64::max)
        .max(1.0);
    for i in 0..3 {
        let d = (oc[i] - uc[i]).abs();
        assert!(
            d < extent * 0.25,
            "box center[{i}] oracle={} ours={} extent={extent}",
            oc[i],
            uc[i]
        );
    }
}

#[test]
fn mgal_detail_create_tileset_json_when_present() {
    let Some(glb) = demo_glb() else {
        eprintln!("skip: mgal_detail.glb not found (set TINYOWL_DEMO_GLB)");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let our_json = tmp.path().join("ours.json");
    create_tileset_json(&glb, &our_json, &CreateTilesetOptions::default()).unwrap();
    let ours: serde_json::Value = serde_json::from_slice(&fs::read(&our_json).unwrap()).unwrap();
    assert_eq!(ours["asset"]["version"], "1.1");
    assert_eq!(ours["root"]["content"]["uri"], "mgal_detail.glb");
    let b = ours["root"]["boundingVolume"]["box"].as_array().unwrap();
    // POSITION min/max in the GLB JSON, Y-up → Z-up: center x≈133.06, z≈127
    assert!((b[0].as_f64().unwrap() - 133.06).abs() < 0.1);
    assert!(b[2].as_f64().unwrap() > 100.0);
    assert!(b[2].as_f64().unwrap() < 150.0);
}

#[test]
fn mgal_detail_create_tileset_json_matches_3d_tiles_tools() {
    let Some(glb) = demo_glb() else {
        eprintln!("skip: mgal_detail.glb not found (set TINYOWL_DEMO_GLB)");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let our_json = tmp.path().join("ours.json");
    eprintln!("tinyowl-tiles createTilesetJson on {}", glb.display());
    create_tileset_json(&glb, &our_json, &CreateTilesetOptions::default()).unwrap();
    let ours: serde_json::Value = serde_json::from_slice(&fs::read(&our_json).unwrap()).unwrap();

    let oracle_json = tmp.path().join("oracle.json");
    eprintln!("npx {ORACLE_NPM} createTilesetJson (slow on 1.5GiB)");
    if !npx_ok(&glb, &oracle_json, &[]) {
        return;
    }
    let oracle: serde_json::Value =
        serde_json::from_slice(&fs::read(&oracle_json).unwrap()).unwrap();
    assert_eq!(oracle["root"]["content"]["uri"], "mgal_detail.glb");
    assert_semantic(&oracle, &ours);
}

#[test]
fn mgal_detail_cartographic_transform_matches_3d_tiles_tools() {
    let Some(glb) = demo_glb() else {
        eprintln!("skip: mgal_detail.glb not found");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let opts = CreateTilesetOptions {
        cartographic: Some(Cartographic::new(132.87, -12.27, 0.0)),
        rotation: None,
        force: true,
    };
    let our_json = tmp.path().join("ours.json");
    create_tileset_json(&glb, &our_json, &opts).unwrap();
    let ours: serde_json::Value = serde_json::from_slice(&fs::read(&our_json).unwrap()).unwrap();

    let oracle_json = tmp.path().join("oracle.json");
    if !npx_ok(
        &glb,
        &oracle_json,
        &["--cartographicPositionDegrees", "132.87", "-12.27", "0"],
    ) {
        return;
    }
    let oracle: serde_json::Value =
        serde_json::from_slice(&fs::read(&oracle_json).unwrap()).unwrap();
    let ot = oracle["root"]["transform"].as_array().unwrap();
    let ut = ours["root"]["transform"].as_array().unwrap();
    for i in 0..16 {
        let a = ot[i].as_f64().unwrap();
        let b = ut[i].as_f64().unwrap();
        let eps = if i >= 12 { 1e-3 } else { 1e-8 };
        assert!((a - b).abs() < eps, "transform[{i}] oracle={a} ours={b}");
    }
}
