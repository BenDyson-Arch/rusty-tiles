//! Actual CLI acceptance without Python, plus independently decoded GLB/3TZ data.
use serde_json::Value;
use std::{io::Read, path::Path, process::Command};

fn call(input: &Path, output: &Path, options: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["--json", "point-cloud", "-i"])
        .arg(input)
        .arg("-o")
        .arg(output)
        .args(options)
        .env("PATH", "")
        .output()
        .unwrap()
}

fn fixture(path: &Path, count: usize, identical: bool, vlrs: Vec<las::Vlr>) {
    let mut builder = las::Builder::from((1, 4));
    builder.point_format = las::point::Format::new(8).unwrap();
    builder.vlrs = vlrs;
    let mut writer = las::Writer::from_path(path, builder.into_header().unwrap()).unwrap();
    for i in 0..count {
        writer
            .write_point(las::Point {
                x: if identical {
                    0.
                } else {
                    (i % 17) as f64 * 0.713
                },
                y: if identical {
                    0.
                } else {
                    (i / 17) as f64 * 0.627
                },
                z: if identical { 0. } else { (i as f64).sin() },
                intensity: (i * 13) as u16,
                return_number: 1,
                number_of_returns: 3,
                classification: las::point::Classification::new(42).unwrap(),
                is_synthetic: i % 2 == 0,
                is_key_point: i % 3 == 0,
                is_withheld: i % 5 == 0,
                is_overlap: i % 7 == 0,
                scanner_channel: (i % 4) as u8,
                gps_time: Some(i as f64 / 7.),
                color: Some(las::Color {
                    red: (i * 233) as u16,
                    green: (i * 431) as u16,
                    blue: (i * 717) as u16,
                }),
                nir: Some((i * 103) as u16),
                ..Default::default()
            })
            .unwrap();
    }
    writer.close().unwrap();
}

struct Glb {
    doc: Value,
    binary: Vec<u8>,
}

impl Glb {
    fn read(zip: &mut zip::ZipArchive<std::fs::File>, uri: &str) -> Self {
        let mut bytes = Vec::new();
        zip.by_name(uri).unwrap().read_to_end(&mut bytes).unwrap();
        assert_eq!(&bytes[..4], b"glTF");
        assert_eq!(
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize,
            bytes.len()
        );
        let size = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        Self {
            doc: serde_json::from_slice(&bytes[20..20 + size]).unwrap(),
            binary: bytes[28 + size..].to_vec(),
        }
    }

    fn view(&self, index: usize) -> &[u8] {
        let view = &self.doc["bufferViews"][index];
        let at = view["byteOffset"].as_u64().unwrap() as usize;
        assert_eq!(at % 8, 0);
        &self.binary[at..at + view["byteLength"].as_u64().unwrap() as usize]
    }

    fn column(&self, name: &str) -> &[u8] {
        self.view(
            self.doc["extensions"]["EXT_structural_metadata"]["propertyTables"][0]["properties"]
                [name]["values"]
                .as_u64()
                .unwrap() as usize,
        )
    }
}

fn document(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Value {
    serde_json::from_reader(zip.by_name(name).unwrap()).unwrap()
}

fn audit_leaves(
    zip: &mut zip::ZipArchive<std::fs::File>,
    node: &Value,
    seen: &mut Vec<u64>,
    budget: usize,
) {
    let glb = Glb::read(zip, node["content"]["uri"].as_str().unwrap());
    let table = &glb.doc["extensions"]["EXT_structural_metadata"]["propertyTables"][0];
    let count = table["count"].as_u64().unwrap() as usize;
    assert!(count <= budget);
    let ids: Vec<_> = glb
        .column("source_index")
        .as_chunks::<8>()
        .0
        .iter()
        .map(|b| u64::from_le_bytes(*b))
        .collect();
    assert_eq!(ids.len(), count);
    for (index, id) in ids.iter().enumerate() {
        let intensity = u16::from_le_bytes(
            glb.column("intensity")[index * 2..index * 2 + 2]
                .try_into()
                .unwrap(),
        );
        assert_eq!(intensity, (*id * 13) as u16);
        assert_eq!(glb.column("classification")[index], 42);
        assert_eq!(glb.column("overlap")[index], u8::from(*id % 7 == 0));
        assert_eq!(glb.column("scanner_channel")[index], (*id % 4) as u8);
        let gps = f64::from_le_bytes(
            glb.column("gps_time")[index * 8..index * 8 + 8]
                .try_into()
                .unwrap(),
        );
        assert_eq!(gps, *id as f64 / 7.);
    }
    if let Some(children) = node["children"].as_array() {
        for child in children {
            assert!(
                child["geometricError"].as_f64().unwrap()
                    <= node["geometricError"].as_f64().unwrap()
            );
            audit_leaves(zip, child, seen, budget);
        }
    } else {
        assert_eq!(node["geometricError"].as_f64().unwrap(), 0.);
        seen.extend(ids);
    }
}

#[test]
fn las_and_laz_stream_without_python_keep_leaf_records_and_reproduce_archives() {
    for suffix in ["las", "laz"] {
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join(format!("cloud.{suffix}"));
        fixture(&input, 257, false, vec![]);
        let out = work.path().join("cloud.3tz");
        let options = [
            "--sourceCrs",
            "local",
            "--maxPoints",
            "16",
            "--chunkPoints",
            "11",
            "--progress",
            "json",
        ];
        let result = call(&input, &out, &options);
        assert!(
            result.status.success(),
            "{} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        let summary: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(summary["ok"], true);
        assert_eq!(summary["counts"]["points"], 257);
        let events: Vec<Value> = String::from_utf8(result.stderr)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(events.first().unwrap()["done"], 0);
        assert_eq!(events.last().unwrap()["done"], 1);
        for phase in ["ingestion", "tiling"] {
            let rows: Vec<_> = events.iter().filter(|e| e["phase"] == phase).collect();
            assert_eq!(rows.last().unwrap()["done"], 257);
            assert!(rows
                .windows(2)
                .all(|p| p[0]["done"].as_u64() <= p[1]["done"].as_u64()));
        }
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
        let manifest = document(&mut zip, "tileset.json");
        let mut seen = Vec::new();
        audit_leaves(&mut zip, &manifest["root"], &mut seen, 16);
        seen.sort_unstable();
        assert_eq!(seen, (0..257).collect::<Vec<_>>());
        assert!(zip.file_names().all(|n| !n.contains("scratch")));
        let validate = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
            .arg("validate")
            .arg(&out)
            .arg("--json")
            .env("PATH", "")
            .output()
            .unwrap();
        assert!(
            validate.status.success(),
            "{}",
            String::from_utf8_lossy(&validate.stdout)
        );
        let repeated = work.path().join("repeated.3tz");
        assert!(call(&input, &repeated, &options).status.success());
        assert_eq!(
            std::fs::read(out).unwrap(),
            std::fs::read(repeated).unwrap()
        );
    }
}

#[test]
fn duplicates_terminate_and_force_replaces_only_after_success() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("cloud.laz");
    fixture(&input, 257, true, vec![]);
    let out = work.path().join("cloud.3tz");
    std::fs::write(&out, b"original").unwrap();
    let options = [
        "--sourceCrs",
        "local",
        "--maxPoints",
        "16",
        "--chunkPoints",
        "11",
    ];
    assert_eq!(call(&input, &out, &options).status.code(), Some(5));
    let bad = work.path().join("invalid.las");
    std::fs::write(&bad, b"invalid input").unwrap();
    let mut force = options.to_vec();
    force.push("--force");
    assert_eq!(call(&bad, &out, &force).status.code(), Some(3));
    assert_eq!(std::fs::read(&out).unwrap(), b"original");
    let result = call(&input, &out, &force);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
    let manifest = document(&mut zip, "tileset.json");
    let mut seen = Vec::new();
    audit_leaves(&mut zip, &manifest["root"], &mut seen, 16);
    seen.sort_unstable();
    assert_eq!(seen, (0..257).collect::<Vec<_>>());
    assert_eq!(std::fs::read_dir(work.path()).unwrap().count(), 3);
}

#[test]
fn large_tiles_use_exact_float_feature_ids_instead_of_wrapping_u16() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("cloud.las");
    fixture(&input, 65_537, true, vec![]);
    let output = work.path().join("cloud.3tz");
    assert!(call(
        &input,
        &output,
        &["--sourceCrs", "local", "--maxPoints", "70000"]
    )
    .status
    .success());
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();
    let glb = Glb::read(&mut zip, "t/0.glb");
    let accessor = &glb.doc["accessors"][glb.doc["meshes"][0]["primitives"][0]["attributes"]
        ["_FEATURE_ID_0"]
        .as_u64()
        .unwrap() as usize];
    assert_eq!(accessor["componentType"], 5126);
    let ids = glb.view(accessor["bufferView"].as_u64().unwrap() as usize);
    assert_eq!(
        f32::from_le_bytes(ids[65_536 * 4..].try_into().unwrap()),
        65_536.
    );
    assert_eq!(glb.column("source_index").len(), 65_537 * 8);
}

#[test]
fn empty_and_truncated_sources_fail_as_data_and_clean_staging() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("cloud.las");
    let output = work.path().join("cloud.3tz");
    for count in [0, 3] {
        fixture(&input, count, false, vec![]);
        if count > 0 {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(&input)
                .unwrap();
            file.set_len(file.metadata().unwrap().len() - 1).unwrap();
        }
        let result = call(&input, &output, &["--sourceCrs", "local"]);
        assert_eq!(
            result.status.code(),
            Some(3),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        assert!(!output.exists());
        assert_eq!(std::fs::read_dir(work.path()).unwrap().count(), 1);
    }
}

#[test]
fn doctor_reports_native_local_readiness_without_python_and_keeps_other_requirements() {
    let result = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["doctor", "--json", "--command", "point-cloud"])
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(result.status.success());
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["commands"]["point-cloud"]["local"]["ready"], true);
    assert_eq!(
        report["commands"]["point-cloud"]["geospatial"]["ready"],
        cfg!(feature = "native-geospatial")
    );
    let result = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["doctor", "--json"])
        .env("PATH", "")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(4));
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["commands"]["point-cloud"]["ready"], true);
    assert_eq!(report["commands"]["vector"]["ready"], false);
}

#[cfg(not(feature = "native-geospatial"))]
#[test]
fn geospatial_placement_on_a_default_build_names_the_required_feature() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("cloud.las");
    fixture(&input, 1, false, vec![]);
    let out = work.path().join("cloud.3tz");
    let result = call(
        &input,
        &out,
        &["--sourceCrs", "EPSG:32631", "--heightOffset", "0"],
    );
    assert_eq!(result.status.code(), Some(4));
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(report["error"]["message"]
        .as_str()
        .unwrap()
        .contains("--features native-geospatial"));
    assert!(!out.exists());
}

#[cfg(feature = "native-geospatial")]
#[test]
fn geotiff_header_and_explicit_crs_place_xyz_with_original_source_metadata() {
    use rusty_tiles::georef::{geodetic_to_ecef, Cartographic};
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("cloud.las");
    let keys: [u16; 16] = [
        1, 1, 0, 3, 1024, 0, 1, 1, 2048, 0, 1, 4326, 3072, 0, 1, 32631,
    ];
    fixture(
        &input,
        1,
        false,
        vec![las::Vlr {
            user_id: "LASF_Projection".into(),
            record_id: 34735,
            data: keys.into_iter().flat_map(u16::to_le_bytes).collect(),
            ..Default::default()
        }],
    );
    // Move the raw first point to UTM central meridian/equator independently.
    let mut bytes = std::fs::read(&input).unwrap();
    let start = u32::from_le_bytes(bytes[96..100].try_into().unwrap()) as usize;
    bytes[start..start + 4].copy_from_slice(&500_000_000_i32.to_le_bytes());
    bytes[start + 8..start + 12].copy_from_slice(&123_000_i32.to_le_bytes());
    std::fs::write(&input, bytes).unwrap();
    let mut reference = None;
    for source in ["header", "EPSG:32631"] {
        let output = work.path().join(format!("{source}.3tz").replace(':', "-"));
        let result = call(
            &input,
            &output,
            &["--sourceCrs", source, "--heightOffset", "7"],
        );
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        let mut zip = zip::ZipArchive::new(std::fs::File::open(output).unwrap()).unwrap();
        let manifest = document(&mut zip, "tileset.json");
        let expected = geodetic_to_ecef(Cartographic::new(3., 0., 130.));
        for i in 0..3 {
            assert!(
                (manifest["root"]["transform"][12 + i].as_f64().unwrap() - expected[i]).abs()
                    < 1e-6
            );
        }
        let glb = Glb::read(&mut zip, "t/0.glb");
        assert_eq!(
            i32::from_le_bytes(glb.column("X").try_into().unwrap()),
            500_000_000
        );
        assert_eq!(
            f64::from_le_bytes(glb.column("source_z").try_into().unwrap()),
            123.
        );
        if let Some(previous) = &reference {
            assert_eq!(previous, &manifest);
        }
        reference = Some(manifest);
    }
    let output = work.path().join("rejected.3tz");
    for options in [
        vec!["--sourceCrs", "header"],
        vec!["--sourceCrs", "EPSG:4979", "--heightOffset", "0"],
        vec!["--sourceCrs", "local", "--heightOffset", "0"],
    ] {
        assert_eq!(call(&input, &output, &options).status.code(), Some(3));
        assert!(!output.exists());
    }
}
