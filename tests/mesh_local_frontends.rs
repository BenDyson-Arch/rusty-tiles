//! CLI request/result parity for the bounded F1a mesh operation.
use serde_json::{json, Value};
use std::{fs, io::Read, path::Path, process::Command};

fn fixture(path: &Path, unsupported: bool) {
    let mut payload = Vec::new();
    for offset in [0., 3., 6.] {
        for point in [[offset, 0., 0.], [offset + 1., 0., 0.], [offset, 1., 0.]] {
            for coordinate in point {
                payload.extend_from_slice(&f32::to_le_bytes(coordinate));
            }
        }
    }
    let mut document = json!({
        "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
        "nodes":[{"mesh":0,"translation":[10,2,3]}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],
        "buffers":[{"byteLength":payload.len()}],
        "bufferViews":[{"buffer":0,"byteLength":payload.len()}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":9,"type":"VEC3",
                      "min":[0,0,0],"max":[7,1,0]}]
    });
    if unsupported {
        document["extras"] = json!({"unsupported":true});
    }
    let mut encoded = serde_json::to_vec(&document).unwrap();
    while !encoded.len().is_multiple_of(4) {
        encoded.push(b' ');
    }
    let mut bytes = Vec::new();
    for word in [0x4654_6c67, 2, (28 + encoded.len() + payload.len()) as u32] {
        bytes.extend_from_slice(&u32::to_le_bytes(word));
    }
    bytes.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&0x4e4f_534au32.to_le_bytes());
    bytes.extend_from_slice(&encoded);
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&0x004e_4942u32.to_le_bytes());
    bytes.extend_from_slice(&payload);
    fs::write(path, bytes).unwrap();
}

fn convert(source: &Path, output: &Path, limit: usize) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["--json", "--progress", "json", "mesh-local-to-3tz", "-i"])
        .arg(source)
        .arg("-o")
        .arg(output)
        .args(["--leaf-triangles", &limit.to_string()])
        .output()
        .unwrap()
}

#[test]
fn forced_multileaf_cli_returns_finalized_report_and_domain_events() {
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("source.glb");
    let output = work.path().join("output.3tz");
    fixture(&source, false);
    let completed = convert(&source, &output, 1);
    assert!(
        completed.status.success(),
        "{}",
        String::from_utf8_lossy(&completed.stderr)
    );
    let summary: Value = serde_json::from_slice(&completed.stdout).unwrap();
    assert_eq!(
        summary["output"],
        fs::canonicalize(&output)
            .unwrap()
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(summary["counts"]["triangles"], 3);
    assert_eq!(summary["counts"]["leaf_tiles"], 3);
    assert_eq!(summary["settings"]["leaf_triangles"], 1);
    assert_eq!(summary["meshReport"]["coordinates"], "local-gltf");
    assert_eq!(summary["cleanupDiagnostics"], json!([]));
    let mut archive = zip::ZipArchive::new(fs::File::open(&output).unwrap()).unwrap();
    let mut report = Vec::new();
    archive
        .by_name("conversion.json")
        .unwrap()
        .read_to_end(&mut report)
        .unwrap();
    assert_eq!(
        summary["meshReport"],
        serde_json::from_slice::<Value>(&report).unwrap()
    );
    let events: Vec<Value> = String::from_utf8(completed.stderr)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(events
        .iter()
        .any(|event| event["phase"] == "ready_to_publish"));
    assert_eq!(events.last().unwrap()["phase"], "conversion");
    assert_eq!(events.last().unwrap()["done"], 1);
}

#[test]
fn invalid_limit_and_unsupported_input_are_distinct_without_output_work() {
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("source.glb");
    let output = work.path().join("absent/output.3tz");
    let invalid = convert(&source, &output, 0);
    assert_eq!(invalid.status.code(), Some(2));
    let failure: Value = serde_json::from_slice(&invalid.stdout).unwrap();
    assert_eq!(failure["error"]["kind"], "invalid_request");
    assert!(!output.parent().unwrap().exists());
    fixture(&source, true);
    let unsupported = convert(&source, &output, 1);
    assert_eq!(unsupported.status.code(), Some(2));
    let failure: Value = serde_json::from_slice(&unsupported.stdout).unwrap();
    assert_eq!(failure["error"]["kind"], "unsupported");
    assert!(!output.parent().unwrap().exists());
    fs::write(&source, b"invalid GLB").unwrap();
    let malformed = convert(&source, &output, 1);
    assert_eq!(malformed.status.code(), Some(3));
    let failure: Value = serde_json::from_slice(&malformed.stdout).unwrap();
    assert_eq!(failure["error"]["kind"], "invalid_input");
    assert!(!output.parent().unwrap().exists());
}
