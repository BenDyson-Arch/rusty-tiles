//! `--reuse-tileset` from implicit-forest archives (design §4.7, A-8). A forest
//! archive holds each payload once: explicit tiles in place as `t/<sha256>`,
//! implicit tiles rebased under `implicit-content/`. Reuse recovers the raw bytes
//! through the `vector-build.json` payload map and the writer's exact inverse.
mod support;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

type Members = BTreeMap<String, Vec<u8>>;

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Squares and lines on a grid with stable GeoJSON ids. `edit` changes one
/// polygon attribute and nudges one line vertex, away from the frame anchor.
fn features(edit: bool) -> Value {
    let mut features = Vec::new();
    for i in 0..64 {
        let (x, y) = ((i % 8) as f64 * 40., (i / 8) as f64 * 40.);
        let ring = json!([
            [x, y, 0],
            [x + 12., y, 0],
            [x + 12., y + 12., 0],
            [x, y + 12., 0],
            [x, y, 0]
        ]);
        let hole = json!([
            [x + 4., y + 4., 0],
            [x + 4., y + 8., 0],
            [x + 8., y + 8., 0],
            [x + 8., y + 4., 0],
            [x + 4., y + 4., 0]
        ]);
        let name = if edit && i == 37 {
            "edited".to_string()
        } else {
            format!("parcel-{i}")
        };
        features.push(
            json!({"type":"Feature","id":format!("p{i}"),"properties":{"name":name,"height":i},
            "geometry":{"type":"Polygon","coordinates":[ring,hole]}}),
        );
        let nudge = if edit && i == 50 { 0.25 } else { 0. };
        let line: Vec<_> = (0..24)
            .map(|j| {
                json!([
                    x + 20. + j as f64,
                    y + 20. + (j as f64 * 0.7).sin() + if j == 11 { nudge } else { 0. },
                    0
                ])
            })
            .collect();
        features.push(
            json!({"type":"Feature","id":format!("l{i}"),"properties":{"name":format!("road-{i}")},
            "geometry":{"type":"LineString","coordinates":line}}),
        );
    }
    json!({"type":"FeatureCollection","features":features})
}

struct Case {
    directory: tempfile::TempDir,
    flags: Vec<String>,
}

impl Case {
    fn new(flags: &[&str]) -> Self {
        let directory = tempfile::tempdir().unwrap();
        // One file name for every input: the layer name is part of the reuse identity.
        for (name, edit) in [("base", false), ("edit", true)] {
            fs::create_dir(directory.path().join(name)).unwrap();
            fs::write(
                directory.path().join(name).join("parcels.geojson"),
                features(edit).to_string(),
            )
            .unwrap();
        }
        Self {
            directory,
            flags: flags.iter().map(|flag| flag.to_string()).collect(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.directory.path().join(name)
    }

    fn command(&self, input: &str, output: &Path, previous: Option<&Path>) -> std::process::Output {
        let mut command = support::rusty_tiles();
        command
            .args(["vector", "--json", "-i"])
            .arg(self.path(input).join("parcels.geojson"))
            .arg("-o")
            .arg(output)
            .args(["--sourceCrs", "local", "--reproducible", "--jobs", "2"])
            .args(&self.flags);
        if let Some(previous) = previous {
            command.arg("--reuseTileset").arg(previous);
        }
        command.output().unwrap()
    }

    /// Convert and return the archive's members and its conversion report.
    fn convert(
        &self,
        input: &str,
        name: &str,
        previous: Option<&Path>,
    ) -> (PathBuf, Members, Value) {
        let output = self.path(name);
        let result = self.command(input, &output, previous);
        assert!(
            result.status.success(),
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        let members = members(&output);
        let report = serde_json::from_slice(&members["conversion.json"]).unwrap();
        (output, members, report)
    }

    /// Reuse must fail with `message`, publish nothing and leave the baseline intact.
    #[track_caller]
    fn refuse(&self, previous: &Path, message: &str) {
        let before = fs::read(previous).unwrap();
        let output = self.path("refused.3tz");
        let result = self.command("base", &output, Some(previous));
        assert!(
            !result.status.success(),
            "reuse of a damaged baseline succeeded"
        );
        let response: Value = serde_json::from_slice(&result.stdout).unwrap();
        let actual = response["error"]["message"].as_str().unwrap();
        assert!(actual.contains(message), "{actual:?} lacks {message:?}");
        assert!(!output.exists(), "failed reuse published output");
        assert_eq!(fs::read(previous).unwrap(), before);
    }
}

fn members(path: &Path) -> Members {
    let mut archive = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut result = BTreeMap::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        assert!(result.insert(file.name().to_string(), bytes).is_none());
    }
    result
}

/// Rebuild an archive with `change` applied to member bytes (`None` drops a member).
fn rewrite(source: &Members, target: &Path, change: impl Fn(&str, &[u8]) -> Option<Vec<u8>>) {
    let mut writer = zip::ZipWriter::new(fs::File::create(target).unwrap());
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in source {
        if let Some(bytes) = change(name, bytes) {
            writer.start_file(name.as_str(), options).unwrap();
            writer.write_all(&bytes).unwrap();
        }
    }
    writer.finish().unwrap();
}

/// Replace `vector-build.json` and rebind it through `vectorBuildStateSha256`.
fn with_state(source: &Members, target: &Path, edit: impl Fn(&mut Value)) {
    let mut state: Value = serde_json::from_slice(&source["vector-build.json"]).unwrap();
    edit(&mut state);
    let raw = serde_json::to_vec(&state).unwrap();
    let mut manifest: Value = serde_json::from_slice(&source["tileset.json"]).unwrap();
    manifest["asset"]["extras"]["vectorBuildStateSha256"] = json!(sha256(&raw));
    let manifest = serde_json::to_vec(&manifest).unwrap();
    rewrite(source, target, |name, bytes| {
        Some(match name {
            "vector-build.json" => raw.clone(),
            "tileset.json" => manifest.clone(),
            _ => bytes.to_vec(),
        })
    });
}

/// Members outside the build's volatile reports: `conversion.json`, the geometry
/// reports (a reuse build reports only newly encoded geometry) and the 3tz index,
/// whose member offsets follow the report sizes.
fn without_report(members: &Members) -> Members {
    let mut members = members.clone();
    for name in [
        "conversion.json",
        "geometry-reports.jsonl",
        "@3dtilesIndex1@",
    ] {
        members.remove(name);
    }
    members
}

#[track_caller]
fn assert_same(actual: &Members, expected: &Members, label: &str) {
    let (actual, expected) = (without_report(actual), without_report(expected));
    let differing: BTreeSet<_> = actual
        .keys()
        .chain(expected.keys())
        .filter(|name| actual.get(*name) != expected.get(*name))
        .collect();
    assert!(
        differing.is_empty(),
        "{label}: members differ: {differing:?}"
    );
}

fn content_uris(node: &Value, out: &mut BTreeSet<String>) {
    let single = node.get("content").into_iter();
    for content in node["contents"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(single)
    {
        if let Some(uri) = content["uri"].as_str() {
            out.insert(uri.to_string());
        }
    }
    for child in node["children"].as_array().into_iter().flatten() {
        content_uris(child, out);
    }
}

/// The v3 layout facts reuse depends on (§4.1 "No retained sources", P4, §4.7).
/// Returns the number of raw contents recoverable only through the payload map,
/// and the number of map entries with a nonzero rebase (the others are byte copies).
fn assert_forest_layout(members: &Members) -> (usize, usize) {
    let mut referenced = BTreeSet::new();
    for (name, bytes) in members {
        if name == "tileset.json" || (name.starts_with("tileset-d") && name.ends_with(".json")) {
            let document: Value = serde_json::from_slice(bytes).unwrap();
            assert!(
                !String::from_utf8_lossy(bytes).contains("rustyTilesSourceContents"),
                "{name} retains raw sources"
            );
            content_uris(&document["root"], &mut referenced);
        }
    }
    for name in members.keys().filter(|name| name.starts_with("t/")) {
        assert!(referenced.contains(name), "unreferenced raw member {name}");
    }
    let state: Value = serde_json::from_slice(&members["vector-build.json"]).unwrap();
    let payloads = state["payloads"].as_object().expect("payload map");
    let rebased = payloads
        .values()
        .filter(|entry| entry["delta"] != json!([0.0, 0.0, 0.0]))
        .count();
    for (key, entry) in payloads {
        let member = entry["member"].as_str().unwrap();
        assert!(member.starts_with("implicit-content/"), "{member}");
        assert!(
            members.contains_key(member),
            "payload {key} names absent {member}"
        );
    }
    let mut raw = BTreeSet::new();
    content_uris(&state["explicitRoot"], &mut raw);
    let mut mapped_only = 0;
    for uri in &raw {
        let stem = uri
            .strip_prefix("t/")
            .and_then(|name| name.rsplit_once('.'))
            .unwrap()
            .0;
        let in_place = members.contains_key(uri);
        assert!(
            in_place || payloads.contains_key(stem),
            "{uri} is unrecoverable"
        );
        mapped_only += usize::from(!in_place);
    }
    (mapped_only, rebased)
}

/// A forest archive is a baseline for unchanged and edited input: reuse equals a
/// fresh build byte for byte (outside the per-build reports), for every encoding.
/// `rebased` requires payloads whose recovery inverts a nonzero rebase.
fn reuse_matches_fresh(flags: &[&str], rebased: bool) {
    let case = Case::new(flags);
    let (base, base_members, _) = case.convert("base", "base.3tz", None);
    let (mapped, nonzero) = assert_forest_layout(&base_members);
    assert!(
        mapped > 0,
        "{flags:?}: no payload is published only under implicit-content/"
    );
    assert!(
        !rebased || nonzero > 0,
        "{flags:?}: no payload is published with a nonzero rebase"
    );
    assert!(
        base_members.keys().any(|name| name.starts_with("t/")),
        "{flags:?}: no explicit tile publishes in place"
    );
    let (_, same, report) = case.convert("base", "same.3tz", Some(&base));
    assert_same(&same, &base_members, &format!("{flags:?} unchanged"));
    let reuse = &report["reuse"];
    assert_eq!(reuse["rebuiltContents"], 0, "{flags:?}");
    assert_eq!(
        reuse["reusedContents"], reuse["publishedContents"],
        "{flags:?}"
    );
    let (_, reused, report) = case.convert("edit", "reused.3tz", Some(&base));
    let (_, fresh, fresh_report) = case.convert("edit", "fresh.3tz", None);
    assert_forest_layout(&reused);
    assert_same(&reused, &fresh, &format!("{flags:?} edited"));
    let reuse = &report["reuse"];
    let rebuilt = reuse["rebuiltContents"].as_u64().unwrap();
    let reused_contents = reuse["reusedContents"].as_u64().unwrap();
    assert!(
        rebuilt > 0 && reused_contents > rebuilt,
        "{flags:?}: {reuse}"
    );
    for key in ["maximumTileBytes", "encoding", "tiles", "pointAggregation"] {
        assert_eq!(report[key], fresh_report[key], "{flags:?}: {key}");
    }
}

#[test]
fn default_forest_archive_is_a_reuse_baseline() {
    reuse_matches_fresh(&["--maxFeatures", "2", "--lodLevels", "2"], true);
}

#[test]
fn quantized_forest_archive_is_a_reuse_baseline() {
    reuse_matches_fresh(
        &["--maxFeatures", "2", "--lodLevels", "2", "--quantize"],
        true,
    );
}

#[test]
fn meshopt_forest_archive_is_a_reuse_baseline() {
    reuse_matches_fresh(
        &["--maxFeatures", "2", "--lodLevels", "2", "--meshopt"],
        true,
    );
}

#[test]
fn quantized_meshopt_forest_archive_is_a_reuse_baseline() {
    reuse_matches_fresh(
        &[
            "--maxFeatures",
            "2",
            "--lodLevels",
            "2",
            "--quantize",
            "--meshopt",
        ],
        true,
    );
}

/// The LOD-chain configuration of the design's mf2-lod3 case. Implicit roots
/// here are LOD chains without child translations, so their payloads are
/// published as byte copies (zero rebase) and recovered through the map.
#[test]
fn lod_chain_forest_archive_is_a_reuse_baseline() {
    reuse_matches_fresh(&["--maxFeatures", "2", "--lodLevels", "3"], false);
}

/// The first `.glb` payload published only under `implicit-content/` with a
/// nonzero (`rebased`) or a zero rebase.
fn mapped_glb(members: &Members, rebased: bool) -> (String, String) {
    let state: Value = serde_json::from_slice(&members["vector-build.json"]).unwrap();
    let (key, entry) = state["payloads"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(key, entry)| {
            entry["member"].as_str().unwrap().ends_with(".glb")
                && !members.contains_key(&format!("t/{key}.glb"))
                && (entry["delta"] != json!([0.0, 0.0, 0.0])) == rebased
        })
        .expect("a mapped GLB payload");
    (key.clone(), entry["member"].as_str().unwrap().to_string())
}

/// Replace a GLB's JSON chunk, keeping the BIN chunk, re-padding to 4.
fn with_glb_json(glb: &[u8], json: &[u8]) -> Vec<u8> {
    let length = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let mut text = json.to_vec();
    while !text.len().is_multiple_of(4) {
        text.push(b' ');
    }
    let rest = &glb[20 + length..];
    let mut result = glb[..4].to_vec();
    result.extend(2u32.to_le_bytes());
    result.extend(((20 + text.len() + rest.len()) as u32).to_le_bytes());
    result.extend((text.len() as u32).to_le_bytes());
    result.extend(b"JSON");
    result.extend(text);
    result.extend(rest);
    result
}

/// D-4: every altered published payload or record is a typed integrity error,
/// never a panic or a raw archive error, and never publishes.
#[test]
fn altered_published_payloads_and_records_fail_integrity() {
    let case = Case::new(&["--maxFeatures", "2", "--lodLevels", "2"]);
    let (_, base, _) = case.convert("base", "base.3tz", None);
    let (key, member) = mapped_glb(&base, true);
    let (_, copy) = mapped_glb(&base, false);
    let damaged = case.path("damaged.3tz");
    let checksum = "previous content checksum failed";
    let record = "invalid previous payload record";

    // The published member: one BIN byte (rebased and byte copy), unparsable
    // JSON, a second scene root, absent.
    for target in [&member, &copy] {
        rewrite(&base, &damaged, |name, bytes| {
            let mut bytes = bytes.to_vec();
            if name == target.as_str() {
                *bytes.last_mut().unwrap() ^= 1;
            }
            Some(bytes)
        });
        case.refuse(&damaged, checksum);
    }
    rewrite(&base, &damaged, |name, bytes| {
        let mut bytes = bytes.to_vec();
        if name == member {
            bytes[20] = b'x';
        }
        Some(bytes)
    });
    case.refuse(&damaged, checksum);
    rewrite(&base, &damaged, |name, bytes| {
        if name != member {
            return Some(bytes.to_vec());
        }
        let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let mut document: Value = serde_json::from_slice(&bytes[20..20 + length]).unwrap();
        let nodes = document["nodes"].as_array_mut().unwrap();
        nodes.push(json!({}));
        let added = nodes.len() - 1;
        document["scenes"][0]["nodes"]
            .as_array_mut()
            .unwrap()
            .push(json!(added));
        Some(with_glb_json(
            bytes,
            &serde_json::to_vec(&document).unwrap(),
        ))
    });
    case.refuse(&damaged, checksum);
    rewrite(&base, &damaged, |name, bytes| {
        (name != member).then(|| bytes.to_vec())
    });
    case.refuse(&damaged, checksum);

    // The record: a short JSON length, a wrong root node, a missing entry with no
    // raw member, a member outside implicit-content/ and a malformed entry.
    with_state(&base, &damaged, |state| {
        state["payloads"][&key]["jsonLength"] = json!(4)
    });
    case.refuse(&damaged, checksum);
    with_state(&base, &damaged, |state| {
        state["payloads"][&key]["rootNode"] = json!("{\"mesh\":0,\"translation\":[1,2,3]}")
    });
    case.refuse(&damaged, checksum);
    with_state(&base, &damaged, |state| {
        state["payloads"].as_object_mut().unwrap().remove(&key);
    });
    case.refuse(&damaged, checksum);
    with_state(&base, &damaged, |state| {
        state["payloads"][&key]["member"] = json!("tileset.json")
    });
    case.refuse(&damaged, record);
    with_state(&base, &damaged, |state| {
        state["payloads"][&key]["delta"] = json!("none")
    });
    case.refuse(&damaged, record);

    // An earlier layout identity (a pre-v3 archive) is not a baseline.
    with_state(&base, &damaged, |state| {
        state["config"]["tiling"] = json!("implicit-quadtree-v2");
        state.as_object_mut().unwrap().remove("payloads");
    });
    case.refuse(&damaged, "run a fresh conversion");
}

/// Re-specified from `native_implicit.rs`: a moved and a deleted point under
/// reuse match a fresh build, and an implicit archive is no `--explicit` cache.
#[test]
fn moved_and_deleted_features_reuse_from_a_forest_archive() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("points.geojson");
    let mut features: Vec<Value> = (0..32)
        .map(|id| {
            json!({"type":"Feature","id":id,"properties":{"name":format!("point-{id}")},
                "geometry":{"type":"Point","coordinates":[id % 8, id / 8, 0]}})
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
    let success = |result: std::process::Output| {
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        )
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
        let archive = members(path);
        assert_forest_layout(&archive);
        let manifest: Value = serde_json::from_slice(&archive["tileset.json"]).unwrap();
        let expanded = rusty_tiles::implicit::expand_tileset(&manifest, |name| {
            archive
                .get(name)
                .cloned()
                .ok_or_else(|| rusty_tiles::Error::Data(format!("absent {name}")))
        })
        .unwrap();
        fn leaves(node: &Value) -> usize {
            node["children"]
                .as_array()
                .map_or(1, |children| children.iter().map(leaves).sum())
        }
        assert_eq!(leaves(&expanded["root"]), 31);
        let report: Value = serde_json::from_slice(&archive["conversion.json"]).unwrap();
        assert_eq!(report["features"], 31);
    }
    let report: Value = serde_json::from_slice(&members(&updated)["conversion.json"]).unwrap();
    assert!(report["reuse"]["reusedContents"].as_u64().unwrap() > 0);
    assert!(report["reuse"]["rebuiltContents"].as_u64().unwrap() > 0);
    let (_, result) = run("explicit-cache.3tz", Some(&updated), true);
    assert!(!result.status.success());
}

/// VALIDATION-DEPENDENT (for R): the develop validator refuses implicit roots on
/// explicit tiles, so forest archives validate only with I1's reader. Enable on
/// the combined I1+P branch; do not weaken it to pass here.
#[test]
#[ignore = "needs I1's validator: run on the combined I1+P branch with --ignored"]
fn reused_and_fresh_forest_archives_validate() {
    let case = Case::new(&[
        "--maxFeatures",
        "2",
        "--lodLevels",
        "2",
        "--quantize",
        "--meshopt",
    ]);
    let (base, _, _) = case.convert("base", "base.3tz", None);
    let (reused, _, _) = case.convert("edit", "reused.3tz", Some(&base));
    let (fresh, _, _) = case.convert("edit", "fresh.3tz", None);
    for path in [&base, &reused, &fresh] {
        let request = rusty_tiles::validate::ValidationRequest::new(path);
        let report = rusty_tiles::validate::inspect(request)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert!(report.ok, "{}", path.display());
    }
}
