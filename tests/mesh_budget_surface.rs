//! The declared memory budget, a hidden option until HLOD is the default: the shared valid/invalid matrix through the
//! CLI and the Rust request, refusal ordering before any source I/O, and the reported budget.
//! Python reads the same matrix in `bindings/python/tests/test_api.py`.
use rusty_tiles::{mesh_to_tileset, JobErrorKind, RunControl, TilerLod, TilerRequest};
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};

const MATRIX: &str = include_str!("fixtures/memory_budget_matrix.json");

/// An indexed, gently curved `n` x `n` grid: 2 n^2 triangles, untextured.
fn grid_glb(path: &Path, n: usize) {
    let mut positions = Vec::new();
    for j in 0..=n {
        for i in 0..=n {
            let (x, y) = (i as f32 * 0.5, j as f32 * 0.5);
            positions.extend([x, (x * 0.31).sin() + (y * 0.17).cos(), y]);
        }
    }
    let mut indices = Vec::new();
    for j in 0..n {
        for i in 0..n {
            let a = (j * (n + 1) + i) as u32;
            let b = a + 1;
            let c = a + (n + 1) as u32;
            indices.extend([a, c, b, b, c, c + 1]);
        }
    }
    let position_bytes: Vec<u8> = positions.iter().flat_map(|v| v.to_le_bytes()).collect();
    let index_bytes: Vec<u8> = indices.iter().flat_map(|v| v.to_le_bytes()).collect();
    let bound = |axis: usize, pick: fn(f32, f32) -> f32, start: f32| {
        positions.chunks(3).map(|p| p[axis]).fold(start, pick)
    };
    let min = [0, 1, 2].map(|axis| bound(axis, f32::min, f32::INFINITY));
    let max = [0, 1, 2].map(|axis| bound(axis, f32::max, f32::NEG_INFINITY));
    let doc = json!({
        "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
        "nodes":[{"mesh":0}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],
        "buffers":[{"byteLength":position_bytes.len()+index_bytes.len()}],
        "bufferViews":[
            {"buffer":0,"byteOffset":0,"byteLength":position_bytes.len()},
            {"buffer":0,"byteOffset":position_bytes.len(),"byteLength":index_bytes.len()}],
        "accessors":[
            {"bufferView":0,"componentType":5126,"count":positions.len()/3,"type":"VEC3",
             "min":min,"max":max},
            {"bufferView":1,"componentType":5125,"count":indices.len(),"type":"SCALAR"}]
    });
    let mut text = serde_json::to_vec(&doc).unwrap();
    text.resize(text.len().next_multiple_of(4), b' ');
    let bin = [position_bytes, index_bytes].concat();
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

fn cli(input: &Path, output: &Path, options: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["--json", "mesh", "-i"])
        .arg(input)
        .arg("-o")
        .arg(output)
        .args(options)
        .output()
        .unwrap()
}

fn error_of(result: &std::process::Output) -> Value {
    serde_json::from_slice::<Value>(&result.stdout).unwrap()["error"].clone()
}

fn matrix() -> Value {
    serde_json::from_str(MATRIX).unwrap()
}

#[test]
fn cli_refuses_the_invalid_matrix_as_usage_before_any_source_io() {
    let work = tempfile::tempdir().unwrap();
    let missing = work.path().join("missing.glb");
    let output = work.path().join("out.3tz");
    for row in matrix()["invalid"].as_array().unwrap() {
        let text = row["text"].as_str().unwrap();
        // `=` keeps a leading sign or an empty value attached to the option.
        let flag = format!("--memory-budget={text}");
        let result = cli(&missing, &output, &["--lod", "hlod", &flag]);
        assert_eq!(result.status.code(), Some(2), "{text:?}");
        let error = error_of(&result);
        assert_eq!(error["code"], "usage", "{text:?}: {error}");
        assert!(!output.exists());
    }
}

#[test]
fn cli_accepts_the_valid_matrix_and_reaches_the_source() {
    let work = tempfile::tempdir().unwrap();
    let missing = work.path().join("missing.glb");
    let output = work.path().join("out.3tz");
    for row in matrix()["valid"].as_array().unwrap() {
        let text = row["text"].as_str().unwrap();
        let flag = format!("--memory-budget={text}");
        let result = cli(&missing, &output, &["--lod", "hlod", &flag]);
        // The budget parsed; the run then stopped at the missing source, not at the option.
        let error = error_of(&result);
        assert_ne!(error["code"], "usage", "{text:?}");
        assert_ne!(error["kind"], "invalid_request", "{text:?}: {error}");
        assert!(!output.exists());
    }
}

#[test]
fn camel_case_spelling_is_the_same_option() {
    let work = tempfile::tempdir().unwrap();
    let missing = work.path().join("missing.glb");
    let output = work.path().join("out.3tz");
    let result = cli(&missing, &output, &["--lod", "hlod", "--memoryBudget=0"]);
    assert_eq!(error_of(&result)["code"], "usage");
}

#[test]
fn rust_request_refuses_zero_and_the_default_method_before_any_path_check() {
    // The output has no .3tz extension and the source is missing: the budget is refused
    // first, so neither path rule runs.
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("missing.glb");
    let output = work.path().join("out.txt");
    for request in [
        TilerRequest::new(&input, &output)
            .with_lod(TilerLod::Hlod)
            .with_memory_budget(0),
        TilerRequest::new(&input, &output).with_memory_budget(0),
        TilerRequest::new(&input, &output).with_memory_budget(2 << 30),
    ] {
        let failure = mesh_to_tileset(request, &RunControl::default()).unwrap_err();
        assert_eq!(failure.error.kind(), JobErrorKind::InvalidRequest);
        assert!(
            failure.error.to_string().contains("memory budget"),
            "{}",
            failure.error
        );
        assert!(!output.exists());
    }
}

#[test]
fn budget_is_refused_before_the_offset_file_or_source_is_read() {
    // The default method cannot take a budget; the missing offset file would be an `io` error
    // if it were read first, and the missing source would fail after it.
    let work = tempfile::tempdir().unwrap();
    let missing = work.path().join("missing.glb");
    let offset = work.path().join("no-such-offset.txt");
    let output = work.path().join("out.3tz");
    let offset = offset.to_str().unwrap();
    let options = |lod: &[&'static str]| {
        let mut args = vec![
            "--source-crs",
            "EPSG:3857",
            "--source-offset-file",
            offset,
            "--memory-budget",
            "2GiB",
        ];
        args.extend(lod);
        args
    };
    let result = cli(&missing, &output, &options(&[]));
    assert_eq!(result.status.code(), Some(2));
    let error = error_of(&result);
    assert_eq!(error["kind"], "invalid_request", "{error}");
    assert!(error["message"].as_str().unwrap().contains("memory budget"));
    assert!(!output.exists());
    // A compatible request does read the offset file, which proves the order is deliberate.
    let result = cli(&missing, &output, &options(&["--lod", "hlod"]));
    let error = error_of(&result);
    assert_ne!(error["code"], "usage");
    assert!(
        !error["message"].as_str().unwrap().contains("memory budget"),
        "{error}"
    );
    assert!(!output.exists());
}

#[test]
fn default_method_refuses_a_budget_on_the_cli_and_writes_nothing() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("grid.glb");
    let output = work.path().join("out.3tz");
    grid_glb(&input, 4);
    let result = cli(&input, &output, &["--memory-budget", "2GiB"]);
    assert_eq!(result.status.code(), Some(2));
    let error = error_of(&result);
    assert_eq!(error["kind"], "invalid_request");
    assert!(error["message"].as_str().unwrap().contains("memory budget"));
    assert!(!output.exists());
}

#[test]
fn hlod_run_reports_requested_and_resolved_budget_and_never_raises_it() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("grid.glb");
    grid_glb(&input, 100);
    let mut archives = Vec::new();
    for (name, budget) in [
        ("two.3tz", Some("2GiB")),
        ("four.3tz", Some("4GiB")),
        ("default.3tz", None),
    ] {
        let output = work.path().join(name);
        let mut options = vec!["--lod", "hlod", "--jobs", "1"];
        if let Some(budget) = budget {
            options.extend(["--memory-budget", budget]);
        }
        let result = cli(&input, &output, &options);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        let envelope: Value = serde_json::from_slice(&result.stdout).unwrap();
        let memory = &envelope["diagnostics"]["memory"]["budget"];
        match budget {
            Some(text) => {
                let bytes = rusty_tiles::parse_memory_budget(text).unwrap();
                assert_eq!(memory["requested_bytes"], bytes);
                assert_eq!(memory["resolved_bytes"], bytes);
                assert_eq!(memory["source"], "explicit");
            }
            None => {
                assert!(memory["requested_bytes"].is_null());
                let resolved = memory["resolved_bytes"].as_u64().unwrap();
                assert!(resolved <= 4 << 30);
                assert!(memory["source"].as_str().unwrap().starts_with("default_"));
            }
        }
        assert!(
            envelope["diagnostics"]["memory"]["minimum_bytes"]
                .as_u64()
                .unwrap()
                > 0
        );
        archives.push(fs::read(&output).unwrap());
    }
    // The budget is a ceiling on admission, not an input: it never changes the archive.
    assert!(archives.windows(2).all(|pair| pair[0] == pair[1]));
}

#[test]
fn a_budget_below_the_stage_minimum_is_a_resource_refusal_that_publishes_nothing() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("grid.glb");
    let output = work.path().join("out.3tz");
    grid_glb(&input, 100);
    let result = cli(
        &input,
        &output,
        &["--lod", "hlod", "--memory-budget", "1MiB"],
    );
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(error_of(&result)["kind"], "resource_limit");
    assert!(!output.exists());
}

#[test]
fn help_advertises_neither_the_budget_nor_the_hlod_selector() {
    let help = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["mesh", "--help"])
        .output()
        .unwrap();
    let help = String::from_utf8(help.stdout).unwrap();
    for hidden in [
        "--memory-budget",
        "--memoryBudget",
        "--lod",
        "hlod",
        "H17",
        "h17",
    ] {
        assert!(!help.contains(hidden), "{hidden} in help:\n{help}");
    }
}
