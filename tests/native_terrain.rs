//! Public terrain consumer admission and directory publication; no Python runtime.
use rusty_tiles::{
    terrain::{terrain_to_directory, TerrainHeights, TerrainOptions, TerrainRequest},
    OutputPolicy, RunControl,
};
use serde_json::Value;
use std::{path::Path, process::Command};
fn source(root: &Path) -> std::path::PathBuf {
    let p = root.join("plane.tif");
    std::fs::write(&p, include_bytes!("fixtures/t1-plane.tif")).unwrap();
    p
}
fn request(input: &Path, output: &Path) -> TerrainRequest {
    TerrainRequest::new(
        input,
        output,
        TerrainHeights::RawMetres {
            height_offset_metres: 10.,
            fill_height_metres: -999.,
        },
        TerrainOptions::new(16),
    )
}
fn cli(input: &Path, output: &Path, extra: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["--json", "terrain", "-i"])
        .arg(input)
        .arg("-o")
        .arg(output)
        .args([
            "--cells-per-leaf",
            "16",
            "--height-offset",
            "10",
            "--fill-height",
            "-999",
        ])
        .args(extra)
        .env("PATH", "")
        .output()
        .unwrap()
}
#[test]
fn terrain_typed_capability_and_cli_parity() {
    let root = tempfile::tempdir().unwrap();
    let input = source(root.path());
    let output = root.path().join("rust");
    let result = terrain_to_directory(request(&input, &output), &RunControl::default());
    #[cfg(not(feature = "native-geospatial"))]
    {
        let e = result.unwrap_err();
        assert_eq!(e.error.kind(), rusty_tiles::JobErrorKind::Unsupported);
        assert!(!output.exists());
        let process = cli(&input, &root.path().join("cli"), &[]);
        assert!(!process.status.success());
        let json: Value = serde_json::from_slice(&process.stdout).unwrap();
        assert_eq!(json["error"]["code"], "unsupported");
    }
    #[cfg(feature = "native-geospatial")]
    {
        let result = result.unwrap();
        assert_eq!(result.report.tiles, 1);
        let report: Value =
            serde_json::from_slice(&std::fs::read(output.join("conversion.json")).unwrap())
                .unwrap();
        assert_eq!(report, serde_json::to_value(&result.report).unwrap());
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(output.join("tileset.json")).unwrap()).unwrap();
        assert_eq!(manifest["asset"]["version"], "1.1");
        assert!(manifest["root"].get("content").is_none());
        assert_eq!(manifest["root"]["children"].as_array().unwrap().len(), 1);
        assert!(!output.join("layer.json").exists());
        let process = cli(&input, &root.path().join("cli"), &[]);
        assert!(
            process.status.success(),
            "{}",
            String::from_utf8_lossy(&process.stderr)
        );
        let other: Value = serde_json::from_slice(
            &std::fs::read(root.path().join("cli/conversion.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(report, other);
    }
}
#[cfg(feature = "native-geospatial")]
#[test]
fn create_new_conflict_and_replace_preserve_until_success() {
    let root = tempfile::tempdir().unwrap();
    let input = source(root.path());
    let output = root.path().join("tiles");
    std::fs::create_dir(&output).unwrap();
    std::fs::write(output.join("sentinel"), b"old").unwrap();
    let error = terrain_to_directory(request(&input, &output), &RunControl::default()).unwrap_err();
    assert_eq!(error.error.kind(), rusty_tiles::JobErrorKind::Conflict);
    assert_eq!(std::fs::read(output.join("sentinel")).unwrap(), b"old");
    let invalid = root.path().join("bad.tif");
    std::fs::write(&invalid, b"not TIFF").unwrap();
    assert!(terrain_to_directory(
        request(&invalid, &output).with_policy(OutputPolicy::Replace),
        &RunControl::default()
    )
    .is_err());
    assert_eq!(std::fs::read(output.join("sentinel")).unwrap(), b"old");
    let result = terrain_to_directory(
        request(&input, &output).with_policy(OutputPolicy::Replace),
        &RunControl::default(),
    )
    .unwrap();
    assert_eq!(result.output, output);
    assert!(output.join("tileset.json").is_file());
    assert!(!output.join("sentinel").exists());
}
#[cfg(feature = "native-geospatial")]
#[test]
fn reject_source_and_containing_directory_aliases() {
    let root = tempfile::tempdir().unwrap();
    let input = source(root.path());
    let before = std::fs::read(&input).unwrap();
    let alias = root.path().join("hardlink.tif");
    std::fs::hard_link(&input, &alias).unwrap();
    for output in [&input, &alias, root.path()] {
        let error = terrain_to_directory(
            request(&input, output).with_policy(OutputPolicy::Replace),
            &RunControl::default(),
        )
        .unwrap_err();
        assert_eq!(
            error.error.kind(),
            rusty_tiles::JobErrorKind::InvalidRequest
        );
        assert_eq!(std::fs::read(&input).unwrap(), before);
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 2);
}
#[cfg(all(unix, feature = "native-geospatial"))]
#[test]
fn replacing_final_symlink_preserves_referent() {
    let root = tempfile::tempdir().unwrap();
    let input = source(root.path());
    let before = std::fs::read(&input).unwrap();
    let output = root.path().join("link");
    std::os::unix::fs::symlink(&input, &output).unwrap();
    terrain_to_directory(
        request(&input, &output).with_policy(OutputPolicy::Replace),
        &RunControl::default(),
    )
    .unwrap();
    assert!(output.is_dir());
    assert_eq!(std::fs::read(input).unwrap(), before);
}
#[test]
fn malformed_request_is_shared_domain_rejection() {
    let root = tempfile::tempdir().unwrap();
    let input = source(root.path());
    let output = root.path().join("tiles");
    let request = TerrainRequest::new(
        &input,
        &output,
        TerrainHeights::RawMetres {
            height_offset_metres: f64::NAN,
            fill_height_metres: 0.,
        },
        TerrainOptions::new(16),
    );
    let error = terrain_to_directory(request, &RunControl::default()).unwrap_err();
    assert_eq!(
        error.error.kind(),
        rusty_tiles::JobErrorKind::InvalidRequest
    );
    assert!(!output.exists());
}

#[cfg(feature = "native-geospatial")]
#[test]
fn multipatch_geometry_and_height_domain_refusal() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("multipatch.tif");
    std::fs::write(&input, include_bytes!("fixtures/t1-multipatch-plane.tif")).unwrap();
    let output = root.path().join("tiles");
    let result = terrain_to_directory(request(&input, &output), &RunControl::default()).unwrap();
    assert_eq!(result.report.tiles, 6);
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(output.join("tileset.json")).unwrap()).unwrap();
    assert_eq!(manifest["root"]["children"].as_array().unwrap().len(), 6);
    let extreme = root.path().join("extreme.tif");
    std::fs::write(&extreme, include_bytes!("fixtures/t1-extreme-height.tif")).unwrap();
    let error = terrain_to_directory(
        request(&extreme, &output).with_policy(OutputPolicy::Replace),
        &RunControl::default(),
    )
    .unwrap_err();
    assert_eq!(error.error.kind(), rusty_tiles::JobErrorKind::Unsupported);
    let after: Value =
        serde_json::from_slice(&std::fs::read(output.join("tileset.json")).unwrap()).unwrap();
    assert_eq!(manifest, after);
}

#[cfg(feature = "native-geospatial")]
#[test]
fn tiled_lossless_sources_and_oversized_decoded_block_admission() {
    let root = tempfile::tempdir().unwrap();
    for (name, bytes) in [
        (
            "deflate",
            include_bytes!("fixtures/t1-plane-deflate.tif").as_slice(),
        ),
        (
            "lzw",
            include_bytes!("fixtures/t1-plane-lzw.tif").as_slice(),
        ),
    ] {
        let input = root.path().join(format!("{name}.tif"));
        std::fs::write(&input, bytes).unwrap();
        let output = root.path().join(name);
        let result =
            terrain_to_directory(request(&input, &output), &RunControl::default()).unwrap();
        assert_eq!(result.report.tiles, 1);
    }
    let input = root.path().join("large-block.tif");
    std::fs::write(&input, include_bytes!("fixtures/t1-oversized-block.tif")).unwrap();
    let output = root.path().join("must-not-stage");
    let error = terrain_to_directory(request(&input, &output), &RunControl::default()).unwrap_err();
    assert_eq!(error.error.kind(), rusty_tiles::JobErrorKind::Unsupported);
    assert!(error.error.to_string().contains("decoded-block"));
    assert!(!output.exists());
}
