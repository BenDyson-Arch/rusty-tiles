//! Complete native archive checks for the bounded implicit forest: explicit
//! skeleton tiles, links to explicit external documents, and implicit roots
//! with one subtree each, whose rows keep tight boxes that cross regular cells.
mod support;
use serde_json::{json, Value};
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

fn member(path: &Path, name: &str) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut bytes = Vec::new();
    zip.by_name(name).unwrap().read_to_end(&mut bytes).unwrap();
    bytes
}

fn templates(tile: &Value) -> Vec<Value> {
    tile["contents"]
        .as_array()
        .cloned()
        .unwrap_or_else(|| tile.get("content").cloned().into_iter().collect())
}

/// The written forest, read from the archive without the product expander.
#[derive(Default)]
struct Forest {
    /// Explicit tiles of the producer hierarchy; a link and its document root
    /// count once.
    explicit: usize,
    /// Every implicit root with the document that holds it.
    roots: Vec<(String, Value)>,
    documents: Vec<String>,
}

/// Walk every document and check the forest's root, link and subtree rules:
/// one subtree per implicit root with `subtreeLevels == availableLevels <= 4`,
/// no child subtrees, a row for every tile, no JSON content template, the
/// metadata schema on every document holding a root and on `tileset.json`,
/// no `implicitChildIndex`, and every tileset document reached by one link.
fn forest(path: &Path, scheme: &str) -> Forest {
    fn walk(path: &Path, scheme: &str, name: &str, doc: &Value, tile: &Value, out: &mut Forest) {
        assert!(tile["extras"].get("implicitChildIndex").is_none());
        if let Some(tiling) = tile.get("implicitTiling") {
            assert!(doc.get("schema").is_some(), "{name}: root without schema");
            assert!(tile.get("children").is_none());
            assert_eq!(tiling["subdivisionScheme"], scheme);
            let levels = tiling["subtreeLevels"].as_u64().unwrap();
            assert_eq!(tiling["availableLevels"].as_u64(), Some(levels));
            assert!((1..=4).contains(&levels), "{levels} subtree levels");
            for template in templates(tile) {
                let uri = template["uri"].as_str().unwrap();
                assert!(!uri.ends_with(".json"), "JSON content template {uri}");
            }
            let subtree = ["level", "x", "y", "z"].iter().fold(
                tiling["subtrees"]["uri"].as_str().unwrap().to_owned(),
                |uri, key| uri.replace(&format!("{{{key}}}"), "0"),
            );
            let bytes = member(path, &subtree);
            assert_eq!(&bytes[..4], b"subt");
            let length = u64::from_le_bytes(bytes[8..16].try_into().unwrap()) as usize;
            let subtree: Value = serde_json::from_slice(&bytes[24..24 + length]).unwrap();
            assert_eq!(subtree["childSubtreeAvailability"], json!({"constant": 0}));
            assert!(subtree["tileMetadata"].is_u64(), "missing tile rows");
            out.roots.push((name.to_owned(), tile.clone()));
            return;
        }
        let uri = tile["content"]["uri"].as_str().unwrap_or_default();
        if uri.ends_with(".json") {
            assert!(tile.get("children").is_none());
            let linked = document(path, uri);
            out.documents.push(uri.to_owned());
            assert!(linked["root"].get("transform").is_none());
            assert_eq!(linked["root"]["boundingVolume"], tile["boundingVolume"]);
            assert_eq!(linked["root"]["geometricError"], tile["geometricError"]);
            return walk(path, scheme, uri, &linked, &linked["root"], out);
        }
        out.explicit += 1;
        for child in tile["children"].as_array().into_iter().flatten() {
            walk(path, scheme, name, doc, child, out);
        }
    }
    let manifest = document(path, "tileset.json");
    let mut out = Forest {
        documents: vec!["tileset.json".into()],
        ..Default::default()
    };
    walk(
        path,
        scheme,
        "tileset.json",
        &manifest,
        &manifest["root"],
        &mut out,
    );
    assert_eq!(manifest.get("schema").is_some(), !out.roots.is_empty());
    let zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let subtrees = zip.file_names().filter(|n| n.ends_with(".subtree")).count();
    assert_eq!(subtrees, out.roots.len(), "one subtree per implicit root");
    let mut documents: Vec<_> = zip
        .file_names()
        // Reports and vector build state are not tileset documents.
        .filter(|n| n.ends_with(".json") && !["conversion.json", "vector-build.json"].contains(n))
        .map(str::to_owned)
        .collect();
    documents.sort();
    let mut linked = out.documents.clone();
    linked.sort();
    assert_eq!(documents, linked, "tileset documents and links");
    out
}

/// The first implicit root strictly below an explicit tile of `tile`.
fn nested_root(tile: &mut Value) -> Option<&mut Value> {
    let children = tile.get_mut("children")?.as_array_mut()?;
    let index = children
        .iter()
        .position(|child| child.get("implicitTiling").is_some());
    match index {
        Some(index) => Some(&mut children[index]),
        None => children.iter_mut().find_map(nested_root),
    }
}

#[test]
fn deep_implicit_mesh_validates_and_corrupt_subtrees_roots_and_templates_are_rejected() {
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
    // The mesh is deeper than the skeleton bound: the root is explicit and the
    // implicit roots hang below it, each with a single subtree.
    let manifest = document(&output, "tileset.json");
    assert!(manifest["root"].get("implicitTiling").is_none());
    let forest = forest(&output, "OCTREE");
    assert!(forest.explicit >= 1 && forest.roots.len() > 1);
    let report =
        rusty_tiles::validate::inspect(rusty_tiles::validate::ValidationRequest::new(&output))
            .unwrap();
    assert!(report.tiles > 100);
    let mut zip = zip::ZipArchive::new(fs::File::open(&output).unwrap()).unwrap();
    for case in [
        "header",
        "availability",
        "metadata",
        "missing",
        "implicit-root",
        "template",
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
        let path = directory.join("tileset.json");
        let mut doc: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        match case {
            "root" => {
                doc["root"]["boundingVolume"]["box"][0] = json!(1000000.);
                fs::write(path, serde_json::to_vec(&doc).unwrap()).unwrap();
            }
            "header" => {
                bytes[0] = 0;
                fs::write(&subtree, bytes).unwrap();
            }
            "missing" => fs::remove_file(subtree).unwrap(),
            // An implicit root's explicit tile must stay coherent with its
            // explicit parent, as an external root had to with its link.
            "implicit-root" => {
                nested_root(&mut doc["root"]).unwrap()["geometricError"] = json!(999999.);
                fs::write(path, serde_json::to_vec(&doc).unwrap()).unwrap();
            }
            // P1 sensitivity: a JSON content template on an implicit root.
            "template" => {
                let root = nested_root(&mut doc["root"]).unwrap();
                let content = if root.get("contents").is_some() {
                    &mut root["contents"][0]
                } else {
                    &mut root["content"]
                };
                let uri = content["uri"].as_str().unwrap();
                content["uri"] = json!(format!("{}.json", uri.rsplit_once('.').unwrap().0));
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
        rusty_tiles::package::package(
            rusty_tiles::package::PackageRequest::directory(&directory, &damaged),
            &rusty_tiles::RunControl::default(),
        )
        .unwrap();
        let failure =
            rusty_tiles::validate::inspect(rusty_tiles::validate::ValidationRequest::new(&damaged))
                .expect_err(case);
        if case == "template" {
            assert!(
                failure.to_string().contains("content template"),
                "{failure}"
            );
        }
    }
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
    let convert = |name: &str, explicit: bool| {
        let output = work.path().join(name);
        let mut command = support::rusty_tiles();
        command
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
            ]);
        if explicit {
            command.arg("--explicit");
        }
        success(command.output().unwrap());
        output
    };
    let output = convert("polygon.3tz", false);
    // The explicit archive publishes every raw payload once, in place. It is
    // the reference now that forest archives retain no raw sources.
    let reference = convert("explicit.3tz", true);
    rusty_tiles::validate::inspect(rusty_tiles::validate::ValidationRequest::new(&output)).unwrap();
    let forest = forest(&output, "QUADTREE");
    fn payload(bytes: &[u8]) -> (Vec<u8>, String) {
        let glb = gltf::Glb::from_slice(bytes).unwrap();
        let mut json: Value = serde_json::from_slice(&glb.json).unwrap();
        // Only scene node placement may change; attributes, textures, feature
        // metadata, compression and all binary buffer bytes must stay exact.
        json.as_object_mut().unwrap().remove("nodes");
        (glb.bin.unwrap_or_default().into_owned(), json.to_string())
    }
    let payloads = |path: &Path| {
        let zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
        zip.file_names()
            .inspect(|name| assert!(!name.ends_with(".b3dm"), "legacy fill {name}"))
            .filter(|name| name.ends_with(".glb"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let sources: BTreeSet<_> = payloads(&reference)
        .iter()
        .map(|name| payload(&member(&reference, name)))
        .collect();
    let mut published = BTreeSet::new();
    let mut fills = 0;
    for name in payloads(&output) {
        let bytes = member(&output, &name);
        let decoded = payload(&bytes);
        assert!(sources.contains(&decoded), "changed payload: {name}");
        published.insert(decoded);
        assert_eq!(bytes.len() % 8, 0, "unaligned {name}");
        // A fragmented fill is one EXT_mesh_polygon polygon per triangle.
        let glb = gltf::Glb::from_slice(&bytes).unwrap();
        let json: Value = serde_json::from_slice(&glb.json).unwrap();
        for primitive in json["meshes"][0]["primitives"].as_array().unwrap() {
            let polygon = &primitive["extensions"]["EXT_mesh_polygon"];
            let indices = &json["accessors"][primitive["indices"].as_u64().unwrap() as usize];
            if polygon["count"]
                .as_u64()
                .is_some_and(|count| count > 1 && count * 3 == indices["count"].as_u64().unwrap())
            {
                fills += 1;
            }
        }
    }
    assert_eq!(published, sources, "every raw payload is published once");
    // Every document carries the extension, and every content header that
    // names a GLB keeps it: explicit tiles and implicit-root templates alike.
    fn headers(tile: &Value) {
        for content in templates(tile) {
            if content["uri"].as_str().unwrap().ends_with(".glb") {
                assert_eq!(
                    content["extensions"]["3DTILES_content_gltf_vector"]["vector"],
                    true
                );
            }
        }
        for child in tile["children"].as_array().into_iter().flatten() {
            headers(child);
        }
    }
    for name in &forest.documents {
        let doc = document(&output, name);
        assert!(doc["extensionsUsed"]
            .as_array()
            .unwrap()
            .contains(&json!("3DTILES_content_gltf_vector")));
        headers(&doc["root"]);
    }
    assert!(fills > 1, "exercise fragmented glTF fill content");
    // Headers are checked on implicit roots below the skeleton, where the
    // chunked layout used to put its external implicit roots.
    assert!(
        forest.explicit >= 1 && forest.roots.len() > 1,
        "exercise implicit-root templates below explicit tiles"
    );
}

#[test]
fn implicit_mesh_accepts_case_insensitive_extensions() {
    for extension in ["GLB", "GlB", "gLtF"] {
        // Keep variants separate on case-insensitive filesystems.
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join(format!("mesh.{extension}"));
        let source = if extension == "gLtF" {
            fs::read("tests/fixtures/example.gltf").unwrap()
        } else {
            support::textured_grid_glb(2)
        };
        fs::write(&input, &source).unwrap();
        let output = work.path().join(format!("{extension}.3tz"));
        success(
            support::rusty_tiles()
                .args(["mesh-to-3tz", "-i"])
                .arg(&input)
                .arg("-o")
                .arg(&output)
                .args(["--maxTriangles", "10000"])
                .output()
                .unwrap(),
        );
        rusty_tiles::validate::inspect(rusty_tiles::validate::ValidationRequest::new(&output))
            .unwrap();
        let manifest = document(&output, "tileset.json");
        let suffix = extension.to_ascii_lowercase();
        // The wrapper is a single one-level implicit root at preorder index 0.
        let forest = forest(&output, "OCTREE");
        assert_eq!((forest.explicit, forest.roots.len()), (0, 1));
        assert_eq!(manifest["root"]["implicitTiling"]["subtreeLevels"], 1);
        assert_eq!(
            manifest["root"]["content"]["uri"],
            format!("implicit-content/r0-{{level}}-{{x}}-{{y}}-{{z}}-0.{suffix}")
        );
        let payload = member(&output, &format!("implicit-content/r0-0-0-0-0-0.{suffix}"));
        if suffix == "glb" {
            assert_eq!(
                gltf::Glb::from_slice(&payload).unwrap().bin,
                gltf::Glb::from_slice(&source).unwrap().bin
            );
        } else {
            assert_eq!(payload, source);
        }
    }
}
