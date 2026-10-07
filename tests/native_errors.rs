//! Native CLI failures stay concise, name the offending flag and do not
//! depend on Python imports.
#![cfg(feature = "native-geospatial")]
use serde_json::{json, Value};

mod support;

#[test]
fn invalid_topology_is_short_and_diagnostics_env_adds_gdal_warnings() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("invalid.geojson");
    let out = root.path().join("out.3tz");
    std::fs::write(
        &source,
        json!({"type":"FeatureCollection","features":[{"type":"Feature","id":2,"properties":{},
            "geometry":{"type":"Polygon","coordinates":[[[0,0,0],[2,2,0],[2,0,0],[0,2,0],[0,0,0]]]}}]})
        .to_string(),
    )
    .unwrap();
    // RUSTY_TILES_PYTHON_TRACEBACK is the deprecated name and still works.
    for (debug, variable) in [
        (false, None),
        (true, Some("RUSTY_TILES_NATIVE_DIAGNOSTICS")),
        (true, Some("RUSTY_TILES_PYTHON_TRACEBACK")),
    ] {
        let mut command = support::rusty_tiles();
        command
            .args(["vector", "-i"])
            .arg(&source)
            .arg("-o")
            .arg(&out)
            .args(["--sourceCrs", "local"])
            .env_remove("RUSTY_TILES_NATIVE_DIAGNOSTICS")
            .env_remove("RUSTY_TILES_PYTHON_TRACEBACK");
        if let Some(variable) = variable {
            command.env(variable, "1");
        }
        let result = command.output().unwrap();
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(!result.status.success(), "{variable:?}");
        assert!(!out.exists());
        assert!(stderr.contains("invalid polygon topology"), "{stderr}");
        for absent in ["NumPy are required", "__source__=", "Traceback"] {
            assert!(!stderr.contains(absent), "{stderr}");
        }
        assert!(stderr.len() < if debug { 5000 } else { 2000 }, "{stderr}");
        assert_eq!(
            stderr.contains("Warning 1:"),
            debug,
            "{variable:?}: {stderr}"
        );
    }
}

#[test]
fn missing_python_modules_do_not_hide_invalid_input() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.geojson");
    let out = root.path().join("out.3tz");
    std::fs::write(&source, "{}").unwrap();
    std::fs::write(
        root.path().join("numpy.py"),
        "raise ImportError('test dependency unavailable')",
    )
    .unwrap();
    let result = support::rusty_tiles()
        .args(["vector", "-i"])
        .arg(&source)
        .arg("-o")
        .arg(&out)
        .env("PYTHONPATH", root.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(!result.status.success());
    assert!(!out.exists());
    assert!(stderr.contains("OGR cannot open vector input"), "{stderr}");
    assert!(!stderr.contains("Python dependency import failed"));
    assert!(!stderr.contains("Traceback"));
}

#[test]
fn option_errors_name_the_flag() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.geojson");
    let out = root.path().join("out.3tz");
    std::fs::write(
        &source,
        json!({"type":"FeatureCollection","features":[{"type":"Feature","id":1,
            "properties":{"name":"example"},"geometry":{"type":"Point","coordinates":[0,0,0]}}]})
        .to_string(),
    )
    .unwrap();
    for (extra, message) in [
        (["--maxBytes", "10"], "--maxBytes must be at least 4096"),
        (
            ["--lodLevels", "17"],
            "--lodLevels must be between 1 and 16",
        ),
        (["--lodTolerance", "0"], "--lodTolerance must be a positive"),
        (["--maxVertices", "3"], "--maxVertices must be at least 4"),
        (["--jobs", "0"], "--jobs must be at least 1"),
        (["--where", " "], "--where must not be empty"),
    ] {
        let result = support::rusty_tiles()
            .args(["--json", "vector", "-i"])
            .arg(&source)
            .arg("-o")
            .arg(&out)
            .args(["--sourceCrs", "local"])
            .args(extra)
            .output()
            .unwrap();
        let report: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(result.status.code(), Some(3), "{extra:?}");
        assert_eq!(report["error"]["code"], "data");
        assert!(
            report["error"]["message"]
                .as_str()
                .unwrap()
                .contains(message),
            "{report}"
        );
        assert!(!out.exists());
    }
}
