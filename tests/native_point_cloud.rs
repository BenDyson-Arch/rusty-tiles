//! Actual CLI acceptance without Python, plus independently decoded GLB/3TZ data.
use serde_json::Value;
use std::{io::Read, path::Path, process::Command};

mod support;

fn call(input: &Path, output: &Path, options: &[&str]) -> std::process::Output {
    call_mode(input, output, options, true)
}

fn call_mode(
    input: &Path,
    output: &Path,
    options: &[&str],
    explicit: bool,
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"));
    command.args(["--json", "point-cloud"]);
    if explicit {
        command.arg("--explicit");
    }
    command
        .arg("-i")
        .arg(input)
        .arg("-o")
        .arg(output)
        .args(options)
        .env("PATH", "")
        .output()
        .unwrap()
}

#[test]
fn implicit_las_laz_keep_every_record_and_root_relative_placement_across_subtrees() {
    for (suffix, identical) in [("las", false), ("laz", false), ("laz", true)] {
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join(format!("source.{suffix}"));
        fixture(&input, 257, identical, vec![]);
        let output = work.path().join("implicit.3tz");
        let options = [
            "--sourceCrs",
            "local",
            "--maxPoints",
            "1",
            "--chunkPoints",
            "11",
        ];
        let result = call_mode(&input, &output, &options, false);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();
        let manifest = document(&mut zip, "tileset.json");
        assert_eq!(
            manifest["root"]["implicitTiling"]["subdivisionScheme"],
            "OCTREE"
        );
        if !identical {
            assert!(zip.file_names().filter(|n| n.ends_with(".subtree")).count() > 1);
        }
        let expanded = rusty_tiles::implicit::expand_tileset(&manifest, |uri| {
            let mut bytes = Vec::new();
            zip.by_name(uri).unwrap().read_to_end(&mut bytes).unwrap();
            Ok(bytes)
        })
        .unwrap();
        let mut seen = Vec::new();
        audit_leaves(&mut zip, &expanded["root"], &mut seen, 1);
        seen.sort_unstable();
        assert_eq!(seen, (0..257).collect::<Vec<_>>());
        let expected: Vec<_> = las::Reader::from_path(&input)
            .unwrap()
            .read_all()
            .unwrap()
            .points()
            .map(Result::unwrap)
            .collect();
        let root_translation = std::array::from_fn::<_, 3, _>(|i| {
            manifest["root"]["transform"][12 + i].as_f64().unwrap()
        });
        fn placement(
            zip: &mut zip::ZipArchive<std::fs::File>,
            node: &Value,
            root: [f64; 3],
            expected: &[las::Point],
        ) {
            if let Some(children) = node["children"].as_array() {
                for child in children {
                    placement(zip, child, root, expected);
                }
                return;
            }
            let glb = Glb::read(zip, node["content"]["uri"].as_str().unwrap());
            let id = u64::from_le_bytes(glb.column("source_index").try_into().unwrap()) as usize;
            let accessor = &glb.doc["accessors"][glb.doc["meshes"][0]["primitives"][0]["attributes"]
                ["POSITION"]
                .as_u64()
                .unwrap() as usize];
            let bytes = glb.view(accessor["bufferView"].as_u64().unwrap() as usize);
            let p: [f64; 3] = std::array::from_fn(|i| {
                f64::from(f32::from_le_bytes(
                    bytes[i * 4..i * 4 + 4].try_into().unwrap(),
                ))
            });
            let t: [f64; 3] = glb.doc["nodes"][0]
                .get("translation")
                .map(|v| serde_json::from_value(v.clone()).unwrap())
                .unwrap_or([0.; 3]);
            let actual = [
                root[0] + p[0] + t[0],
                root[1] - p[2] - t[2],
                root[2] + p[1] + t[1],
            ];
            for (a, e) in actual
                .into_iter()
                .zip([expected[id].x, expected[id].y, expected[id].z])
            {
                assert!((a - e).abs() < 1e-5, "{id}: {a} != {e}");
            }
            for (name, expected) in [
                ("red", (id * 233) as u16),
                ("green", (id * 431) as u16),
                ("blue", (id * 717) as u16),
                ("nir", (id * 103) as u16),
            ] {
                assert_eq!(
                    u16::from_le_bytes(glb.column(name).try_into().unwrap()),
                    expected
                );
            }
        }
        placement(&mut zip, &expanded["root"], root_translation, &expected);
        let report = support::rusty_tiles()
            .args(["validate", "--json"])
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            report.status.success(),
            "{}",
            String::from_utf8_lossy(&report.stdout)
        );
        let repeated = work.path().join("repeated.3tz");
        assert!(call_mode(&input, &repeated, &options, false)
            .status
            .success());
        assert_eq!(
            std::fs::read(output).unwrap(),
            std::fs::read(repeated).unwrap()
        );
    }
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
fn force_replaces_only_successful_output() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("cloud.las");
    fixture(&input, 2, false, vec![]);
    support::force_replaces_only_successful_output(
        "point-cloud",
        &input,
        &["--sourceCrs", "local"],
        false,
    );
}

/// tempfile creates 0600 files. The published archive must follow the umask.
#[cfg(unix)]
#[test]
fn published_archive_mode_follows_umask() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("cloud.las");
    fixture(&input, 2, false, vec![]);
    let output = work.path().join("cloud.3tz");
    let result = support::with_umask_022(&[
        "point-cloud".as_ref(),
        "-i".as_ref(),
        input.as_os_str(),
        "-o".as_ref(),
        output.as_os_str(),
        "--sourceCrs".as_ref(),
        "local".as_ref(),
    ] as &[&std::ffi::OsStr]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(support::mode(&output), 0o644);
    let tree = work.path().join("tree");
    std::fs::create_dir(&tree).unwrap();
    std::fs::write(tree.join("tileset.json"), "{}").unwrap();
    let rebuilt = work.path().join("rebuilt.3tz");
    let result = support::with_umask_022(&[
        "convert".as_ref(),
        "-i".as_ref(),
        tree.as_os_str(),
        "-o".as_ref(),
        rebuilt.as_os_str(),
    ] as &[&std::ffi::OsStr]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(support::mode(&rebuilt), 0o644);
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
        true
    );
    assert_eq!(
        report["commands"]["point-cloud"]["geospatial"]["backend"],
        "pure Rust (proj4rs)"
    );
    assert_eq!(
        report["commands"]["point-cloud"]["geospatial"]["nativeFallback"]["ready"],
        cfg!(feature = "native-geospatial")
    );
    assert!(
        report["commands"]["point-cloud"]["geospatial"]["crsClasses"]
            .as_array()
            .unwrap()
            .len()
            >= 4
    );
    let result = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["doctor", "--json"])
        .env("PATH", "")
        .output()
        .unwrap();
    assert_eq!(
        result.status.code(),
        Some(if cfg!(feature = "native-geospatial") {
            0
        } else {
            4
        })
    );
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["commands"]["point-cloud"]["ready"], true);
    assert_eq!(
        report["commands"]["vector"]["ready"],
        cfg!(feature = "native-geospatial")
    );
}

#[cfg(not(feature = "native-geospatial"))]
#[test]
fn unverified_datum_on_a_default_build_names_the_required_feature() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("cloud.las");
    fixture(&input, 1, false, vec![]);
    let out = work.path().join("cloud.3tz");
    let result = call(
        &input,
        &out,
        &["--sourceCrs", "EPSG:26910", "--heightOffset", "0"],
    );
    assert_eq!(result.status.code(), Some(4));
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(report["error"]["message"]
        .as_str()
        .unwrap()
        .contains("--features native-geospatial"));
    assert!(!out.exists());
}

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

#[test]
fn wkt_las_and_laz_headers_place_globe_coordinates_without_external_resources() {
    use rusty_tiles::georef::{geodetic_to_ecef, Cartographic};
    let wkt = r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]]"#;
    for suffix in ["las", "laz"] {
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join(format!("cloud.{suffix}"));
        let mut builder = las::Builder::from((1, 4));
        builder.vlrs.push(las::Vlr {
            user_id: "LASF_Projection".into(),
            record_id: 2112,
            data: format!("{wkt}\0").into_bytes(),
            ..Default::default()
        });
        let mut writer = las::Writer::from_path(&input, builder.into_header().unwrap()).unwrap();
        writer
            .write_point(las::Point {
                x: 153.02,
                y: -27.47,
                z: 123.,
                ..Default::default()
            })
            .unwrap();
        writer.close().unwrap();
        let output = work.path().join("cloud.3tz");
        let result = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
            .args(["--json", "point-cloud", "--explicit", "-i"])
            .arg(&input)
            .arg("-o")
            .arg(&output)
            .args(["--source-crs", "header", "--height-offset", "7"])
            .env("PATH", "")
            .env("PROJ_DATA", work.path().join("absent-database"))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        let mut zip = zip::ZipArchive::new(std::fs::File::open(output).unwrap()).unwrap();
        let manifest = document(&mut zip, "tileset.json");
        let expected = geodetic_to_ecef(Cartographic::new(153.02, -27.47, 130.));
        for (i, expected) in expected.iter().enumerate() {
            assert!(
                (manifest["root"]["transform"][12 + i].as_f64().unwrap() - expected).abs() < 0.001
            );
        }
    }
}

#[cfg(not(feature = "native-geospatial"))]
#[test]
fn compound_geoid_header_refuses_default_build_without_publishing() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("cloud.las");
    let wkt = r#"COMPD_CS["WGS84 + invented geoid",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],VERT_CS["Invented geoid height",VERT_DATUM["Invented geoid",2005,EXTENSION["PROJ4_GRIDS","missing.gtx"]],UNIT["metre",1],AXIS["Up",UP]]]"#;
    fixture(
        &input,
        1,
        false,
        vec![las::Vlr {
            user_id: "LASF_Projection".into(),
            record_id: 2112,
            data: format!("{wkt}\0").into_bytes(),
            ..Default::default()
        }],
    );
    let output = work.path().join("cloud.3tz");
    let result = call(
        &input,
        &output,
        &["--source-crs", "header", "--height-offset", "0"],
    );
    assert_eq!(result.status.code(), Some(4));
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    let message = report["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("--features native-geospatial"),
        "{message}"
    );
    assert!(message.contains("grids"), "{message}");
    assert!(!output.exists());
    assert!(!std::fs::read_dir(work.path()).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".tiles-work-")));
}

#[test]
fn invalid_projection_parameters_and_units_never_publish() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("cloud.las");
    fixture(&input, 1, false, vec![]);
    let wkt = r#"PROJCS["Invented invalid scale",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Transverse_Mercator"],PARAMETER["scale_factor",0],UNIT["metre",1]]"#;
    for definition in [
        "+proj=stere +lat_0=45 +k=0 +datum=WGS84",
        "+proj=stere +lat_0=45 +k=-1 +datum=WGS84",
        "+proj=stere +lat_0=45 +k_0=0 +datum=WGS84",
        "+proj=stere +lat_0=45 +k_0=-1 +datum=WGS84",
        "+proj=utm +datum=WGS84",
        "+proj=utm +lon_0=10 +datum=WGS84",
        "+proj=tmerc +lat_0=100 +datum=WGS84",
        "+proj=tmerc +lat_0=-100 +datum=WGS84",
        "+proj=aea +lat_1=100 +lat_2=45 +datum=WGS84",
        "+proj=merc +lat_ts=-100 +datum=WGS84",
        "+proj=lcc +lat_1=-90 +lat_2=45 +datum=WGS84",
        "+proj=utm +zone=32 +datum=WGS84 +units=degrees",
        "+proj=stere +lat_0=90 +lat_ts=70 +k=0.99 +datum=WGS84",
        "+proj=stere +lat_0=-90 +lat_ts=-70 +k_0=2 +datum=WGS84",
        "+proj=stere +lat_0=90 +lat_ts=-90 +k=0.99 +datum=WGS84",
        "+proj=stere +lat_0=-90 +lat_ts=90 +k_0=0.99 +datum=WGS84",
        "+proj=aea +lat_1=30 +lat_2=-29.99999999 +datum=WGS84",
        "+proj=lcc +lat_1=30 +lat_2=-29.99999999 +lat_0=0 +datum=WGS84",
        wkt,
    ] {
        let output = work.path().join("rejected.3tz");
        let result = call(
            &input,
            &output,
            &["--source-crs", definition, "--height-offset", "7"],
        );
        assert_eq!(
            result.status.code(),
            Some(3),
            "{definition}: {}",
            String::from_utf8_lossy(&result.stdout)
        );
        let report: Value = serde_json::from_slice(&result.stdout).unwrap();
        let message = report["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("positive")
                || message.contains("explicit +zone")
                || message.contains("projection latitude")
                || message.contains("linear +units")
                || message.contains("conflicts")
                || message.contains("conic standard parallels"),
            "{definition}: {message}"
        );
        assert!(!output.exists());
    }
    assert!(!std::fs::read_dir(work.path()).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".tiles-work-")));
}

#[test]
fn native_crs_guards_preserve_placement_or_refuse_without_publishing() {
    let work = tempfile::tempdir().unwrap();
    for (point_index, point) in [
        [1000., 2000., 123.],
        [200000., 567890., 123.],
        [3000000., -3000000., 500.],
    ]
    .into_iter()
    .enumerate()
    {
        let input = work.path().join(format!("cloud-{point_index}.las"));
        let mut builder = las::Builder::from((1, 4));
        builder.transforms.x.scale = 0.01;
        builder.transforms.y.scale = 0.01;
        let header = builder.into_header().unwrap();
        let mut writer = las::Writer::from_path(&input, header).unwrap();
        writer
            .write_point(las::Point {
                x: point[0],
                y: point[1],
                z: point[2],
                ..Default::default()
            })
            .unwrap();
        writer.close().unwrap();
        for (index, definition) in [
            "+proj=tmerc +ellps=WGS84 +towgs84=0,0,0 +pm=0dE",
            "+proj=tmerc +a=6371000 +b=6371000 +towgs84=0,0,0",
            "+proj=sterea +lat_0=-90 +datum=WGS84",
            "+proj=sterea +lat_0=90 +datum=WGS84",
            "+proj=sterea +lat_0=89.99999999 +datum=WGS84",
            "+proj=sterea +lat_0=-89.99999999 +datum=WGS84",
            "+proj=sterea +lat_0=80 +datum=WGS84",
            "+proj=sterea +lat_0=-80 +datum=WGS84",
            "+proj=stere +lat_0=89.99999999 +datum=WGS84",
            "+proj=stere +lat_0=-89.99999999 +datum=WGS84",
            "+proj=lcc +lat_1=-80 +lat_2=-79.99999999 +lat_0=-80 +datum=WGS84",
            "+proj=lcc +lat_1=33 +datum=WGS84",
            "+proj=lcc +lat_1=-89.999999 +lat_2=-89.999999 +lat_0=-89.999999 +datum=WGS84",
            "+proj=aea +lat_1=-80 +lat_2=-79.99999999 +lat_0=-80 +datum=WGS84",
            "+init=epsg:32632",
            "+proj=utm +zone = 32 +datum=WGS84",
            "EPSG:32632@2020",
            r#"+proj=tmerc +k="0.9996" +datum=WGS84"#,
            r#"+proj=tmerc +ellps=WGS84 +towgs84="1,2,3""#,
            "+proj=laea +lat_0=-15 +lon_0=135 +datum=WGS84",
        ]
        .into_iter()
        .enumerate()
        {
            let output = work.path().join(format!("{point_index}-{index}.3tz"));
            let result = call(
                &input,
                &output,
                &["--source-crs", definition, "--height-offset", "7"],
            );
            #[cfg(feature = "native-geospatial")]
            {
                use rusty_tiles::geospatial::{Crs, EcefTransform};
                assert!(
                    result.status.success(),
                    "{definition}: {}",
                    String::from_utf8_lossy(&result.stdout)
                );
                let expected =
                    EcefTransform::new(Crs::from_definition(definition).unwrap(), Some(7.))
                        .unwrap()
                        .transform(&[point])
                        .unwrap()[0];
                let mut zip = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();
                let manifest = document(&mut zip, "tileset.json");
                let distance = expected
                    .iter()
                    .enumerate()
                    .map(|(i, expected)| {
                        (manifest["root"]["transform"][12 + i].as_f64().unwrap() - expected).powi(2)
                    })
                    .sum::<f64>()
                    .sqrt();
                assert!(
                    distance < 0.001,
                    "{definition}: ECEF difference {distance} m"
                );
                rusty_tiles::validate::archive(&output, None).unwrap();
            }
            #[cfg(not(feature = "native-geospatial"))]
            {
                assert_eq!(
                    result.status.code(),
                    Some(4),
                    "{definition}: {}",
                    String::from_utf8_lossy(&result.stdout)
                );
                let report: Value = serde_json::from_slice(&result.stdout).unwrap();
                assert!(report["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("--features native-geospatial"));
                assert!(!output.exists());
            }
        }
    }
    assert!(!std::fs::read_dir(work.path()).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".tiles-work-")));
}

fn exact_point_fixture(input: &Path, point: [f64; 3]) {
    let mut builder = las::Builder::from((1, 4));
    builder.transforms.x.offset = point[0];
    builder.transforms.y.offset = point[1];
    builder.transforms.z.offset = point[2];
    let mut writer = las::Writer::from_path(input, builder.into_header().unwrap()).unwrap();
    writer
        .write_point(las::Point {
            x: point[0],
            y: point[1],
            z: point[2],
            ..Default::default()
        })
        .unwrap();
    writer.close().unwrap();
}

#[test]
fn wkt_method_parameters_and_albers_preserve_native_or_refuse() {
    let work = tempfile::tempdir().unwrap();
    let incomplete = include_str!("fixtures/crs/lcc-2sp-missing-parallel.wkt");
    let extra_scale = incomplete.replace(r#",PARAMETER["false_easting""#,
        r#",PARAMETER["standard_parallel_2",45],PARAMETER["scale_factor",2],PARAMETER["false_easting""#);
    for (index, (definition, point, outside_accuracy_domain)) in [
        (incomplete, [200000., 567890., 123.], false),
        (extra_scale.as_str(), [200000., 567890., 123.], false),
        (
            include_str!("fixtures/crs/mercator-redundant-origin.wkt"),
            [200000., 567890., 123.],
            false,
        ),
        (
            include_str!("fixtures/crs/mercator-wrong-parallel.wkt"),
            [200000., 567890., 123.],
            false,
        ),
        (
            "+proj=aea +lat_1=33 +lat_2=45 +lat_0=10 +datum=WGS84",
            [0., 7279556.2302497085, 123.],
            true,
        ),
        (
            "+proj=aea +lat_1=-33 +lat_2=-45 +lat_0=-10 +datum=WGS84",
            [0., -7279556.2302497085, 123.],
            true,
        ),
        (
            "+proj=aea +lat_1=80 +lat_2=80 +lat_0=80 +ellps=sphere +towgs84=0,0,0",
            [612606.1867269421, 13949033.598816177, 123.],
            true,
        ),
        (
            include_str!("fixtures/crs/lcc-michigan-near-equator.wkt"),
            [5000., 6000., 123.],
            true,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let input = work.path().join(format!("source-{index}.las"));
        exact_point_fixture(&input, point);
        let output = work.path().join(format!("source-{index}.3tz"));
        let result = call(
            &input,
            &output,
            &["--source-crs", definition, "--height-offset", "7"],
        );
        #[cfg(not(feature = "native-geospatial"))]
        {
            let _ = outside_accuracy_domain;
            assert_eq!(
                result.status.code(),
                Some(if definition.starts_with("+proj=aea") {
                    3
                } else {
                    4
                }),
                "{definition}: {}",
                String::from_utf8_lossy(&result.stdout)
            );
            assert!(!output.exists());
        }
        #[cfg(feature = "native-geospatial")]
        {
            use rusty_tiles::geospatial::{Crs, EcefTransform};
            if outside_accuracy_domain {
                assert_eq!(result.status.code(), Some(3));
                let report: Value = serde_json::from_slice(&result.stdout).unwrap();
                assert!(report["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("verified point-cloud accuracy"));
                assert!(!output.exists());
                continue;
            }
            assert!(
                result.status.success(),
                "{definition}: {}",
                String::from_utf8_lossy(&result.stdout)
            );
            let expected = EcefTransform::new(Crs::from_definition(definition).unwrap(), Some(7.))
                .unwrap()
                .transform(&[point])
                .unwrap()[0];
            let mut zip = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();
            let manifest = document(&mut zip, "tileset.json");
            let distance = expected
                .iter()
                .enumerate()
                .map(|(i, expected)| {
                    (manifest["root"]["transform"][12 + i].as_f64().unwrap() - expected).powi(2)
                })
                .sum::<f64>()
                .sqrt();
            assert!(
                distance < 0.001,
                "{definition}: ECEF difference {distance} m"
            );
            rusty_tiles::validate::archive(&output, None).unwrap();
        }
    }
    assert!(!std::fs::read_dir(work.path()).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".tiles-work-")));
}

#[test]
fn stereographic_points_and_hemispheres_use_native_or_refuse_without_publishing() {
    let work = tempfile::tempdir().unwrap();
    for (index, (definition, point)) in [
        (
            "+proj=sterea +lat_0=45 +datum=WGS84",
            [0.9141389405141953, 5290076.530007198, 123.],
        ),
        (
            "+proj=sterea +lat_0=79.999999 +ellps=airy +towgs84=12,-34,56",
            [0.00112530269295305, 1119554.6907156024, 123.],
        ),
        (
            "+proj=stere +lat_0=75 +datum=WGS84",
            [0., 1685039.1152903102, 123.],
        ),
        (
            "+proj=stere +lat_0=-75 +datum=WGS84",
            [0., -1685039.1152903102, 123.],
        ),
        (
            "+proj=stere +lat_0=90 +lat_ts=-70 +datum=WGS84",
            [0., 0., 123.],
        ),
        (
            "+proj=stere +lat_0=-90 +lat_ts=70 +datum=WGS84",
            [0., 0., 123.],
        ),
        (
            "+proj=stere +lat_0=-90 +lat_ts=0 +datum=WGS84",
            [0., 0., 123.],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let input = work.path().join(format!("polar-point-{index}.las"));
        exact_point_fixture(&input, point);
        let output = work.path().join(format!("polar-point-{index}.3tz"));
        let result = call(
            &input,
            &output,
            &["--source-crs", definition, "--height-offset", "7"],
        );
        #[cfg(feature = "native-geospatial")]
        {
            use rusty_tiles::geospatial::{Crs, EcefTransform};
            let expected = EcefTransform::new(Crs::from_definition(definition).unwrap(), Some(7.))
                .unwrap()
                .transform(&[point]);
            let expected = match expected {
                Ok(expected) => {
                    assert!(
                        result.status.success(),
                        "{definition}: {}",
                        String::from_utf8_lossy(&result.stdout)
                    );
                    expected[0]
                }
                Err(error) => {
                    assert_eq!(
                        result.status.code(),
                        Some(i32::from(error.category().1)),
                        "{definition}"
                    );
                    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
                    assert_eq!(report["error"]["code"], error.category().0);
                    assert!(matches!(error, rusty_tiles::Error::Data(_)), "{error}");
                    assert!(!output.exists());
                    continue;
                }
            };
            let mut zip = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();
            let manifest = document(&mut zip, "tileset.json");
            let distance = expected
                .iter()
                .enumerate()
                .map(|(i, expected)| {
                    (manifest["root"]["transform"][12 + i].as_f64().unwrap() - expected).powi(2)
                })
                .sum::<f64>()
                .sqrt();
            assert!(distance < 0.001, "ECEF difference {distance} m");
            rusty_tiles::validate::archive(&output, None).unwrap();
        }
        #[cfg(not(feature = "native-geospatial"))]
        {
            assert_eq!(result.status.code(), Some(4));
            let report: Value = serde_json::from_slice(&result.stdout).unwrap();
            assert!(report["error"]["message"]
                .as_str()
                .unwrap()
                .contains("pure-Rust point-cloud CRS transform unavailable"));
            assert!(!output.exists());
        }
    }
    assert!(!std::fs::read_dir(work.path()).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".tiles-work-")));
}

#[test]
fn native_lcc_variant_b_conditioning_cannot_be_bypassed_with_alternate_syntax() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("cloud.las");
    fixture(&input, 1, false, vec![]);
    for definition in [
        r#"+proj=lcc +lat_1="1e-08" +lat_0=5 +datum=WGS84"#,
        "+proj=lcc +lat_1 = 1e-08 +lat_0=5 +datum=WGS84",
    ] {
        let output = work.path().join("rejected.3tz");
        let result = call(
            &input,
            &output,
            &["--source-crs", definition, "--height-offset", "7"],
        );
        #[cfg(feature = "native-geospatial")]
        let (code, message) = (3, "conic standard parallels");
        #[cfg(not(feature = "native-geospatial"))]
        let (code, message) = (4, "pure-Rust point-cloud CRS transform unavailable");
        assert_eq!(
            result.status.code(),
            Some(code),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        let report: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert!(report["error"]["message"]
            .as_str()
            .unwrap()
            .contains(message));
        assert!(!output.exists());
    }
    assert!(!std::fs::read_dir(work.path()).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".tiles-work-")));
}

#[test]
fn metadata_attributes_match_source_tables_through_las_laz_lods() {
    for suffix in ["las", "laz"] {
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join(format!("source.{suffix}"));
        fixture(&input, 257, false, vec![]);
        for explicit in [false, true] {
            let output = work.path().join(format!("{explicit}.3tz"));
            let result = call_mode(
                &input,
                &output,
                &[
                    "--source-crs",
                    "local",
                    "--max-points",
                    "16",
                    "--chunk-points",
                    "11",
                    "--metadata-attributes",
                ],
                explicit,
            );
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stdout)
            );
            let mut zip = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();
            let files: Vec<_> = zip
                .file_names()
                .filter(|s| s.ends_with(".glb"))
                .map(str::to_owned)
                .collect();
            assert!(files.len() > 1);
            for file in files {
                let glb = Glb::read(&mut zip, &file);
                let metadata = &glb.doc["extensions"]["EXT_structural_metadata"];
                let primitive = &glb.doc["meshes"][0]["primitives"][0];
                let classes = &metadata["schema"]["classes"];
                assert_eq!(metadata["propertyAttributes"][0]["class"], "pointAttribute");
                for name in classes["pointAttribute"]["properties"]
                    .as_object()
                    .unwrap()
                    .keys()
                {
                    assert!(
                        classes["point"]["properties"].get(name).is_none(),
                        "duplicate shader property {name}"
                    );
                }
                assert_eq!(
                    primitive["extensions"]["EXT_structural_metadata"]["propertyAttributes"],
                    serde_json::json!([0])
                );
                let count = metadata["propertyTables"][0]["count"].as_u64().unwrap() as usize;
                for (name, width, component) in [
                    ("classification", 1, 5121),
                    ("intensity", 2, 5123),
                    ("return_number", 1, 5121),
                ] {
                    let attribute = metadata["propertyAttributes"][0]["properties"]
                        [format!("vertex_{name}")]["attribute"]
                        .as_str()
                        .unwrap();
                    let accessor = &glb.doc["accessors"]
                        [primitive["attributes"][attribute].as_u64().unwrap() as usize];
                    assert_eq!(accessor["count"], count);
                    assert_eq!(accessor["componentType"], component);
                    assert!(accessor.get("normalized").is_none());
                    let v = accessor["bufferView"].as_u64().unwrap() as usize;
                    assert_eq!(glb.doc["bufferViews"][v]["byteStride"], 4);
                    let data = glb.view(v);
                    let table = glb.column(name);
                    for i in 0..count {
                        assert_eq!(
                            &data[i * 4..i * 4 + width],
                            &table[i * width..(i + 1) * width]
                        );
                    }
                }
            }
            rusty_tiles::validate::archive(&output, None).unwrap();
            if suffix == "las" {
                if let Some(dir) = std::env::var_os("RUSTY_TILES_METADATA_ACCEPTANCE_DIR") {
                    let dir = std::path::PathBuf::from(dir).join(format!("point-{explicit}"));
                    std::fs::create_dir_all(&dir).unwrap();
                    zip.extract(dir).unwrap();
                }
            }
        }
    }
}

#[test]
fn metadata_attributes_decode_legacy_classification_flags_and_extended_returns() {
    for (format, returns) in [(0, 6), (3, 7), (6, 14), (8, 15)] {
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join("source.las");
        let mut builder = las::Builder::from((1, 4));
        builder.point_format = las::point::Format::new(format).unwrap();
        let mut writer = las::Writer::from_path(&input, builder.into_header().unwrap()).unwrap();
        for i in 0..17 {
            writer
                .write_point(las::Point {
                    x: i as f64,
                    intensity: 65000 + i,
                    return_number: returns,
                    number_of_returns: returns,
                    classification: las::point::Classification::new(17).unwrap(),
                    is_synthetic: true,
                    is_key_point: true,
                    is_withheld: true,
                    gps_time: (format != 0).then_some(1.),
                    color: matches!(format, 3 | 8).then_some(las::Color {
                        red: 1,
                        green: 2,
                        blue: 3,
                    }),
                    nir: (format == 8).then_some(4),
                    ..Default::default()
                })
                .unwrap();
        }
        writer.close().unwrap();
        let output = work.path().join("cloud.3tz");
        let result = call_mode(
            &input,
            &output,
            &[
                "--source-crs",
                "local",
                "--max-points",
                "4",
                "--metadata-attributes",
            ],
            false,
        );
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();
        let files: Vec<_> = zip
            .file_names()
            .filter(|s| s.ends_with(".glb"))
            .map(str::to_owned)
            .collect();
        for file in files {
            let glb = Glb::read(&mut zip, &file);
            let primitive = &glb.doc["meshes"][0]["primitives"][0];
            for (i, id) in glb
                .column("source_index")
                .as_chunks::<8>()
                .0
                .iter()
                .enumerate()
            {
                let source = u64::from_le_bytes(*id) as u16;
                for (attribute, expected) in [
                    ("_CLASSIFICATION", 17),
                    ("_INTENSITY", 65000 + source),
                    ("_RETURN_NUMBER", returns as u16),
                ] {
                    let accessor = &glb.doc["accessors"]
                        [primitive["attributes"][attribute].as_u64().unwrap() as usize];
                    let values = glb.view(accessor["bufferView"].as_u64().unwrap() as usize);
                    let actual = if attribute == "_INTENSITY" {
                        u16::from_le_bytes(values[i * 4..i * 4 + 2].try_into().unwrap())
                    } else {
                        values[i * 4] as u16
                    };
                    assert_eq!(actual, expected, "LAS format {format}: {attribute}");
                }
            }
        }
    }
}
