//! Shared vector acceptance for portable and native builds, without executables.
mod support;

#[cfg(not(feature = "native-geospatial"))]
#[test]
fn portable_case_distinct_property_filters_publish_or_refuse_without_clobbering() {
    use serde_json::{json, Value};
    use std::{fs, process::Command};
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("case-sensitive.geojson");
    fs::write(
        &input,
        serde_json::to_vec(&json!({"type":"FeatureCollection","features":[
            {"type":"Feature","properties":{"keep":1,"A":1,"a":2},
                "geometry":{"type":"Point","coordinates":[1,2,3]}}
        ]}))
        .unwrap(),
    )
    .unwrap();
    let before = fs::read(&input).unwrap();
    for (name, expression, count) in [("matched", "keep = 1", 1), ("empty", "0", 0)] {
        let output = directory.path().join(format!("{name}.3tz"));
        let result = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
            .args(["vector", "--json", "-i"])
            .arg(&input)
            .arg("-o")
            .arg(&output)
            .args([
                "--source-crs",
                "local",
                "--fields",
                "keep",
                "--where",
                expression,
            ])
            .env("PATH", "")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        let mut archive = zip::ZipArchive::new(fs::File::open(&output).unwrap()).unwrap();
        let report: Value =
            serde_json::from_reader(archive.by_name("conversion.json").unwrap()).unwrap();
        assert_eq!(report["features"], count);
        rusty_tiles::validate_3tz(&output).unwrap();
    }
    let output = directory.path().join("previous.3tz");
    fs::write(&output, b"previous output").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["vector", "--json", "-i"])
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .args([
            "--force",
            "--source-crs",
            "local",
            "--fields",
            "keep",
            "--where",
            "\"a\" = 2",
        ])
        .env("PATH", "")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(3));
    let response: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(response["error"]["message"]
        .as_str()
        .unwrap()
        .contains("ambiguous GeoJSON property names"));
    assert_eq!(fs::read(&output).unwrap(), b"previous output");
    assert_eq!(fs::read(&input).unwrap(), before);
}

#[test]
fn vector_is_reproducible_and_reuses_native_content_without_executables() {
    use serde_json::{json, Value};
    use std::{fs, process::Command};
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source with spaces.geojson");
    let features:Vec<_>=(0..12).map(|i| json!({"type":"Feature","id":i,"properties":{"large":1152921504606846979i64,"name":format!("line-{i}")},"geometry":{"type":"LineString","coordinates":(0..100).map(|j|[i as f64*100.+j as f64,(j as f64*0.1).sin()*0.01,0.]).collect::<Vec<_>>()}})).collect();
    fs::write(
        &source,
        serde_json::to_vec(&json!({"type":"FeatureCollection","features":features})).unwrap(),
    )
    .unwrap();
    let run = |name: &str, jobs: &str, previous: Option<&std::path::Path>| {
        let output = directory.path().join(name);
        let mut command = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"));
        command
            .args(["vector", "-i"])
            .arg(&source)
            .arg("-o")
            .arg(&output)
            .args([
                "--sourceCrs",
                "local",
                "--jobs",
                jobs,
                "--maxFeatures",
                "2",
                "--reproducible",
                "--meshopt",
                "--quantize",
                "--json",
            ])
            .env("PATH", "")
            .env("PYTHONPATH", "/unavailable");
        if let Some(previous) = previous {
            command.arg("--reuseTileset").arg(previous);
        }
        let result = command.output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(report["ok"], true);
        output
    };
    let a = run("one.3tz", "1", None);
    let b = run("four.3tz", "4", None);
    assert_eq!(fs::read(&a).unwrap(), fs::read(&b).unwrap());
    let reused = run("reused.3tz", "4", Some(&a));
    let mut zip = zip::ZipArchive::new(fs::File::open(reused).unwrap()).unwrap();
    let report: Value = serde_json::from_reader(zip.by_name("conversion.json").unwrap()).unwrap();
    assert!(report["encoding"]["maximumTilePrimitives"]
        .as_u64()
        .is_some());
    assert_eq!(report["reuse"]["rebuiltContents"], 0);
    assert!(report["reuse"]["reusedContents"].as_u64().unwrap() > 0);
    let result = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["doctor", "--command", "vector", "--json"])
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(result.status.success());
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["commands"]["vector"]["geometry"]["ready"], true);
    assert!(report.get("python").is_none());
}

#[test]
fn compressed_point_aggregates_refine_and_reuse_without_executables() {
    use serde_json::{json, Value};
    use std::{fs, process::Command};
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("points.geojson");
    let features: Vec<_> = (0..128)
        .map(|i| {
            json!({"type":"Feature","id":i,
        "properties":{"large":1152921504606846979i64,"flag":i%2==0},
        "geometry":{"type":"Point","coordinates":[i%16,i/16,0]}})
        })
        .collect();
    fs::write(
        &source,
        serde_json::to_vec(&json!({"type":"FeatureCollection","features":features})).unwrap(),
    )
    .unwrap();
    let run = |name: &str, jobs: &str, previous: Option<&std::path::Path>| {
        let output = directory.path().join(name);
        let mut command = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"));
        command
            .args(["vector", "-i"])
            .arg(&source)
            .arg("-o")
            .arg(&output)
            .args([
                "--sourceCrs",
                "local",
                "--aggregatePoints",
                "--maxFeatures",
                "16",
                "--maxParentFeatures",
                "8",
                "--lodTolerance",
                "5",
                "--jobs",
                jobs,
                "--meshopt",
                "--quantize",
                "--reproducible",
            ])
            .env("PATH", "")
            .env("PYTHONPATH", "/unavailable");
        if let Some(previous) = previous {
            command.arg("--reuseTileset").arg(previous);
        }
        let result = command.output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        output
    };
    let a = run("one.3tz", "1", None);
    let b = run("four.3tz", "4", None);
    assert_eq!(fs::read(&a).unwrap(), fs::read(&b).unwrap());
    let reused = run("reused.3tz", "4", Some(&a));
    let mut zip = zip::ZipArchive::new(fs::File::open(reused).unwrap()).unwrap();
    let manifest: Value = serde_json::from_reader(zip.by_name("tileset.json").unwrap()).unwrap();
    let summary = &manifest["root"]["extras"]["pointAggregation"];
    assert!(
        manifest["geometricError"].as_f64().unwrap()
            > manifest["root"]["geometricError"].as_f64().unwrap()
    );
    assert_eq!(summary["sourcePointCount"], 128);
    assert!(summary["aggregateCount"].as_u64().unwrap() <= 8);
    assert!(manifest["root"]["geometricError"].as_f64().unwrap() > 0.);
    let report: Value = serde_json::from_reader(zip.by_name("conversion.json").unwrap()).unwrap();
    assert!(report["pointAggregation"]["contentTiles"].as_u64().unwrap() > 0);
    assert_eq!(report["reuse"]["rebuiltContents"], 0);
}

#[test]
fn force_replaces_only_successful_output() {
    support::force_replaces_only_successful_output(
        "vector",
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vector.geojson"),
        &[],
        false,
    );
}
