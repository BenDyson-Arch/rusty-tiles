//! I1 implicit effective-volume reads through the public Rust and CLI entry
//! points: one outcome per exit-status row X-1 to X-16 (contract v3.1), with
//! Rust/CLI parity. Facts parity lives in `validate.rs` (RC-16).
use rusty_tiles::package::{package, PackageRequest};
use rusty_tiles::validate::{inspect, ValidationFailure, ValidationReport, ValidationRequest};
use rusty_tiles::RunControl;
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::PathBuf, process::Command};

type Files = BTreeMap<String, Vec<u8>>;

fn point_glb() -> Vec<u8> {
    let bin = [0u8; 12];
    let doc = json!({
        "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
        "buffers":[{"byteLength":12}],"bufferViews":[{"buffer":0,"byteLength":12}],
        "accessors":[{"bufferView":0,"componentType":5126,"type":"VEC3","count":1,
            "min":[0,0,0],"max":[0,0,0]}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":0}]}]
    });
    let mut json = serde_json::to_vec(&doc).unwrap();
    json.resize(json.len().next_multiple_of(4), b' ');
    let mut out = b"glTF".to_vec();
    out.extend(2u32.to_le_bytes());
    out.extend((28 + json.len() as u32 + 12).to_le_bytes());
    out.extend((json.len() as u32).to_le_bytes());
    out.extend(b"JSON");
    out.extend(json);
    out.extend(12u32.to_le_bytes());
    out.extend(b"BIN\0");
    out.extend(bin);
    out
}

fn bits(count: usize, set: &[usize]) -> Vec<u8> {
    let mut out = vec![0u8; count.div_ceil(8)];
    for &i in set {
        out[i / 8] |= 1 << (i % 8);
    }
    out
}

/// A binary subtree: JSON plus one internal buffer of 8-byte aligned views.
#[derive(Clone)]
struct Sub {
    doc: Value,
    bin: Vec<u8>,
}

impl Sub {
    fn new(tile: Value, child: Value) -> Self {
        Self {
            doc: json!({"tileAvailability":tile,"childSubtreeAvailability":child}),
            bin: Vec::new(),
        }
    }
    fn set(mut self, key: &str, value: Value) -> Self {
        self.doc[key] = value;
        self
    }
    fn view(&mut self, bytes: &[u8]) -> usize {
        self.bin.resize(self.bin.len().next_multiple_of(8), 0);
        let offset = self.bin.len();
        self.bin.extend_from_slice(bytes);
        if self.doc.get("bufferViews").is_none() {
            self.doc["bufferViews"] = json!([]);
        }
        let views = self.doc["bufferViews"].as_array_mut().unwrap();
        views.push(json!({"buffer":0,"byteOffset":offset,"byteLength":bytes.len()}));
        views.len() - 1
    }
    fn bitstream(&mut self, count: usize, set: &[usize]) -> Value {
        let view = self.view(&bits(count, set));
        json!({"bitstream":view,"availableCount":set.len()})
    }
    fn f64s(&mut self, values: &[f64]) -> usize {
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        self.view(&bytes)
    }
    fn build(&self) -> Vec<u8> {
        let mut doc = self.doc.clone();
        if !self.bin.is_empty() && doc.get("buffers").is_none() {
            doc["buffers"] = json!([{"byteLength":self.bin.len()}]);
        }
        let mut json = serde_json::to_vec(&doc).unwrap();
        json.resize(json.len().next_multiple_of(8), b' ');
        let mut bin = self.bin.clone();
        bin.resize(bin.len().next_multiple_of(8), 0);
        let mut out = b"subt".to_vec();
        out.extend(1u32.to_le_bytes());
        out.extend((json.len() as u64).to_le_bytes());
        out.extend((bin.len() as u64).to_le_bytes());
        out.extend(json);
        out.extend(bin);
        out
    }
}

fn constant(value: u8) -> Value {
    json!({"constant":value})
}

fn half(h: f64) -> Value {
    json!([0, 0, 0, h, 0, 0, 0, h, 0, 0, 0, h])
}

/// An implicit root R: QUADTREE, box half 8, error 16, the given templates.
fn implicit_root(levels: u32, available: u32, contents: &[&str]) -> Value {
    let mut root = json!({
        "boundingVolume":{"box":half(8.)},"geometricError":16,"refine":"REPLACE",
        "implicitTiling":{"subdivisionScheme":"QUADTREE","subtreeLevels":levels,
            "availableLevels":available,"subtrees":{"uri":"s/{level}/{x}/{y}.subtree"}}
    });
    match contents {
        [] => {}
        [one] => root["content"] = json!({"uri":one}),
        many => root["contents"] = many.iter().map(|uri| json!({"uri":uri})).collect(),
    }
    root
}

fn tileset(root: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"asset":{"version":"1.1"},"geometricError":16,"root":root})).unwrap()
}

/// The base archive: QUADTREE L = 2, A = 2, every tile available with
/// content: five tiles and five payloads.
fn base() -> Files {
    let mut files = Files::new();
    files.insert(
        "tileset.json".into(),
        tileset(implicit_root(2, 2, &["c/{level}/{x}/{y}.glb"])),
    );
    files.insert(
        "s/0/0/0.subtree".into(),
        Sub::new(constant(1), constant(0))
            .set("contentAvailability", json!([constant(1)]))
            .build(),
    );
    for name in ["0/0/0", "1/0/0", "1/1/0", "1/0/1", "1/1/1"] {
        files.insert(format!("c/{name}.glb"), point_glb());
    }
    files
}

/// One tile I_0 (L = 1, A = 1) with one payload and the given subtree.
fn single(root: Value, sub: Sub) -> Files {
    let mut files = Files::new();
    files.insert("tileset.json".into(), tileset(root));
    files.insert("s/0/0/0.subtree".into(), sub.build());
    files.insert("c/0/0/0.glb".into(), point_glb());
    files
}

fn package_files(files: &Files) -> (tempfile::TempDir, PathBuf) {
    let work = tempfile::tempdir().unwrap();
    let data = work.path().join("data");
    for (name, bytes) in files {
        let file = data.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, bytes).unwrap();
    }
    let output = work.path().join("test.3tz");
    package(
        PackageRequest::directory(&data, &output),
        &RunControl::default(),
    )
    .unwrap();
    (work, output)
}

/// Public API and CLI outcomes of one archive, with parity asserted.
fn run(files: &Files) -> Result<ValidationReport, ValidationFailure> {
    let (_work, path) = package_files(files);
    let result = inspect(ValidationRequest::new(&path));
    let cli = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .arg("validate")
        .arg(&path)
        .arg("--json")
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&cli.stdout).unwrap();
    match &result {
        Ok(inspected) => {
            assert_eq!(cli.status.code(), Some(0), "{report}");
            assert_eq!(serde_json::to_value(inspected).unwrap(), report);
        }
        Err(failure) => {
            let (code, exit) = failure.category();
            assert_eq!(cli.status.code(), Some(i32::from(exit)), "{report}");
            assert_eq!(report["error"]["code"], code, "{report}");
        }
    }
    result
}

fn ok(files: &Files) -> ValidationReport {
    run(files).unwrap_or_else(|failure| panic!("expected success, got {failure:?}"))
}

#[track_caller]
fn fails(files: &Files, category: &str, needle: &str) {
    let failure = run(files).expect_err("expected a failure");
    assert_eq!(failure.category().0, category, "{failure:?}");
    assert!(failure.to_string().contains(needle), "{failure}");
}

fn edit(files: &mut Files, name: &str, patch: impl FnOnce(&mut Value)) {
    let mut doc: Value = serde_json::from_slice(&files[name]).unwrap();
    patch(&mut doc);
    files.insert(name.into(), serde_json::to_vec(&doc).unwrap());
}

fn set_templates(files: &mut Files, templates: &[&str]) {
    edit(files, "tileset.json", |doc| {
        let root = doc["root"].as_object_mut().unwrap();
        root.remove("content");
        root.insert(
            "contents".into(),
            templates.iter().map(|uri| json!({"uri":uri})).collect(),
        );
    });
}

#[test]
fn x14_report_labels_and_counts() {
    let report = ok(&base());
    assert_eq!((report.tiles, report.content_references), (5, 5));
    let at = |label: &str| report.checks.iter().position(|c| c == label);
    let implicit = at("implicitAddressingAndAvailability").expect("I1 check listed");
    assert_eq!(Some(implicit), at("hierarchyBounds").map(|i| i + 1));
    for label in [
        "recordedBudgets.implicitTileExtras",
        "decodedContentBounds",
        "decodedContentBounds.implicitTiling",
        "metadataSemantics",
    ] {
        assert!(report.not_inspected.iter().any(|c| c == label), "{label}");
    }
    assert!(!report
        .not_inspected
        .iter()
        .any(|c| c == "implicitAddressingAndAvailability"));
    // Explicit-only archives: no I1 label.
    let mut explicit = Files::new();
    explicit.insert(
        "tileset.json".into(),
        tileset(json!({"boundingVolume":{"box":half(8.)},"geometricError":0,
            "content":{"uri":"a.glb"}})),
    );
    explicit.insert("a.glb".into(), point_glb());
    let report = ok(&explicit);
    assert!(!report.checks.iter().any(|c| c.starts_with("implicit")));
    assert!(!report.not_inspected.iter().any(|c| c.contains("implicit")));
}

#[test]
fn x01_x02_json_content_templates_are_unsupported_at_admission() {
    // Today's chunked producer pattern (W3/W5), the #151 shape and the DC-2
    // forms. The subtree is absent: the verdict precedes any subtree read.
    for template in [
        "c/{level}/{x}/{y}.json",
        "c/{level}/{x}/{y}.JSON//./",
        "c/{level}/{x}/{y}.json/.",
        "c/{level}/{x}/{y}.json/",
        "c/{level}/{x}-{y}.json/{level}/..",
        "c/{level}/{x}/{y}.js%6Fn",
    ] {
        let mut files = base();
        files.remove("s/0/0/0.subtree");
        set_templates(&mut files, &["c/{level}/{x}/{y}.glb", template]);
        fails(
            &files,
            "unsupported",
            "implicit: unsupported external tileset content template in tileset.json",
        );
    }
}

#[test]
fn x03_x04_profile_exclusions_are_unsupported() {
    let root = |patch: &dyn Fn(&mut Value)| {
        let mut files = base();
        edit(&mut files, "tileset.json", |doc| patch(&mut doc["root"]));
        files
    };
    fails(
        &root(&|r| r["implicitTiling"]["subdivisionScheme"] = json!("HEXTREE")),
        "unsupported",
        "subdivisionScheme",
    );
    fails(
        &root(&|r| r["implicitTiling"]["availableLevels"] = json!(33)),
        "unsupported",
        "availableLevels above 32",
    );
    fails(
        &root(&|r| r["implicitTiling"]["subtreeLevels"] = json!(33)),
        "unsupported",
        "subtreeLevels above 32",
    );
    fails(
        &root(&|r| r["boundingVolume"] = json!({"region":[-1,-1,1,1,0,10]})),
        "unsupported",
        "region implicit root volume",
    );
    fails(
        &root(&|r| {
            r["boundingVolume"] = json!({"extensions":{"3DTILES_bounding_volume_S2":
                {"token":"1","minimumHeight":0,"maximumHeight":1}}})
        }),
        "unsupported",
        "3DTILES_bounding_volume_S2",
    );
    // Sphere roots stay InvalidInput (`:116`).
    fails(
        &root(&|r| r["boundingVolume"] = json!({"sphere":[0,0,0,8]})),
        "invalid_input",
        "sphere implicit root volumes are disallowed",
    );
    // JSON-format subtrees (v3.2 ruling 1), with and without a BOM; any other
    // magic and a bad version stay InvalidInput (D-4).
    for (member, category, needle) in [
        (
            &b"{\"tileAvailability\":{\"constant\":1}}"[..],
            "unsupported",
            "JSON-format subtree",
        ),
        (
            &b"\xef\xbb\xbf  {}"[..],
            "unsupported",
            "JSON-format subtree",
        ),
        (
            &b"subs\x01\x00\x00\x00 padding padding padding"[..],
            "invalid_input",
            "invalid subtree magic: s/0/0/0.subtree",
        ),
    ] {
        let mut files = base();
        files.insert("s/0/0/0.subtree".into(), member.to_vec());
        fails(&files, category, needle);
    }
    let mut files = base();
    files.get_mut("s/0/0/0.subtree").unwrap()[4] = 2;
    fails(&files, "invalid_input", "subtree version must be 1");
}

#[test]
fn x05_sparse_wide_subtrees_are_read() {
    // N^L = 4^9 > 65,536 tile slots, two available tiles.
    let mut sub = Sub::new(Value::Null, constant(0));
    let tiles = sub.bitstream(87_381, &[0, 1]);
    sub.doc["tileAvailability"] = tiles;
    let mut files = Files::new();
    files.insert("tileset.json".into(), tileset(implicit_root(9, 9, &[])));
    files.insert("s/0/0/0.subtree".into(), sub.build());
    let report = ok(&files);
    assert_eq!((report.tiles, report.content_references), (2, 0));
}

#[test]
fn x06_new_unsupported_profile_cases() {
    // `extensions` in subtree JSON.
    let mut files = base();
    files.insert(
        "s/0/0/0.subtree".into(),
        Sub::new(constant(1), constant(0))
            .set("contentAvailability", json!([constant(1)]))
            .set("extensions", json!({"EXT_x":{}}))
            .build(),
    );
    fails(&files, "unsupported", "extensions in subtree JSON");
    // CONTENT_GROUP_ID in a template header `metadata` class (DC-5).
    let mut files = base();
    edit(&mut files, "tileset.json", |doc| {
        doc["schema"] = json!({"id":"s","classes":{"group":{"properties":{
            "g":{"type":"SCALAR","componentType":"UINT32","semantic":"CONTENT_GROUP_ID"}}}}});
        doc["root"]["content"]["metadata"] = json!({"class":"group","properties":{"g":0}});
    });
    fails(&files, "unsupported", "CONTENT_GROUP_ID");
    // CONTENT_URI in a `contentMetadata` class (RC-1), naming JSON content.
    let mut files = base();
    edit(&mut files, "tileset.json", |doc| {
        doc["schema"] = json!({"id":"s","classes":{"content":{"properties":{
            "u":{"type":"STRING","semantic":"CONTENT_URI"}}}}});
    });
    let mut sub =
        Sub::new(constant(1), constant(0)).set("contentAvailability", json!([constant(1)]));
    let text = b"x.jsonx.jsonx.jsonx.jsonx.json";
    let values = sub.view(text);
    let offsets: Vec<u8> = (0..=5u32).flat_map(|i| (i * 6).to_le_bytes()).collect();
    let offsets = sub.view(&offsets);
    sub = sub
        .set(
            "propertyTables",
            json!([{"class":"content","count":5,"properties":{"u":{"values":values,"stringOffsets":offsets}}}]),
        )
        .set("contentMetadata", json!([0]));
    files.insert("s/0/0/0.subtree".into(), sub.build());
    fails(&files, "unsupported", "CONTENT_URI");
    // `scale` on an interpreted property.
    let mut files = single(
        implicit_root(1, 1, &["c/{level}/{x}/{y}.glb"]),
        Sub::new(constant(1), constant(0)).set("contentAvailability", json!([constant(1)])),
    );
    edit(&mut files, "tileset.json", |doc| {
        doc["schema"] = json!({"id":"s","classes":{"tile":{"properties":{
            "e":{"type":"SCALAR","componentType":"FLOAT64","semantic":"TILE_GEOMETRIC_ERROR","scale":2}}}}});
    });
    let mut sub =
        Sub::new(constant(1), constant(0)).set("contentAvailability", json!([constant(1)]));
    let rows = sub.f64s(&[16.]);
    sub = sub
        .set(
            "propertyTables",
            json!([{"class":"tile","count":1,"properties":{"e":{"values":rows}}}]),
        )
        .set("tileMetadata", json!(0));
    files.insert("s/0/0/0.subtree".into(), sub.build());
    fails(&files, "unsupported", "scale on interpreted property");
    // A repeated subtree member: one external implicit tileset placed twice (OD-7).
    let mut files = base();
    let external = files.remove("tileset.json").unwrap();
    files.insert("ext.json".into(), external);
    // Distinct `refine` keeps the bundled schema's unique `children` items.
    let child = |refine: &str| {
        json!({"boundingVolume":{"box":half(8.)},"geometricError":16,"refine":refine,
            "content":{"uri":"ext.json"}})
    };
    files.insert(
        "tileset.json".into(),
        tileset(
            json!({"boundingVolume":{"box":half(8.)},"geometricError":16,"refine":"ADD",
            "children":[child("ADD"), child("REPLACE")]}),
        ),
    );
    fails(
        &files,
        "unsupported",
        "repeated subtree member s/0/0/0.subtree",
    );
}

#[test]
fn x07_semantics_by_name_float32_rows_and_external_buffers_are_read() {
    // Rows under arbitrary class and property names, FLOAT32, in an external
    // buffer at index 1 (today: InvalidInput through class/property gating).
    let cells: [[f64; 2]; 5] = [[0., 8.], [-4., 4.], [4., 4.], [-4., 4.], [4., 4.]];
    let ys = [0., -4., -4., 4., 4.];
    let mut boxes = Vec::new();
    for (tile, [x, h]) in cells.iter().enumerate() {
        let b = [*x, ys[tile], 0., *h, 0., 0., 0., *h, 0., 0., 0., 8.];
        boxes.extend(b.iter().flat_map(|v| (*v as f32).to_le_bytes()));
    }
    let errors: Vec<u8> = [16f32, 8., 8., 8., 8.]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let mut external = boxes.clone();
    external.resize(external.len().next_multiple_of(8), 0);
    let error_offset = external.len();
    external.extend(&errors);
    let mut files = base();
    edit(&mut files, "tileset.json", |doc| {
        doc["schema"] = json!({"id":"s","classes":{"anything":{"properties":{
            "volume":{"type":"SCALAR","componentType":"FLOAT32","array":true,"count":12,
                "semantic":"TILE_BOUNDING_BOX"},
            "err":{"type":"SCALAR","componentType":"FLOAT32","semantic":"TILE_GEOMETRIC_ERROR"}}}}});
    });
    let mut sub =
        Sub::new(constant(1), constant(0)).set("contentAvailability", json!([constant(1)]));
    sub.doc["buffers"] = json!([{"uri":"rows.bin","byteLength":external.len()}]);
    sub.doc["bufferViews"] = json!([
        {"buffer":0,"byteOffset":0,"byteLength":boxes.len()},
        {"buffer":0,"byteOffset":error_offset,"byteLength":errors.len()}
    ]);
    sub = sub
        .set(
            "propertyTables",
            json!([{"class":"anything","count":5,"properties":{"volume":{"values":0},"err":{"values":1}}}]),
        )
        .set("tileMetadata", json!(0));
    files.insert("s/0/0/0.subtree".into(), sub.build());
    files.insert("s/0/0/rows.bin".into(), external);
    let report = ok(&files);
    assert_eq!((report.tiles, report.content_references), (5, 5));
}

#[test]
fn x08_implicit_roots_on_non_root_explicit_tiles_are_read() {
    let mut files = base();
    edit(&mut files, "tileset.json", |doc| {
        let implicit = doc["root"].take();
        doc["root"] = json!({"boundingVolume":{"box":half(16.)},"geometricError":16,
            "refine":"ADD","children":[implicit]});
    });
    let report = ok(&files);
    assert_eq!((report.tiles, report.content_references), (6, 5));
}

#[test]
fn x09_new_invalid_input_rules() {
    // A template missing a required variable.
    let mut files = base();
    set_templates(&mut files, &["c/{level}/{x}.glb"]);
    fails(
        &files,
        "invalid_input",
        "template lacks a required variable",
    );
    // The post-walk content rule (`:299`).
    let mut files = base();
    files.insert(
        "s/0/0/0.subtree".into(),
        Sub::new(constant(1), constant(0))
            .set("contentAvailability", json!([constant(0)]))
            .build(),
    );
    files.retain(|name, _| !name.starts_with("c/"));
    fails(&files, "invalid_input", "no tile has available content");
    // Schema `minItems` on subtree arrays.
    let mut files = base();
    files.insert(
        "s/0/0/0.subtree".into(),
        Sub::new(constant(1), constant(0))
            .set("contentAvailability", json!([constant(1)]))
            .set("bufferViews", json!([]))
            .build(),
    );
    fails(
        &files,
        "invalid_input",
        "bufferViews must be an array with at least 1 item",
    );
    // A missing `bufferView.byteOffset`.
    let mut sub =
        Sub::new(Value::Null, constant(0)).set("contentAvailability", json!([constant(1)]));
    let tiles = sub.bitstream(5, &[0, 1, 2, 3, 4]);
    sub.doc["tileAvailability"] = tiles;
    sub.doc["bufferViews"][0]
        .as_object_mut()
        .unwrap()
        .remove("byteOffset");
    let mut files = base();
    files.insert("s/0/0/0.subtree".into(), sub.build());
    fails(&files, "invalid_input", "byteOffset");
    // DC-8: `contentMetadata` in a subtree whose second slot is empty.
    for (count, needle) in [
        (0, "property table count must be at least 1"),
        (
            1,
            "property table count 1 differs from availability popcount 0",
        ),
    ] {
        let mut files = base();
        set_templates(
            &mut files,
            &["c/{level}/{x}/{y}.glb", "d/{level}/{x}/{y}.glb"],
        );
        edit(&mut files, "tileset.json", |doc| {
            doc["schema"] =
                json!({"id":"s","classes":{"c":{"properties":{"flag":{"type":"BOOLEAN"}}}}});
        });
        let mut sub = Sub::new(constant(1), constant(0))
            .set("contentAvailability", json!([constant(1), constant(0)]));
        let flags = sub.view(&[0]);
        sub = sub
            .set(
                "propertyTables",
                json!([
                    {"class":"c","count":5,"properties":{"flag":{"values":flags}}},
                    {"class":"c","count":count,"properties":{"flag":{"values":flags}}}
                ]),
            )
            .set("contentMetadata", json!([0, 1]));
        files.insert("s/0/0/0.subtree".into(), sub.build());
        fails(&files, "invalid_input", needle);
    }
}

/// One tile I_0 with TILE_BOUNDING_BOX and TILE_GEOMETRIC_ERROR rows.
fn rows(root: Value, volume: [f64; 12], error: f64) -> Files {
    let mut sub =
        Sub::new(constant(1), constant(0)).set("contentAvailability", json!([constant(1)]));
    let b = sub.f64s(&volume);
    let e = sub.f64s(&[error]);
    sub = sub
        .set(
            "propertyTables",
            json!([{"class":"tile","count":1,"properties":{"b":{"values":b},"e":{"values":e}}}]),
        )
        .set("tileMetadata", json!(0));
    let mut files = single(root, sub);
    edit(&mut files, "tileset.json", |doc| {
        doc["schema"] = json!({"id":"s","classes":{"tile":{"properties":{
            "b":{"type":"SCALAR","componentType":"FLOAT64","array":true,"count":12,
                "semantic":"TILE_BOUNDING_BOX"},
            "e":{"type":"SCALAR","componentType":"FLOAT64","semantic":"TILE_GEOMETRIC_ERROR"}}}}});
    });
    files
}

fn shifted(delta: f64) -> [f64; 12] {
    [delta, 0., 0., 8., 0., 0., 0., 8., 0., 0., 0., 8.]
}

#[test]
fn x10_root_tolerances_are_the_explicit_tile_rules() {
    // ‖h‖ = √192, so the scaled tolerance is about 1.49e-5 (T-1, T-2).
    let root = implicit_root(1, 1, &["c/{level}/{x}/{y}.glb"]);
    ok(&rows(root.clone(), shifted(2e-6), 16.));
    fails(
        &rows(root.clone(), shifted(3e-5), 16.),
        "invalid_input",
        "child bounds escape parent at depth 0",
    );
    ok(&rows(root.clone(), shifted(0.), 16. * (1. + 1e-9)));
    fails(
        &rows(root, shifted(0.), 16. + 2. * 1e-8 * 17.),
        "invalid_input",
        "geometricError increases at depth 0",
    );
}

#[test]
fn x11_the_root_itself_is_checked_against_its_explicit_parent() {
    // T-3: an external implicit root whose V_R (or e_R) escapes the
    // referencing tile while V_0 (or e_0) does not.
    for (patch, needle) in [
        (
            &(|r: &mut Value| r["boundingVolume"]["box"] = half(9.)) as &dyn Fn(&mut Value),
            "child bounds escape parent at depth 1",
        ),
        (
            &|r: &mut Value| r["geometricError"] = json!(20),
            "geometricError increases at depth 1",
        ),
    ] {
        let mut root = implicit_root(1, 1, &["c/{level}/{x}/{y}.glb"]);
        patch(&mut root);
        let mut files = rows(root, shifted(0.), 16.);
        let external = files.remove("tileset.json").unwrap();
        files.insert("ext.json".into(), external);
        files.insert(
            "tileset.json".into(),
            tileset(
                json!({"boundingVolume":{"box":half(8.)},"geometricError":16,
                "refine":"REPLACE","content":{"uri":"ext.json"}}),
            ),
        );
        fails(&files, "invalid_input", needle);
    }
}

#[test]
fn x13_implicit_tiling_extension_is_unsupported() {
    let mut files = base();
    edit(&mut files, "tileset.json", |doc| {
        let root = doc["root"].as_object_mut().unwrap();
        let tiling = root.remove("implicitTiling").unwrap();
        root.insert(
            "extensions".into(),
            json!({"3DTILES_implicit_tiling":tiling}),
        );
    });
    fails(
        &files,
        "unsupported",
        "3DTILES_implicit_tiling (1.0) extension",
    );
}

#[test]
fn x15_template_profile_is_unsupported() {
    for (template, needle) in [
        (
            "c%2{x}/{level}/{y}.glb",
            "escape adjacent to template variable",
        ),
        (
            "c/{level}/../{x}/../{y}/../a.glb",
            "template variables eliminated by dot-segments",
        ),
    ] {
        let mut files = base();
        set_templates(&mut files, &[template]);
        fails(&files, "unsupported", needle);
    }
    let long = format!("c/{}/{{level}}/{{x}}/{{y}}.glb", "a".repeat(1_010));
    let mut files = base();
    set_templates(&mut files, &[&long]);
    fails(&files, "unsupported", "normalized template exceeds 1 KiB");
}

#[test]
fn x16_content_index_reservation_refuses_at_subtree_admission() {
    // R9 at the raised default `references` (2^20, #160): 7,200 `Const1`
    // slots naming the same members. Each three-level OCTREE subtree has 73
    // tiles, so the root subtree reserves E = 525,600 and the first child
    // subtree's reservation (1,051,200 in total) exceeds 1,048,576 references
    // before its index allocation.
    let mut root = implicit_root(3, 6, &[]);
    root["implicitTiling"]["subdivisionScheme"] = json!("OCTREE");
    root["implicitTiling"]["subtrees"]["uri"] = json!("s/{level}/{x}/{y}/{z}.subtree");
    root["contents"] = (0..7_200)
        .map(|_| json!({"uri":"c/{level}-{x}-{y}-{z}.glb"}))
        .collect();
    let layers = json!(vec![constant(1); 7_200]);
    let sub = |child| {
        Sub::new(constant(1), constant(child))
            .set("contentAvailability", layers.clone())
            .build()
    };
    let mut files = Files::new();
    files.insert("tileset.json".into(), tileset(root));
    files.insert("s/0/0/0/0.subtree".into(), sub(1));
    // g + L = A at the child subtree: its child availability is Const0.
    files.insert("s/3/0/0/0.subtree".into(), sub(0));
    for name in ["0-0-0-0", "1-0-0-0", "2-0-0-0"] {
        files.insert(format!("c/{name}.glb"), point_glb());
    }
    fails(
        &files,
        "resource_limit",
        "subtree content entries exceed remaining references",
    );
}

/// A producer-style forest: an explicit root whose children are link tiles to
/// external explicit tileset documents `tileset-d{n}.json`, each holding one
/// implicit root (one tile, one payload), with the given recorded budgets.
fn forest(budgets: Value) -> Files {
    let mut files = Files::new();
    let mut links = Vec::new();
    for n in 0..2 {
        let mut root = implicit_root(1, 1, &[&format!("c{n}/{{level}}/{{x}}/{{y}}.glb")]);
        root["implicitTiling"]["subtrees"]["uri"] =
            json!(format!("s{n}/{{level}}/{{x}}/{{y}}.subtree"));
        // Padding keeps each document larger than any payload (D-2).
        let doc = json!({"asset":{"version":"1.1","extras":{"pad":"x".repeat(1_000)}},
            "geometricError":16,"root":root});
        files.insert(
            format!("tileset-d{n}.json"),
            serde_json::to_vec(&doc).unwrap(),
        );
        files.insert(
            format!("s{n}/0/0/0.subtree"),
            Sub::new(constant(1), constant(0))
                .set("contentAvailability", json!([constant(1)]))
                .build(),
        );
        files.insert(format!("c{n}/0/0/0.glb"), point_glb());
        links.push(
            json!({"boundingVolume":{"box":half(8.)},"geometricError":16,
            "refine":"REPLACE","content":{"uri":format!("tileset-d{n}.json")}}),
        );
    }
    files.insert(
        "tileset.json".into(),
        tileset(
            json!({"boundingVolume":{"box":half(8.)},"geometricError":16,
            "refine":"REPLACE","children":links}),
        ),
    );
    files.insert(
        "conversion.json".into(),
        serde_json::to_vec(&json!({"budgets":budgets})).unwrap(),
    );
    files
}

#[test]
fn amendment_d1_link_tiles_are_reported_but_not_budgeted_as_tiles() {
    // Root + 2 link tiles + 2 implicit roots: 5 reported, 3 budgeted.
    let report = ok(&forest(json!({"tiles":3})));
    assert_eq!((report.tiles, report.content_references), (5, 4));
    fails(
        &forest(json!({"tiles":2})),
        "invalid_input",
        "hierarchy exceeds recorded tile budget",
    );
}

#[test]
fn amendment_d2_external_tileset_documents_are_not_tile_payload_bytes() {
    let glb = point_glb().len() as u64;
    let files = forest(json!({"bytes":glb}));
    assert!(files["tileset-d0.json"].len() as u64 > glb);
    ok(&files);
    // The budget still applies to the implicit tiles' payloads.
    fails(
        &forest(json!({"bytes":glb - 1})),
        "invalid_input",
        "tile exceeds recorded encoded byte budget",
    );
}
