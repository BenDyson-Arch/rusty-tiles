//! Legitimate producer output validates at the default `totalPayloadElements` budget with a 1.5×
//! margin (validator-limits-amendment.md, "Additive budget (2026-10-31)"). The densest class
//! measured is many primitives of isolated tiny triangles: every triangle is seven scratch rows
//! (three points, three edges, one face) in a few stored bytes. The first case is the PR #306
//! review-2 fixture's untextured leaf, where the 16 M floor carries the budget; the second is
//! the densest output of the PR #309 review-2 sweep (`dup`: every triangle twice, 40 per
//! primitive), large enough that 130·S, not the floor, sets the budget.
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};

/// `primitives` primitives of `per` isolated triangles each, every triangle `copies` times with
/// one winding, on a 0.4 m lattice of primitives (the review's `gendense.py` layout).
fn tiny_triangles_glb(path: &Path, primitives: usize, per: usize, copies: usize) {
    let side = if per <= 20 {
        5
    } else {
        (per as f32).sqrt().ceil() as usize
    };
    let step = 0.4 / side as f32;
    let size = (step * 0.875).min(0.07);
    let mut bin = Vec::new();
    let (mut views, mut accessors, mut prims) = (Vec::new(), Vec::new(), Vec::new());
    for k in 0..primitives {
        let mut positions = Vec::new();
        for t in 0..per {
            let x = (k % 20) as f32 * 0.4 + (t % side) as f32 * step;
            let y = (k / 20) as f32 * 0.4 + (t / side) as f32 * step;
            for _ in 0..copies {
                positions.extend([[x, y, 0.], [x + size, y, 0.], [x, y + size, 0.]]);
            }
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

/// Bakes `primitives` × `per` (× `copies`) and validates the output at the default budget and at
/// two thirds of it. Returns the stored bytes.
fn bake_and_validate(primitives: usize, per: usize, copies: usize) -> u64 {
    let work = tempfile::tempdir().unwrap();
    let (input, output) = (work.path().join("tiny.glb"), work.path().join("out.3tz"));
    tiny_triangles_glb(&input, primitives, per, copies);
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
    let validate = |extra: &[&str]| {
        let mut args = vec![
            "--json",
            "validate",
            output,
            "--expect-profile",
            "hlod",
            "--scratchDir",
            scratch,
        ];
        args.extend(extra);
        run(&args)
    };
    let report = validate(&[]);
    assert_eq!(report["ok"], true, "{report}");
    let stored = report["storedBytes"].as_u64().unwrap();
    assert_eq!(
        report["limits"]["totalPayloadElements"],
        16_000_000 + 130 * stored
    );
    // The same validation at two thirds of the default budget still fits: a 1.5× margin.
    let margin = format!(
        "totalPayloadElements={}",
        (16_000_000 + 130 * stored) * 2 / 3
    );
    let tight = validate(&["--maxVolume", &margin]);
    assert_eq!(tight["ok"], true, "{tight}");
    stored
}

#[test]
fn dense_tiny_triangle_output_validates_with_a_one_and_a_half_margin() {
    bake_and_validate(400, 20, 1);
}

#[test]
fn duplicated_dense_output_validates_with_a_one_and_a_half_margin_where_r_sets_the_budget() {
    let stored = bake_and_validate(300, 40, 2);
    assert!(130 * stored > 3 * 16_000_000, "S = {stored}");
}
