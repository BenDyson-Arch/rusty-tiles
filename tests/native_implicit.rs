//! Complete native archive checks, including tight boxes crossing regular cells.
mod support;
use serde_json::{json, Value};
#[cfg(feature = "native-geospatial")]
use std::io::Read;
use std::{fs, path::Path};

fn success(output: std::process::Output) {
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn document(path: &Path, name: &str) -> Value {
    let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    serde_json::from_reader(zip.by_name(name).unwrap()).unwrap()
}

#[test]
fn deep_implicit_mesh_validates_and_corrupt_subtrees_and_boundary_links_are_rejected() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("mesh.glb");
    fs::write(&input, support::textured_grid_glb(12)).unwrap();
    let output = work.path().join("mesh.3tz");
    success(
        support::rusty_tiles()
            .args(["mesh-to-3tz", "-i"])
            .arg(&input)
            .arg("-o")
            .arg(&output)
            .args(["--maxTriangles", "2", "--maxBytes", "0", "--tileSize", "64"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        document(&output, "tileset.json")["root"]["implicitTiling"]["subdivisionScheme"],
        "OCTREE"
    );
    let report = rusty_tiles::validate::archive(&output, None).unwrap();
    assert!(report["tiles"].as_u64().unwrap() > 100);
    let mut zip = zip::ZipArchive::new(fs::File::open(&output).unwrap()).unwrap();
    assert!(zip
        .file_names()
        .any(|name| name.starts_with("implicit-tileset-")));
    for case in [
        "header",
        "availability",
        "metadata",
        "missing",
        "link",
        "root",
    ] {
        let directory = work.path().join(case);
        fs::create_dir(&directory).unwrap();
        zip.extract(&directory).unwrap();
        fs::remove_file(directory.join("@3dtilesIndex1@")).unwrap();
        let subtree = directory.join(
            zip.file_names()
                .find(|name| name.ends_with(".subtree"))
                .unwrap(),
        );
        let mut bytes = fs::read(&subtree).unwrap();
        match case {
            "root" => {
                let path = directory.join("tileset.json");
                let mut doc: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                doc["root"]["boundingVolume"]["box"][0] = json!(1000000.);
                fs::write(path, serde_json::to_vec(&doc).unwrap()).unwrap();
            }
            "header" => {
                bytes[0] = 0;
                fs::write(&subtree, bytes).unwrap();
            }
            "missing" => fs::remove_file(subtree).unwrap(),
            "link" => {
                let path = directory.join(
                    zip.file_names()
                        .find(|name| name.starts_with("implicit-tileset-"))
                        .unwrap(),
                );
                let mut doc: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                doc["root"]["geometricError"] = json!(999999.);
                fs::write(path, serde_json::to_vec(&doc).unwrap()).unwrap();
            }
            "availability" | "metadata" => {
                let length = u64::from_le_bytes(bytes[8..16].try_into().unwrap()) as usize;
                let mut doc: Value = serde_json::from_slice(&bytes[24..24 + length]).unwrap();
                let binary = bytes[24 + length..].to_vec();
                if case == "availability" {
                    doc["tileAvailability"]["availableCount"] = json!(999999);
                } else {
                    doc["propertyTables"][0]["count"] = json!(1);
                }
                let mut data = serde_json::to_vec(&doc).unwrap();
                data.resize(data.len().next_multiple_of(8), b' ');
                bytes.truncate(24);
                bytes[8..16].copy_from_slice(&(data.len() as u64).to_le_bytes());
                bytes.extend(data);
                bytes.extend(binary);
                fs::write(subtree, bytes).unwrap();
            }
            _ => unreachable!(),
        }
        let damaged = work.path().join(format!("{case}.3tz"));
        rusty_tiles::convert_to_3tz(&directory, &damaged, &Default::default()).unwrap();
        assert!(
            rusty_tiles::validate::archive(&damaged, None).is_err(),
            "{case}"
        );
    }
}

#[cfg(feature = "native-geospatial")]
#[test]
fn implicit_vector_reuse_survives_spatial_edits_and_deletion_and_rejects_explicit_cache() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("source.geojson");
    let mut features: Vec<_> = (0..32)
        .map(|id| {
            json!({"type":"Feature","id":id,"properties":{"name":format!("point-{id}")},
        "geometry":{"type":"Point","coordinates":[id%8,id/8,0]}})
        })
        .collect();
    let write = |features: &Vec<Value>| {
        fs::write(
            &input,
            json!({"type":"FeatureCollection","features":features}).to_string(),
        )
        .unwrap()
    };
    let run = |name: &str, previous: Option<&Path>, explicit: bool| {
        let path = work.path().join(name);
        let mut command = support::rusty_tiles();
        command
            .args(["vector", "-i"])
            .arg(&input)
            .arg("-o")
            .arg(&path)
            .args([
                "--sourceCrs",
                "local",
                "--maxFeatures",
                "1",
                "--reproducible",
            ]);
        if let Some(previous) = previous {
            command.arg("--reuseTileset").arg(previous);
        }
        if explicit {
            command.arg("--explicit");
        }
        (path, command.output().unwrap())
    };
    write(&features);
    let (original, result) = run("original.3tz", None, false);
    success(result);
    // Keep the first source anchor and move a later feature across partitions.
    features[24]["geometry"]["coordinates"] = json!([3.5, 1.5, 0]);
    features.remove(31);
    write(&features);
    let (updated, result) = run("updated.3tz", Some(&original), false);
    success(result);
    let (fresh, result) = run("fresh.3tz", None, false);
    success(result);
    for path in [&updated, &fresh] {
        let report = rusty_tiles::validate::archive(path, None).unwrap();
        assert!(report["ok"].as_bool().unwrap());
        let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
        let manifest = document(path, "tileset.json");
        let expanded = rusty_tiles::implicit::expand_tileset(&manifest, |name| {
            let mut bytes = Vec::new();
            zip.by_name(name)?.read_to_end(&mut bytes)?;
            Ok(bytes)
        })
        .unwrap();
        fn leaves(node: &Value) -> usize {
            node["children"]
                .as_array()
                .map_or(1, |children| children.iter().map(leaves).sum())
        }
        assert_eq!(leaves(&expanded["root"]), 31);
        assert_eq!(document(path, "conversion.json")["features"], 31);
    }
    let reuse = document(&updated, "conversion.json");
    assert!(reuse["reuse"]["reusedContents"].as_u64().unwrap() > 0);
    assert!(reuse["reuse"]["rebuiltContents"].as_u64().unwrap() > 0);
    let (_, result) = run("explicit-cache.3tz", Some(&updated), true);
    assert!(!result.status.success());
}

#[cfg(feature = "native-geospatial")]
#[test]
fn fragmented_vector_preserves_content_headers_and_binary_payloads() {
    use std::collections::BTreeSet;
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("polygon.geojson");
    let ring = |radius: f64, count: usize| {
        (0..=count)
            .map(|i| {
                let angle = (i % count) as f64 * std::f64::consts::TAU / count as f64;
                json!([radius * angle.cos(), radius * angle.sin(), 0])
            })
            .collect::<Vec<_>>()
    };
    fs::write(
        &input,
        json!({"type":"FeatureCollection","features":[{
            "type":"Feature","id":"polygon","properties":{"name":"hole"},
            "geometry":{"type":"Polygon","coordinates":[ring(20.,40),ring(5.,20)]}
        }]})
        .to_string(),
    )
    .unwrap();
    let output = work.path().join("polygon.3tz");
    success(
        support::rusty_tiles()
            .args(["vector", "-i"])
            .arg(&input)
            .arg("-o")
            .arg(&output)
            .args([
                "--sourceCrs",
                "local",
                "--maxVertices",
                "32",
                "--maxBytes",
                "16384",
                "--lodTolerance",
                "0.2",
                "--lodLevels",
                "3",
                "--quantize",
                "--meshopt",
                "--reproducible",
            ])
            .output()
            .unwrap(),
    );
    rusty_tiles::validate::archive(&output, None).unwrap();
    let manifest = document(&output, "tileset.json");
    let mut zip = zip::ZipArchive::new(fs::File::open(&output).unwrap()).unwrap();
    let names: Vec<_> = zip.file_names().map(str::to_owned).collect();
    let mut read = |name: &str| {
        let mut bytes = Vec::new();
        zip.by_name(name).unwrap().read_to_end(&mut bytes).unwrap();
        bytes
    };
    fn payload(bytes: &[u8]) -> (Vec<u8>, String) {
        let offset = if bytes.starts_with(b"b3dm") {
            (3..7).fold(28, |offset, i| {
                offset + u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap()) as usize
            })
        } else {
            0
        };
        let glb = gltf::Glb::from_slice(&bytes[offset..]).unwrap();
        let mut json: Value = serde_json::from_slice(&glb.json).unwrap();
        // Only scene node placement may change; attributes, textures, feature
        // metadata, compression and all binary buffer bytes must stay exact.
        json.as_object_mut().unwrap().remove("nodes");
        (glb.bin.unwrap_or_default().into_owned(), json.to_string())
    }
    let sources: BTreeSet<_> = manifest["extras"]["rustyTilesSourceContents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| payload(&read(name.as_str().unwrap())))
        .collect();
    let mut fills = 0;
    for name in &names {
        if name.starts_with("implicit-content/") {
            let bytes = read(name);
            assert!(
                sources.contains(&payload(&bytes)),
                "changed payload: {name}"
            );
            if name.ends_with(".b3dm") {
                assert_eq!(bytes.len() % 8, 0, "unaligned {name}");
                fills += 1;
            }
        }
        if name == "tileset.json" || name.starts_with("implicit-tileset-") {
            let doc: Value = serde_json::from_slice(&read(name)).unwrap();
            assert!(doc["extensionsUsed"]
                .as_array()
                .unwrap()
                .contains(&json!("3DTILES_content_gltf_vector")));
            let root = &doc["root"];
            let headers = root["contents"]
                .as_array()
                .cloned()
                .unwrap_or_else(|| vec![root["content"].clone()]);
            for content in headers {
                if content["uri"].as_str().unwrap().ends_with(".glb") {
                    assert_eq!(
                        content["extensions"]["3DTILES_content_gltf_vector"]["vector"],
                        true
                    );
                }
            }
        }
    }
    assert!(fills > 1, "exercise fragmented b3dm content");
    assert!(
        names
            .iter()
            .any(|name| name.starts_with("implicit-tileset-")),
        "exercise external implicit chunk headers"
    );
}
