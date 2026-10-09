//! Machine protocol: one stdout JSON value, NDJSON progress on stderr and
//! categorized exit codes. Human summaries are covered in `cli_contract.rs`.
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

mod support;

fn call(args: &[&std::ffi::OsStr], env: &[(&str, &Path)]) -> (std::process::Output, Value) {
    let output = support::rusty_tiles()
        .arg("--json")
        .args(args)
        .envs(env.iter().copied())
        .output()
        .unwrap();
    let report = serde_json::from_slice(&output.stdout).unwrap();
    (output, report)
}

#[cfg_attr(not(feature = "native-geospatial"), allow(dead_code))]
fn source(root: &Path) -> PathBuf {
    let path = root.join("source.geojson");
    std::fs::write(
        &path,
        json!({"type":"FeatureCollection","features":[{"type":"Feature","id":1,
            "properties":{"name":"example"},"geometry":{"type":"Point","coordinates":[0,0,0]}}]})
        .to_string(),
    )
    .unwrap();
    path
}

#[cfg(feature = "native-geospatial")]
#[test]
fn success_summary_progress_and_conflict_protocol() {
    let root = tempfile::tempdir().unwrap();
    let source = source(root.path());
    let out = root.path().join("out.3tz");
    let args = [
        "vector".as_ref(),
        "-i".as_ref(),
        source.as_os_str(),
        "-o".as_ref(),
        out.as_os_str(),
        "--sourceCrs".as_ref(),
        "local".as_ref(),
        "--meshopt".as_ref(),
        "--progress".as_ref(),
        "json".as_ref(),
    ];
    let (result, report) = call(&args, &[]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(report["ok"], true);
    assert_eq!(report["output"], out.to_str().unwrap());
    assert_eq!(report["counts"]["features"], 1);
    assert_eq!(report["skippedFeatures"], 0);
    // Settings are separate from counts.
    assert_eq!(report["settings"]["lodLevels"], 3);
    assert!(report["counts"].get("lodLevels").is_none());
    assert!(report["counts"].get("lodToleranceMetres").is_none());
    assert_eq!(
        report["conversionReport"],
        json!({"archive":out.to_str().unwrap(),"entry":"conversion.json"})
    );
    let events: Vec<Value> = String::from_utf8(result.stderr)
        .unwrap()
        .lines()
        .filter(|line| line.starts_with('{'))
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        events[0],
        json!({"event":"progress","phase":"conversion","done":0,"total":1})
    );
    let last = events.last().unwrap();
    assert_eq!((&last["done"], &last["total"]), (&json!(1), &json!(1)));
    assert!(events.iter().any(|event| event["phase"] == "ingestion"));
    let before = std::fs::read(&out).unwrap();
    let (result, report) = call(&args[..5], &[]);
    assert_eq!(result.status.code(), Some(5));
    assert_eq!(report["error"]["code"], "output_conflict");
    assert_eq!(std::fs::read(&out).unwrap(), before);
}

#[cfg(feature = "native-geospatial")]
#[test]
fn kebab_aliases_reach_the_published_budgets() {
    let root = tempfile::tempdir().unwrap();
    let source = source(root.path());
    let out = root.path().join("out.3tz");
    let result = support::rusty_tiles()
        .args(["vector", "-i"])
        .arg(&source)
        .arg("-o")
        .arg(&out)
        .args(["--source-crs", "local", "--max-features", "8"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stdout.is_empty());
    let mut archive = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
    let report: Value =
        serde_json::from_reader(archive.by_name("conversion.json").unwrap()).unwrap();
    assert_eq!(report["budgets"]["features"], 8);
}

#[test]
fn usage_error_has_its_own_code() {
    let (result, report) = call(&["vector".as_ref()], &[]);
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(report["error"]["code"], "usage");
}

#[test]
fn failed_conversion_emits_ndjson_without_completion() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing");
    let out = root.path().join("out.3tz");
    let result = support::rusty_tiles()
        .args(["--json", "convert", "-i"])
        .arg(&missing)
        .arg("-o")
        .arg(&out)
        .args(["--progress", "json"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["error"]["code"], "io");
    let events: Vec<Value> = String::from_utf8(result.stderr)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        events.last().unwrap(),
        &json!({"event":"failed","phase":"conversion","code":"io"})
    );
    assert!(!events
        .iter()
        .any(|e| e["event"] == "progress" && e["phase"] == "conversion" && e["done"] == 1));
    assert!(!out.exists());
}

#[cfg(feature = "native-geospatial")]
#[test]
fn data_and_environment_errors_have_distinct_codes() {
    let root = tempfile::tempdir().unwrap();
    let source = source(root.path());
    let out = root.path().join("out.3tz");
    let args = |crs: &'static str| {
        [
            "vector".as_ref(),
            "-i".as_ref(),
            source.as_os_str(),
            "-o".as_ref(),
            out.as_os_str(),
            "--sourceCrs".as_ref(),
            std::ffi::OsStr::new(crs),
        ]
    };
    std::fs::write(&source, "invalid GeoJSON").unwrap();
    let (result, report) = call(&args("local"), &[]);
    assert_eq!(
        result.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(report["error"]["code"], "data");
    assert!(!out.exists());
    // Restore a valid source so only the PROJ database is missing.
    self::source(root.path());
    let missing = root.path().join("missing-proj-data");
    std::fs::create_dir(&missing).unwrap();
    let (result, report) = call(
        &args("EPSG:4326"),
        &[("PROJ_DATA", &missing), ("PROJ_LIB", &missing)],
    );
    assert_eq!(result.status.code(), Some(4));
    assert_eq!(report["error"]["code"], "environment");
    assert!(report["error"]["message"]
        .as_str()
        .unwrap()
        .contains("PROJ database"));
    assert!(!out.exists());
}

#[test]
fn doctor_failure_keeps_inventory_in_single_result() {
    let root = tempfile::tempdir().unwrap();
    let (result, report) = call(
        &["doctor".as_ref(), "--command".as_ref(), "raster".as_ref()],
        &[("PROJ_DATA", root.path()), ("PROJ_LIB", root.path())],
    );
    assert_eq!(result.status.code(), Some(4));
    assert_eq!(report["ok"], false);
    assert_eq!(report["error"]["code"], "environment");
    assert!(report["commands"]["raster"].is_object());
}
