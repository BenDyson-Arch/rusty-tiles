//! VS-B controls: inline streaming against the reference traversal (phase 2 after
//! traversal), outcome precedence, the facts store in both catalog profiles, the memo budget,
//! live documents, the label cap and budget scaling.
use super::*;
use crate::archive3tz::FORCED;
use serde_json::json;
use std::path::PathBuf;

fn glb(doc: &Value, bin: &[u8]) -> Vec<u8> {
    let mut json = serde_json::to_vec(doc).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut bin = bin.to_vec();
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let mut out = b"glTF".to_vec();
    out.extend(2u32.to_le_bytes());
    let chunks = if bin.is_empty() { 0 } else { 8 + bin.len() };
    out.extend((20 + json.len() as u32 + chunks as u32).to_le_bytes());
    out.extend((json.len() as u32).to_le_bytes());
    out.extend(b"JSON");
    out.extend(json);
    if !bin.is_empty() {
        out.extend((bin.len() as u32).to_le_bytes());
        out.extend(b"BIN\0");
        out.extend(bin);
    }
    out
}
/// POINTS at `positions` under `nested` empty parent nodes (node depth `nested + 1`), with
/// `extensions` declared as used.
fn points_at(positions: &[[f32; 3]], nested: usize, extensions: &[String]) -> Vec<u8> {
    let mut bin = Vec::new();
    for p in positions {
        for c in p {
            bin.extend(c.to_le_bytes());
        }
    }
    let bound = |f: fn(f32, f32) -> f32, start: f32| -> Vec<f64> {
        (0..3)
            .map(|c| f64::from(positions.iter().map(|p| p[c]).fold(start, f)))
            .collect()
    };
    let mut nodes: Vec<Value> = (0..nested).map(|i| json!({"children":[i + 1]})).collect();
    nodes.push(json!({"mesh":0}));
    let mut doc = json!({
        "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":nodes,
        "buffers":[{"byteLength":bin.len()}],"bufferViews":[{"buffer":0,"byteLength":bin.len()}],
        "accessors":[{"bufferView":0,"componentType":5126,"type":"VEC3","count":positions.len(),
            "min":bound(f32::min, f32::INFINITY),"max":bound(f32::max, f32::NEG_INFINITY)}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":0}]}]
    });
    if !extensions.is_empty() {
        doc["extensionsUsed"] = json!(extensions);
    }
    glb(&doc, &bin)
}
fn point(x: f32) -> Vec<u8> {
    points_at(&[[x, 0., 0.]], 0, &[])
}
fn boxed(center: [f64; 3], half: f64) -> Value {
    json!({"box":[center[0],center[1],center[2],half,0,0,0,half,0,0,0,half]})
}
fn tile(half: f64, error: f64) -> Value {
    json!({"boundingVolume":boxed([0.; 3], half),"geometricError":error})
}
fn with_content(mut tile: Value, uri: &str) -> Value {
    tile["content"] = json!({"uri":uri});
    tile
}
fn doc(root: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"asset":{"version":"1.1"},"geometricError":1000,"root":root}))
        .unwrap()
}
type Files = Vec<(String, Vec<u8>)>;
fn package(files: &Files) -> (tempfile::TempDir, PathBuf) {
    let work = tempfile::tempdir().unwrap();
    let data = work.path().join("data");
    for (name, bytes) in files {
        let file = data.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, bytes).unwrap();
    }
    let output = work.path().join("test.3tz");
    crate::package::package(
        crate::package::PackageRequest::directory(&data, &output),
        &crate::RunControl::default(),
    )
    .unwrap();
    (work, output)
}

/// How one inspection runs.
#[derive(Clone, Copy, Debug)]
struct Mode {
    /// The reference traversal of VS-B-4: phase 2 after traversal.
    deferred: bool,
    memoize: bool,
    forced_spill: bool,
    memo_budget: Option<usize>,
}
const INLINE: Mode = Mode {
    deferred: false,
    memoize: true,
    forced_spill: false,
    memo_budget: None,
};
/// The comparable outcome: report or failure, C2 units and the C1 counters.
#[derive(Debug, PartialEq)]
struct Outcome {
    result: String,
    units: u64,
    counters: [u64; 3],
}
fn inspect_in(path: &Path, budgets: Budgets, mode: Mode) -> (Outcome, u64, usize) {
    DEFERRED_STREAM.with(|d| d.set(mode.deferred));
    MEMOIZE.with(|m| m.set(mode.memoize));
    MEMO_BUDGET.with(|b| b.set(mode.memo_budget));
    FORCED.with(|f| f.set(Some(mode.forced_spill)));
    let inspection = inspect_facts(ValidationRequest::new(path), budgets);
    DEFERRED_STREAM.with(|d| d.set(false));
    MEMOIZE.with(|m| m.set(true));
    MEMO_BUDGET.with(|b| b.set(None));
    FORCED.with(|f| f.set(None));
    match inspection {
        Ok(inspection) => {
            let mut report = serde_json::to_value(&inspection.report).unwrap();
            // The scratch member records where the catalog spilled, not a verdict.
            report.as_object_mut().unwrap().remove("scratch");
            (
                Outcome {
                    result: report.to_string(),
                    units: inspection.units,
                    counters: inspection.counters,
                },
                inspection.prepared,
                inspection.memo_peak,
            )
        }
        Err(error) => (
            Outcome {
                result: format!("{}: {error}", error.category().0),
                units: 0,
                counters: [0; 3],
            },
            0,
            0,
        ),
    }
}
fn run(path: &Path, limits: &ValidationLimits, mode: Mode) -> Outcome {
    inspect_in(path, Budgets::Absolute(limits.clone()), mode).0
}
/// VS-B-1/4: inline streaming equals the reference traversal in every mode.
fn agreed(files: &Files, limits: &ValidationLimits) -> Outcome {
    let (_work, path) = package(files);
    let inline = run(&path, limits, INLINE);
    for mode in [
        Mode {
            deferred: true,
            ..INLINE
        },
        Mode {
            memoize: false,
            ..INLINE
        },
        Mode {
            forced_spill: true,
            ..INLINE
        },
        Mode {
            deferred: true,
            memoize: false,
            forced_spill: true,
            memo_budget: None,
        },
    ] {
        assert_eq!(run(&path, limits, mode), inline, "{mode:?}");
    }
    inline
}
fn reasons(outcome: &Outcome) -> Vec<String> {
    let report: Value = serde_json::from_str(&outcome.result).unwrap();
    report["notInspected"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str()?.strip_prefix("decodedContentBounds."))
        .map(String::from)
        .collect()
}
fn files(entries: Vec<(&str, Vec<u8>)>) -> Files {
    entries
        .into_iter()
        .map(|(name, bytes)| (name.to_owned(), bytes))
        .collect()
}
/// A root over children, each `(tile patch, content name, payload)`.
fn siblings(children: Vec<(Value, &str, Vec<u8>)>) -> Files {
    let mut root = tile(100., 10.);
    let mut out = Vec::new();
    root["children"] = Value::Array(
        children
            .into_iter()
            .map(|(patch, name, payload)| {
                let mut child = with_content(tile(10., 1.), name);
                for (key, value) in patch.as_object().unwrap() {
                    child[key] = value.clone();
                }
                out.push((name.to_owned(), payload));
                child
            })
            .collect(),
    );
    out.push(("tileset.json".into(), doc(root)));
    out
}

#[test]
fn a_recorded_outside_yields_to_a_later_exclusion_or_precheck_stop() {
    let limits = ValidationLimits {
        containment_node_depth: 1,
        ..ValidationLimits::default()
    };
    // Outside first, then a tile with metadata: the exclusion is the result.
    let excluded = siblings(vec![
        (json!({}), "a.glb", point(50.)),
        (json!({"metadata":{"class":"c"}}), "b.glb", point(0.)),
    ]);
    let outcome = agreed(&excluded, &limits);
    assert_eq!(reasons(&outcome), ["tileMetadata"], "{outcome:?}");
    // Outside first, then a reference over the node-depth cap: the precheck reason wins.
    let stopped = siblings(vec![
        (json!({}), "a.glb", point(50.)),
        (json!({}), "b.glb", points_at(&[[0., 0., 0.]], 2, &[])),
    ]);
    assert_eq!(reasons(&agreed(&stopped, &limits)), ["workLimit"]);
    // Control: alone, the escape is the result.
    let alone = siblings(vec![(json!({}), "a.glb", point(50.))]);
    let outcome = agreed(&alone, &limits);
    assert!(
        outcome.result.starts_with(
            "invalid_input: decoded content outside bounding volume: content=a.glb depth=1 volume=own"
        ),
        "{outcome:?}"
    );
}

#[test]
fn a_passing_stream_yields_to_a_later_exclusion() {
    let files = siblings(vec![
        (json!({}), "a.glb", point(1.)),
        (json!({"metadata":{"class":"c"}}), "b.glb", point(1.)),
    ]);
    assert_eq!(
        reasons(&agreed(&files, &ValidationLimits::default())),
        ["tileMetadata"]
    );
}

#[test]
fn a_recorded_outside_yields_to_any_c1_failure() {
    // A later geometric-error increase.
    let mut later = siblings(vec![
        (json!({}), "a.glb", point(50.)),
        (json!({"geometricError":20}), "b.glb", point(0.)),
    ]);
    let outcome = agreed(&later, &ValidationLimits::default());
    assert_eq!(
        outcome.result,
        "invalid_input: geometricError increases at depth 1"
    );
    // A closure failure after traversal.
    later = siblings(vec![
        (json!({}), "a.glb", point(50.)),
        (json!({}), "b.glb", point(0.)),
    ]);
    later.push(("spare.bin".into(), vec![1, 2, 3]));
    let outcome = agreed(&later, &ValidationLimits::default());
    assert_eq!(
        outcome.result,
        "invalid_input: unreferenced archive entries: spare.bin"
    );
}

#[test]
fn the_first_failure_in_traversal_order_is_published() {
    // The deferred reference after an external tileset streams after that tileset.
    let mut root = tile(100., 10.);
    root["contents"] = json!([{"uri":"ext.json"},{"uri":"a.glb"}]);
    let ext = doc(with_content(tile(100., 5.), "b.glb"));
    let files = files(vec![
        ("tileset.json", doc(root)),
        ("ext.json", ext),
        ("a.glb", point(150.)),
        ("b.glb", point(150.)),
    ]);
    let outcome = agreed(&files, &ValidationLimits::default());
    assert!(
        outcome.result.contains("content=b.glb depth=1"),
        "{outcome:?}"
    );
}

/// A deterministic generator of small mixed archives: explicit trees with shared external
/// tilesets (memo replay), transforms, content volumes, repeat references, escapes,
/// exclusions, node-depth stops and late C1 failures.
struct Gen(u64);
impl Gen {
    fn next(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }
    fn payload(&mut self) -> Vec<u8> {
        let x = [0., 3., 9., 40., 400.][self.next(5) as usize];
        let nested = usize::from(self.next(6) == 0) * 2;
        let extensions = if self.next(8) == 0 {
            vec!["EXT_generated".to_owned()]
        } else {
            vec![]
        };
        points_at(&[[x, 0., 0.], [-1., 1., 0.5]], nested, &extensions)
    }
    fn tile(&mut self, half: f64, depth: u32, files: &mut Files, docs: &[String]) -> Value {
        let mut t = tile(half, f64::from(10 - depth));
        if self.next(5) == 0 {
            let mut m = IDENTITY;
            m[12] = [0.5, -2., 30.][self.next(3) as usize];
            t["transform"] = json!(m);
        }
        if self.next(12) == 0 {
            t["metadata"] = json!({"class":"c"});
        }
        if self.next(30) == 0 {
            t["geometricError"] = json!(50);
        }
        let mut contents = Vec::new();
        for _ in 0..self.next(3) {
            let pick = self.next(10);
            if pick < 2 && !docs.is_empty() && depth < 6 {
                contents.push(json!({"uri": docs[self.next(docs.len() as u64) as usize]}));
            } else {
                let name = format!("p{}.glb", self.next(6));
                if !files.iter().any(|(n, _)| *n == name) {
                    let payload = self.payload();
                    files.push((name.clone(), payload));
                }
                let mut content = json!({"uri": name});
                if self.next(4) == 0 {
                    content["boundingVolume"] = boxed([0.; 3], half * 0.75);
                }
                contents.push(content);
            }
        }
        if !contents.is_empty() {
            t["contents"] = Value::Array(contents);
        }
        if depth < 5 {
            let children: Vec<Value> = (0..self.next(3))
                .map(|_| self.tile(half * 0.9, depth + 1, files, docs))
                .collect();
            if !children.is_empty() {
                t["children"] = Value::Array(children);
            }
        }
        t
    }
    fn archive(&mut self) -> Files {
        let mut files = Files::new();
        let mut docs: Vec<String> = Vec::new();
        // Leaf documents first, so that later documents may reference them (a DAG).
        for i in 0..self.next(4) {
            let root = self.tile(80., 1, &mut files, &docs);
            let name = format!("d{i}.json");
            files.push((name.clone(), doc(root)));
            docs.push(name);
        }
        let root = self.tile(100., 0, &mut files, &docs);
        files.push(("tileset.json".into(), doc(root)));
        // Unreferenced members are dropped, so most archives reach C2.
        let used: Vec<String> = files
            .iter()
            .filter(|(_, bytes)| bytes.starts_with(b"{"))
            .flat_map(|(_, bytes)| {
                let text = String::from_utf8_lossy(bytes).into_owned();
                text.split('"')
                    .filter(|s| s.ends_with(".glb") || s.ends_with(".json"))
                    .map(String::from)
                    .collect::<Vec<_>>()
            })
            .collect();
        files.retain(|(name, _)| name == "tileset.json" || used.contains(name));
        files
    }
}

#[test]
fn generated_archives_match_the_reference_traversal_in_every_mode() {
    let mut seen = BTreeSet::new();
    let mut generator = Gen(0x9e37_79b9_7f4a_7c15);
    for _ in 0..160 {
        let files = generator.archive();
        for limits in [
            ValidationLimits::default(),
            ValidationLimits {
                containment_node_depth: 1,
                ..ValidationLimits::default()
            },
        ] {
            let outcome = agreed(&files, &limits);
            let class = if outcome.result.starts_with('{') {
                match reasons(&outcome).first() {
                    Some(reason) => format!("notInspected:{reason}"),
                    None => "contained".into(),
                }
            } else {
                outcome
                    .result
                    .split(':')
                    .take(2)
                    .collect::<Vec<_>>()
                    .join(":")
            };
            seen.insert(class);
        }
    }
    // The generator reaches every outcome class this control is about.
    for class in [
        "contained",
        "notInspected:tileMetadata",
        "notInspected:workLimit",
        "notInspected:gltfExtension",
        "invalid_input: decoded content outside bounding volume",
        "invalid_input: geometricError increases",
    ] {
        assert!(
            seen.iter().any(|s| s.starts_with(class)),
            "{class} not reached: {seen:?}"
        );
    }
}

#[test]
fn an_exhausted_memo_falls_back_to_the_reference_traversal() {
    let mut generator = Gen(0x2545_f491_4f6c_dd1d);
    let mut fell_back = 0;
    for _ in 0..80 {
        let files = generator.archive();
        let (_work, path) = package(&files);
        let limits = Budgets::Absolute(ValidationLimits::default());
        let (full, prepared, peak) = inspect_in(&path, limits.clone(), INLINE);
        assert!(peak <= MEMO_BYTES);
        for budget in [0, size_of::<Event>(), 8 * size_of::<Event>(), 4096] {
            let mode = Mode {
                memo_budget: Some(budget),
                ..INLINE
            };
            let (bounded, bounded_prepared, bounded_peak) = inspect_in(&path, limits.clone(), mode);
            assert_eq!(bounded, full, "budget {budget}");
            assert!(
                bounded_peak <= budget,
                "memo held {bounded_peak} over {budget}"
            );
            if full.result.starts_with('{') {
                assert!(bounded_prepared >= prepared, "budget {budget}");
                fell_back += usize::from(bounded_prepared > prepared);
            }
        }
    }
    assert!(fell_back > 0, "a small budget re-traverses some document");
}

#[test]
fn live_documents_are_charged_while_on_the_path_and_released_at_exit() {
    let charge = |bytes: &[u8]| {
        let nodes = json::admit(
            bytes,
            content_limits(&ValidationLimits::default()).unwrap().json,
        )
        .unwrap()
        .value_nodes();
        bytes.len() as u64 + 128 * nodes as u64
    };
    // root -> a.json -> b.json is one path; c.json is a sibling of a.json.
    let mut root = tile(100., 10.);
    root["children"] = json!([
        with_content(tile(90., 9.), "a.json"),
        with_content(tile(90., 9.), "c.json")
    ]);
    let a = doc(with_content(tile(80., 8.), "b.json"));
    let b = doc(with_content(tile(70., 7.), "p.glb"));
    let mut big = with_content(tile(80., 8.), "p.glb");
    big["extras"] = json!({"padding": "x".repeat(4000)});
    let c = doc(big);
    let root = doc(root);
    let path = charge(&root) + charge(&a) + charge(&b);
    let other = charge(&root) + charge(&c);
    assert!(other > path, "the sibling alone is the larger live set");
    let files = files(vec![
        ("tileset.json", root),
        ("a.json", a),
        ("b.json", b),
        ("c.json", c),
        ("p.glb", point(0.)),
    ]);
    let at = |cap: u64| ValidationLimits {
        live_document_bytes: cap,
        ..ValidationLimits::default()
    };
    // a.json and b.json are released before c.json is read: the cap is the larger set.
    assert!(agreed(&files, &at(other)).result.starts_with('{'));
    assert_eq!(
        agreed(&files, &at(other - 1)).result,
        format!(
            "resource_limit: live tileset documents exceed {} bytes",
            other - 1
        )
    );
}

#[test]
fn archive_named_labels_keep_the_smallest_sixty_four() {
    let names = |n: usize| -> Vec<String> { (0..n).map(|i| format!("EXT_x{i:03}")).collect() };
    let report = |extensions: &[String]| -> Value {
        let files = siblings(vec![
            (
                json!({}),
                "a.glb",
                points_at(&[[0., 0., 0.]], 0, &extensions[..extensions.len() / 2]),
            ),
            (
                json!({}),
                "b.glb",
                points_at(&[[0., 0., 0.]], 0, &extensions[extensions.len() / 2..]),
            ),
        ]);
        serde_json::from_str(&agreed(&files, &ValidationLimits::default()).result).unwrap()
    };
    let listed = |report: &Value| -> Vec<String> {
        report["notInspected"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| {
                v.as_str()
                    .filter(|s| s.starts_with("EXT_"))
                    .map(String::from)
            })
            .collect()
    };
    let exact = report(&names(64));
    assert_eq!(listed(&exact), names(64));
    assert!(exact.get("notInspectedTruncated").is_none());
    let mut reversed = names(70);
    reversed.reverse();
    let over = report(&reversed);
    assert_eq!(listed(&over), names(64));
    assert_eq!(over["notInspectedTruncated"], true);
    let mut long = names(3);
    long.push(format!("EXT_{}", "y".repeat(253)));
    let long = report(&long);
    assert_eq!(listed(&long), names(3));
    assert_eq!(long["notInspectedTruncated"], true);
    let mut fits = names(3);
    fits.push(format!("EXT_{}", "y".repeat(252)));
    let fits = report(&fits);
    assert_eq!(listed(&fits).len(), 4);
    assert!(fits.get("notInspectedTruncated").is_none());
}

#[test]
fn budgets_scale_from_stored_bytes_at_their_crossovers() {
    let floors = ValidationLimits::default();
    let at = |stored: u64| super::LimitPlan::default().resolve(stored).0;
    let budgets = |l: &ValidationLimits| {
        [
            l.hierarchy_visits,
            l.references,
            l.total_bytes_read,
            l.total_payload_elements,
            l.containment_work_units,
            l.containment_bytes_read,
            l.containment_decoded_bytes,
        ]
    };
    let floor = budgets(&floors);
    // (crossover S, index, S step, budget step): at S the floor holds; one step above it,
    // the budget is κ·S.
    for (stored, index, step, grows) in [
        (1u64 << 21, 0, 8, 1),
        (1 << 23, 1, 8, 1),
        (1 << 29, 2, 1, 4),
        (1_000_000, 3, 1, 16),
        (1 << 22, 4, 1, 128),
        (1 << 29, 5, 1, 4),
        (1 << 27, 6, 1, 16),
    ] {
        assert_eq!(budgets(&at(stored))[index], floor[index], "index {index}");
        assert_eq!(
            budgets(&at(stored - 1))[index],
            floor[index],
            "index {index}"
        );
        assert_eq!(
            budgets(&at(stored + step))[index],
            floor[index] + grows,
            "index {index}"
        );
    }
    assert_eq!(budgets(&at(0)), floor);
    assert_eq!(budgets(&at(u64::MAX))[2], u64::MAX);
    assert_eq!(budgets(&at(u64::MAX))[0], u64::MAX / 8);
    // Per-item and volume limits never scale.
    let scaled = at(1 << 40);
    assert_eq!(
        (
            scaled.json_bytes,
            scaled.member_bytes,
            scaled.archive_entries
        ),
        (
            floors.json_bytes,
            floors.member_bytes,
            floors.archive_entries
        )
    );
    assert_eq!(scaled.live_document_bytes, floors.live_document_bytes);
}

#[test]
fn default_validation_scales_the_visit_budget_with_stored_bytes() {
    // OCTREE, subtreeLevels 7, constant availability: 299,593 tiles, more than 2^18 visits.
    let mut root = tile(100., 10.);
    root["implicitTiling"] = json!({"subdivisionScheme":"OCTREE","subtreeLevels":7,
        "availableLevels":7,"subtrees":{"uri":"s/{level}/{x}/{y}/{z}.subtree"}});
    let subtree = {
        let json =
            br#"{"tileAvailability":{"constant":1},"childSubtreeAvailability":{"constant":0}}"#;
        let mut text = json.to_vec();
        while !text.len().is_multiple_of(8) {
            text.push(b' ');
        }
        let mut out = b"subt".to_vec();
        out.extend(1u32.to_le_bytes());
        out.extend((text.len() as u64).to_le_bytes());
        out.extend(0u64.to_le_bytes());
        out.extend(text);
        out
    };
    // Whitespace makes S large enough for S/8 visits: 1 + 1 + 299,593 + 1 = 299,596.
    let mut tileset = doc(root);
    tileset.extend(std::iter::repeat_n(b' ', 2_500_000));
    let files = files(vec![
        ("tileset.json", tileset),
        ("s/0/0/0/0.subtree", subtree),
    ]);
    let (_work, path) = package(&files);
    let refused = inspect_selected(ValidationRequest::new(&path), ValidationLimits::default());
    assert!(
        matches!(&refused, Err(Error::ResourceLimit(m)) if m.message().contains("hierarchy visits")),
        "{refused:?}"
    );
    let report = inspect(ValidationRequest::new(&path)).unwrap();
    assert_eq!(report.tiles, 299_593);
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    let stored: u64 = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().compressed_size())
        .sum();
    assert!(stored / 8 > 299_596);
    assert_eq!(report.limits.hierarchy_visits, stored / 8);
    assert_eq!(
        report.limits.json_bytes,
        ValidationLimits::default().json_bytes
    );
}

#[test]
fn duplicate_children_fail_with_the_schema_library_message() {
    let mut root = tile(100., 10.);
    root["children"] = json!([tile(10., 1.), tile(10., 1.0)]);
    let tileset = doc(root);
    let schema: Value =
        serde_json::from_str(include_str!("../../docs/schema/tileset.schema.json")).unwrap();
    let parsed: Value = serde_json::from_slice(&tileset).unwrap();
    let expected = jsonschema::validator_for(&schema)
        .unwrap()
        .validate(&parsed)
        .unwrap_err()
        .to_string();
    let outcome = agreed(
        &files(vec![("tileset.json", tileset)]),
        &ValidationLimits::default(),
    );
    assert_eq!(
        outcome.result,
        format!("invalid_input: tileset schema: {expected}")
    );
}

#[test]
fn a_replayed_document_composes_its_own_tiles_under_each_placement() {
    // One shared document (root and child) placed twice; the second placement is replayed.
    // Only the second placement's referencing content volume is a rotated box, so only the
    // replayed root carries the `rotatedBox` chain bit, which its child must inherit.
    let mut root = tile(50., 5.);
    root["children"] = json!([with_content(tile(40., 4.), "p.glb")]);
    let shared = doc(root);
    let mut top = tile(100., 10.);
    let mut a = tile(90., 9.);
    a["content"] = json!({"uri":"d.json","boundingVolume":boxed([0.; 3], 60.)});
    let mut b = tile(90., 9.);
    b["content"] = json!({"uri":"d.json","boundingVolume":
        {"box":[0,0,0, 30,30,0, -30,30,0, 0,0,40]}});
    top["children"] = json!([a, b]);
    let files = files(vec![
        ("tileset.json", doc(top)),
        ("d.json", shared),
        ("p.glb", point(1.)),
    ]);
    let outcome = agreed(&files, &ValidationLimits::default());
    assert_eq!(reasons(&outcome), ["rotatedBox"], "{outcome:?}");
}
