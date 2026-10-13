//! EM controls: the memoized traversal against the reference traversal, and
//! per-placement checks of a document that is valid under its first placement.
use super::containment_controls::points;
use super::*;
use std::path::PathBuf;

type Files = Vec<(String, Vec<u8>)>;

fn boxed(center: [f64; 3], half: f64) -> Value {
    json!({"box":[center[0],center[1],center[2],half,0,0,0,half,0,0,0,half]})
}
fn tile(half: f64, error: f64) -> Value {
    json!({"boundingVolume":boxed([0.; 3], half),"geometricError":error})
}
fn translated(mut tile: Value, x: f64) -> Value {
    let mut m = IDENTITY;
    m[12] = x;
    tile["transform"] = json!(m);
    tile
}
fn doc(root: Value, error: f64) -> Vec<u8> {
    serde_json::to_vec(&json!({"asset":{"version":"1.1"},"geometricError":error,"root":root}))
        .unwrap()
}
fn files(entries: &[(&str, Vec<u8>)]) -> Files {
    entries
        .iter()
        .map(|(name, bytes)| (name.to_string(), bytes.clone()))
        .collect()
}
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

/// One inspection's comparable outcome: the serialized report or the error
/// category and message, with the C2 units, documents prepared and counters.
#[derive(Debug, PartialEq)]
struct Outcome {
    result: String,
    units: u64,
    prepared: u64,
    counters: [u64; 3],
}
fn inspect_with(path: &Path, limits: ValidationLimits, memoize: bool) -> Outcome {
    MEMOIZE.with(|m| m.set(memoize));
    let inspection = inspect_facts(ValidationRequest::new(path), limits);
    MEMOIZE.with(|m| m.set(true));
    match inspection {
        Ok(inspection) => Outcome {
            result: serde_json::to_string(&inspection.report).unwrap(),
            units: inspection.units,
            prepared: inspection.prepared,
            counters: inspection.counters,
        },
        Err(error) => Outcome {
            result: format!("{}: {error}", error.category().0),
            units: 0,
            prepared: 0,
            counters: [0; 3],
        },
    }
}
/// EM-1: the memoized result equals the reference traversal's, including
/// the final counters (EM-2). Returns the memoized outcome.
fn same(path: &Path, limits: &ValidationLimits) -> Outcome {
    let memo = inspect_with(path, limits.clone(), true);
    let reference = inspect_with(path, limits.clone(), false);
    assert_eq!(
        (&memo.result, memo.units, memo.counters),
        (&reference.result, reference.units, reference.counters),
        "memo and reference traversals differ under {limits:?}"
    );
    memo
}
fn run(files: &Files, limits: ValidationLimits) -> Outcome {
    let (_work, path) = package(files);
    same(&path, &limits)
}
fn error(outcome: &Outcome) -> &str {
    assert!(!outcome.result.starts_with('{'), "expected a failure");
    &outcome.result
}

/// A `K`-way DAG of `D + 1` documents: document `i` names `d(i+1).json` `K`
/// times, and the last names `g.glb`.
fn dag(depth: usize, branch: usize) -> Files {
    let name = |i: usize| match i {
        0 => "tileset.json".to_string(),
        i => format!("d{i}.json"),
    };
    let mut out = Files::new();
    for i in 0..=depth {
        let mut root = tile(10., 0.);
        let uri = if i < depth {
            format!("d{}.json", i + 1)
        } else {
            "g.glb".into()
        };
        let n = if i < depth { branch } else { 1 };
        root["contents"] = (0..n).map(|_| json!({"uri": uri})).collect();
        out.push((name(i), doc(root, 0.)));
    }
    out.push(("g.glb".into(), points(&[[1., 0., 0.]])));
    out
}

#[test]
fn each_distinct_document_is_prepared_once() {
    let (_work, path) = package(&dag(6, 2));
    let limits = ValidationLimits::default();
    let memo = inspect_with(&path, limits.clone(), true);
    let reference = inspect_with(&path, limits, false);
    assert_eq!(
        (memo.result.clone(), memo.counters),
        (reference.result, reference.counters)
    );
    assert!(memo.result.contains("\"tiles\":127"), "{}", memo.result);
    assert_eq!((memo.prepared, reference.prepared), (7, 127));
    // The C2 stream ran: every placement's reference is registered again.
    assert!(memo.result.contains("decodedContentContainment"));
}

#[test]
fn root_bounds_are_checked_at_every_placement() {
    let ext = doc(tile(10., 0.), 0.);
    let tree = |second: f64| {
        let mut root = tile(20., 0.);
        let mut first = tile(10., 0.);
        first["content"] = json!({"uri":"ext.json"});
        // Distinct children: the tileset schema requires unique items.
        first["refine"] = json!("REPLACE");
        let mut other = tile(second, 0.);
        other["content"] = json!({"uri":"ext.json"});
        root["children"] = json!([first, other]);
        files(&[("tileset.json", doc(root, 0.)), ("ext.json", ext.clone())])
    };
    let valid = run(&tree(10.), ValidationLimits::default());
    assert_eq!(valid.prepared, 2, "{}", valid.result);
    let failed = run(&tree(9.), ValidationLimits::default());
    assert_eq!(
        error(&failed),
        "invalid_input: child bounds escape parent at depth 2"
    );
}

#[test]
fn root_error_is_checked_at_every_placement() {
    let ext = doc(tile(10., 5.), 100.);
    let tree = |second: f64| {
        let mut root = tile(20., 20.);
        let mut first = tile(10., 10.);
        first["content"] = json!({"uri":"ext.json"});
        first["refine"] = json!("REPLACE");
        let mut other = tile(10., second);
        other["content"] = json!({"uri":"ext.json"});
        root["children"] = json!([first, other]);
        files(&[("tileset.json", doc(root, 20.)), ("ext.json", ext.clone())])
    };
    let valid = run(&tree(10.), ValidationLimits::default());
    assert_eq!(valid.prepared, 2, "{}", valid.result);
    let failed = run(&tree(1.), ValidationLimits::default());
    assert_eq!(
        error(&failed),
        "invalid_input: geometricError increases at depth 2"
    );
    // The document's own geometricError also bounds its root at each placement.
    let tight = doc(tile(10., 5.), 4.);
    let mut root = tile(20., 20.);
    root["content"] = json!({"uri":"ext.json"});
    let failed = run(
        &files(&[("tileset.json", doc(root, 20.)), ("ext.json", tight)]),
        ValidationLimits::default(),
    );
    assert_eq!(
        error(&failed),
        "invalid_input: geometricError increases at depth 1"
    );
}

/// The same document is contained under its first placement and escapes an
/// ancestor under the second, whose tile transform moves it by 1e-5: within
/// C1's hierarchy tolerance, outside C2's exact test.
#[test]
fn decoded_containment_composes_each_placements_transform() {
    let mut ext_root = tile(10., 0.);
    ext_root["content"] = json!({"uri":"a.glb"});
    let tree = |x: f64| {
        let mut root = tile(10., 0.);
        let mut first = tile(10., 0.);
        first["content"] = json!({"uri":"ext.json"});
        let mut other = translated(tile(10., 0.), x);
        other["content"] = json!({"uri":"ext.json"});
        root["children"] = json!([first, other]);
        files(&[
            ("tileset.json", doc(root, 0.)),
            ("ext.json", doc(ext_root.clone(), 0.)),
            ("a.glb", points(&[[10., 0., 0.]])),
        ])
    };
    let valid = run(&tree(0.), ValidationLimits::default());
    assert!(valid.result.contains("decodedContentContainment"));
    assert_eq!(valid.prepared, 2);
    let failed = run(&tree(1e-5), ValidationLimits::default());
    assert!(
        error(&failed).contains("content=a.glb depth=2 volume=ancestor:2 "),
        "{}",
        failed.result
    );
}

/// The same document is placed at depth 1, then at depth 3 below a chain, and
/// escapes only under the second. The failure reports the second placement's
/// depth, not the first's (a replay that kept the first absolute depth would
/// report depth 2).
#[test]
fn decoded_containment_reports_each_placements_depth() {
    let mut ext_root = tile(10., 0.);
    ext_root["content"] = json!({"uri":"a.glb"});
    let tree = |x: f64| {
        let mut first = tile(10., 0.);
        first["content"] = json!({"uri":"ext.json"});
        let mut deep = translated(tile(10., 0.), x);
        deep["content"] = json!({"uri":"ext.json"});
        let mut mid = tile(10., 0.);
        mid["children"] = json!([deep]);
        let mut chain = tile(10., 0.);
        chain["children"] = json!([mid]);
        let mut root = tile(10., 0.);
        root["children"] = json!([first, chain]);
        files(&[
            ("tileset.json", doc(root, 0.)),
            ("ext.json", doc(ext_root.clone(), 0.)),
            ("a.glb", points(&[[10., 0., 0.]])),
        ])
    };
    let valid = run(&tree(0.), ValidationLimits::default());
    assert!(valid.result.contains("decodedContentContainment"));
    assert_eq!(valid.prepared, 2);
    let failed = run(&tree(1e-5), ValidationLimits::default());
    assert!(
        error(&failed).contains("content=a.glb depth=4 "),
        "{}",
        failed.result
    );
}

/// The same document is contained under its first placement and escapes the
/// referencing content's `boundingVolume` under the second (I2).
#[test]
fn decoded_containment_tests_each_placements_external_content_volume() {
    let mut ext_root = tile(10., 0.);
    ext_root["content"] = json!({"uri":"a.glb"});
    let tree = |half: f64| {
        let mut root = tile(20., 0.);
        root["contents"] = json!([
            {"uri":"ext.json"},
            {"uri":"ext.json","boundingVolume":boxed([0.; 3], half)}
        ]);
        files(&[
            ("tileset.json", doc(root, 0.)),
            ("ext.json", doc(ext_root.clone(), 0.)),
            ("a.glb", points(&[[8., 0., 0.]])),
        ])
    };
    let valid = run(&tree(9.), ValidationLimits::default());
    assert!(valid.result.contains("decodedContentContainment"));
    assert_eq!(valid.prepared, 2);
    let failed = run(&tree(5.), ValidationLimits::default());
    assert!(
        error(&failed).contains("content=a.glb depth=1 volume=externalContent:1 "),
        "{}",
        failed.result
    );
}

/// A three-level document placed at depth 1, then at depth 3 below a chain.
#[test]
fn depth_limits_apply_at_every_placement() {
    let mut inner = tile(10., 0.);
    inner["children"] = json!([{"boundingVolume":boxed([0.; 3], 10.),"geometricError":0,
        "children":[tile(10., 0.)]}]);
    let mut root = tile(10., 0.);
    let mut chain = tile(10., 0.);
    chain["children"] = json!([{"boundingVolume":boxed([0.; 3], 10.),"geometricError":0,
        "content":{"uri":"ext.json"}}]);
    let mut first = tile(10., 0.);
    first["content"] = json!({"uri":"ext.json"});
    root["children"] = json!([first, chain]);
    let archive = files(&[
        ("tileset.json", doc(root, 0.)),
        ("ext.json", doc(inner, 0.)),
    ]);
    // Tiles reach depth 3 + 2 = 5 under the second placement.
    let fits = run(
        &archive,
        ValidationLimits {
            hierarchy_depth: 5,
            ..ValidationLimits::default()
        },
    );
    assert_eq!(fits.prepared, 2);
    let failed = run(
        &archive,
        ValidationLimits {
            hierarchy_depth: 4,
            ..ValidationLimits::default()
        },
    );
    assert_eq!(
        error(&failed),
        "resource_limit: hierarchy exceeds validation depth limit"
    );
    // The 128 tileset-task bound, with tiles allowed deeper: `ext.json` names
    // `leaf.json`, at depth 2 under the first placement and 129 under the
    // second, which a chain of documents `c2.json` … `c127.json` reaches.
    let linked = |uri: &str| {
        let mut tile = tile(10., 0.);
        tile["content"] = json!({ "uri": uri });
        tile
    };
    let mut root = tile(10., 0.);
    root["children"] = json!([linked("ext.json"), linked("c2.json")]);
    let mut archive = files(&[
        ("tileset.json", doc(root, 0.)),
        ("ext.json", doc(linked("leaf.json"), 0.)),
        ("leaf.json", doc(tile(10., 0.), 0.)),
    ]);
    for k in 2..=127 {
        let next = if k == 127 {
            "ext.json".to_string()
        } else {
            format!("c{}.json", k + 1)
        };
        archive.push((format!("c{k}.json"), doc(linked(&next), 0.)));
    }
    let failed = run(
        &archive,
        ValidationLimits {
            hierarchy_depth: 1000,
            ..ValidationLimits::default()
        },
    );
    assert_eq!(
        error(&failed),
        "resource_limit: hierarchy exceeds validation depth limit"
    );
}

/// Every visit, reference and byte cap from the first charge to one past the
/// total: the memo trips each limit where the reference traversal does (EM-6).
#[test]
fn every_counter_limit_trips_where_the_reference_traversal_trips() {
    let mut archive = dag(4, 2);
    // A shared schemaUri: one steady reference and read per placement.
    let mut d2: Value = serde_json::from_slice(&archive[2].1).unwrap();
    d2["schemaUri"] = json!("schema.json");
    archive[2].1 = serde_json::to_vec(&d2).unwrap();
    archive.push(("schema.json".into(), br#"{"id":"s","classes":{}}"#.to_vec()));
    let (_work, path) = package(&archive);
    let total = same(&path, &ValidationLimits::default());
    let [visits, references, bytes] = total.counters;
    assert_eq!(total.prepared, 5);
    let stride = sweep_stride();
    let mut checked = 0;
    for cap in (1..=visits + 1).step_by(stride) {
        same(
            &path,
            &ValidationLimits {
                hierarchy_visits: cap,
                ..Default::default()
            },
        );
        checked += 1;
    }
    for cap in (1..=references + 1).step_by(stride) {
        same(
            &path,
            &ValidationLimits {
                references: cap,
                ..Default::default()
            },
        );
        checked += 1;
    }
    let hashed = std::fs::metadata(&path).unwrap().len() / 2;
    let step = stride
        .max(((bytes + 1 - hashed) / byte_points()) as usize)
        .max(1);
    for cap in (hashed..=bytes + 1).step_by(step) {
        same(
            &path,
            &ValidationLimits {
                total_bytes_read: cap,
                ..Default::default()
            },
        );
        checked += 1;
    }
    assert!(checked > 30);
    for (field, cap) in [
        ("visits", visits - 1),
        ("references", references - 1),
        ("bytes", bytes - 1),
    ] {
        let limits = match field {
            "visits" => ValidationLimits {
                hierarchy_visits: cap,
                ..Default::default()
            },
            "references" => ValidationLimits {
                references: cap,
                ..Default::default()
            },
            _ => ValidationLimits {
                total_bytes_read: cap,
                ..Default::default()
            },
        };
        assert!(error(&same(&path, &limits)).starts_with("resource_limit: "));
    }
}

/// Byte caps per sweep: every byte would be thousands of runs per archive.
fn byte_points() -> u64 {
    std::env::var("EM_BYTE_POINTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10)
}
fn sweep_stride() -> usize {
    std::env::var("EM_SWEEP_STRIDE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
}

/// A small deterministic generator (SplitMix64).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn pick<T: Copy>(&mut self, values: &[T]) -> T {
        values[self.below(values.len() as u64) as usize]
    }
}

/// A random DAG of shared documents: placements under varied volumes, errors,
/// transforms and content volumes, shared payloads that escape some volumes,
/// a shared `schemaUri`, and, rarely, a back reference (a cycle).
fn random_archive(seed: u64) -> Files {
    let mut rng = Rng(seed);
    let documents = 2 + rng.below(4) as usize;
    let name = |i: usize| match i {
        0 => "tileset.json".to_string(),
        i => format!("d{i}.json"),
    };
    let mut used_payloads = BTreeSet::new();
    let mut schema = false;
    let mut out = Files::new();
    for i in 0..documents {
        // Mostly valid; each rare choice fails C1 or C2 under some placements.
        let mut serial = 0;
        let mut content = |rng: &mut Rng, required: Option<usize>| -> Value {
            let target = match required {
                Some(j) => Some(j),
                None if i + 1 < documents && rng.below(3) > 0 => {
                    Some(i + 1 + rng.below((documents - i - 1) as u64) as usize)
                }
                None if i > 0 && rng.below(40) == 0 => Some(rng.below(i as u64) as usize),
                None => None,
            };
            let mut entry = match target {
                Some(j) => json!({"uri": name(j)}),
                None => {
                    let payload = rng.pick(&["p0.glb", "p0.glb", "p1.glb", "p2.glb"]);
                    used_payloads.insert(payload);
                    json!({"uri": payload})
                }
            };
            if rng.below(4) == 0 {
                entry["boundingVolume"] = boxed([0.; 3], rng.pick(&[9.5, 10., 10.]));
            }
            // Distinct entries: the tileset schema requires unique items.
            serial += 1;
            entry["extras"] = json!({ "n": serial });
            entry
        };
        let rare = |rng: &mut Rng, n: u64| rng.below(n) == 0;
        let half = if rare(&mut rng, 20) { 10.5 } else { 10. };
        let mut root = tile(half, if rare(&mut rng, 20) { 1. } else { 0. });
        if rng.below(4) == 0 {
            root = translated(root, if rare(&mut rng, 10) { 0.25 } else { 1e-5 });
        }
        let mut entries = vec![];
        if i + 1 < documents {
            entries.push(content(&mut rng, Some(i + 1)));
        }
        for _ in 0..rng.below(3) {
            entries.push(content(&mut rng, None));
        }
        let mut children = vec![];
        for k in 0..rng.below(3) {
            let mut child = tile(if rare(&mut rng, 20) { 9.75 } else { 10. }, 0.);
            child["extras"] = json!({ "k": k });
            if rng.below(3) == 0 {
                child = translated(child, if rare(&mut rng, 10) { 0.5 } else { 1e-5 });
            }
            if rng.below(3) > 0 {
                child["content"] = content(&mut rng, None);
            }
            children.push(child);
        }
        if !entries.is_empty() {
            root["contents"] = json!(entries);
        }
        if !children.is_empty() {
            root["children"] = json!(children);
        }
        let mut value = json!({"asset":{"version":"1.1"},
            "geometricError": if rare(&mut rng, 12) { 0.5 } else { 1. }, "root": root});
        if rng.below(3) == 0 {
            value["schemaUri"] = json!("schema.json");
            schema = true;
        }
        out.push((name(i), serde_json::to_vec(&value).unwrap()));
    }
    for payload in used_payloads {
        let x = match payload {
            "p0.glb" => 1.,
            "p1.glb" => 9.75,
            _ => 10.,
        };
        out.push((payload.to_string(), points(&[[x, 0., 0.]])));
    }
    if schema {
        out.push(("schema.json".into(), br#"{"id":"s","classes":{}}"#.to_vec()));
    }
    out
}

/// EM-1 over a generated corpus, at the default limits and across every
/// counter, depth and C2 work cap of each archive.
#[test]
fn generated_dags_match_the_reference_traversal_at_every_cap() {
    let seeds: u64 = std::env::var("EM_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let stride = sweep_stride();
    let (mut accepted, mut failed, mut memoized) = (0, 0, 0);
    for seed in 0..seeds {
        let archive = random_archive(seed);
        let (_work, path) = package(&archive);
        let base = same(&path, &ValidationLimits::default());
        if std::env::var_os("EM_DEBUG").is_some() {
            eprintln!(
                "seed {seed}: {}",
                base.result.chars().take(160).collect::<String>()
            );
        }
        if base.result.starts_with('{') {
            accepted += 1;
        } else {
            failed += 1;
        }
        let reference = inspect_with(&path, ValidationLimits::default(), false);
        if base.prepared > 0 && base.prepared < reference.prepared {
            memoized += 1;
        }
        let [visits, references, bytes] = if base.result.starts_with('{') {
            base.counters
        } else {
            [400, 800, std::fs::metadata(&path).unwrap().len() * 8]
        };
        for cap in (1..=visits + 1).step_by(stride) {
            same(
                &path,
                &ValidationLimits {
                    hierarchy_visits: cap,
                    ..Default::default()
                },
            );
        }
        for cap in (1..=references + 1).step_by(stride) {
            same(
                &path,
                &ValidationLimits {
                    references: cap,
                    ..Default::default()
                },
            );
        }
        let low = std::fs::metadata(&path).unwrap().len() / 2;
        let points = byte_points();
        for cap in
            (low..=bytes + 1).step_by(stride.max(((bytes + 1 - low) / points) as usize).max(1))
        {
            same(
                &path,
                &ValidationLimits {
                    total_bytes_read: cap,
                    ..Default::default()
                },
            );
        }
        for cap in 0..12 {
            same(
                &path,
                &ValidationLimits {
                    hierarchy_depth: cap,
                    ..Default::default()
                },
            );
        }
        for units in [1, 1500, 5000, 20_000, 100_000] {
            same(
                &path,
                &ValidationLimits {
                    containment_work_units: units,
                    ..Default::default()
                },
            );
        }
    }
    // The corpus exercises both verdicts and the memo itself.
    assert!(
        accepted >= 3 && failed >= 3 && memoized >= 3,
        "{accepted} {failed} {memoized}"
    );
}
