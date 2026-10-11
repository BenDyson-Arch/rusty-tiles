//! C2 decoded-content containment through the public Rust and CLI entry points.
//! Fixtures are built in-test; the oracle's materialized fixtures are added separately.
use rusty_tiles::package::{package, PackageRequest};
use rusty_tiles::validate::{inspect, ValidationFailure, ValidationRequest};
use rusty_tiles::RunControl;
use serde_json::{json, Value};
use std::{path::PathBuf, process::Command};

fn point_glb(position: [f32; 3], node: Value) -> Vec<u8> {
    let bin: Vec<u8> = position.iter().flat_map(|c| c.to_le_bytes()).collect();
    let bound: Vec<f64> = position.iter().map(|c| f64::from(*c)).collect();
    let doc = json!({
        "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[node],
        "buffers":[{"byteLength":12}],"bufferViews":[{"buffer":0,"byteLength":12}],
        "accessors":[{"bufferView":0,"componentType":5126,"type":"VEC3","count":1,"min":bound,"max":bound}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":0}]}]
    });
    let mut json = serde_json::to_vec(&doc).unwrap();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let mut out = b"glTF".to_vec();
    out.extend(2u32.to_le_bytes());
    out.extend((28 + json.len() as u32 + 12).to_le_bytes());
    out.extend((json.len() as u32).to_le_bytes());
    out.extend(b"JSON");
    out.extend(json);
    out.extend(12u32.to_le_bytes());
    out.extend(b"BIN\0");
    out.extend(bin);
    out
}

fn archive(work: &tempfile::TempDir, half: [f64; 3], glb: Vec<u8>) -> PathBuf {
    let data = work.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let tileset = json!({"asset":{"version":"1.1"},"geometricError":0,"root":{
        "boundingVolume":{"box":[0,0,0,half[0],0,0,0,half[1],0,0,0,half[2]]},
        "geometricError":0,"content":{"uri":"a.glb"}}});
    std::fs::write(
        data.join("tileset.json"),
        serde_json::to_vec(&tileset).unwrap(),
    )
    .unwrap();
    std::fs::write(data.join("a.glb"), glb).unwrap();
    let output = work.path().join("test.3tz");
    package(
        PackageRequest::directory(&data, &output),
        &RunControl::default(),
    )
    .unwrap();
    output
}

fn cli(path: &PathBuf) -> (Option<i32>, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .arg("validate")
        .arg(path)
        .arg("--json")
        .output()
        .unwrap();
    (
        output.status.code(),
        serde_json::from_slice(&output.stdout).unwrap(),
    )
}

#[test]
fn contained_and_outside_pair_with_cli_parity() {
    let work = tempfile::tempdir().unwrap();
    // glTF (1, 2, 3) is (1, -3, 2) in the z-up tile frame: on the box faces.
    let path = archive(
        &work,
        [1., 3., 2.],
        point_glb([1., 2., 3.], json!({"mesh":0})),
    );
    let report = serde_json::to_value(inspect(ValidationRequest::new(&path)).unwrap()).unwrap();
    let checks = report["checks"].as_array().unwrap();
    assert!(
        checks.contains(&json!("decodedContentContainment")),
        "{report}"
    );
    let not_inspected = report["notInspected"].as_array().unwrap();
    assert!(!not_inspected.contains(&json!("decodedContentBounds")));
    assert!(!not_inspected.contains(&json!("scene transforms")));
    assert!(not_inspected.contains(&json!(
        "skins, animations, morph targets and rendered extent"
    )));
    assert_eq!(report["limits"]["containmentSpanBits"], 4096);
    assert_eq!(report["limits"]["containmentWorkUnits"], 4294967296u64);
    assert_eq!(report["limits"]["containmentBytesRead"], 2147483648u64);
    assert_eq!(report["limits"]["containmentDecodedBytes"], 2147483648u64);
    assert_eq!(report["limits"]["containmentNodeDepth"], 64);
    let (code, cli_report) = cli(&path);
    assert_eq!(code, Some(0));
    assert_eq!(cli_report, report);

    let work = tempfile::tempdir().unwrap();
    let path = archive(
        &work,
        [1., 3., 2f64.next_down()],
        point_glb([1., 2., 3.], json!({"mesh":0})),
    );
    let failure = inspect(ValidationRequest::new(&path)).unwrap_err();
    let expected = "decoded content outside bounding volume: content=a.glb depth=0 volume=own node=0 mesh=0 primitive=0 vertex=0";
    assert!(
        matches!(&failure, ValidationFailure::InvalidInput(m) if m == expected),
        "{failure:?}"
    );
    let (code, cli_report) = cli(&path);
    assert_eq!(code, Some(3));
    assert_eq!(cli_report["error"]["code"], "invalid_input");
    assert_eq!(cli_report["error"]["message"], expected);
}

#[test]
fn excluded_archive_is_c1_success_with_reasons() {
    let work = tempfile::tempdir().unwrap();
    // Out of the box, but skinned: not inspected rather than failed (I3).
    let path = archive(
        &work,
        [1.; 3],
        point_glb([9., 9., 9.], json!({"mesh":0,"skin":0})),
    );
    let report = inspect(ValidationRequest::new(&path)).unwrap();
    assert!(!report
        .checks
        .iter()
        .any(|c| c == "decodedContentContainment"));
    assert!(report
        .not_inspected
        .iter()
        .any(|c| c == "decodedContentBounds"));
    assert!(report
        .not_inspected
        .iter()
        .any(|c| c == "decodedContentBounds.skin"));
    assert!(report.not_inspected.iter().any(|c| c == "scene transforms"));
    let (code, cli_report) = cli(&path);
    assert_eq!(code, Some(0));
    assert_eq!(cli_report, serde_json::to_value(report).unwrap());
}

#[test]
fn malformed_node_is_invalid_input_even_when_excluded() {
    let work = tempfile::tempdir().unwrap();
    let path = archive(
        &work,
        [1.; 3],
        point_glb([0.; 3], json!({"mesh":0,"skin":0,"rotation":[0,0,0,0]})),
    );
    let failure = inspect(ValidationRequest::new(&path)).unwrap_err();
    assert_eq!(failure.category(), ("invalid_input", 3), "{failure:?}");
}
