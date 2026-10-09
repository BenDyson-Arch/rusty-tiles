//! Existing archive conversion, audited independently of subtree writing.
use rusty_tiles::{convert_to_implicit_reported, ConvertToImplicitOptions, Reporter};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, io::Read, path::Path, process::Command};

fn call(command: &str, input: &Path, output: &Path, args: &[&str]) {
    let result = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args([command, "-i"])
        .arg(input)
        .arg("-o")
        .arg(output)
        .args(args)
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
fn members(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    (0..zip.len())
        .map(|i| {
            let mut entry = zip.by_index(i).unwrap();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            (entry.name().to_owned(), bytes)
        })
        .collect()
}
fn document(files: &BTreeMap<String, Vec<u8>>, name: &str) -> Value {
    serde_json::from_slice(&files[name]).unwrap()
}
fn repackage(files: &BTreeMap<String, Vec<u8>>, output: &Path) {
    let tmp = tempfile::tempdir().unwrap();
    let mut paths = Vec::new();
    for (name, bytes) in files {
        if name == rusty_tiles::TZ_INDEX_NAME {
            continue;
        }
        let target = tmp.path().join(name);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, bytes).unwrap();
        paths.push((name.clone(), target));
    }
    rusty_tiles::pack_named_files(&paths, output, &Default::default()).unwrap();
}
fn convert(
    input: &Path,
    output: &Path,
    force: bool,
) -> Result<rusty_tiles::ConversionResult, rusty_tiles::Error> {
    convert_to_implicit_reported(
        input,
        output,
        &ConvertToImplicitOptions { force },
        &Reporter::silent(),
    )
}
fn export(path: &Path, name: &str) {
    if let Some(root) = std::env::var_os("RUSTY_TILES_CONVERT_IMPLICIT_FIXTURES") {
        let root = std::path::PathBuf::from(root);
        fs::create_dir_all(&root).unwrap();
        fs::copy(path, root.join(format!("{name}.3tz"))).unwrap();
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        zip::ZipArchive::new(fs::File::open(path).unwrap())
            .unwrap()
            .extract(dir)
            .unwrap();
    }
}
fn cloud(path: &Path, spread: bool) {
    let mut builder = las::Builder::from((1, 4));
    builder.point_format = las::point::Format::new(8).unwrap();
    let mut writer = las::Writer::from_path(path, builder.into_header().unwrap()).unwrap();
    for i in 0..16 {
        let base = if i < 8 { -100. } else { 100. };
        writer
            .write_point(las::Point {
                x: base + (i % 8) as f64 * 0.01,
                y: if spread {
                    (i % 8) as f64 * 20.
                } else {
                    base + (i % 8) as f64 * 0.02
                },
                z: base + (i % 8) as f64 * 0.03,
                intensity: 100 + i as u16,
                gps_time: Some(i as f64),
                classification: las::point::Classification::new(42).unwrap(),
                return_number: 1,
                number_of_returns: 1,
                nir: Some(55),
                color: Some(las::Color::new(100, 200, 300)),
                ..Default::default()
            })
            .unwrap();
    }
}
fn resolve(base: &str, uri: &str) -> String {
    let mut parts = base
        .rsplit_once('/')
        .map_or(Vec::new(), |(dir, _)| dir.split('/').collect());
    for p in uri.split('/') {
        match p {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            _ => parts.push(p),
        }
    }
    parts.join("/")
}
// Independently follow external tilesets and inspect GLB buffers. The library
// expansion decodes availability only; placement and payload audits are here.
fn audit(files: &BTreeMap<String, Vec<u8>>) -> (Vec<Vec<u8>>, Vec<[f64; 3]>) {
    fn walk(
        files: &BTreeMap<String, Vec<u8>>,
        doc: &Value,
        base: &str,
        parent: [f64; 16],
        bins: &mut Vec<Vec<u8>>,
        points: &mut Vec<[f64; 3]>,
        leaf: bool,
    ) {
        let t: [f64; 16] = doc
            .get("transform")
            .map(|v| serde_json::from_value(v.clone()).unwrap())
            .unwrap_or([
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
            ]);
        let matrix = std::array::from_fn::<_, 16, _>(|i| {
            let row = i % 4;
            let col = i / 4;
            (0..4)
                .map(|k| parent[k * 4 + row] * t[col * 4 + k])
                .sum::<f64>()
        });
        let children = doc["children"].as_array();
        let leaf = leaf && children.is_none_or(|c| c.is_empty());
        let contents: Vec<Value> = doc["contents"]
            .as_array()
            .cloned()
            .unwrap_or_else(|| doc.get("content").cloned().into_iter().collect());
        for c in contents {
            let name = resolve(base, c["uri"].as_str().unwrap());
            if name.ends_with(".json") {
                let external = document(files, &name);
                let external = rusty_tiles::implicit::expand_tileset(&external, |n| {
                    Ok(files[&resolve(&name, n)].clone())
                })
                .unwrap();
                walk(files, &external["root"], &name, matrix, bins, points, leaf);
            } else if leaf {
                let glb = gltf::Glb::from_slice(&files[&name]).unwrap();
                let doc: Value = serde_json::from_slice(&glb.json).unwrap();
                let bin = glb.bin.unwrap().into_owned();
                bins.push(bin.clone());
                for mesh in doc["meshes"].as_array().unwrap() {
                    for primitive in mesh["primitives"].as_array().unwrap() {
                        if primitive["mode"] != 0 {
                            continue;
                        }
                        let a = &doc["accessors"]
                            [primitive["attributes"]["POSITION"].as_u64().unwrap() as usize];
                        let view = &doc["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
                        let start = (view["byteOffset"].as_u64().unwrap_or(0)
                            + a["byteOffset"].as_u64().unwrap_or(0))
                            as usize;
                        let shift: [f64; 3] = doc["nodes"][0]
                            .get("translation")
                            .map(|v| serde_json::from_value(v.clone()).unwrap())
                            .unwrap_or([0.; 3]);
                        for index in 0..a["count"].as_u64().unwrap() as usize {
                            let p: [f64; 3] = std::array::from_fn(|i| {
                                f32::from_le_bytes(
                                    bin[start + index * 12 + i * 4..start + index * 12 + i * 4 + 4]
                                        .try_into()
                                        .unwrap(),
                                ) as f64
                                    + shift[i]
                            });
                            let local = [p[0], -p[2], p[1]];
                            points.push(std::array::from_fn(|i| {
                                matrix[12 + i]
                                    + (0..3).map(|k| matrix[k * 4 + i] * local[k]).sum::<f64>()
                            }));
                        }
                    }
                }
            }
        }
        if let Some(children) = children {
            for child in children {
                walk(files, child, base, matrix, bins, points, true);
            }
        }
    }
    let manifest = document(files, "tileset.json");
    let expanded =
        rusty_tiles::implicit::expand_tileset(&manifest, |n| Ok(files[n].clone())).unwrap();
    let mut bins = Vec::new();
    let mut points = Vec::new();
    walk(
        files,
        &expanded["root"],
        "tileset.json",
        [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ],
        &mut bins,
        &mut points,
        true,
    );
    bins.sort();
    points.sort_by(|a, b| {
        a[0].total_cmp(&b[0])
            .then(a[1].total_cmp(&b[1]))
            .then(a[2].total_cmp(&b[2]))
    });
    (bins, points)
}
#[test]
fn point_conversion_keeps_bytes_metadata_extras_and_world_positions() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("cloud.las");
    cloud(&input, false);
    let explicit = tmp.path().join("explicit.3tz");
    let fresh = tmp.path().join("fresh.3tz");
    let converted = tmp.path().join("converted.3tz");
    let options = [
        "--source-crs",
        "local",
        "--max-points",
        "8",
        "--metadata-attributes",
    ];
    let mut explicit_opts = options.to_vec();
    explicit_opts.push("--explicit");
    call("point-cloud", &input, &explicit, &explicit_opts);
    call("point-cloud", &input, &fresh, &options);
    let mut source = members(&explicit);
    let mut manifest = document(&source, "tileset.json");
    let lon = 153.02_f64.to_radians();
    let lat = (-27.47_f64).to_radians();
    let n = 6378137. / (1. - 0.00669437999014 * lat.sin().powi(2)).sqrt();
    let placed = json!([
        -lon.sin(),
        lon.cos(),
        0.,
        0.,
        -lat.sin() * lon.cos(),
        -lat.sin() * lon.sin(),
        lat.cos(),
        0.,
        lat.cos() * lon.cos(),
        lat.cos() * lon.sin(),
        lat.sin(),
        0.,
        (n + 100.) * lat.cos() * lon.cos(),
        (n + 100.) * lat.cos() * lon.sin(),
        (n * (1. - 0.00669437999014) + 100.) * lat.sin(),
        1.
    ]);
    manifest["root"]["transform"] = placed.clone();
    manifest["schema"] = json!({"id":"original","enums":{"label":{"valueType":"UINT8","values":[{"name":"KEEP","value":1}]}},"classes":{"user":{"properties":{"label":{"type":"ENUM","enumType":"label"}}}}});
    manifest["root"]["children"][0]["extras"] = json!({"user":{"tag":"preserved"}});
    manifest["root"]["children"][1]["extras"] = json!(["array extras", 42]);
    manifest["root"]["children"][0]["content"]["extras"] = json!({"source":"keep"});
    source.insert(
        "tileset.json".into(),
        serde_json::to_vec(&manifest).unwrap(),
    );
    let edited = tmp.path().join("edited.3tz");
    repackage(&source, &edited);
    let result = convert(&edited, &converted, false).unwrap();
    assert_eq!(result.report.unwrap()["contentBytesPreserved"], true);
    rusty_tiles::validate::inspect(rusty_tiles::validate::ValidationRequest::new(&converted))
        .unwrap();
    let target = members(&converted);
    assert_eq!(
        document(&target, "tileset.json")["schema"]["enums"],
        manifest["schema"]["enums"]
    );
    for (name, bytes) in source.iter().filter(|(n, _)| n.ends_with(".glb")) {
        assert_eq!(&target[name], bytes);
    }
    assert!(target
        .values()
        .filter_map(|v| serde_json::from_slice::<Value>(v).ok())
        .any(|doc| doc["root"]["extras"]["user"]["tag"] == "preserved"
            && doc["root"]["content"]["extras"]["source"] == "keep"));
    let original_audit = audit(&source);
    let converted_audit = audit(&target);
    let mut fresh_files = members(&fresh);
    let mut fresh_doc = document(&fresh_files, "tileset.json");
    fresh_doc["root"]["transform"] = placed;
    fresh_files.insert(
        "tileset.json".into(),
        serde_json::to_vec(&fresh_doc).unwrap(),
    );
    let fresh_audit = audit(&fresh_files);
    let placed_fresh = tmp.path().join("placed-fresh.3tz");
    repackage(&fresh_files, &placed_fresh);
    export(&edited, "point-explicit");
    export(&converted, "point-converted");
    export(&placed_fresh, "point-fresh");
    assert_eq!(original_audit.0, converted_audit.0);
    assert_eq!(original_audit.0, fresh_audit.0);
    assert_eq!(original_audit.1.len(), 16);
    for (a, b) in original_audit.1.iter().zip(converted_audit.1.iter()) {
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() < 1e-8);
        }
    }
    for (a, b) in original_audit.1.iter().zip(fresh_audit.1.iter()) {
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() < 1e-5);
        }
    }
    let original = fs::read(&converted).unwrap();
    assert!(matches!(
        convert(&edited, &converted, false),
        Err(rusty_tiles::Error::OutputExists(_))
    ));
    assert_eq!(fs::read(&converted).unwrap(), original);
    convert(&edited, &converted, true).unwrap();
    assert_eq!(fs::read(&converted).unwrap(), original);
    assert!(convert(&converted, &tmp.path().join("again.3tz"), false)
        .unwrap_err()
        .to_string()
        .contains("already implicit"));
}
#[test]
fn vector_conversion_preserves_immutable_content_and_decoded_leaf_buffers() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("points.geojson");
    let features:Vec<_>=(0..8).map(|i| {let base=if i<4 {12.} else {12.1};json!({"type":"Feature","properties":{"name":format!("point-{i}"),"value":i},"geometry":{"type":"Point","coordinates":[base+i as f64*0.00001,base+30.+i as f64*0.00001,100.]}})}).collect();
    fs::write(
        &input,
        serde_json::to_vec(&json!({"type":"FeatureCollection","features":features})).unwrap(),
    )
    .unwrap();
    let explicit = tmp.path().join("explicit.3tz");
    let fresh = tmp.path().join("fresh.3tz");
    let output = tmp.path().join("implicit.3tz");
    let options = [
        "--max-features",
        "4",
        "--max-parent-features",
        "4",
        "--lod-levels",
        "1",
        "--height-offset",
        "0",
        "--source-crs",
        "EPSG:4326",
        "--reproducible",
    ];
    let mut explicit_options = options.to_vec();
    explicit_options.push("--explicit");
    call("vector", &input, &explicit, &explicit_options);
    call("vector", &input, &fresh, &options);
    convert(&explicit, &output, false).unwrap();
    export(&explicit, "vector-explicit");
    export(&output, "vector-converted");
    export(&fresh, "vector-fresh");
    rusty_tiles::validate::inspect(rusty_tiles::validate::ValidationRequest::new(&output)).unwrap();
    let source = members(&explicit);
    let target = members(&output);
    for (name, bytes) in source.iter().filter(|(n, _)| n.starts_with("t/")) {
        assert_eq!(&target[name], bytes);
    }
    assert_eq!(
        document(&target, "tileset.json")["root"]["transform"],
        document(&source, "tileset.json")["root"]["transform"]
    );
    assert_eq!(audit(&source).0, audit(&target).0);
    assert_eq!(audit(&target).0, audit(&members(&fresh)).0);
}
#[test]
fn irregular_foreign_mesh_and_corrupt_archives_refuse_without_publication() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("cloud.las");
    cloud(&input, true);
    let explicit = tmp.path().join("explicit.3tz");
    call(
        "point-cloud",
        &input,
        &explicit,
        &["--explicit", "--source-crs", "local", "--max-points", "8"],
    );
    let output = tmp.path().join("result.3tz");
    fs::write(&output, b"keep").unwrap();
    assert!(convert(&explicit, &output, true)
        .unwrap_err()
        .to_string()
        .contains("irregular explicit hierarchy"));
    assert_eq!(fs::read(&output).unwrap(), b"keep");
    cloud(&input, false);
    fs::remove_file(&explicit).unwrap();
    call(
        "point-cloud",
        &input,
        &explicit,
        &["--explicit", "--source-crs", "local", "--max-points", "8"],
    );
    let source = members(&explicit);
    for (label, needle) in [
        ("foreign", "foreign"),
        ("mesh", "mesh-to-3tz"),
        ("rotation", "child transform"),
        ("index", "index"),
        ("missing", "missing archive entry"),
        ("tile-metadata", "explicit tile metadata"),
        ("content-bounds", "content bounding volumes"),
    ] {
        let mut files = source.clone();
        let mut manifest = document(&files, "tileset.json");
        match label {
            "tile-metadata" => {
                manifest["root"]["children"][0]["metadata"] =
                    json!({"class":"user","properties":{"label":"KEEP"}});
            }
            "content-bounds" => {
                manifest["root"]["children"][0]["content"]["boundingVolume"] =
                    manifest["root"]["children"][0]["boundingVolume"].clone();
            }
            "foreign" => {
                files.remove("conversion.json");
            }
            "mesh" => {
                manifest["asset"]["generator"] = json!("rusty-tiles");
                files.insert("conversion.json".into(), b"{}".to_vec());
            }
            "rotation" => {
                manifest["root"]["children"][0]["transform"][0] = json!(2.);
            }
            "index" => {
                manifest["root"]["children"][0]["extras"]["implicitChildIndex"] = json!(999);
            }
            "missing" => {
                files.remove("t/1.glb");
            }
            _ => unreachable!(),
        }
        files.insert(
            "tileset.json".into(),
            serde_json::to_vec(&manifest).unwrap(),
        );
        let case = tmp.path().join(format!("{label}.3tz"));
        repackage(&files, &case);
        let error = convert(&case, &output, true).unwrap_err().to_string();
        assert!(error.contains(needle), "{label}: {error}");
        assert_eq!(fs::read(&output).unwrap(), b"keep");
    }
}

#[test]
fn deep_regular_cells_cross_subtrees_and_reserved_paths_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("diagonal.las");
    let mut writer =
        las::Writer::from_path(&input, las::Builder::from((1, 2)).into_header().unwrap()).unwrap();
    for i in 0..32 {
        writer
            .write_point(las::Point {
                x: i as f64,
                y: i as f64,
                z: i as f64,
                return_number: 1,
                number_of_returns: 1,
                ..Default::default()
            })
            .unwrap();
    }
    drop(writer);
    let explicit = tmp.path().join("explicit.3tz");
    let output = tmp.path().join("converted.3tz");
    call(
        "point-cloud",
        &input,
        &explicit,
        &["--explicit", "--source-crs", "local", "--max-points", "1"],
    );
    convert(&explicit, &output, false).unwrap();
    let source = members(&explicit);
    let target = members(&output);
    assert!(target.keys().filter(|n| n.ends_with(".subtree")).count() > 1);
    assert_eq!(audit(&source), audit(&target));
    for (name, needle) in [
        ("implicit-source-0.json", "reserved"),
        ("schemaUri", "schemaUri"),
        ("duplicate-cell", "same regular cell"),
    ] {
        let mut files = source.clone();
        let mut manifest = document(&files, "tileset.json");
        match name {
            "schemaUri" => {
                manifest["schemaUri"] = json!("schema.json");
            }
            "duplicate-cell" => {
                manifest["root"]["children"][1] = manifest["root"]["children"][0].clone();
            }
            _ => {
                files.insert(name.into(), b"{}".to_vec());
            }
        }
        files.insert(
            "tileset.json".into(),
            serde_json::to_vec(&manifest).unwrap(),
        );
        let bad = tmp.path().join(format!("{name}.3tz"));
        repackage(&files, &bad);
        let error = convert(&bad, &output, true).unwrap_err().to_string();
        assert!(error.contains(needle), "{name}: {error}");
        assert_eq!(members(&output), target);
    }
}

#[test]
fn padded_vector_cells_use_bounded_external_subtree_roots() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("flat.geojson");
    let features:Vec<_>=(0..32).map(|i|json!({"type":"Feature","properties":{"id":i},"geometry":{"type":"Point","coordinates":[i,0,0]}})).collect();
    fs::write(
        &input,
        serde_json::to_vec(&json!({"type":"FeatureCollection","features":features})).unwrap(),
    )
    .unwrap();
    let explicit = tmp.path().join("explicit.3tz");
    let output = tmp.path().join("converted.3tz");
    call(
        "vector",
        &input,
        &explicit,
        &[
            "--explicit",
            "--source-crs",
            "local",
            "--max-features",
            "1",
            "--max-parent-features",
            "1",
            "--lod-levels",
            "1",
            "--reproducible",
        ],
    );
    convert(&explicit, &output, false).unwrap();
    let source = members(&explicit);
    let target = members(&output);
    assert!(target.keys().any(|n| n.starts_with("implicit-owned-")));
    assert_eq!(audit(&source).0, audit(&target).0);
    rusty_tiles::validate::inspect(rusty_tiles::validate::ValidationRequest::new(&output)).unwrap();
}

#[test]
fn fragmented_quantized_compressed_vector_content_arrays_keep_glb_and_b3dm_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("polygon.geojson");
    let ring = |radius: f64, count: usize| {
        (0..=count)
            .map(|i| {
                let a = (i % count) as f64 * std::f64::consts::TAU / count as f64;
                json!([radius * a.cos(), radius * a.sin(), 0])
            })
            .collect::<Vec<_>>()
    };
    fs::write(&input,json!({"type":"FeatureCollection","features":[{"type":"Feature","properties":{"name":"hole","precise":1152921504606846979i64},"geometry":{"type":"Polygon","coordinates":[ring(20.,40),ring(5.,20)]}}]}).to_string()).unwrap();
    let full = tmp.path().join("full.3tz");
    call(
        "vector",
        &input,
        &full,
        &[
            "--explicit",
            "--source-crs",
            "local",
            "--max-vertices",
            "32",
            "--max-bytes",
            "16384",
            "--lod-tolerance",
            "0.2",
            "--lod-levels",
            "3",
            "--quantize",
            "--meshopt",
            "--reproducible",
        ],
    );
    let mut files = members(&full);
    let mut manifest = document(&files, "tileset.json");
    fn leaf(node: &Value, files: &BTreeMap<String, Vec<u8>>) -> Option<Value> {
        if let Some(children) = node["children"].as_array() {
            for child in children {
                if let Some(node) = leaf(child, files) {
                    return Some(node);
                }
            }
        } else if node["contents"].as_array().is_some_and(|c| {
            c.len() == 2
                && c.iter().all(|v| {
                    !v["uri"].as_str().unwrap().ends_with(".b3dm")
                        || files[v["uri"].as_str().unwrap()].len().is_multiple_of(8)
                })
        }) {
            return Some(node.clone());
        }
        None
    }
    // A selected original converter leaf is already one regular root. Its
    // fragment payloads exercise the original two-slot GLB/b3dm declarations.
    let mut root = leaf(&manifest["root"], &files)
        .expect("fragmented source has an aligned mixed-content leaf");
    root["transform"] =
        json!([1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 100., 200., 300., 1.]);
    for (index, content) in root["contents"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        content["extras"] = json!({"slot":index,"keep":[true,"metadata"]});
    }
    manifest["root"] = root.clone();
    manifest["geometricError"] = root["geometricError"].clone();
    let uris: Vec<_> = root["contents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["uri"].as_str().unwrap().to_owned())
        .collect();
    files.retain(|name, _| !name.starts_with("t/") || uris.contains(name));
    files.insert(
        "tileset.json".into(),
        serde_json::to_vec(&manifest).unwrap(),
    );
    let explicit = tmp.path().join("leaf.3tz");
    repackage(&files, &explicit);
    let output = tmp.path().join("converted.3tz");
    convert(&explicit, &output, false).unwrap();
    let converted = members(&output);
    for uri in &uris {
        assert_eq!(files[uri], converted[uri]);
    }
    assert!(uris.iter().any(|n| n.ends_with(".b3dm")));
    assert!(uris.iter().any(|n| n.ends_with(".glb")));
    let wrapper = document(&converted, "tileset.json");
    let mut actual = wrapper["root"]["contents"].clone();
    for (content, original) in actual
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .zip(root["contents"].as_array().unwrap())
    {
        content["uri"] = original["uri"].clone();
    }
    assert_eq!(actual, root["contents"]);
    assert_eq!(wrapper["root"]["extras"], root["extras"]);
    for uri in &uris {
        let bytes = &files[uri];
        let offset = if bytes.starts_with(b"b3dm") {
            (3..7).fold(28, |n, i| {
                n + u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap()) as usize
            })
        } else {
            0
        };
        let glb = gltf::Glb::from_slice(&bytes[offset..]).unwrap();
        let doc: Value = serde_json::from_slice(&glb.json).unwrap();
        assert!(doc["extensionsUsed"]
            .as_array()
            .unwrap()
            .contains(&json!("KHR_mesh_quantization")));
        assert!(doc["extensionsUsed"]
            .as_array()
            .unwrap()
            .contains(&json!("EXT_meshopt_compression")));
    }
    rusty_tiles::validate::inspect(rusty_tiles::validate::ValidationRequest::new(&output)).unwrap();
    export(&explicit, "vector-fragment-explicit");
    export(&output, "vector-fragment-converted");
    // Deliberately reproduce the older explicit encoder's legal GLB framing
    // inside a b3dm whose total length is only four-byte aligned. Preserve all
    // framing/checksums so refusal specifically diagnoses byte-preserving
    // migration of malformed source content, rather than unrelated corruption.
    use sha2::Digest;
    let old_uri = uris.iter().find(|u| u.ends_with(".b3dm")).unwrap();
    let mut bytes = files.remove(old_uri).unwrap();
    let offset = (3..7).fold(28, |n, i| {
        n + u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap()) as usize
    });
    let json_length = u32::from_le_bytes(bytes[offset + 12..offset + 16].try_into().unwrap());
    bytes.splice(
        offset + 20 + json_length as usize..offset + 20 + json_length as usize,
        [b' '; 4],
    );
    let total = bytes.len() as u32;
    bytes[8..12].copy_from_slice(&total.to_le_bytes());
    bytes[offset + 8..offset + 12].copy_from_slice(&(total - offset as u32).to_le_bytes());
    bytes[offset + 12..offset + 16].copy_from_slice(&(json_length + 4).to_le_bytes());
    let new_uri = format!("t/{:x}.b3dm", sha2::Sha256::digest(&bytes));
    files.insert(new_uri.clone(), bytes);
    for c in manifest["root"]["contents"].as_array_mut().unwrap() {
        if c["uri"] == *old_uri {
            c["uri"] = json!(new_uri);
        }
    }
    manifest["root"]["extras"]["encodedBytes"] =
        json!(root["extras"]["encodedBytes"].as_u64().unwrap() + 4);
    files.insert(
        "tileset.json".into(),
        serde_json::to_vec(&manifest).unwrap(),
    );
    let malformed = tmp.path().join("unaligned.3tz");
    repackage(&files, &malformed);
    let original_output = fs::read(&output).unwrap();
    assert!(convert(&malformed, &output, true)
        .unwrap_err()
        .to_string()
        .contains("b3dm length/alignment invalid"));
    assert_eq!(fs::read(&output).unwrap(), original_output);
}

#[test]
fn owned_roots_have_compliant_templates_and_only_terminal_external_links() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("source.las");
    cloud(&source, false);
    let explicit = tmp.path().join("explicit.3tz");
    call(
        "point-cloud",
        &source,
        &explicit,
        &["--explicit", "--source-crs", "local", "--max-points", "8"],
    );
    let output = tmp.path().join("implicit.3tz");
    convert(&explicit, &output, false).unwrap();
    let files = members(&output);
    let mut documents = 0;
    for (name, bytes) in files
        .iter()
        .filter(|(name, _)| name.as_str() == "tileset.json" || name.starts_with("implicit-owned-"))
    {
        let doc: Value = serde_json::from_slice(bytes).unwrap();
        let root = &doc["root"];
        assert!(root.get("metadata").is_none());
        assert!(root.get("children").is_none());
        let templates = root["contents"]
            .as_array()
            .cloned()
            .unwrap_or_else(|| root.get("content").cloned().into_iter().collect());
        let subtree_template = root["implicitTiling"]["subtrees"]["uri"].as_str().unwrap();
        for uri in templates
            .iter()
            .map(|c| c["uri"].as_str().unwrap())
            .chain(std::iter::once(subtree_template))
        {
            for variable in ["{level}", "{x}", "{y}", "{z}"] {
                assert!(uri.contains(variable), "{name}: {uri}");
            }
        }
        let actual_subtree = subtree_template
            .replace("{level}", "0")
            .replace("{x}", "0")
            .replace("{y}", "0")
            .replace("{z}", "0");
        let subtree = &files[&actual_subtree];
        let json_len = u64::from_le_bytes(subtree[8..16].try_into().unwrap()) as usize;
        let header: Value = serde_json::from_slice(&subtree[24..24 + json_len]).unwrap();
        let binary = &subtree[24 + json_len..];
        let bit = |availability: &Value, index: usize| -> bool {
            if let Some(constant) = availability["constant"].as_u64() {
                return constant == 1;
            }
            let view = &header["bufferViews"][availability["bitstream"].as_u64().unwrap() as usize];
            let offset = view["byteOffset"].as_u64().unwrap_or(0) as usize;
            binary[offset + index / 8] & (1 << (index % 8)) != 0
        };
        assert_eq!(header["childSubtreeAvailability"]["constant"], 0);
        for (slot, template) in templates.iter().enumerate() {
            assert!(template.get("boundingVolume").is_none());
            let availability = &header["contentAvailability"][slot];
            if template["uri"].as_str().unwrap().ends_with(".json") {
                // The root has no external tileset content. Its only actual
                // external references are terminal level-one child cells.
                assert!(!bit(availability, 0));
            } else {
                assert!(bit(availability, 0));
                for index in 1..9 {
                    if root["implicitTiling"]["subtreeLevels"] == 2 {
                        assert!(!bit(availability, index));
                    }
                }
            }
        }
        let expanded =
            rusty_tiles::implicit::expand_tileset(&doc, |uri| Ok(files[uri].clone())).unwrap();
        for content in expanded["root"]["contents"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(expanded["root"].get("content"))
        {
            assert!(!content["uri"].as_str().unwrap().ends_with(".json"));
        }
        for child in expanded["root"]["children"]
            .as_array()
            .into_iter()
            .flatten()
        {
            assert!(child.get("children").is_none());
            assert!(child["content"]["uri"].as_str().unwrap().ends_with(".json"));
        }
        documents += 1;
    }
    assert_eq!(documents, 3);
}

#[test]
fn nested_payload_aliases_preserve_relative_metadata_schema_resources() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("source.las");
    cloud(&input, false);
    let explicit = tmp.path().join("explicit.3tz");
    call(
        "point-cloud",
        &input,
        &explicit,
        &[
            "--explicit",
            "--source-crs",
            "local",
            "--max-points",
            "8",
            "--metadata-attributes",
        ],
    );
    let mut files = members(&explicit);
    let mut manifest = document(&files, "tileset.json");
    let glbs: Vec<_> = files
        .keys()
        .filter(|n| n.ends_with(".glb"))
        .cloned()
        .collect();
    for uri in glbs {
        let bytes = files.remove(&uri).unwrap();
        let mut glb = gltf::Glb::from_slice(&bytes).unwrap();
        let mut doc: Value = serde_json::from_slice(&glb.json).unwrap();
        let metadata = &mut doc["extensions"]["EXT_structural_metadata"];
        let schema = metadata.as_object_mut().unwrap().remove("schema").unwrap();
        metadata["schemaUri"] = json!("schema.json");
        glb.json = std::borrow::Cow::Owned(serde_json::to_vec(&doc).unwrap());
        let encoded = glb.to_vec().unwrap();
        let new_uri = uri.replace("t/", "t/nested/");
        files.insert(new_uri.clone(), encoded);
        files.insert(
            "t/nested/schema.json".into(),
            serde_json::to_vec(&schema).unwrap(),
        );
        fn replace(node: &mut Value, old: &str, new: &str) {
            if node["content"]["uri"] == old {
                node["content"]["uri"] = json!(new);
            }
            if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
                for child in children {
                    replace(child, old, new);
                }
            }
        }
        replace(&mut manifest["root"], &uri, &new_uri);
    }
    files.insert(
        "tileset.json".into(),
        serde_json::to_vec(&manifest).unwrap(),
    );
    let nested = tmp.path().join("nested.3tz");
    repackage(&files, &nested);
    let output = tmp.path().join("converted.3tz");
    convert(&nested, &output, false).unwrap();
    let target = members(&output);
    assert!(target
        .keys()
        .any(|name| name.starts_with("t/nested/owned-")));
    for (name, bytes) in files
        .iter()
        .filter(|(name, _)| name.starts_with("t/nested/"))
    {
        assert_eq!(&target[name], bytes);
    }
    rusty_tiles::validate::inspect(rusty_tiles::validate::ValidationRequest::new(&output)).unwrap();
}
