//! Archive validation through the CLI, using invented Rust-generated inputs.
#![cfg(feature = "native-geospatial")]
mod support;

use serde_json::{json, Value};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

fn fixture(root: &Path) -> PathBuf {
    let source = root.join("source.geojson");
    let features: Vec<_> = (0..4)
        .map(|i| {
            json!({
                "type":"Feature", "id":i, "properties":{"name":format!("line-{i}")},
                "geometry":{"type":"LineString", "coordinates":(0..30).map(|j|
                    json!([i*100+j, f64::from(j%3)*0.01, f64::from(i)*0.0001])
                ).collect::<Vec<_>>()}}
            )
        })
        .collect();
    fs::write(
        &source,
        json!({"type":"FeatureCollection","features":features}).to_string(),
    )
    .unwrap();
    let out = root.join("valid.3tz");
    let result = support::rusty_tiles()
        .args(["vector", "--explicit", "-i"])
        .arg(&source)
        .arg("-o")
        .arg(&out)
        .args([
            "--sourceCrs",
            "local",
            "--maxFeatures",
            "1",
            "--jobs",
            "2",
            "--quantize",
            "--meshopt",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    out
}

fn validate(path: &Path, extra: &[&std::ffi::OsStr]) -> (std::process::Output, Value) {
    let result = support::rusty_tiles()
        .arg("validate")
        .arg(path)
        .arg("--json")
        .args(extra)
        .output()
        .unwrap();
    let report = serde_json::from_slice(&result.stdout).unwrap();
    (result, report)
}

fn change_json(directory: &Path, name: &str, mutate: impl FnOnce(&mut Value)) {
    let path = directory.join(name);
    let mut doc = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    mutate(&mut doc);
    fs::write(path, serde_json::to_vec(&doc).unwrap()).unwrap();
}

fn payload(directory: &Path) -> PathBuf {
    fs::read_dir(directory.join("t"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "glb"))
        .unwrap()
}

#[test]
fn valid_archive_and_corruption_categories() {
    let root = tempfile::tempdir().unwrap();
    let original = fixture(root.path());
    let before = fs::read(&original).unwrap();
    let (result, report) = validate(&original, &[]);
    assert!(result.status.success(), "{report}");
    assert_eq!(report["ok"], true);
    assert!(report["tiles"].as_u64().unwrap() > 4);
    assert_eq!(fs::read(&original).unwrap(), before);
    for (name, message) in [
        ("hash", "checksum"),
        ("missing", "missing archive"),
        ("unused", "unreferenced"),
        ("bytes", "byte budget"),
        ("vertices", "vertex budget"),
        ("error", "geometricError"),
        ("bounds", "bounds escape"),
        ("schema", "asset.version"),
        ("state", "build-state checksum"),
    ] {
        let directory = root.path().join(name);
        fs::create_dir(&directory).unwrap();
        let mut archive = zip::ZipArchive::new(fs::File::open(&original).unwrap()).unwrap();
        archive.extract(&directory).unwrap();
        fs::remove_file(directory.join("@3dtilesIndex1@")).unwrap();
        match name {
            "hash" => {
                let p = payload(&directory);
                let mut bytes = fs::read(&p).unwrap();
                *bytes.last_mut().unwrap() ^= 1;
                fs::write(p, bytes).unwrap();
            }
            "missing" => fs::remove_file(payload(&directory)).unwrap(),
            "unused" => fs::write(directory.join("unrelated.txt"), "unreferenced").unwrap(),
            "bytes" | "vertices" => change_json(&directory, "conversion.json", |v| {
                v["budgets"][name] = json!(1)
            }),
            "error" => change_json(&directory, "tileset.json", |v| {
                v["root"]["geometricError"] = json!(v["geometricError"].as_f64().unwrap() * 2.0)
            }),
            "bounds" => change_json(&directory, "tileset.json", |v| {
                v["root"]["boundingVolume"]["box"] =
                    json!([0, 0, 0, 0.001, 0, 0, 0, 0.001, 0, 0, 0, 0.001])
            }),
            "schema" => change_json(&directory, "tileset.json", |v| {
                v["asset"]["version"] = json!("2.0")
            }),
            "state" => fs::write(directory.join("vector-build.json"), "{}").unwrap(),
            _ => unreachable!(),
        }
        let out = root.path().join(format!("{name}.3tz"));
        let packed = support::rusty_tiles()
            .args(["convert", "-i"])
            .arg(&directory)
            .arg("-o")
            .arg(&out)
            .output()
            .unwrap();
        assert!(
            packed.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&packed.stderr)
        );
        let (result, report) = validate(&out, &[]);
        assert!(!result.status.success(), "{name}: {report}");
        assert!(
            report["error"]["message"]
                .as_str()
                .unwrap()
                .contains(message),
            "{name}: {report}"
        );
    }
}

#[test]
fn point_archive_and_feature_attributes_use_valid_core_types() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("thin.las");
    let mut builder = las::Builder::from((1, 2));
    builder.point_format = las::point::Format::new(3).unwrap();
    builder.transforms = las::Vector {
        x: las::Transform {
            scale: 1e-7,
            offset: 0.0,
        },
        y: las::Transform {
            scale: 1e-7,
            offset: 0.0,
        },
        z: las::Transform {
            scale: 1e-7,
            offset: 0.0,
        },
    };
    let mut writer = las::Writer::from_path(&source, builder.into_header().unwrap()).unwrap();
    for i in 0..60 {
        writer
            .write_point(las::Point {
                x: f64::from(i) * 10.0 / 59.0,
                y: f64::from(i) * 1e-6 / 59.0,
                gps_time: Some(0.0),
                color: Some(las::Color::default()),
                ..Default::default()
            })
            .unwrap();
    }
    writer.close().unwrap();
    let out = root.path().join("points.3tz");
    let result = support::rusty_tiles()
        .args(["point-cloud", "-i"])
        .arg(&source)
        .arg("-o")
        .arg(&out)
        .args(["--sourceCrs", "local", "--maxPoints", "10"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let (result, report) = validate(&out, &[]);
    assert!(result.status.success(), "{report}");
    for path in [out, fixture(root.path())] {
        let mut archive = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).unwrap();
            if !entry.name().ends_with(".glb") {
                continue;
            }
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
            let doc: Value = serde_json::from_slice(&bytes[20..20 + length]).unwrap();
            let id = doc["extensions"]["EXT_structural_metadata"]["schema"]["id"]
                .as_str()
                .unwrap();
            assert!(
                id.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_'),
                "{id}"
            );
            assert!(
                id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "{id}"
            );
            for mesh in doc["meshes"].as_array().unwrap() {
                for primitive in mesh["primitives"].as_array().unwrap() {
                    let index = primitive["attributes"]["_FEATURE_ID_0"].as_u64().unwrap() as usize;
                    assert!([json!(5123), json!(5126)]
                        .contains(&doc["accessors"][index]["componentType"]));
                }
            }
        }
    }
}

#[test]
fn index_and_missing_external_validator_failures() {
    let root = tempfile::tempdir().unwrap();
    let original = fixture(root.path());
    let bad = root.path().join("bad-index.3tz");
    let mut source = zip::ZipArchive::new(fs::File::open(&original).unwrap()).unwrap();
    let mut target = zip::ZipWriter::new(fs::File::create(&bad).unwrap());
    for i in 0..source.len() {
        let mut entry = source.by_index(i).unwrap();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        if entry.name() == "@3dtilesIndex1@" {
            bytes.pop();
        }
        target
            .start_file(
                entry.name(),
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        std::io::Write::write_all(&mut target, &bytes).unwrap();
    }
    target.finish().unwrap();
    let (result, report) = validate(&bad, &[]);
    assert!(!result.status.success());
    assert!(
        report["error"]["message"]
            .as_str()
            .unwrap()
            .contains("index"),
        "{report}"
    );
    let missing = root.path().join("absent");
    let (result, report) = validate(
        &original,
        &["--external-validator".as_ref(), missing.as_os_str()],
    );
    assert_eq!(result.status.code(), Some(4));
    assert_eq!(report["error"]["code"], "environment");
}

#[cfg(unix)]
#[test]
fn external_validator_reported_errors_fail_even_with_successful_exit() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let original = fixture(root.path());
    let executable = root.path().join("validator");
    fs::write(&executable, "#!/bin/sh\n[ \"$1\" = --tilesetFile ] && [ \"$3\" = --reportFile ] || exit 9\nprintf '%s' '{\"issues\":[{\"severity\":\"ERROR\",\"message\":\"fixture\"}]}' > \"$4\"\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let (result, report) = validate(
        &original,
        &["--external-validator".as_ref(), executable.as_os_str()],
    );
    assert!(!result.status.success());
    assert!(
        report["error"]["message"]
            .as_str()
            .unwrap()
            .contains("external validation"),
        "{report}"
    );
}
