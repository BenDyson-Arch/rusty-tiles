//! Human-facing CLI contract of the built binary: the completion summary of
//! every converter subcommand and the `validate` usage errors.
mod support;

use serde_json::Value;
use std::{path::Path, process::Command};
use support::{bin, enabled_recipes, recipes, run, write_inputs};

/// The `next:` suggestion each converter prints for its output path.
fn expected_next(command: &str, output: &Path) -> String {
    // These fixtures deliberately use spaces, so the suggested shell argument
    // must be quoted on Unix and Windows alike.
    assert!(output.to_string_lossy().contains(' '));
    let path = format!("'{}'", output.display());
    match command {
        "raster" => format!("next: rusty-tiles preview --cesium <Build/Cesium> --imagery {path}"),
        "terrain" => format!("next: rusty-tiles preview --cesium <Build/Cesium> --terrain {path}"),
        "createTilesetJson" => format!("next: rusty-tiles convert -i {path} -o <archive>.3tz"),
        _ => format!("next: rusty-tiles validate {path}"),
    }
}

/// The summary lines: from `<command>: wrote <output>` to the end of stderr.
fn summary(stderr: &[u8], command: &str, output: &Path) -> Vec<String> {
    let text = String::from_utf8_lossy(stderr);
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    let wrote = format!("{command}: wrote {}", output.display());
    let start = lines
        .iter()
        .rposition(|l| l.starts_with(&wrote))
        .unwrap_or_else(|| panic!("no '{wrote}' line in:\n{text}"));
    lines[start..].to_vec()
}

#[test]
fn every_converter_prints_wrote_reports_and_next_lines() {
    let work = tempfile::tempdir().unwrap();
    // Resolve output identity; input recipes append URI-style separators and
    // therefore must not receive Windows verbatim paths from canonicalize.
    let root = std::fs::canonicalize(work.path()).unwrap();
    let inputs = work.path().join("inputs");
    let outputs = root.join("outputs with spaces");
    std::fs::create_dir_all(&outputs).unwrap();
    write_inputs(&inputs);
    let mut covered = std::collections::BTreeSet::new();
    for recipe in enabled_recipes() {
        let result = run(&recipe, &inputs, &outputs, &[]);
        assert!(
            result.status.success(),
            "{}: {}",
            recipe.name,
            String::from_utf8_lossy(&result.stderr)
        );
        let output = outputs.join(recipe.output);
        let lines = summary(&result.stderr, recipe.command, &output);
        let first = &lines[0];
        match recipe.command {
            "point-cloud" => assert!(first.ends_with(" (3000 points, 15 tiles)"), "{first}"),
            "vector" => assert!(first.ends_with(" (4 features, 3 tiles)"), "{first}"),
            "terrain" => assert!(first.ends_with(" (1 tile)"), "{first}"),
            "glb-to-3tz" | "createTilesetJson" => {
                assert!(first.ends_with(" (1152 triangles)"), "{first}")
            }
            "raster" => {
                // Both literal WGS84 fixtures cover four requested z9..11 tiles.
                assert_eq!(
                    first,
                    &format!("raster: wrote {} (4 tiles)", output.display())
                );
                let report: Value =
                    serde_json::from_slice(&std::fs::read(output.join("report.json")).unwrap())
                        .unwrap();
                assert_eq!(report["profile"], "r2-source-cog-direct-nearest-pyramid");
                assert_eq!(report["tiles"], 4);
                let pngs = walkdir::WalkDir::new(output.join("tiles"))
                    .into_iter()
                    .map(Result::unwrap)
                    .filter(|entry| entry.file_type().is_file())
                    .collect::<Vec<_>>();
                assert_eq!(pngs.len(), 4);
                assert!(pngs
                    .iter()
                    .all(|entry| entry.path().extension().is_some_and(|x| x == "png")));
                assert!(output.join("source.cog.tif").is_file());
                assert!(output.join("tilejson.json").is_file());
                assert!(!output.join("conversion.json").exists());

                let json_outputs = outputs.join("machine summary");
                std::fs::create_dir_all(&json_outputs).unwrap();
                let machine = run(&recipe, &inputs, &json_outputs, &["--json"]);
                assert!(
                    machine.status.success(),
                    "{}",
                    String::from_utf8_lossy(&machine.stderr)
                );
                assert!(!String::from_utf8_lossy(&machine.stderr).contains("next:"));
                let summary: Value = serde_json::from_slice(&machine.stdout).unwrap();
                let json_output = json_outputs.join(recipe.output);
                let published: Value = serde_json::from_slice(
                    &std::fs::read(json_output.join("report.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(summary["ok"], true);
                assert_eq!(summary["rasterReport"], published);
                assert_eq!(
                    summary["conversionReport"],
                    serde_json::json!({
                        "path": json_output.join("report.json").to_string_lossy()
                    })
                );
                assert_eq!(
                    summary["counts"],
                    serde_json::json!({
                        "tiles": 4, "sourceBands": if recipe.name == "raster-gray" { 1 } else { 3 }
                    })
                );
                assert_eq!(
                    summary["settings"],
                    serde_json::json!({
                        "minZoom": 9, "maxZoom": 11, "workers": 2
                    })
                );
            }
            // Single files without counts report their size.
            _ => assert!(first.ends_with(" MiB)"), "{first}"),
        }
        // The fixture's vertical polygon and hole are reported, not skipped.
        let reported = (recipe.command == "vector")
            .then(|| "reported: 2 geometry reports in geometry-reports.jsonl".to_string());
        let want: Vec<String> = std::iter::once(first.clone())
            .chain(reported)
            .chain([expected_next(recipe.command, &output)])
            .collect();
        assert_eq!(lines, want, "{}", recipe.name);
        covered.insert(recipe.command);
    }
    let all: std::collections::BTreeSet<_> = recipes()
        .iter()
        .filter(|r| cfg!(feature = "native-geospatial") || !r.native)
        .map(|r| r.command)
        .collect();
    assert_eq!(covered, all);
    #[cfg(feature = "native-geospatial")]
    assert_eq!(covered.len(), 8, "{covered:?}");
}

/// `--json` replaces the human lines with one machine summary on stdout.
#[test]
fn json_mode_prints_no_human_summary() {
    let work = tempfile::tempdir().unwrap();
    let inputs = work.path().join("inputs");
    write_inputs(&inputs);
    let recipe = recipes()
        .into_iter()
        .find(|r| r.name == "glb-to-3tz")
        .unwrap();
    let result = run(&recipe, &inputs, work.path(), &["--json"]);
    assert!(result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(!stderr.contains("next:"), "{stderr}");
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["ok"], true);
}

#[test]
fn sibling_manifest_json_returns_inline_report_without_claiming_a_report_file() {
    let work = tempfile::tempdir().unwrap();
    let inputs = work.path().join("inputs");
    let outputs = work.path().join("outputs");
    std::fs::create_dir(&outputs).unwrap();
    write_inputs(&inputs);
    let recipe = recipes()
        .into_iter()
        .find(|r| r.command == "createTilesetJson")
        .unwrap();
    let result = run(&recipe, &inputs, &outputs, &["--json"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let summary: Value = serde_json::from_slice(&result.stdout).unwrap();
    let output = outputs.join(recipe.output);
    assert!(output.is_file());
    assert!(summary["conversionReport"].is_null());
    assert_eq!(summary["modelReport"]["product"], "manifest");
    assert_eq!(summary["modelReport"]["triangles"], 1152);
    assert_eq!(summary["counts"]["triangles"], 1152);
    assert!(!outputs.join("conversion.json").exists());
}

#[cfg(feature = "native-geospatial")]
#[test]
fn vector_warnings_line_lists_skipped_missing_and_reported_features() {
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("bad.geojson");
    std::fs::write(
        &source,
        r#"{"type":"FeatureCollection","features":[
{"type":"Feature","id":1,"properties":{"n":1},"geometry":{"type":"Point","coordinates":[12,42,5]}},
{"type":"Feature","id":2,"properties":{"n":2},"geometry":null},
{"type":"Feature","id":3,"properties":{"n":3},"geometry":{"type":"Polygon","coordinates":[[[12,42,0],[12.001,42.001,0],[12.001,42,0],[12,42.001,0],[12,42,0]]]}}]}"#,
    )
    .unwrap();
    for (flag, counts, warnings) in [
        (
            "--skipInvalid",
            "(1 feature, 1 tile)",
            "warnings: 1 skipped feature, 1 feature without geometry, 2 geometry reports; see conversion.json and geometry-reports.jsonl",
        ),
        (
            "--repair",
            "(2 features, 1 tile)",
            "warnings: 1 feature without geometry, 2 geometry reports; see conversion.json and geometry-reports.jsonl",
        ),
    ] {
        let output = work.path().join(format!("out {flag}.3tz"));
        let result = Command::new(bin())
            .args(["vector", "-i"])
            .arg(&source)
            .arg("-o")
            .arg(&output)
            .args([flag, "--jobs", "1"])
            .env("PATH", "")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            summary(&result.stderr, "vector", &output),
            [
                format!("vector: wrote {} {counts}", output.display()),
                warnings.to_string(),
                expected_next("vector", &output),
            ],
            "{flag}"
        );
    }
}

#[test]
fn validate_rejects_directories_non_archives_and_missing_paths_with_typed_outcomes() {
    let work = tempfile::tempdir().unwrap();
    let directory = work.path().join("tiles");
    std::fs::create_dir(&directory).unwrap();
    let not_zip = work.path().join("plain.3tz");
    std::fs::write(&not_zip, b"not a zip archive").unwrap();
    let missing = work.path().join("missing.3tz");
    let unsupported = "validate currently checks .3tz archives only; raster and terrain output directories are not validated yet";
    for (path, message, exit_code, category) in [
        (
            &directory,
            format!("{} is a directory: {unsupported}", directory.display()),
            3,
            "invalid_input",
        ),
        (
            &not_zip,
            format!(
                "{} is not a ZIP/.3tz archive: {unsupported}",
                not_zip.display()
            ),
            3,
            "invalid_input",
        ),
        (
            &missing,
            format!("input not found: {}", missing.display()),
            1,
            "io",
        ),
    ] {
        let human = Command::new(bin())
            .arg("validate")
            .arg(path)
            .output()
            .unwrap();
        assert_eq!(human.status.code(), Some(exit_code), "{}", path.display());
        assert!(human.stdout.is_empty());
        let stderr = String::from_utf8_lossy(&human.stderr);
        assert!(stderr.starts_with(&message), "{stderr}");

        let json = Command::new(bin())
            .args(["--json", "validate"])
            .arg(path)
            .output()
            .unwrap();
        assert_eq!(json.status.code(), Some(exit_code), "{}", path.display());
        let error: Value = serde_json::from_slice(&json.stdout).unwrap();
        assert_eq!(error["ok"], false);
        assert_eq!(error["exitCode"], exit_code);
        assert_eq!(error["error"]["code"], category);
        assert!(
            error["error"]["message"]
                .as_str()
                .unwrap()
                .starts_with(&message),
            "{error}"
        );
    }
}
