//! Legitimate producer output validates at the default `totalPayloadElements` budget with a 1.5×
//! margin (validator-limits-amendment.md, "Additive budget (2026-10-31)"). The densest class
//! measured is many primitives of isolated tiny triangles: every triangle is seven scratch rows
//! (three points, three edges, one face) in a few stored bytes. This is the PR #306 review-2
//! fixture's untextured leaf, at CI size.
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};

/// `primitives` primitives of 20 isolated triangles each, 0.07 m across, on a 0.4 m lattice.
fn tiny_triangles_glb(path: &Path, primitives: usize) {
    let mut bin = Vec::new();
    let (mut views, mut accessors, mut prims) = (Vec::new(), Vec::new(), Vec::new());
    for k in 0..primitives {
        let mut positions = Vec::new();
        for t in 0..20 {
            let x = (k % 20) as f32 * 0.4 + (t % 5) as f32 * 0.08;
            let y = (k / 20) as f32 * 0.4 + (t / 5) as f32 * 0.08;
            positions.extend([[x, y, 0.], [x + 0.07, y, 0.], [x, y + 0.07, 0.]]);
        }
        let bound = |pick: fn(f32, f32) -> f32, start: f32| {
            [0, 1, 2].map(|axis| positions.iter().map(|p| p[axis]).fold(start, pick))
        };
        let (min, max) = (
            bound(f32::min, f32::INFINITY),
            bound(f32::max, f32::NEG_INFINITY),
        );
        views.push(json!({"buffer":0,"byteOffset":bin.len(),"byteLength":positions.len() * 12}));
        bin.extend(positions.iter().flatten().flat_map(|v| v.to_le_bytes()));
        accessors.push(
            json!({"bufferView":k,"componentType":5126,"count":positions.len(),
            "type":"VEC3","min":min,"max":max}),
        );
        prims.push(json!({"attributes":{"POSITION":k}}));
    }
    let doc = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
        "nodes":[{"mesh":0}],"meshes":[{"primitives":prims}],
        "buffers":[{"byteLength":bin.len()}],"bufferViews":views,"accessors":accessors});
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

fn run(args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(args)
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn dense_tiny_triangle_output_validates_with_a_one_and_a_half_margin() {
    let work = tempfile::tempdir().unwrap();
    let (input, output) = (work.path().join("tiny.glb"), work.path().join("out.3tz"));
    tiny_triangles_glb(&input, 400);
    let (input, output) = (input.to_str().unwrap(), output.to_str().unwrap());
    let baked = run(&[
        "--json",
        "mesh",
        "-i",
        input,
        "-o",
        output,
        "--profile",
        "balanced",
        "--jobs",
        "2",
        "--memory-budget",
        "4GiB",
    ]);
    assert_eq!(baked["ok"], true, "{baked}");
    let scratch = work.path().to_str().unwrap();
    let report = run(&[
        "--json",
        "validate",
        output,
        "--expect-profile",
        "hlod",
        "--scratchDir",
        scratch,
    ]);
    assert_eq!(report["ok"], true, "{report}");
    let stored = report["storedBytes"].as_u64().unwrap();
    assert_eq!(
        report["limits"]["totalPayloadElements"],
        16_000_000 + 105 * stored
    );
    // The same validation at two thirds of the default budget still fits.
    let margin = format!(
        "totalPayloadElements={}",
        (16_000_000 + 105 * stored) * 2 / 3
    );
    let tight = run(&[
        "--json",
        "validate",
        output,
        "--expect-profile",
        "hlod",
        "--scratchDir",
        scratch,
        "--maxVolume",
        &margin,
    ]);
    assert_eq!(tight["ok"], true, "{tight}");
}
