//! Public terrain consumer admission and directory publication; no Python runtime.
#[cfg(feature = "native-geospatial")]
use rusty_tiles::OutputPolicy;
use rusty_tiles::{
    terrain::{terrain_to_directory, TerrainHeights, TerrainOptions, TerrainRequest},
    RunControl,
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
        assert_eq!((result.report.tiles, result.report.levels), (1, 1));
        let report: Value =
            serde_json::from_slice(&std::fs::read(output.join("conversion.json")).unwrap())
                .unwrap();
        assert_eq!(report, serde_json::to_value(&result.report).unwrap());
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(output.join("tileset.json")).unwrap()).unwrap();
        assert_eq!(manifest["asset"]["version"], "1.1");
        // A single leaf is the root: full detail, error zero, one implicit level.
        assert_eq!(manifest["root"]["geometricError"], 0.);
        assert_eq!(manifest["root"]["implicitTiling"]["availableLevels"], 1);
        assert!(manifest["root"].get("children").is_none());
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
    assert_eq!(result.report.tiles, 9);
    assert_eq!(result.report.tiles_per_level, [1, 2, 6]);
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(output.join("tileset.json")).unwrap()).unwrap();
    assert_eq!(manifest["root"]["implicitTiling"]["subtreeLevels"], 3);
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

#[cfg(feature = "native-geospatial")]
#[test]
fn concurrent_jobs_and_repeated_publication_keep_independent_state() {
    use rusty_tiles::{JobError, JobErrorKind, Observer, RunEvent};
    use std::{
        collections::BTreeMap,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Barrier,
        },
    };

    struct SourceGate {
        barrier: Arc<Barrier>,
        reached: AtomicBool,
    }
    impl Observer for SourceGate {
        fn observe(&self, event: &RunEvent<'_>) -> Result<(), JobError> {
            if matches!(
                event,
                RunEvent::Progress {
                    phase: "terrain_source",
                    ..
                }
            ) && !self.reached.swap(true, Ordering::SeqCst)
            {
                // Both independently owned GDAL datasets are live at this point.
                self.barrier.wait();
            }
            Ok(())
        }
    }
    fn inventory(root: &Path) -> BTreeMap<String, Vec<u8>> {
        fn visit(root: &Path, path: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
            for entry in std::fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let kind = entry.file_type().unwrap();
                if kind.is_dir() {
                    visit(root, &path, files);
                } else {
                    assert!(kind.is_file());
                    let name = path
                        .strip_prefix(root)
                        .unwrap()
                        .components()
                        .map(|part| part.as_os_str().to_str().unwrap())
                        .collect::<Vec<_>>()
                        .join("/");
                    assert!(files.insert(name, std::fs::read(path).unwrap()).is_none());
                }
            }
        }
        let mut files = BTreeMap::new();
        visit(root, root, &mut files);
        files
    }
    fn verify(result: &rusty_tiles::terrain::TerrainResult) -> BTreeMap<String, Vec<u8>> {
        let files = inventory(&result.output);
        let manifest: Value = serde_json::from_slice(&files["tileset.json"]).unwrap();
        let report: Value = serde_json::from_slice(&files["conversion.json"]).unwrap();
        assert_eq!(report, serde_json::to_value(&result.report).unwrap());
        // Every tile has content; a forest of at most four levels is one subtree.
        let tiling = &manifest["root"]["implicitTiling"];
        assert_eq!(
            tiling["subtrees"]["uri"],
            "subtrees/r0-{level}-{x}-{y}.subtree"
        );
        let mut expected = vec![
            "conversion.json".to_owned(),
            "subtrees/r0-0-0-0.subtree".to_owned(),
            "tileset.json".to_owned(),
        ];
        assert_eq!(
            manifest["root"]["content"]["uri"],
            "implicit-content/r0-{level}-{x}-{y}-0.glb"
        );
        let payloads = files
            .keys()
            .filter(|name| name.starts_with("implicit-content/r0-") && name.ends_with("-0.glb"))
            .count() as u64;
        assert_eq!(payloads, result.report.tiles);
        expected.extend(
            files
                .keys()
                .filter(|name| name.starts_with("implicit-content/"))
                .cloned(),
        );
        expected.sort();
        assert_eq!(files.keys().cloned().collect::<Vec<_>>(), expected);
        assert_eq!(
            files.values().map(|bytes| bytes.len() as u64).sum::<u64>(),
            result.report.generated_bytes
        );
        assert!(result.cleanup_diagnostics.is_empty());
        files
    }
    fn configured(input: &Path, output: &Path, offset: f64, cells: u16) -> TerrainRequest {
        TerrainRequest::new(
            input,
            output,
            TerrainHeights::RawMetres {
                height_offset_metres: offset,
                fill_height_metres: -999.,
            },
            TerrainOptions::new(cells),
        )
    }

    let root = tempfile::tempdir().unwrap();
    let input_a = root.path().join("a.tif");
    let input_b = root.path().join("b.tif");
    let bytes_a = include_bytes!("fixtures/t1-multipatch-plane.tif");
    let bytes_b = include_bytes!("fixtures/t1-plane.tif");
    std::fs::write(&input_a, bytes_a).unwrap();
    std::fs::write(&input_b, bytes_b).unwrap();
    let output_a = root.path().join("a-output");
    let output_b = root.path().join("b-output");
    let barrier = Arc::new(Barrier::new(2));
    let (a, b) = std::thread::scope(|scope| {
        let spawn = |input: &Path, output: &Path, offset, cells| {
            let request = configured(input, output, offset, cells);
            let observer = Arc::new(SourceGate {
                barrier: barrier.clone(),
                reached: AtomicBool::new(false),
            });
            scope.spawn(move || {
                let control = RunControl::new(Some(observer.clone()));
                let result = terrain_to_directory(request, &control).unwrap();
                assert!(observer.reached.load(Ordering::SeqCst));
                (result, control)
            })
        };
        let a = spawn(&input_a, &output_a, 10., 16);
        let b = spawn(&input_b, &output_b, 100., 32);
        (a.join().unwrap(), b.join().unwrap())
    });
    assert_eq!(
        (
            a.0.report.tiles,
            a.0.report.cells_per_leaf,
            a.0.report.height_offset_metres
        ),
        (9, 16, 10.)
    );
    assert_eq!(
        (
            b.0.report.tiles,
            b.0.report.cells_per_leaf,
            b.0.report.height_offset_metres
        ),
        (1, 32, 100.)
    );
    assert_ne!(
        (a.0.report.width, a.0.report.height),
        (b.0.report.width, b.0.report.height)
    );
    let first_a = verify(&a.0);
    let first_b = verify(&b.0);
    let top = "implicit-content/r0-0-0-0-0.glb";
    assert_ne!(first_a[top], first_b[top]);

    // A used control cannot claim another job, and fresh CreateNew cannot alter publication.
    let error = terrain_to_directory(configured(&input_a, &output_a, 250., 64), &a.1).unwrap_err();
    assert_eq!(error.error.kind(), JobErrorKind::InvalidState);
    let error = terrain_to_directory(
        configured(&input_a, &output_a, 250., 64),
        &RunControl::default(),
    )
    .unwrap_err();
    assert_eq!(error.error.kind(), JobErrorKind::Conflict);
    assert_eq!(inventory(&output_a), first_a);
    assert_eq!(inventory(&output_b), first_b);

    let replaced = terrain_to_directory(
        configured(&input_a, &output_a, 250., 64).with_policy(OutputPolicy::Replace),
        &RunControl::default(),
    )
    .unwrap();
    assert_eq!(
        (
            replaced.report.tiles,
            replaced.report.cells_per_leaf,
            replaced.report.height_offset_metres
        ),
        (1, 64, 250.)
    );
    let replacement = verify(&replaced);
    assert_ne!(replacement, first_a);
    assert_eq!(inventory(&output_b), first_b);
    assert_eq!(std::fs::read(input_a).unwrap(), bytes_a.as_slice());
    assert_eq!(std::fs::read(input_b).unwrap(), bytes_b.as_slice());
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 4);
}

#[cfg(feature = "native-geospatial")]
fn validate(output: &Path) -> Value {
    let archive = output.with_extension("3tz");
    let packed = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["--json", "convert", "-i"])
        .arg(output)
        .arg("-o")
        .arg(&archive)
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(
        packed.status.success(),
        "{}",
        String::from_utf8_lossy(&packed.stdout)
    );
    let checked = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["--json", "validate"])
        .arg(&archive)
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stdout)
    );
    serde_json::from_slice(&checked.stdout).unwrap()
}

#[cfg(feature = "native-geospatial")]
#[test]
fn packed_lod_forest_passes_validation_and_keeps_flat_leaf_surfaces() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("multipatch.tif");
    std::fs::write(&input, include_bytes!("fixtures/t1-multipatch-plane.tif")).unwrap();
    let output = root.path().join("tiles");
    let process = cli(&input, &output, &[]);
    assert!(
        process.status.success(),
        "{}",
        String::from_utf8_lossy(&process.stderr)
    );
    let report = validate(&output);
    let checks = report["checks"].as_array().unwrap();
    for check in [
        "implicitAddressingAndAvailability",
        "hierarchyBounds",
        "geometricErrorOrder",
    ] {
        assert!(checks.contains(&check.into()), "{report}");
    }
    // Position and index bytes of each leaf surface primitive are those of the
    // flat T1 leaves (develop e29cd19, tiles/{row}/{column}.glb, same request).
    for (row, column, sha256) in [
        (
            0,
            0,
            "1c92900b64e48e216e611039136a88d1d12bc1889d39a662447fbdf0aeed3998",
        ),
        (
            0,
            1,
            "e02e903671861fb5cd5f9da67f080bde1aca11f9657234fcc94a556b135f133b",
        ),
        (
            0,
            2,
            "da22b51e99317ea9e9f43a34ae9833f9b2dc83263b6243ccc8a6300e33e635d4",
        ),
        (
            1,
            0,
            "790cd41fc394d0926af4e2c0c44d5d0ff70d32672aca2bf06962cdafb335e53c",
        ),
        (
            1,
            1,
            "1658eea9cb5683f27f78359139532dc88c86273e0bf21b6674b9da11bade4016",
        ),
        (
            1,
            2,
            "3395eb2bb4cf3626d86544b24a85e26a9758fca92167030bacb41ab8cd0caa9c",
        ),
    ] {
        let glb = std::fs::read(output.join(format!("implicit-content/r0-2-{column}-{row}-0.glb")))
            .unwrap();
        let length = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let document: Value = serde_json::from_slice(&glb[20..20 + length]).unwrap();
        let binary = &glb[28 + length..];
        let primitive = &document["meshes"][0]["primitives"][0];
        assert_eq!(
            (
                primitive["attributes"]["POSITION"].as_u64(),
                primitive["indices"].as_u64()
            ),
            (Some(0), Some(1))
        );
        let mut digest = Sha256::new();
        for view in &document["bufferViews"].as_array().unwrap()[..2] {
            let start = view["byteOffset"].as_u64().unwrap() as usize;
            digest.update(&binary[start..start + view["byteLength"].as_u64().unwrap() as usize]);
        }
        assert_eq!(
            format!("{:x}", digest.finalize()),
            sha256,
            "leaf {row}/{column}"
        );
    }
}
