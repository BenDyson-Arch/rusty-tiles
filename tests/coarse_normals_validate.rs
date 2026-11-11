//! `validate` reports coarse corner normals more than 90° from their face as the
//! `coarseNormalInversion` warning, never a failure (#336). The pre-fix archive
//! (`fixtures/coarse_normals/prefix.3tz`, see its README) was baked from
//! [`rough_glb`] by the producer before #336, which averaged every face at a vertex;
//! it must still validate `ok: true`, with the warning and its per-depth counts. The
//! same input baked by this build carries no warning.
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};

/// A 66×66 grid (8,450 triangles: two capped leaves and a coarse root) with
/// deterministic whole-metre relief of ±1 (integer heights compress; the leaves are
/// most of the archive) and area-weighted vertex normals.
pub fn rough_glb(path: &Path) {
    let n = 66u32;
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
    let (input, output) = (work.path().join("rough.glb"), work.path().join("out.3tz"));
    rough_glb(&input);
    let (input, output) = (input.to_str().unwrap(), output.to_str().unwrap());
    let (baked, _, _) = run(&["--json", "mesh", "-i", input, "-o", output, "--jobs", "2"]);
    assert_eq!(baked["ok"], true, "{baked}");
    let report = validate(output, work.path().to_str().unwrap());
    let census = &report["interfaceCensus"]["coarseNormals"];
    assert!(census["tiles"].as_u64().unwrap() > 0, "{census}");
    assert!(census["faces"].as_u64().unwrap() > 0, "{census}");
    assert_eq!(census["invertedFaces"], 0, "{census}");
    assert_eq!(census["invertedTilesByDepth"], json!([]), "{census}");
    assert!(report.get("warnings").is_none(), "{report}");
}

/// Writes [`rough_glb`] to `$RT_COARSE_NORMALS_INPUT`, the input of the pre-fix fixture.
#[test]
#[ignore = "fixture regeneration only"]
fn write_rough_input() {
    rough_glb(Path::new(
        &std::env::var("RT_COARSE_NORMALS_INPUT").unwrap(),
    ));
}
