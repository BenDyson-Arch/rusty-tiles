//! End-to-end default-build coverage using independent GeoJSON and SQLite/WKB fixtures.
#![cfg(not(feature = "native-geospatial"))]

use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, io::Read, path::Path, process::Command};

const WGS84: &str = r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433],AUTHORITY["EPSG","4326"]]"#;

fn geojson(path: &Path, features: Vec<Value>) {
    fs::write(
        path,
        json!({"type":"FeatureCollection","name":"invented","features":features}).to_string(),
    )
    .unwrap();
}

fn point(id: Value, properties: Value, xyz: [f64; 3]) -> Value {
    json!({"type":"Feature","id":id,"properties":properties,"geometry":{"type":"Point","coordinates":xyz}})
}

fn polygon(id: &str, rings: Value) -> Value {
    json!({"type":"Feature","id":id,"properties":{"name":id},"geometry":{"type":"Polygon","coordinates":rings}})
}

fn run(input: &Path, output: &Path, arguments: &[&str]) -> (std::process::Output, Value) {
    let result = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["vector", "-i"])
        .arg(input)
        .arg("-o")
        .arg(output)
        .args(["--json", "--reproducible"])
        .args(arguments)
        .env("PATH", "")
        .env("PYTHONPATH", "/unavailable")
        .env("PROJ_DATA", "/unavailable")
        .env("PROJ_LIB", "/unavailable")
        .output()
        .unwrap();
    let value = serde_json::from_slice(&result.stdout).unwrap_or_else(|_| {
        panic!(
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        )
    });
    (result, value)
}

fn convert(input: &Path, output: &Path, arguments: &[&str]) -> Value {
    let (result, report) = run(input, output, arguments);
    assert!(
        result.status.success(),
        "{report}\n{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(report["ok"], true);
    assert_eq!(
        rusty_tiles::validate::archive(output, None).unwrap()["ok"],
        true
    );
    archive_json(output, "conversion.json")
}

fn archive_json(path: &Path, name: &str) -> Value {
    let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    serde_json::from_reader(zip.by_name(name).unwrap()).unwrap()
}

fn reports(path: &Path) -> Vec<Value> {
    let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut text = String::new();
    zip.by_name("geometry-reports.jsonl")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn glbs(path: &Path) -> Vec<(Value, Vec<u8>)> {
    let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut content = Vec::new();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).unwrap();
        if !entry.name().ends_with(".glb") {
            continue;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        assert_eq!(&bytes[..4], b"glTF");
        let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let document = serde_json::from_slice(&bytes[20..20 + length]).unwrap();
        content.push((document, bytes[28 + length..].to_vec()));
    }
    content
}

fn view<'a>(document: &Value, binary: &'a [u8], index: &Value) -> &'a [u8] {
    let v = &document["bufferViews"][index.as_u64().unwrap() as usize];
    let start = v["byteOffset"].as_u64().unwrap_or(0) as usize;
    &binary[start..start + v["byteLength"].as_u64().unwrap() as usize]
}

/// Decode standard property-table columns independently of the Rust encoder.
fn metadata(path: &Path) -> BTreeMap<String, Value> {
    let mut rows = BTreeMap::new();
    for (document, binary) in glbs(path) {
        let metadata = &document["extensions"]["EXT_structural_metadata"];
        for table in metadata["propertyTables"].as_array().unwrap() {
            let schema =
                &metadata["schema"]["classes"][table["class"].as_str().unwrap()]["properties"];
            for row in 0..table["count"].as_u64().unwrap() as usize {
                let mut values = serde_json::Map::new();
                for (key, column) in table["properties"].as_object().unwrap() {
                    let definition = &schema[key];
                    let bytes = view(&document, &binary, &column["values"]);
                    let mut value = match definition["type"].as_str().unwrap() {
                        "STRING" => {
                            let offsets = view(&document, &binary, &column["stringOffsets"]);
                            let start = u32::from_le_bytes(
                                offsets[row * 4..row * 4 + 4].try_into().unwrap(),
                            ) as usize;
                            let end = u32::from_le_bytes(
                                offsets[row * 4 + 4..row * 4 + 8].try_into().unwrap(),
                            ) as usize;
                            json!(std::str::from_utf8(&bytes[start..end]).unwrap())
                        }
                        "BOOLEAN" => json!(bytes[row / 8] & (1 << (row % 8)) != 0),
                        "SCALAR" if definition["componentType"] == "INT64" => json!(
                            i64::from_le_bytes(bytes[row * 8..row * 8 + 8].try_into().unwrap())
                        ),
                        "SCALAR" if definition["componentType"] == "FLOAT64" => json!(
                            f64::from_le_bytes(bytes[row * 8..row * 8 + 8].try_into().unwrap())
                        ),
                        _ => panic!("unexpected metadata type: {definition}"),
                    };
                    if definition.get("noData") == Some(&value) {
                        value = Value::Null;
                    }
                    values.insert(key.clone(), value);
                }
                if let Some(id) = values.get("_source_id").and_then(Value::as_str) {
                    let identity = format!("{}:{id}", values["_source_layer"].as_str().unwrap());
                    let row = Value::Object(values);
                    if let Some(previous) = rows.insert(identity, row.clone()) {
                        assert_eq!(previous, row);
                    }
                }
            }
        }
    }
    rows
}

/// GeoPackage header + ISO WKB Point, generated without GDAL or the reader.
fn gpkg_point(xyz: [f64; 3], z: bool) -> Vec<u8> {
    let mut bytes = b"GP\0\x01".to_vec(); // little-endian, standard geometry, no envelope
    bytes.extend(4326i32.to_le_bytes());
    bytes.push(1);
    bytes.extend((if z { 1001u32 } else { 1u32 }).to_le_bytes());
    for value in xyz.iter().take(if z { 3 } else { 2 }) {
        bytes.extend(value.to_le_bytes());
    }
    bytes
}

fn geopackage(path: &Path) {
    let db = Connection::open(path).unwrap();
    db.execute_batch("PRAGMA application_id=1196444487; PRAGMA user_version=10300;
        CREATE TABLE gpkg_spatial_ref_sys(srs_name TEXT NOT NULL,srs_id INTEGER PRIMARY KEY,organization TEXT NOT NULL,organization_coordsys_id INTEGER NOT NULL,definition TEXT NOT NULL,description TEXT);
        CREATE TABLE gpkg_contents(table_name TEXT PRIMARY KEY,data_type TEXT NOT NULL,identifier TEXT UNIQUE,description TEXT DEFAULT '',last_change TEXT NOT NULL DEFAULT '2026-01-01T00:00:00.000Z',min_x DOUBLE,min_y DOUBLE,max_x DOUBLE,max_y DOUBLE,srs_id INTEGER);
        CREATE TABLE gpkg_geometry_columns(table_name TEXT PRIMARY KEY,column_name TEXT NOT NULL,geometry_type_name TEXT NOT NULL,srs_id INTEGER NOT NULL,z INTEGER NOT NULL,m INTEGER NOT NULL);
        CREATE TABLE roads(fid INTEGER PRIMARY KEY,geom BLOB,name TEXT,large INTEGER,keep INTEGER,visible BOOLEAN);
        CREATE TABLE survey(fid INTEGER PRIMARY KEY,geom BLOB,name TEXT,large INTEGER,keep INTEGER,visible BOOLEAN);").unwrap();
    db.execute(
        "INSERT INTO gpkg_spatial_ref_sys VALUES('WGS 84',4326,'EPSG',4326,?1,'')",
        [WGS84],
    )
    .unwrap();
    for (table, z) in [("roads", 0), ("survey", 1)] {
        db.execute("INSERT INTO gpkg_contents(table_name,data_type,identifier,srs_id) VALUES(?1,'features',?1,4326)", [table]).unwrap();
        db.execute(
            "INSERT INTO gpkg_geometry_columns VALUES(?1,'geom','POINT',4326,?2,0)",
            params![table, z],
        )
        .unwrap();
    }
    db.execute(
        "INSERT INTO roads VALUES(1,?1,'chosen',1152921504606846979,1,1)",
        [gpkg_point([0., 0., 0.], false)],
    )
    .unwrap();
    db.execute(
        "INSERT INTO roads VALUES(2,?1,NULL,NULL,0,0)",
        [gpkg_point([0.001, 0., 0.], false)],
    )
    .unwrap();
    db.execute(
        "INSERT INTO survey VALUES(9,?1,'height',42,1,1)",
        [gpkg_point([0., 0., 123.], true)],
    )
    .unwrap();
}

#[test]
fn exact_geojson_metadata_lists_filters_and_empty_selection() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("exact.geojson");
    geojson(
        &source,
        vec![
            point(
                json!(1152921504606846979i64),
                json!({"large":1152921504606846979i64,"name":"chosen","keep":1,"values":[1152921504606846979i64,null,2]}),
                [0., 0., 0.],
            ),
            point(
                json!("nullable"),
                json!({"large":null,"name":null,"keep":0,"values":[]}),
                [1., 0., 0.],
            ),
        ],
    );
    let output = root.path().join("filtered.3tz");
    let report = convert(
        &source,
        &output,
        &[
            "--source-crs",
            "local",
            "--where",
            "keep = 1",
            "--fields",
            "large,name,values",
            "--list-fields",
            "json",
        ],
    );
    assert_eq!(report["features"], 1);
    let rows = metadata(&output);
    let row = &rows["invented:1152921504606846979"];
    assert_eq!(row["large"], json!(1152921504606846979i64));
    assert_eq!(row["values"], "[1152921504606846979,null,2]");
    assert!(row.get("keep").is_none());
    let all = root.path().join("all.3tz");
    convert(
        &source,
        &all,
        &["--source-crs", "local", "--list-fields", "json"],
    );
    let rows = metadata(&all);
    assert_eq!(rows["invented:\"nullable\""]["large"], Value::Null);
    assert_eq!(rows["invented:\"nullable\""]["name"], Value::Null);
    let empty = root.path().join("empty.3tz");
    assert_eq!(
        convert(
            &source,
            &empty,
            &[
                "--source-crs",
                "local",
                "--where",
                "keep = 99",
                "--list-fields",
                "json"
            ]
        )["features"],
        0
    );
}

#[test]
fn independent_geopackage_layers_filters_and_z_height_are_read_only() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("invented.gpkg");
    geopackage(&source);
    let original = fs::read(&source).unwrap();
    let rejected = root.path().join("ambiguous.3tz");
    let (result, _) = run(&source, &rejected, &["--source-crs", "local"]);
    assert_eq!(result.status.code(), Some(3));
    assert!(!rejected.exists());
    let filtered = root.path().join("filtered.3tz");
    assert_eq!(
        convert(
            &source,
            &filtered,
            &[
                "--layer",
                "roads",
                "--where",
                "keep = 1",
                "--fields",
                "large,name,visible"
            ]
        )["features"],
        1
    );
    let rows = metadata(&filtered);
    assert_eq!(rows["roads:1"]["large"], json!(1152921504606846979i64));
    assert_eq!(rows["roads:1"]["visible"], true);
    assert!(rows["roads:1"].get("keep").is_none());
    let all = root.path().join("all.3tz");
    assert_eq!(
        convert(&source, &all, &["--all-layers", "--source-crs", "local"])["features"],
        3
    );
    assert_eq!(metadata(&all)["roads:2"]["large"], Value::Null);
    let height = root.path().join("height.3tz");
    let (result, _) = run(&source, &height, &["--layer", "survey"]);
    assert_eq!(result.status.code(), Some(3));
    assert!(!height.exists());
    let report = convert(
        &source,
        &height,
        &["--layer", "survey", "--height-offset", "7"],
    );
    assert_eq!(report["layers"][0]["heightMode"], "explicit offset");
    let manifest = archive_json(&height, "tileset.json");
    for (actual, expected) in manifest["root"]["transform"].as_array().unwrap()[12..15]
        .iter()
        .zip([6378267., 0., 0.])
    {
        assert!(
            (actual.as_f64().unwrap() - expected).abs() < 0.001,
            "{manifest}"
        );
    }
    assert_eq!(fs::read(&source).unwrap(), original);
}

#[test]
fn portable_library_entry_point_and_both_hierarchies_validate() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("library.geojson");
    geojson(
        &source,
        (0..6)
            .map(|i| point(json!(i), json!({"n":i}), [i as f64, 0., 0.]))
            .collect(),
    );
    for explicit in [false, true] {
        let output = root.path().join(format!("library-{explicit}.3tz"));
        let options = rusty_tiles::vector::VectorOptions {
            source_crs: Some("local".into()),
            explicit,
            reproducible: true,
            jobs: 2,
            ..Default::default()
        };
        let result = rusty_tiles::vector::vector_to_3tz_reported(
            &source,
            &output,
            2,
            false,
            false,
            &options,
            &rusty_tiles::Reporter::silent(),
        )
        .unwrap();
        assert!(result.archive);
        assert_eq!(result.report.unwrap()["features"], 6);
        assert_eq!(
            rusty_tiles::validate::archive(&output, None).unwrap()["ok"],
            true
        );
        let manifest = archive_json(&output, "tileset.json");
        assert_eq!(manifest["root"].get("implicitTiling").is_some(), !explicit);
    }
}

#[test]
fn jobs_and_reuse_are_reproducible_in_both_tiling_modes() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("repeatable.geojson");
    geojson(
        &source,
        (0..16)
            .map(|i| point(json!(i), json!({"n":i}), [i as f64, 0., 0.]))
            .collect(),
    );
    for explicit in [false, true] {
        let mut arguments = vec![
            "--source-crs",
            "local",
            "--max-features",
            "2",
            "--quantize",
            "--meshopt",
        ];
        if explicit {
            arguments.push("--explicit");
        }
        let one = root.path().join(format!("one-{explicit}.3tz"));
        let four = root.path().join(format!("four-{explicit}.3tz"));
        let mut first = arguments.clone();
        first.extend(["--jobs", "1"]);
        convert(&source, &one, &first);
        let mut second = arguments.clone();
        second.extend(["--jobs", "4"]);
        convert(&source, &four, &second);
        assert_eq!(fs::read(&one).unwrap(), fs::read(&four).unwrap());
        let previous = one.to_str().unwrap();
        second.extend(["--reuse-tileset", previous]);
        let reused = root.path().join(format!("reused-{explicit}.3tz"));
        let report = convert(&source, &reused, &second);
        assert_eq!(report["reuse"]["rebuiltContents"], 0);
        assert!(report["reuse"]["reusedContents"].as_u64().unwrap() > 0);
    }
}

#[test]
fn polygons_preserve_holes_and_opt_in_repair_reports() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("hole.geojson");
    geojson(
        &source,
        vec![polygon(
            "hole",
            json!([
                [[0, 0, 5], [10, 0, 5], [10, 10, 5], [0, 10, 5], [0, 0, 5]],
                [[3, 3, 5], [3, 7, 5], [7, 7, 5], [7, 3, 5], [3, 3, 5]]
            ]),
        )],
    );
    let output = root.path().join("hole.3tz");
    convert(&source, &output, &["--source-crs", "local", "--explicit"]);
    let mut area = 0.;
    for (document, binary) in glbs(&output) {
        for primitive in document["meshes"][0]["primitives"].as_array().unwrap() {
            if primitive["mode"] != 4 {
                continue;
            }
            let position = &document["accessors"]
                [primitive["attributes"]["POSITION"].as_u64().unwrap() as usize];
            let bytes = view(&document, &binary, &position["bufferView"]);
            let positions: Vec<_> = bytes
                .chunks_exact(12)
                .take(position["count"].as_u64().unwrap() as usize)
                .map(|p| {
                    [
                        f32::from_le_bytes(p[..4].try_into().unwrap()) as f64,
                        f32::from_le_bytes(p[4..8].try_into().unwrap()) as f64,
                        f32::from_le_bytes(p[8..12].try_into().unwrap()) as f64,
                    ]
                })
                .collect();
            let indices = &document["accessors"][primitive["indices"].as_u64().unwrap() as usize];
            let bytes = view(&document, &binary, &indices["bufferView"]);
            assert_eq!(indices["componentType"], 5125);
            let values: Vec<_> = bytes
                .chunks_exact(4)
                .take(indices["count"].as_u64().unwrap() as usize)
                .map(|p| u32::from_le_bytes(p.try_into().unwrap()) as usize)
                .collect();
            for triangle in values.chunks_exact(3) {
                let [a, b, c] = [
                    positions[triangle[0]],
                    positions[triangle[1]],
                    positions[triangle[2]],
                ];
                let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                let cross = [
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ];
                area += cross.iter().map(|value| value * value).sum::<f64>().sqrt() / 2.;
            }
        }
    }
    assert!((area - 84.).abs() < 1e-5, "hole fill area: {area}");
    geojson(
        &source,
        vec![polygon(
            "bow",
            json!([[[0, 0, 0], [2, 2, 0], [2, 0, 0], [0, 2, 0], [0, 0, 0]]]),
        )],
    );
    let rejected = root.path().join("bow-rejected.3tz");
    let (result, _) = run(&source, &rejected, &["--source-crs", "local"]);
    assert_eq!(result.status.code(), Some(3));
    assert!(!rejected.exists());
    let repaired = root.path().join("bow-repaired.3tz");
    convert(&source, &repaired, &["--source-crs", "local", "--repair"]);
    assert!(reports(&repaired)
        .iter()
        .any(|record| record["topologyRepaired"] == true));
}

#[test]
fn invalid_null_and_collapsed_features_have_explicit_publication_policy() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("mixed.geojson");
    geojson(
        &source,
        vec![
            point(json!("good"), json!({"name":"good"}), [0., 0., 0.]),
            json!({"type":"Feature","id":"null","properties":{"name":"null"},"geometry":null}),
            polygon(
                "bad",
                json!([[[0, 0, 0], [2, 2, 0], [2, 0, 0], [0, 2, 0], [0, 0, 0]]]),
            ),
        ],
    );
    let rejected = root.path().join("rejected.3tz");
    assert_eq!(
        run(&source, &rejected, &["--source-crs", "local"])
            .0
            .status
            .code(),
        Some(3)
    );
    assert!(!rejected.exists());
    let skipped = root.path().join("skipped.3tz");
    let report = convert(
        &source,
        &skipped,
        &["--source-crs", "local", "--skip-invalid"],
    );
    assert_eq!(report["features"], 1);
    assert_eq!(report["skippedFeatures"], 1);
    assert_eq!(report["featuresWithoutGeometry"], 1);
    assert!(reports(&skipped)
        .iter()
        .any(|record| record["outcome"] == "no-geometry"));
    geojson(
        &source,
        vec![polygon(
            "collapsed",
            json!([[[0, 0, 0], [1, 0, 0], [2, 0, 0], [1, 0, 0], [0, 0, 0]]]),
        )],
    );
    let failed = root.path().join("collapsed-failed.3tz");
    assert_eq!(
        run(&source, &failed, &["--source-crs", "local", "--repair"])
            .0
            .status
            .code(),
        Some(3)
    );
    assert!(!failed.exists());
    let outline = root.path().join("collapsed-outline.3tz");
    convert(
        &source,
        &outline,
        &["--source-crs", "local", "--repair", "--ambiguous-outlines"],
    );
    assert!(reports(&outline)
        .iter()
        .any(|record| record["outputGeometry"] == "outline"));
}

#[test]
fn unsupported_driver_and_grid_crs_are_environment_errors_without_archives() {
    let root = tempfile::tempdir().unwrap();
    let unsupported = root.path().join("unsupported.shp");
    fs::write(&unsupported, b"invented unsupported driver").unwrap();
    let source = root.path().join("point.geojson");
    geojson(&source, vec![point(json!(1), json!({}), [0., 0., 0.])]);
    for (input, arguments) in [
        (&unsupported, vec!["--source-crs", "local"]),
        (
            &source,
            vec!["--source-crs", "EPSG:26910", "--height-offset", "0"],
        ),
        (
            &source,
            vec![
                "--source-crs",
                "+proj=longlat +datum=WGS84 +nadgrids=missing.gsb",
                "--height-offset",
                "0",
            ],
        ),
    ] {
        let output = root.path().join("unpublished.3tz");
        let (result, report) = run(input, &output, &arguments);
        assert_eq!(result.status.code(), Some(4), "{report}");
        assert_eq!(report["error"]["code"], "environment");
        assert!(
            report["error"]["message"]
                .as_str()
                .unwrap()
                .contains("native-geospatial"),
            "{report}"
        );
        assert!(!output.exists());
    }
}
