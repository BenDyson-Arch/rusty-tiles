use std::fs;
use std::process::Command;

use rusty_tiles::bbox::bounding_box_from_gltf_path;
use rusty_tiles::fixtures::triangle_glb;
use rusty_tiles::pack::list_zip_names;
use rusty_tiles::package::{package, PackageRequest};
use rusty_tiles::vector;
use rusty_tiles::TZ_INDEX_NAME;
use rusty_tiles::{
    model_to_archive, model_to_manifest, terrain, MeshPlacement, ModelManifestRequest,
    ModelWrapRequest, OutputPolicy, RunControl,
};

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
fn single_model_manifest_is_full_detail_and_uses_its_source_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let glb = write_triangle(tmp.path());
    let result = model_to_manifest(
        ModelManifestRequest::local_gltf(&glb),
        &RunControl::default(),
    )
    .unwrap();
    assert_eq!(
        result.output,
        tmp.path().join("tileset.json").canonicalize().unwrap()
    );
    let ts: serde_json::Value = serde_json::from_slice(&fs::read(&result.output).unwrap()).unwrap();
    assert_eq!(ts["asset"]["version"], "1.1");
    assert!(ts["geometricError"].as_f64().unwrap() >= 1.0);
    assert_eq!(ts["root"]["refine"], "REPLACE");
    assert_eq!(ts["root"]["geometricError"], 0.0);
    assert_eq!(ts["root"]["content"]["uri"], "triangle.glb");
    assert!(ts["root"]["boundingVolume"]["box"].is_array());
    assert_eq!(
        ts["root"]["transform"],
        serde_json::json!([1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.])
    );
    assert_eq!(fs::read(glb).unwrap(), triangle_glb());
}

#[test]
fn manifest_report_records_actual_explicit_placement() {
    let tmp = tempfile::tempdir().unwrap();
    let glb = write_triangle(tmp.path());
    let placement = MeshPlacement::from_parameters(Some([0., 0., 10.]), None, None).unwrap();
    let result = model_to_manifest(
        ModelManifestRequest::local_gltf(&glb).with_placement(placement),
        &RunControl::default(),
    )
    .unwrap();
    let ts: serde_json::Value = serde_json::from_slice(&fs::read(&result.output).unwrap()).unwrap();
    assert_eq!(
        ts["root"]["transform"],
        serde_json::to_value(result.report.root_transform).unwrap()
    );
    assert_eq!(result.report.root_transform[12], 6_378_147.);
    assert_eq!(result.report.root_transform[13], 0.);
    assert_eq!(result.report.root_transform[14], 0.);
}

#[test]
fn model_archive_has_exact_source_and_required_report() {
    let tmp = tempfile::tempdir().unwrap();
    let glb = write_triangle(tmp.path());
    let tz = tmp.path().join("out.3tz");
    let result = model_to_archive(
        ModelWrapRequest::local_gltf(&glb, &tz),
        &RunControl::default(),
    )
    .unwrap();
    let names = list_zip_names(&tz).unwrap();
    assert_eq!(
        names,
        [
            TZ_INDEX_NAME,
            "conversion.json",
            "model/source.glb",
            "tileset.json"
        ]
    );
    let mut zip = zip::ZipArchive::new(fs::File::open(&tz).unwrap()).unwrap();
    let mut source = Vec::new();
    std::io::Read::read_to_end(&mut zip.by_name("model/source.glb").unwrap(), &mut source).unwrap();
    assert_eq!(source, triangle_glb());
    let ts: serde_json::Value =
        serde_json::from_reader(zip.by_name("tileset.json").unwrap()).unwrap();
    assert_eq!(ts["root"]["content"]["uri"], "model/source.glb");
    let report: serde_json::Value =
        serde_json::from_reader(zip.by_name("conversion.json").unwrap()).unwrap();
    assert_eq!(report, serde_json::to_value(result.report).unwrap());
    rusty_tiles::validate::inspect(rusty_tiles::validate::ValidationRequest::new(&tz)).unwrap();
}

#[test]
fn package_refuses_without_tileset_json() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("source");
    fs::create_dir(&source).unwrap();
    let err = package(
        PackageRequest::directory(&source, tmp.path().join("x.3tz"))
            .with_policy(OutputPolicy::Replace),
        &RunControl::default(),
    )
    .unwrap_err();
    assert_eq!(err.error.kind(), rusty_tiles::JobErrorKind::InvalidInput);
}

#[test]
fn derivative_missing_inputs_do_not_publish_output() {
    let tmp = tempfile::tempdir().unwrap();
    let output = tmp.path().join("out");
    let input = tmp.path().join("missing");
    let err = vector::vector_to_archive(
        vector::VectorRequest::new(
            &input,
            output.with_extension("3tz"),
            vector::VectorOptions::default(),
        ),
        &rusty_tiles::RunControl::default(),
    )
    .unwrap_err();
    assert_eq!(err.error.kind(), rusty_tiles::JobErrorKind::Io);
    let err = terrain::terrain_to_directory(
        terrain::TerrainRequest::new(
            &input,
            &output,
            terrain::TerrainHeights::RawMetres {
                height_offset_metres: 0.,
                fill_height_metres: 0.,
            },
            terrain::TerrainOptions::new(16),
        ),
        &rusty_tiles::RunControl::default(),
    )
    .unwrap_err();
    assert_eq!(
        err.error.kind(),
        if cfg!(feature = "native-geospatial") {
            rusty_tiles::JobErrorKind::Io
        } else {
            rusty_tiles::JobErrorKind::Unsupported
        }
    );
    assert!(!output.exists());
}

// Historical 3d-tiles-tools comparisons for fixed 4096/512 errors and Euler
// placement were deliberately retired: W1 has independently defined full-detail
// semantics and F1c1 quaternion placement. Frozen audit probes remain historical.
#[test]
fn cli_single_model_manifest_accepts_explicit_anchor_and_rejects_output_override() {
    let tmp = tempfile::tempdir().unwrap();
    let glb = write_triangle(tmp.path());
    let bin = env!("CARGO_BIN_EXE_rusty-tiles");
    let st = Command::new(bin)
        .args(["createTilesetJson", "-i"])
        .arg(&glb)
        .args(["--anchor", "0", "0", "10"])
        .status()
        .unwrap();
    assert!(st.success(), "{st}");
    let ts: serde_json::Value =
        serde_json::from_slice(&fs::read(tmp.path().join("tileset.json")).unwrap()).unwrap();
    assert_eq!(ts["root"]["transform"][12], 6_378_147.);
    assert_eq!(ts["root"]["content"]["uri"], "triangle.glb");
    let refused = Command::new(bin)
        .args(["createTilesetJson", "-i"])
        .arg(&glb)
        .arg("-o")
        .arg(tmp.path().join("elsewhere.json"))
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(!tmp.path().join("elsewhere.json").exists());
}

#[test]
fn convert_via_cli_accepts_tileset_json_path() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("source");
    fs::create_dir(&source).unwrap();
    let glb = write_triangle(&source);
    let json = source.join("tileset.json");
    model_to_manifest(
        ModelManifestRequest::local_gltf(&glb),
        &RunControl::default(),
    )
    .unwrap();
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
