use std::fs;
use std::process::Command;

use rusty_tiles::bbox::bounding_box_from_gltf_path;
use rusty_tiles::fixtures::triangle_glb;
use rusty_tiles::georef::Cartographic;
use rusty_tiles::pack::{convert_to_3tz, list_zip_names, PackOptions};
use rusty_tiles::tileset::{
    create_tileset_json, glb_to_3tz, CreateTilesetOptions, LEAF_GEOMETRIC_ERROR,
    TILESET_GEOMETRIC_ERROR,
};
use rusty_tiles::vector;
use rusty_tiles::TZ_INDEX_NAME;
use rusty_tiles::{terrain, ORACLE_NPM};

fn write_triangle(dir: &std::path::Path) -> std::path::PathBuf {
    let p = dir.join("triangle.glb");
    fs::write(&p, triangle_glb()).unwrap();
    p
}

#[test]
fn bbox_from_triangle_glb() {
    let tmp = tempfile::tempdir().unwrap();
    let glb = write_triangle(tmp.path());
    let b = bounding_box_from_gltf_path(&glb).unwrap();
    // Y-up triangle (0,0,0)-(1,0,0)-(0,1,0) → Z-up (0,0,0)-(1,0,0)-(0,0,1)
    assert!((b[0] - 0.5).abs() < 1e-5);
    assert!(b[1].abs() < 1e-5);
    assert!((b[2] - 0.5).abs() < 1e-5);
    assert!((b[3] - 0.5).abs() < 1e-5);
    assert!(b[7].abs() < 1e-5);
    assert!((b[11] - 0.5).abs() < 1e-5);
}

#[test]
fn create_tileset_json_single_glb() {
    let tmp = tempfile::tempdir().unwrap();
    let glb = write_triangle(tmp.path());
    let out = tmp.path().join("tileset.json");
    let ts = create_tileset_json(&glb, &out, &CreateTilesetOptions::default()).unwrap();
    assert_eq!(ts["asset"]["version"], "1.1");
    assert_eq!(ts["geometricError"].as_f64(), Some(TILESET_GEOMETRIC_ERROR));
    assert_eq!(ts["root"]["refine"], "ADD");
    assert_eq!(
        ts["root"]["geometricError"].as_f64(),
        Some(LEAF_GEOMETRIC_ERROR)
    );
    assert_eq!(ts["root"]["content"]["uri"], "triangle.glb");
    assert!(ts["root"]["boundingVolume"]["box"].is_array());
    assert!(ts["root"].get("transform").is_none());
}

#[test]
fn create_tileset_json_with_cartographic() {
    let tmp = tempfile::tempdir().unwrap();
    let glb = write_triangle(tmp.path());
    let out = tmp.path().join("tileset.json");
    let opts = CreateTilesetOptions {
        cartographic: Some(Cartographic::new(151.2, -33.9, 10.0)),
        rotation: None,
        force: false,
    };
    let ts = create_tileset_json(&glb, &out, &opts).unwrap();
    let xf = ts["root"]["transform"].as_array().expect("transform");
    assert_eq!(xf.len(), 16);
    let x = xf[12].as_f64().unwrap();
    let y = xf[13].as_f64().unwrap();
    let z = xf[14].as_f64().unwrap();
    let r = (x * x + y * y + z * z).sqrt();
    assert!((r - 6.37e6).abs() < 5e4);
}

#[test]
fn convert_and_glb_to_3tz_zip_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let glb = write_triangle(tmp.path());
    let tz = tmp.path().join("out.3tz");
    glb_to_3tz(
        &glb,
        &tz,
        &CreateTilesetOptions {
            force: true,
            ..Default::default()
        },
    )
    .unwrap();
    let names = list_zip_names(&tz).unwrap();
    assert!(names
        .iter()
        .any(|n| n == "tileset.json" || n.ends_with("/tileset.json")));
    assert!(names.iter().any(|n| n.contains("triangle.glb")));
    assert!(
        names.iter().any(|n| n == TZ_INDEX_NAME),
        "3TZ index missing: {names:?}"
    );

    let raw = fs::read(&tz).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(raw)).unwrap();
    let mut json_file = zip.by_name("tileset.json").unwrap();
    let mut s = String::new();
    std::io::Read::read_to_string(&mut json_file, &mut s).unwrap();
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v["asset"]["version"], "1.1");
    assert_eq!(v["root"]["content"]["uri"], "triangle.glb");
    rusty_tiles::validate_3tz(&tz).unwrap();
}

#[test]
fn convert_refuses_without_tileset_json() {
    let tmp = tempfile::tempdir().unwrap();
    let err = convert_to_3tz(
        tmp.path(),
        &tmp.path().join("x.3tz"),
        &PackOptions { force: true },
    )
    .unwrap_err();
    assert!(matches!(err, rusty_tiles::Error::MissingTilesetJson));
}

#[test]
fn derivative_missing_inputs_do_not_publish_output() {
    let tmp = tempfile::tempdir().unwrap();
    let output = tmp.path().join("out");
    let input = tmp.path().join("missing");
    let err = vector::vector_to_3tz(&input, &output, 64, false, false).unwrap_err();
    assert!(matches!(err, rusty_tiles::Error::InputNotFound(_)));
    let err = terrain::dem_to_terrain(
        &input,
        &output,
        &terrain::TerrainOptions {
            max_zoom: 1,
            grid: 17,
            height_offset: 0.0,
            fill_height: 0.0,
        },
    )
    .unwrap_err();
    assert!(matches!(err, rusty_tiles::Error::InputNotFound(_)));
    assert!(!output.exists());
}

#[test]
#[ignore = "requires external pinned 3d-tiles-tools oracle via npx"]
fn golden_vs_3d_tiles_tools_when_npx_present() {
    let tmp = tempfile::tempdir().unwrap();
    let glb = write_triangle(tmp.path());

    let our_json = tmp.path().join("ours.json");
    create_tileset_json(&glb, &our_json, &CreateTilesetOptions::default()).unwrap();
    let ours: serde_json::Value = serde_json::from_slice(&fs::read(&our_json).unwrap()).unwrap();

    let oracle_dir = tmp.path().join("oracle-in");
    fs::create_dir(&oracle_dir).unwrap();
    fs::copy(&glb, oracle_dir.join("triangle.glb")).unwrap();
    let oracle_json = tmp.path().join("oracle.json");

    let status = Command::new("npx")
        .args([
            "--yes",
            ORACLE_NPM,
            "createTilesetJson",
            "-i",
            oracle_dir.to_str().unwrap(),
            "-o",
            oracle_json.to_str().unwrap(),
            "-f",
        ])
        .status();

    let Ok(st) = status else {
        eprintln!("skip golden: npx not available");
        return;
    };
    if !st.success() {
        eprintln!("skip golden: {ORACLE_NPM} failed ({st})");
        return;
    }

    let oracle: serde_json::Value =
        serde_json::from_slice(&fs::read(&oracle_json).unwrap()).unwrap();
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
    let ub = ours["root"]["boundingVolume"]["box"].as_array().unwrap();
    assert_eq!(ob.len(), 12);
    assert_eq!(ub.len(), 12);
    // Oracle uses a tight OBB; we use a Z-up AABB. Both must be finite and
    // rather than a fixed placeholder box.
    for (i, (a, b)) in ob.iter().zip(ub.iter()).enumerate() {
        let a = a.as_f64().unwrap();
        let b = b.as_f64().unwrap();
        assert!(a.is_finite() && b.is_finite(), "box[{i}] non-finite");
        assert!(b.abs() < 50.0, "ours box[{i}]={b} looks like the 50m dummy");
    }
    // Z-up AABB center for this triangle is (0.5, 0, 0.5)
    assert!((ub[0].as_f64().unwrap() - 0.5).abs() < 1e-4);
    assert!(ub[1].as_f64().unwrap().abs() < 1e-4);
    assert!((ub[2].as_f64().unwrap() - 0.5).abs() < 1e-4);
}

fn npx_create_tileset_json(
    input: &std::path::Path,
    output: &std::path::Path,
    extra: &[&str],
) -> bool {
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
        Ok(st) => {
            eprintln!("skip golden: {ORACLE_NPM} failed ({st})");
            false
        }
        Err(_) => {
            eprintln!("skip golden: npx not available");
            false
        }
    }
}

fn f64_arr(v: &serde_json::Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

#[test]
#[ignore = "requires external pinned 3d-tiles-tools oracle via npx"]
fn golden_cartographic_and_rotation_match_3d_tiles_tools() {
    let tmp = tempfile::tempdir().unwrap();
    let glb = write_triangle(tmp.path());
    let opts = CreateTilesetOptions {
        cartographic: Some(Cartographic::new(151.2, -33.9, 10.0)),
        rotation: Some(rusty_tiles::RotationDegrees {
            heading: 90.0,
            pitch: 0.0,
            roll: 0.0,
        }),
        force: true,
    };
    let our_json = tmp.path().join("ours.json");
    create_tileset_json(&glb, &our_json, &opts).unwrap();
    let ours: serde_json::Value = serde_json::from_slice(&fs::read(&our_json).unwrap()).unwrap();

    let oracle_dir = tmp.path().join("oracle-in");
    fs::create_dir(&oracle_dir).unwrap();
    fs::copy(&glb, oracle_dir.join("triangle.glb")).unwrap();
    let oracle_json = tmp.path().join("oracle.json");
    if !npx_create_tileset_json(
        &oracle_dir,
        &oracle_json,
        &[
            "--cartographicPositionDegrees",
            "151.2",
            "-33.9",
            "10",
            "--rotationDegrees",
            "90",
            "0",
            "0",
        ],
    ) {
        return;
    }
    let oracle: serde_json::Value =
        serde_json::from_slice(&fs::read(&oracle_json).unwrap()).unwrap();
    let ot = f64_arr(&oracle["root"]["transform"]);
    let ut = f64_arr(&ours["root"]["transform"]);
    assert_eq!(ot.len(), 16);
    assert_eq!(ut.len(), 16);
    for i in 0..16 {
        let eps = if i >= 12 { 1e-3 } else { 1e-8 };
        assert!(
            (ot[i] - ut[i]).abs() < eps,
            "transform[{i}] oracle={} ours={}",
            ot[i],
            ut[i]
        );
    }
}

#[test]
fn cli_create_tileset_json_camel_case_matches_tools_argv() {
    let tmp = tempfile::tempdir().unwrap();
    let glb = write_triangle(tmp.path());
    let out = tmp.path().join("tileset.json");
    let bin = env!("CARGO_BIN_EXE_rusty-tiles");
    let st = Command::new(bin)
        .args([
            "createTilesetJson",
            "-i",
            glb.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "-f",
            "--cartographicPositionDegrees",
            "151.2",
            "-33.9",
            "10",
        ])
        .status()
        .unwrap();
    assert!(st.success(), "{st}");
    let ts: serde_json::Value = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
    assert!(ts["root"]["transform"].is_array());
    assert_eq!(ts["root"]["content"]["uri"], "triangle.glb");
}

#[test]
fn convert_via_cli_accepts_tileset_json_path() {
    let tmp = tempfile::tempdir().unwrap();
    let glb = write_triangle(tmp.path());
    let json = tmp.path().join("tileset.json");
    create_tileset_json(&glb, &json, &CreateTilesetOptions::default()).unwrap();
    let tz = tmp.path().join("out.3tz");
    let bin = env!("CARGO_BIN_EXE_rusty-tiles");
    let st = Command::new(bin)
        .args([
            "convert",
            "-i",
            json.to_str().unwrap(),
            "-o",
            tz.to_str().unwrap(),
            "-f",
        ])
        .status()
        .unwrap();
    assert!(st.success(), "{st}");
    let names = list_zip_names(&tz).unwrap();
    assert!(names.iter().any(|n| n == TZ_INDEX_NAME));
}
