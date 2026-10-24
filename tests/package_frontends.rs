//! CLI contract for the same package request used by Rust and Python.
use serde_json::Value;
use std::{fs, io::Read, path::Path, process::Command};

fn convert(source: &Path, output: &Path, progress: bool) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"));
    command
        .args(["--json", "convert", "-i"])
        .arg(source)
        .arg("-o")
        .arg(output);
    if progress {
        command.args(["--progress", "json"]);
    }
    command.output().unwrap()
}

fn source(root: &Path) -> std::path::PathBuf {
    let source = root.join("source");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("tileset.json"),
        b"{\"asset\":{\"version\":\"1.1\"},\"root\":{}}\n",
    )
    .unwrap();
    fs::write(source.join("opaque.bin"), [0, 1, 255, 4]).unwrap();
    // Packaging treats producer report bytes as opaque selected source data.
    fs::write(
        source.join("conversion.json"),
        b"arbitrary producer bytes\n",
    )
    .unwrap();
    source
}

#[test]
fn cli_package_receipt_is_separate_from_unchanged_source_report() {
    let work = tempfile::tempdir().unwrap();
    let source = source(work.path());
    let output = work.path().join("package.3tz");
    let completed = convert(&source.join("tileset.json"), &output, true);
    assert!(
        completed.status.success(),
        "{}",
        String::from_utf8_lossy(&completed.stderr)
    );
    let summary: Value = serde_json::from_slice(&completed.stdout).unwrap();
    assert_eq!(summary["ok"], true);
    assert_eq!(summary["conversionReport"], Value::Null);
    assert_eq!(summary["counts"], serde_json::json!({}));
    assert_eq!(summary["packageReceipt"]["memberCount"], 3);
    assert_eq!(
        summary["packageReceipt"]["archiveBytes"],
        fs::metadata(&output).unwrap().len()
    );
    let expected_bytes: u64 = fs::read_dir(&source)
        .unwrap()
        .map(|entry| entry.unwrap().metadata().unwrap().len())
        .sum();
    assert_eq!(summary["packageReceipt"]["sourceBytes"], expected_bytes);
    assert_eq!(summary["cleanupDiagnostics"], serde_json::json!([]));

    let events: Vec<Value> = String::from_utf8(completed.stderr)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(events.iter().any(|event| event["phase"] == "encoding"));
    assert!(events
        .iter()
        .any(|event| event["phase"] == "ready_to_publish"));
    assert_eq!(events.last().unwrap()["phase"], "conversion");
    assert_eq!(events.last().unwrap()["done"], 1);
    let mut archive = zip::ZipArchive::new(fs::File::open(output).unwrap()).unwrap();
    for name in ["tileset.json", "opaque.bin", "conversion.json"] {
        let mut payload = Vec::new();
        archive
            .by_name(name)
            .unwrap()
            .read_to_end(&mut payload)
            .unwrap();
        assert_eq!(payload, fs::read(source.join(name)).unwrap());
    }
}

#[test]
fn cli_typed_package_failures_leave_destination_parents_uncreated() {
    for (case, kind, status) in [
        ("missing_source", "io", 1),
        ("missing_tileset", "invalid_input", 3),
        ("reserved_index", "invalid_request", 2),
        ("source_overlap", "invalid_request", 2),
    ] {
        let work = tempfile::tempdir().unwrap();
        let source = source(work.path());
        let mut input = source.clone();
        let mut output = work.path().join("not-created/package.3tz");
        match case {
            "missing_source" => input = work.path().join("missing"),
            "missing_tileset" => fs::remove_file(source.join("tileset.json")).unwrap(),
            "reserved_index" => fs::write(source.join("@3dtilesIndex1@"), b"stale index").unwrap(),
            "source_overlap" => output = source.join("not-created/package.3tz"),
            _ => unreachable!(),
        }
        let completed = convert(&input, &output, false);
        assert_eq!(
            completed.status.code(),
            Some(status),
            "{case}: {}",
            String::from_utf8_lossy(&completed.stdout)
        );
        let failure: Value = serde_json::from_slice(&completed.stdout).unwrap();
        assert_eq!(failure["error"]["kind"], kind, "{case}");
        assert_eq!(failure["error"]["code"], kind, "{case}");
        assert_eq!(failure["error"]["retainedPaths"], serde_json::json!([]));
        assert!(
            !output.parent().unwrap().exists(),
            "{case} created output parent"
        );
    }
}

#[test]
fn cli_no_clobber_conflict_preserves_existing_bytes() {
    let work = tempfile::tempdir().unwrap();
    let source = source(work.path());
    let output = work.path().join("existing.3tz");
    fs::write(&output, b"competing destination bytes").unwrap();
    let completed = convert(&source, &output, false);
    assert_eq!(completed.status.code(), Some(5));
    let failure: Value = serde_json::from_slice(&completed.stdout).unwrap();
    assert_eq!(failure["error"]["kind"], "output_conflict");
    assert_eq!(fs::read(output).unwrap(), b"competing destination bytes");
}

// APFS rejects this invalid UTF-8 filename before the operation can run.
#[cfg(target_os = "linux")]
#[test]
fn cli_package_non_utf8_output_ancestor_returns_success_json_after_installation() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let work = tempfile::tempdir().unwrap();
    let source = source(work.path());
    let parent = work
        .path()
        .join(OsString::from_vec(b"nonutf8-\xff".to_vec()));
    fs::create_dir(&parent).unwrap();
    let output = parent.join("package.3tz");
    let completed = convert(&source, &output, false);
    assert!(
        completed.status.success(),
        "{}",
        String::from_utf8_lossy(&completed.stderr)
    );
    let summary: Value = serde_json::from_slice(&completed.stdout).unwrap();
    assert_eq!(summary["ok"], true);
    assert_eq!(
        summary["output"],
        fs::canonicalize(&output)
            .unwrap()
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(
        summary["packageReceipt"]["archiveBytes"],
        fs::metadata(&output).unwrap().len()
    );
    let mut archive = zip::ZipArchive::new(fs::File::open(&output).unwrap()).unwrap();
    let mut bytes = Vec::new();
    archive
        .by_name("tileset.json")
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, fs::read(source.join("tileset.json")).unwrap());
}
