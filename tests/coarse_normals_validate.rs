//! `validate` reports coarse corner normals more than 90° from their face as the
//! `coarseNormalInversion` warning, never a failure (#336). The pre-fix archive
//! (`fixtures/coarse_normals/prefix.3tz`, see its README) was baked from
//! [`rough_glb`] by the producer before #336, which averaged every face at a vertex;
//! it must still validate `ok: true`, with the warning and its per-depth counts. The
//! same input baked by this build carries no warning.
use rusty_tiles::{
    package::{package, PackageRequest},
    RunControl,
};
use serde_json::{json, Value};
use std::{fs, io::Read, path::Path, process::Command};

/// An `n`×`n` grid with deterministic whole-metre relief of ±1 (integer heights compress;
/// the leaves are most of an archive) and area-weighted vertex normals. `n = 66` (8,450
/// triangles: two capped leaves and a coarse root) is the fixture's input.
pub fn rough_glb(path: &Path, n: u32) {
    let mut seed = 0x9e37_79b9_u32;
    let mut noise = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (f64::from(seed >> 16) / 65535. * 3. - 1.5).round() as f32
    };
    let positions: Vec<[f32; 3]> = (0..n * n)
        .map(|i| [(i % n) as f32, noise(), (i / n) as f32])
        .collect();
    let mut faces = Vec::new();
    for y in 0..n - 1 {
        for x in 0..n - 1 {
            let v = y * n + x;
            faces.push([v, v + n, v + 1]);
            faces.push([v + 1, v + n, v + n + 1]);
        }
    }
    let mut sum = vec![[0f64; 3]; positions.len()];
    for f in &faces {
        let p = f.map(|v| positions[v as usize].map(f64::from));
        let (a, b): ([f64; 3], [f64; 3]) = (
            std::array::from_fn(|i| p[1][i] - p[0][i]),
            std::array::from_fn(|i| p[2][i] - p[0][i]),
        );
        let c = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        for v in f {
            for i in 0..3 {
                sum[*v as usize][i] += c[i];
            }
        }
    }
    let normals: Vec<[f32; 3]> = sum
        .iter()
        .map(|s| {
            let l = (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt();
            s.map(|v| (v / l) as f32)
        })
        .collect();
    let mut bin = Vec::new();
    bin.extend(positions.iter().flatten().flat_map(|v| v.to_le_bytes()));
    bin.extend(normals.iter().flatten().flat_map(|v| v.to_le_bytes()));
    bin.extend(faces.iter().flatten().flat_map(|v| v.to_le_bytes()));
    let (p, q) = (positions.len() * 12, positions.len() * 24);
    let bound = |pick: fn(f32, f32) -> f32, start: f32| {
        [0, 1, 2].map(|axis| positions.iter().map(|p| p[axis]).fold(start, pick))
    };
    let doc = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
        "nodes":[{"mesh":0}],"materials":[{"doubleSided":true}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1},"indices":2,"material":0}]}],
        "buffers":[{"byteLength":bin.len()}],
        "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":p},
            {"buffer":0,"byteOffset":p,"byteLength":p},
            {"buffer":0,"byteOffset":q,"byteLength":bin.len() - q}],
        "accessors":[
            {"bufferView":0,"componentType":5126,"count":positions.len(),"type":"VEC3",
             "min":bound(f32::min, f32::INFINITY),"max":bound(f32::max, f32::NEG_INFINITY)},
            {"bufferView":1,"componentType":5126,"count":positions.len(),"type":"VEC3"},
            {"bufferView":2,"componentType":5125,"count":faces.len() * 3,"type":"SCALAR"}]});
    let mut text = serde_json::to_vec(&doc).unwrap();
    text.resize(text.len().next_multiple_of(4), b' ');
    let mut bytes = b"glTF".to_vec();
    for word in [
        2u32,
        (28 + text.len() + bin.len()) as u32,
        text.len() as u32,
    ] {
        bytes.extend(word.to_le_bytes());
    }
    bytes.extend(b"JSON");
    bytes.extend(text);
    bytes.extend((bin.len() as u32).to_le_bytes());
    bytes.extend(b"BIN\0");
    bytes.extend(bin);
    fs::write(path, bytes).unwrap();
}

fn run(args: &[&str]) -> (Value, String, bool) {
    let output = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(args)
        .output()
        .unwrap();
    let value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    (
        value,
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

fn validate(archive: &str, scratch: &str) -> Value {
    let (report, _, success) = run(&[
        "--json",
        "validate",
        archive,
        "--expect-profile",
        "hlod",
        "--scratchDir",
        scratch,
    ]);
    assert!(success, "{report}");
    assert_eq!(report["ok"], true, "{report}");
    report
}

const PREFIX: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/coarse_normals/prefix.3tz"
);

#[test]
fn pre_fix_coarse_normals_validate_with_a_warning() {
    let work = tempfile::tempdir().unwrap();
    let scratch = work.path().to_str().unwrap();
    let report = validate(PREFIX, scratch);
    let census = &report["interfaceCensus"]["coarseNormals"];
    assert_eq!(census["boundDegrees"], 90, "{census}");
    let tiles = census["tiles"].as_u64().unwrap();
    let inverted = census["invertedFaces"].as_u64().unwrap();
    assert!(tiles > 0 && inverted > 0, "{census}");
    let by_depth = |key: &str| -> u64 {
        census[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap())
            .sum()
    };
    assert_eq!(by_depth("invertedFacesByDepth"), inverted);
    assert_eq!(
        by_depth("invertedTilesByDepth"),
        census["invertedTiles"].as_u64().unwrap()
    );
    let warnings = report["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1, "{report}");
    assert_eq!(warnings[0]["code"], "coarseNormalInversion");
    assert!(warnings[0]["message"]
        .as_str()
        .unwrap()
        .contains(&format!("({inverted} of ")));
    // The warning is not a check: `checks` lists what ran and passed.
    assert!(!report["checks"].to_string().contains("coarseNormal"));
    // Text mode: exit 0, the warning on stderr.
    let (_, stderr, success) = run(&["validate", PREFIX, "--scratchDir", scratch]);
    assert!(success);
    assert!(
        stderr.contains("warning: coarseNormalInversion: "),
        "{stderr}"
    );
}

#[test]
fn this_build_bakes_coarse_normals_without_the_warning() {
    let work = tempfile::tempdir().unwrap();
    let output = bake(work.path(), 66, "2");
    let report = validate(&output, work.path().to_str().unwrap());
    let census = &report["interfaceCensus"]["coarseNormals"];
    assert!(census["tiles"].as_u64().unwrap() > 0, "{census}");
    assert!(census["faces"].as_u64().unwrap() > 0, "{census}");
    assert_eq!(census["invertedFaces"], 0, "{census}");
    assert_eq!(census["invertedTilesByDepth"], json!([]), "{census}");
    assert!(report.get("warnings").is_none(), "{report}");
}

/// Bakes [`rough_glb`] of side `n` with this build at `--jobs jobs`; returns the archive.
fn bake(dir: &Path, n: u32, jobs: &str) -> String {
    let input = dir.join(format!("rough{n}.glb"));
    if !input.exists() {
        rough_glb(&input, n);
    }
    let output = dir.join(format!("rough{n}-jobs-{jobs}.3tz"));
    let (input, output) = (input.to_str().unwrap(), output.to_str().unwrap());
    let (baked, stderr, success) =
        run(&["--json", "mesh", "-i", input, "-o", output, "--jobs", jobs]);
    assert!(success, "{baked} {stderr}");
    assert_eq!(baked["ok"], true, "{baked}");
    output.to_string()
}

/// Coarse appearance (and the rest of the archive) does not depend on the worker count:
/// five coarse tiles are built by parallel parent jobs or by one worker, byte for byte.
#[test]
fn coarse_output_is_the_same_for_one_worker_and_auto() {
    let work = tempfile::tempdir().unwrap();
    let one = bake(work.path(), 130, "1");
    let auto = bake(work.path(), 130, "auto");
    assert!(fs::read(&one).unwrap() == fs::read(&auto).unwrap());
    let report = validate(&one, work.path().to_str().unwrap());
    let census = &report["interfaceCensus"]["coarseNormals"];
    assert!(census["tiles"].as_u64().unwrap() >= 2, "{census}");
    assert_eq!(census["invertedFaces"], 0, "{census}");
}

/// The archive with its root coarse payload's NORMAL accessor changed by `mutate`.
fn mutant(work: &Path, archive: &str, name: &str, mutate: fn(&mut Value)) -> String {
    let dir = work.join(name);
    let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
    let mut members = Vec::new();
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        members.push((file.name().to_string(), bytes));
    }
    let tileset: Value =
        serde_json::from_slice(&members.iter().find(|m| m.0 == "tileset.json").unwrap().1).unwrap();
    let root = tileset["root"]["content"]["uri"]
        .as_str()
        .unwrap()
        .to_string();
    for (member, bytes) in members {
        if member == "@3dtilesIndex1@" {
            continue;
        }
        let bytes = if member == root {
            let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
            let mut doc: Value = serde_json::from_slice(&bytes[20..20 + length]).unwrap();
            for mesh in doc["meshes"].clone().as_array().unwrap() {
                for p in mesh["primitives"].as_array().unwrap() {
                    let a = p["attributes"]["NORMAL"].as_u64().unwrap() as usize;
                    mutate(&mut doc["accessors"][a]);
                }
            }
            let mut text = serde_json::to_vec(&doc).unwrap();
            text.resize(text.len().next_multiple_of(4), b' ');
            let rest = &bytes[20 + length..];
            let mut out = b"glTF".to_vec();
            for word in [
                2u32,
                (20 + text.len() + rest.len()) as u32,
                text.len() as u32,
            ] {
                out.extend(word.to_le_bytes());
            }
            out.extend(b"JSON");
            out.extend(text);
            out.extend(rest);
            out
        } else {
            bytes
        };
        let path = dir.join(&member);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    let output = work.join(format!("{name}.3tz"));
    package(
        PackageRequest::directory(&dir, &output),
        &RunControl::default(),
    )
    .unwrap();
    output.to_str().unwrap().to_string()
}

/// A foreign coarse NORMAL the comparison does not read keeps the verdict the validator
/// gave before this check (verified against `origin/develop` 85157619): viewless zero
/// storage still passes, now with the skip named in `notInspected` and no warning;
/// sparse and wrongly typed NORMAL still fail the earlier payload checks, not this one.
#[test]
fn foreign_coarse_normal_mutants_keep_their_verdict() {
    let work = tempfile::tempdir().unwrap();
    let scratch = work.path().to_str().unwrap();
    let archive = bake(work.path(), 66, "2");
    let viewless = mutant(work.path(), &archive, "viewless", |a| {
        a.as_object_mut().unwrap().remove("bufferView");
        a.as_object_mut().unwrap().remove("byteOffset");
    });
    let report = validate(&viewless, scratch);
    let census = &report["interfaceCensus"]["coarseNormals"];
    assert_eq!(census["skippedTiles"], 1, "{census}");
    assert_eq!(census["tiles"], 0, "{census}");
    assert!(report.get("warnings").is_none(), "{report}");
    assert!(report["notInspected"]
        .as_array()
        .unwrap()
        .contains(&json!("coarseNormalInversion.viewlessNormal")));
    let sparse: fn(&mut Value) = |a| {
        let view = a["bufferView"].clone();
        a["sparse"] = json!({"count":1,"indices":{"bufferView":view,"componentType":5125},
            "values":{"bufferView":view}});
    };
    let unsigned: fn(&mut Value) = |a| a["componentType"] = json!(5121);
    for (name, mutate, refusal) in [
        ("sparse", sparse, "sparse accessor"),
        ("unsigned", unsigned, "invalid NORMAL accessor type"),
    ] {
        let archive = mutant(work.path(), &archive, name, mutate);
        let (report, _, success) = run(&[
            "--json",
            "validate",
            &archive,
            "--expect-profile",
            "hlod",
            "--scratchDir",
            scratch,
        ]);
        assert!(!success, "{name}: {report}");
        let message = report["error"]["message"].as_str().unwrap();
        assert!(message.contains(refusal), "{name}: {message}");
    }
}

/// Writes [`rough_glb`] to `$RT_COARSE_NORMALS_INPUT`, the input of the pre-fix fixture.
#[test]
#[ignore = "fixture regeneration only"]
fn write_rough_input() {
    rough_glb(
        Path::new(&std::env::var("RT_COARSE_NORMALS_INPUT").unwrap()),
        66,
    );
}
