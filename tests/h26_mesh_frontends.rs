//! Independent CLI controls for the single source-mesh tiler facade.
use serde_json::{json, Value};
use std::{fs, io::Read, path::Path, process::Command};

fn source(path: &Path, extras: bool) {
    let positions = [0f32, 0., 0., 1., 0., 0., 0., 1., 0.];
    let bin = positions
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<_>>();
    let mut doc = json!({
        "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
        "nodes":[{"mesh":0}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],
        "buffers":[{"byteLength":36}],"bufferViews":[{"buffer":0,"byteLength":36}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}]
    });
    if extras {
        doc["extras"] = json!({"unadmitted":true});
    }
    let mut text = serde_json::to_vec(&doc).unwrap();
    text.resize(text.len().next_multiple_of(4), b' ');
    let mut bytes = b"glTF".to_vec();
    for word in [
        2u32,
        (28 + text.len() + bin.len()) as u32,
        text.len() as u32,
    ] {
        bytes.extend(word.to_le_bytes());
    }
    bytes.extend(b"JSON");
    bytes.extend(text);
    bytes.extend((bin.len() as u32).to_le_bytes());
    bytes.extend(b"BIN\0");
    bytes.extend(bin);
    fs::write(path, bytes).unwrap();
}

fn mesh(input: &Path, output: &Path, options: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["--json", "--progress", "json", "mesh", "-i"])
        .arg(input)
        .arg("-o")
        .arg(output)
        .args(options)
        .output()
        .unwrap()
}

fn archive_json(path: &Path, name: &str) -> Value {
    let mut archive = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut bytes = Vec::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn cli_report_is_the_archive_report_with_separate_resource_diagnostics() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("mesh.glb");
    let output = work.path().join("mesh.3tz");
    source(&input, false);
    let result = mesh(&input, &output, &["--jobs", "1"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let envelope: Value = serde_json::from_slice(&result.stdout).unwrap();
    let report = &envelope["tilerReport"];
    assert_eq!(report, &archive_json(&output, "conversion.json"));
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["operation"], "mesh_tiler");
    assert_eq!(report["tiles_version"], "1.1");
    assert_eq!(report["gltf_version"], "2.0");
    assert_eq!(report["profile"], "balanced");
    assert_eq!(report["triangles"], 1);
    assert_eq!(report["leaves"], 1);
    assert_eq!(report["placement"]["kind"], "local");
    assert_eq!(report["source_coordinates"], "local-gltf");
    assert_eq!(report["coordinates"], "local-gltf");
    assert!(report["root_transform"].is_null());
    assert!(envelope["diagnostics"]["capture"].is_object());
    assert_eq!(envelope["diagnostics"]["leaf_delivery"]["jobs"], 1);
    assert_eq!(
        envelope["diagnostics"]["archive_bytes"],
        fs::metadata(&output).unwrap().len()
    );
    assert_eq!(envelope["cleanupDiagnostics"], json!([]));
    assert_eq!(
        envelope["output"],
        fs::canonicalize(&output)
            .unwrap()
            .to_string_lossy()
            .as_ref()
    );
    assert!(report.get("diagnostics").is_none());
    let events = String::from_utf8(result.stderr).unwrap();
    let events = events
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert!(events
        .iter()
        .any(|event| event["phase"] == "ready_to_publish"));
    assert_eq!(events.last().unwrap()["phase"], "conversion");
    assert_eq!(events.last().unwrap()["done"], 1);
    // The work inside the leaf stage is reported as named phases with the unchanged
    // progress schema, and each finished phase logs its wall seconds.
    let phases: Vec<&str> = events
        .iter()
        .filter(|e| e["event"] == "progress")
        .map(|e| e["phase"].as_str().unwrap())
        .collect();
    for phase in ["capture", "leaf_build", "leaf_payloads", "archive"] {
        assert!(phases.contains(&phase), "{phase} in {phases:?}");
    }
    for event in events.iter().filter(|e| e["event"] == "progress") {
        let mut keys: Vec<&str> = event
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["done", "event", "phase", "total"]);
    }
    assert!(events.iter().any(|e| e["event"] == "log"
        && e["message"]
            .as_str()
            .unwrap()
            .starts_with("mesh phase leaf_build: ")));
    let seconds = &envelope["diagnostics"]["phase_seconds"];
    for phase in ["capture", "leaf_build", "proxy_bake", "archive"] {
        assert!(seconds[phase].as_f64().unwrap() >= 0., "{phase}");
    }
    assert_eq!(
        envelope["diagnostics"]["proxy_bake"]["facing_fallback_faces"],
        0
    );
}

#[test]
fn jobs_and_scratch_do_not_change_published_semantic_bytes() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("mesh.glb");
    source(&input, false);
    let scratch = work.path().join("scratch");
    fs::create_dir(&scratch).unwrap();
    let a = work.path().join("one.3tz");
    let b = work.path().join("auto.3tz");
    let first = mesh(&input, &a, &["--jobs", "1"]);
    let second = mesh(
        &input,
        &b,
        &["--jobs", "auto", "--scratch-dir", scratch.to_str().unwrap()],
    );
    assert!(
        first.status.success() && second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(fs::read(&a).unwrap(), fs::read(&b).unwrap());
    assert!(scratch.read_dir().unwrap().next().is_none());
}

#[test]
fn request_errors_precede_source_io_and_preserve_existing_outputs() {
    let work = tempfile::tempdir().unwrap();
    let missing = work.path().join("missing.glb");
    for (output, args) in [
        (work.path().join("bad.zip"), vec![]),
        (
            work.path().join("orientation.3tz"),
            vec!["--orientation-xyzw", "0", "0", "0", "1"],
        ),
        (
            work.path().join("auto.3tz"),
            vec![
                "--source-crs",
                "auto",
                "--source-axes",
                "y-up",
                "--height-offset",
                "0",
            ],
        ),
        (
            work.path().join("both-offsets.3tz"),
            vec![
                "--source-crs",
                "EPSG:3857",
                "--source-offset-file",
                "offset.txt",
                "--source-offset",
                "1",
                "2",
            ],
        ),
        (
            work.path().join("local-offset-file.3tz"),
            vec!["--source-offset-file", "offset.txt"],
        ),
        (
            work.path().join("local-offset.3tz"),
            vec!["--source-offset", "1", "2"],
        ),
        (
            work.path().join("exclusive.3tz"),
            vec![
                "--source-crs",
                "EPSG:3857",
                "--source-axes",
                "y-up",
                "--height-offset",
                "0",
                "--anchor",
                "153",
                "-27",
                "0",
            ],
        ),
        (
            work.path().join("quaternion.3tz"),
            vec![
                "--anchor",
                "153",
                "-27",
                "0",
                "--orientationXyzw",
                "0",
                "0",
                "0",
                "2",
            ],
        ),
    ] {
        fs::write(&output, b"prior destination").unwrap();
        let result = mesh(&missing, &output, &args);
        assert_eq!(result.status.code(), Some(2));
        let error: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(error["error"]["kind"], "invalid_request");
        assert_eq!(fs::read(&output).unwrap(), b"prior destination");
    }
}

#[test]
fn parser_rejects_legacy_tuning_and_internal_commands_as_usage() {
    let work = tempfile::tempdir().unwrap();
    for args in [
        vec!["--profile", "unknown"],
        vec!["--jobs", "0"],
        vec!["--jobs", "-2"],
        vec!["--leaf-triangles", "1"],
        vec!["--root-proxy-triangles", "1"],
        vec!["--max-proxy-error-metres", "1"],
        vec!["--source-axes", "z-up"],
        vec!["--source-offset", "1"],
        vec!["--texture-format", "jpeg"],
        vec!["--explicit"],
    ] {
        let result = mesh(
            &work.path().join("missing.glb"),
            &work.path().join("out.3tz"),
            &args,
        );
        assert_eq!(result.status.code(), Some(2));
        let error: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(error["error"]["code"], "usage");
        assert!(error["error"].get("kind").is_none());
    }
    for name in [
        "mesh-leaf-payloads",
        "mesh-leaves-to-3tz",
        "mesh-large-to-3tz",
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
            .args(["--json", name])
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(2));
        assert_eq!(
            serde_json::from_slice::<Value>(&result.stdout).unwrap()["error"]["code"],
            "usage"
        );
    }
}

#[test]
fn unsupported_malformed_io_and_conflict_have_distinct_transport_kinds() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("mesh.glb");
    let output = work.path().join("mesh.3tz");
    source(&input, true);
    fs::write(&output, b"old").unwrap();
    for (data, code, kind) in [
        (None, 2, "unsupported"),
        (Some(b"invalid".as_slice()), 3, "invalid_input"),
    ] {
        if let Some(data) = data {
            fs::write(&input, data).unwrap();
        }
        let result = mesh(&input, &output, &["--force", "--jobs", "1"]);
        assert_eq!(result.status.code(), Some(code));
        assert_eq!(
            serde_json::from_slice::<Value>(&result.stdout).unwrap()["error"]["kind"],
            kind
        );
        assert_eq!(fs::read(&output).unwrap(), b"old");
    }
    fs::remove_file(&input).unwrap();
    let result = mesh(&input, &output, &["--force"]);
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stdout).unwrap()["error"]["kind"],
        "io"
    );
    source(&input, false);
    let result = mesh(&input, &output, &["--jobs", "1"]);
    assert_eq!(result.status.code(), Some(5));
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stdout).unwrap()["error"]["kind"],
        "output_conflict"
    );
    assert_eq!(fs::read(&output).unwrap(), b"old");
}

#[test]
fn rigid_placement_spellings_publish_the_declared_root_transform() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("mesh.glb");
    source(&input, false);
    for (orientation, offset, name) in [
        ("--orientation-xyzw", "--scene-offset", "kebab.3tz"),
        ("--orientationXyzw", "--sceneOffset", "camel.3tz"),
    ] {
        let output = work.path().join(name);
        let result = mesh(
            &input,
            &output,
            &[
                "--jobs",
                "1",
                "--anchor",
                "153",
                "-27",
                "25",
                orientation,
                "0",
                "0",
                "0",
                "1",
                offset,
                "10",
                "2",
                "-5",
            ],
        );
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let summary: Value = serde_json::from_slice(&result.stdout).unwrap();
        let placement = &summary["tilerReport"]["placement"];
        assert_eq!(placement["kind"], "wgs84");
        assert_eq!(placement["anchor_degrees_metres"], json!([153., -27., 25.]));
        assert_eq!(placement["scene_offset_metres"], json!([10., 2., -5.]));
        let root = archive_json(&output, "tileset.json");
        assert_eq!(root["root"]["transform"].as_array().unwrap().len(), 16);
        for (reported, emitted) in summary["tilerReport"]["root_transform"]
            .as_array()
            .unwrap()
            .iter()
            .zip(root["root"]["transform"].as_array().unwrap())
        {
            assert_eq!(reported.as_f64(), emitted.as_f64());
        }
        assert_eq!(summary["tilerReport"]["coordinates"], "wgs84-ecef");
    }
    assert_eq!(
        fs::read(work.path().join("kebab.3tz")).unwrap(),
        fs::read(work.path().join("camel.3tz")).unwrap()
    );
}

#[test]
fn mesh_doctor_is_read_only_and_does_not_require_source_crs() {
    let work = tempfile::tempdir().unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .current_dir(work.path())
        .args([
            "--json",
            "doctor",
            "--command",
            "mesh",
            "--cesium",
            "absent",
        ])
        .output()
        .unwrap();
    assert!(result.status.success());
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["commands"]["mesh"]["status"], "ready");
    assert_eq!(report["commands"]["mesh"]["requires"], json!([]));
    assert_eq!(report["commands"]["mesh"]["profilesProvisional"], true);
    assert_eq!(work.path().read_dir().unwrap().count(), 0);
}

/// Kebab and camelCase spellings of the source CRS options publish the same archive,
/// whose report states the resolved CRS and whose root is its ENU frame.
#[test]
fn source_crs_spellings_publish_identical_archives_with_a_derived_root() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("mesh.glb");
    source(&input, false);
    let offset_file = work.path().join("mesh_offset.txt");
    fs::write(&offset_file, "E: 14812000\nN: -1384000\nA: 100\n").unwrap();
    let mut archives = Vec::new();
    for (name, crs, axes, offset, height) in [
        (
            "kebab.3tz",
            "--source-crs",
            "--source-axes",
            "--source-offset",
            "--height-offset",
        ),
        (
            "camel.3tz",
            "--sourceCrs",
            "--sourceAxes",
            "--sourceOffset",
            "--heightOffset",
        ),
    ] {
        let output = work.path().join(name);
        let result = mesh(
            &input,
            &output,
            &[
                "--jobs",
                "1",
                crs,
                "EPSG:3857",
                axes,
                "y-up",
                offset,
                "14812000",
                "-1384000",
                "100",
                height,
                "0",
            ],
        );
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let summary: Value = serde_json::from_slice(&result.stdout).unwrap();
        let report = &summary["tilerReport"];
        assert_eq!(report, &archive_json(&output, "conversion.json"));
        assert_eq!(report["source_coordinates"], "source-crs");
        assert_eq!(report["coordinates"], "wgs84-ecef");
        assert_eq!(report["placement"]["kind"], "wgs84");
        let crs = &report["source_crs"];
        assert_eq!(crs["definition"], "EPSG:3857");
        assert_eq!(crs["axes"], "y-up");
        assert_eq!(crs["source_offset"], json!([14812000., -1384000., 100.]));
        assert_eq!(crs["height_offset_metres"], 0.);
        assert_eq!(crs["operation"], "portable");
        // Independent EPSG:3857 inverse of the source-bounds centre (0.5, 0.5, 0).
        let a = 6_378_137_f64;
        let origin = crs["origin_degrees_metres"].as_array().unwrap();
        let lon = (14_812_000.5 / a).to_degrees();
        let lat = ((-1_384_000_f64 / a).sinh().atan()).to_degrees();
        assert!((origin[0].as_f64().unwrap() - lon).abs() < 1e-10);
        assert!((origin[1].as_f64().unwrap() - lat).abs() < 1e-10);
        assert!((origin[2].as_f64().unwrap() - 100.5).abs() < 1e-6);
        let emitted = archive_json(&output, "tileset.json")["root"]["transform"].clone();
        let reported = report["root_transform"].as_array().unwrap();
        assert_eq!(reported.len(), 16);
        for (reported, emitted) in reported.iter().zip(emitted.as_array().unwrap()) {
            assert_eq!(reported.as_f64(), emitted.as_f64());
        }
        archives.push(fs::read(&output).unwrap());
    }
    // The Metashape offset file with default y-up axes and zero height offset is the same request.
    let output = work.path().join("file.3tz");
    let result = mesh(
        &input,
        &output,
        &[
            "--jobs",
            "1",
            "--source-crs",
            "EPSG:3857",
            "--source-offset-file",
            offset_file.to_str().unwrap(),
        ],
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(archives[0], fs::read(&output).unwrap());
    assert_eq!(archives[0], archives[1]);
    // A malformed offset file is a typed request error.
    fs::write(&offset_file, "E=14812000\nN=-1384000\n").unwrap();
    let result = mesh(
        &input,
        &work.path().join("bad.3tz"),
        &[
            "--source-crs",
            "EPSG:3857",
            "--source-offset-file",
            offset_file.to_str().unwrap(),
        ],
    );
    assert_eq!(result.status.code(), Some(2));
    let error: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(error["error"]["kind"], "invalid_request");
}
