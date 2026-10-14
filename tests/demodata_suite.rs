//! Fast harness checks; the openly licensed corpus is explicitly opt-in.
#![cfg(feature = "native-geospatial")]
#[path = "../bench/demodata/runner.rs"]
mod runner;
mod support;
use std::{collections::BTreeSet, fs, path::PathBuf};

fn tiny_manifest(root: &std::path::Path) -> runner::Manifest {
    fs::write(root.join("sample.glb"), b"pinned fixture").unwrap();
    let mut manifest = runner::Manifest::load().unwrap();
    manifest.cases.truncate(1);
    manifest.cases[0].input = "sample.glb".into();
    let attribution = manifest.files.values().next().unwrap().source.clone();
    manifest.files.clear();
    manifest.files.insert(
        "sample.glb".into(),
        runner::Input {
            bytes: 14,
            sha256: runner::hash_file(&root.join("sample.glb")).unwrap(),
            source: attribution,
        },
    );
    manifest
}

#[test]
fn manifest_pins_all_recipes_and_selects_profiles_without_silent_skips() {
    let manifest = runner::Manifest::load().unwrap();
    assert_eq!(manifest.files.len(), 416);
    assert_eq!(manifest.cases.len(), 28);
    assert_eq!(manifest.select("smoke", &[]).unwrap().len(), 8);
    assert_eq!(manifest.select("core", &[]).unwrap().len(), 21);
    assert_eq!(manifest.select("scale", &[]).unwrap().len(), 7);
    assert!(manifest.select("all", &["typo".into()]).is_err());
    assert!(manifest.select("typo", &[]).is_err());
}

#[test]
fn changed_missing_and_escaping_inputs_fail_verification() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path().join("corpus");
    fs::create_dir(&root).unwrap();
    let manifest = tiny_manifest(&root);
    let paths = BTreeSet::from(["sample.glb"]);
    assert_eq!(runner::verify_all(&manifest, &root).unwrap(), 1);
    fs::write(root.join("sample.glb"), b"changed fixture").unwrap();
    assert!(runner::verify_inputs(&manifest, &root, &paths)
        .unwrap_err()
        .to_string()
        .contains("changed input"));
    fs::remove_file(root.join("sample.glb")).unwrap();
    assert!(runner::verify_inputs(&manifest, &root, &paths)
        .unwrap_err()
        .to_string()
        .contains("missing input"));
    #[cfg(unix)]
    {
        let outside = work.path().join("outside.glb");
        fs::write(&outside, b"pinned fixture").unwrap();
        std::os::unix::fs::symlink(outside, root.join("sample.glb")).unwrap();
        assert!(runner::verify_inputs(&manifest, &root, &paths)
            .unwrap_err()
            .to_string()
            .contains("escapes corpus"));
    }
}

#[test]
fn malformed_paths_and_recipe_output_overrides_are_rejected() {
    let work = tempfile::tempdir().unwrap();
    for path in ["../sample.glb", "/sample.glb", "C:\\sample.glb", ""] {
        let mut manifest = tiny_manifest(work.path());
        manifest.cases[0].input = path.into();
        assert!(manifest.validate().is_err());
    }
    let mut manifest = tiny_manifest(work.path());
    manifest.cases[0]
        .args
        .extend(["--output".into(), "somewhere".into()]);
    assert!(manifest.validate().is_err());
}

#[test]
fn output_overlap_is_rejected_before_creating_any_directories() {
    let work = tempfile::tempdir().unwrap();
    let manifest = tiny_manifest(work.path());
    let output = work.path().join("new/subdir/results");
    let error = runner::run(
        &manifest,
        runner::Options {
            data_root: work.path().into(),
            binary: PathBuf::from(env!("CARGO_BIN_EXE_rusty-tiles")),
            baseline: None,
            output,
            profile: "smoke".into(),
            cases: vec![],
            benchmark: false,
            repeats: 1,
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("overlaps the corpus"));
    assert!(!work.path().join("new").exists());
}

#[test]
fn measurement_records_success_and_failure_with_empty_executable_path() {
    let work = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_rusty-tiles").to_owned();
    let good = runner::measure_process(
        &[binary.clone(), "--version".into()],
        &work.path().join("good.log"),
    )
    .unwrap();
    assert_eq!(good.exit_code, Some(0));
    assert!(good.wall_seconds > 0.);
    #[cfg(unix)]
    {
        assert!(good.cpu_seconds.unwrap() >= 0.);
        assert!(good.peak_rss_mib.unwrap() > 0.);
    }
    let bad = runner::measure_process(
        &[binary, "--unrecognized-option".into()],
        &work.path().join("bad.log"),
    )
    .unwrap();
    assert_eq!(bad.exit_code, Some(2));
}

#[test]
fn mesh_audit_rejects_changed_winding_even_when_archive_validation_passes() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("source.glb");
    let output = work.path().join("mesh.3tz");
    let write = |indices| {
        fs::write(
            &input,
            rusty_tiles::glb_write::write_glb(&[rusty_tiles::glb_write::TilePrimitive {
                positions: vec![[0., 0., 0.], [10., 0., 0.], [0., 10., 0.]],
                normals: vec![[0., 0., 1.]; 3],
                uvs: vec![[0., 0.]; 3],
                indices,
                jpeg: None,
            }])
            .unwrap(),
        )
        .unwrap();
    };
    write(vec![0, 1, 2]);
    let result = support::rusty_tiles()
        .args(["mesh-to-3tz", "-i"])
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    runner::audit::check("mesh", &input, &output).unwrap();
    write(vec![0, 2, 1]);
    assert!(
        rusty_tiles::validate::inspect(rusty_tiles::validate::ValidationRequest::new(&output))
            .is_ok()
    );
    assert!(runner::audit::check("mesh", &input, &output)
        .unwrap_err()
        .to_string()
        .contains("changed triangle or winding"));
}

#[test]
fn point_audit_rejects_changed_attributes_even_when_archive_validation_passes() {
    use std::io::{Seek, SeekFrom, Write};
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("source.las");
    let output = work.path().join("points.3tz");
    let mut writer = las::Writer::from_path(&input, las::Header::default()).unwrap();
    for i in 0..12 {
        writer
            .write_point(las::Point {
                x: i as f64,
                y: (i % 3) as f64,
                z: (i % 2) as f64,
                intensity: i as u16,
                return_number: 1,
                number_of_returns: 1,
                ..Default::default()
            })
            .unwrap();
    }
    writer.close().unwrap();
    drop(writer);
    let result = support::rusty_tiles()
        .args(["point-cloud", "-i"])
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .args(["--sourceCrs", "local", "--maxPoints", "3"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    runner::audit::check("points", &input, &output).unwrap();
    let header = las::Reader::from_path(&input)
        .unwrap()
        .header()
        .clone()
        .into_raw()
        .unwrap();
    let mut file = fs::OpenOptions::new().write(true).open(&input).unwrap();
    file.seek(SeekFrom::Start(u64::from(header.offset_to_point_data) + 12))
        .unwrap();
    file.write_all(&999u16.to_le_bytes()).unwrap();
    drop(file);
    assert!(
        rusty_tiles::validate::inspect(rusty_tiles::validate::ValidationRequest::new(&output))
            .is_ok()
    );
    assert!(runner::audit::check("points", &input, &output)
        .unwrap_err()
        .to_string()
        .contains("column intensity"));
}

#[test]
#[ignore = "requires the pinned open demodata corpus and RUSTY_TILES_DEMODATA"]
fn demodata_smoke() {
    let root = std::env::var_os("RUSTY_TILES_DEMODATA")
        .expect("set RUSTY_TILES_DEMODATA to the existing open corpus");
    let work = tempfile::tempdir().unwrap();
    runner::run(
        &runner::Manifest::load().unwrap(),
        runner::Options {
            data_root: root.into(),
            binary: env!("CARGO_BIN_EXE_rusty-tiles").into(),
            baseline: None,
            output: work.path().join("results"),
            profile: "smoke".into(),
            cases: vec![],
            benchmark: false,
            repeats: 1,
        },
    )
    .unwrap();
}
