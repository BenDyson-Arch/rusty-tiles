//! Issue #370: producers omit `EXT_meshopt_compression`'s default `"filter": "NONE"`
//! (Blender 5.2's glTF importer refuses the explicit value) and keep `OCTAHEDRAL`.
//! `validate` must accept both spellings of NONE in the same archive. A QUATERNION
//! rewrite of the same view is the sensitive control: it must reach the payload
//! check and be refused as Unsupported.
mod support;

use rusty_tiles::{
    package::{package, PackageRequest},
    validate::{inspect, ValidationRequest},
    RunControl,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

const MESHOPT: &str = "EXT_meshopt_compression";

/// An untextured n×n quad grid with normals: views without a filter (positions,
/// indices, feature tables) and OCTAHEDRAL normals under the default profile.
fn untextured_grid(n: usize) -> Vec<u8> {
    let mut positions = Vec::new();
    for j in 0..=n {
        for i in 0..=n {
            let (u, v) = (i as f32 / n as f32, j as f32 / n as f32);
            positions.push([u * 20.0, v * 20.0, (u * 7.0).sin() * (v * 5.0).cos()]);
        }
    }
    let mut indices = Vec::new();
    for j in 0..n {
        for i in 0..n {
            let a = (j * (n + 1) + i) as u32;
            let c = a + (n + 1) as u32;
            indices.extend([a, a + 1, c, a + 1, c + 1, c]);
        }
    }
    support::write_glb(&support::Primitive {
        normals: vec![[0.0, 0.0, 1.0]; positions.len()],
        positions,
        indices,
        ..Default::default()
    })
}

/// (JSON document, BIN chunk) of a GLB.
fn split(glb: &[u8]) -> (Value, &[u8]) {
    let word = |at: usize| u32::from_le_bytes(glb[at..at + 4].try_into().unwrap()) as usize;
    assert_eq!(&glb[..4], b"glTF");
    let length = word(12);
    assert_eq!(&glb[16..20], b"JSON");
    let bin = 20 + length;
    assert_eq!(&glb[bin + 4..bin + 8], b"BIN\0");
    (
        serde_json::from_slice(&glb[20..bin]).unwrap(),
        &glb[bin + 8..bin + 8 + word(bin)],
    )
}

fn join(doc: &Value, bin: &[u8]) -> Vec<u8> {
    let mut text = serde_json::to_vec(doc).unwrap();
    text.resize(text.len().next_multiple_of(4), b' ');
    let mut out = b"glTF".to_vec();
    for word in [2, 28 + text.len() + bin.len(), text.len()] {
        out.extend((word as u32).to_le_bytes());
    }
    out.extend(b"JSON");
    out.extend(text);
    out.extend((bin.len() as u32).to_le_bytes());
    out.extend(b"BIN\0");
    out.extend(bin);
    out
}

/// The `filter` member of every compressed view, `None` when absent.
fn filters(doc: &Value) -> Vec<Option<String>> {
    doc["bufferViews"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v["extensions"].get(MESHOPT))
        .map(|e| e.get("filter").map(|f| f.as_str().unwrap().to_owned()))
        .collect()
}

fn members(archive: &Path) -> Vec<(String, Vec<u8>)> {
    let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
    (0..zip.len())
        .map(|i| {
            let mut entry = zip.by_index(i).unwrap();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            (entry.name().to_owned(), bytes)
        })
        .collect()
}

/// Repackage `archive` with `edit` applied to every unfiltered compressed view of
/// every GLB member. Content-addressed member names are renamed with their bytes.
fn rewrite(archive: &Path, out: &Path, mut edit: impl FnMut(&mut Value)) -> usize {
    let directory = out.with_extension("dir");
    let mut renames = Vec::new();
    let mut texts = Vec::new();
    let mut edited = 0;
    for (name, bytes) in members(archive) {
        if name == "@3dtilesIndex1@" {
            continue;
        }
        let mut path = directory.join(&name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bytes = if name.ends_with(".glb") {
            let (mut doc, bin) = split(&bytes);
            for view in doc["bufferViews"].as_array_mut().unwrap() {
                // `get_mut`, not `IndexMut`: indexing would add `"extensions": null`.
                let codec = view.get_mut("extensions").and_then(|x| x.get_mut(MESHOPT));
                if let Some(e) = codec {
                    if e.get("filter").is_none() {
                        edit(e);
                        edited += 1;
                    }
                }
            }
            let joined = join(&doc, bin);
            // The BIN chunk, compressed streams included, is unchanged.
            assert_eq!(split(&joined).1, bin);
            let stem = Path::new(&name).file_stem().unwrap().to_str().unwrap();
            if stem == format!("{:x}", Sha256::digest(&bytes)) {
                let renamed = format!("{:x}", Sha256::digest(&joined));
                path = path.with_file_name(format!("{renamed}.glb"));
                renames.push((stem.to_owned(), renamed));
            }
            joined
        } else {
            if name.ends_with(".json") {
                texts.push(path.clone());
            }
            bytes
        };
        fs::write(&path, bytes).unwrap();
    }
    for path in texts {
        let mut text = fs::read_to_string(&path).unwrap();
        for (old, new) in &renames {
            text = text.replace(old.as_str(), new);
        }
        fs::write(&path, text).unwrap();
    }
    package(
        PackageRequest::directory(&directory, out),
        &RunControl::default(),
    )
    .unwrap();
    edited
}

#[test]
fn default_filter_is_omitted_and_both_none_spellings_validate() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("grid.glb");
    fs::write(&input, untextured_grid(48)).unwrap();
    let archive = root.path().join("grid.3tz");
    let produced = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["mesh", "-i"])
        .arg(&input)
        .arg("-o")
        .arg(&archive)
        .args(["--anchor", "133.0", "-12.0", "10", "--jobs", "1"])
        .output()
        .unwrap();
    assert!(
        produced.status.success(),
        "{}",
        String::from_utf8_lossy(&produced.stderr)
    );

    let mut seen: Vec<Option<String>> = Vec::new();
    for (name, bytes) in members(&archive) {
        if name.ends_with(".glb") {
            seen.extend(filters(&split(&bytes).0));
        }
    }
    assert!(seen.contains(&None), "{seen:?}");
    assert!(seen.contains(&Some("OCTAHEDRAL".into())), "{seen:?}");
    assert!(
        seen.iter().all(|f| f.as_deref() != Some("NONE")),
        "{seen:?}"
    );

    let absent = inspect(ValidationRequest::new(&archive)).unwrap();
    let explicit_path: PathBuf = root.path().join("explicit.3tz");
    let edited = rewrite(&archive, &explicit_path, |e| e["filter"] = "NONE".into());
    assert_eq!(edited, seen.iter().filter(|f| f.is_none()).count());
    let explicit = inspect(ValidationRequest::new(&explicit_path)).unwrap();
    // Same checks and payload facts: the spelling is not a different stream.
    // Rows are compared as a multiset: renamed content-addressed URIs reorder them.
    let json = |report: &rusty_tiles::validate::ValidationReport| -> Value {
        let mut out = Vec::new();
        report.write_json(&mut out).unwrap();
        serde_json::from_slice(&out).unwrap()
    };
    let (absent, explicit) = (json(&absent), json(&explicit));
    let rows = |report: &Value| -> Vec<String> {
        let mut rows: Vec<String> = report["payloads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                format!(
                    "{}:{}:{}",
                    p["accessorsChecked"], p["primitivesChecked"], p["vertices"]
                )
            })
            .collect();
        rows.sort();
        rows
    };
    assert!(!rows(&absent).is_empty());
    assert_eq!(rows(&absent), rows(&explicit));
    assert_eq!(absent["checks"], explicit["checks"]);
    assert_eq!(absent["ok"], true);
    assert_eq!(explicit["ok"], true);

    let control = root.path().join("quaternion.3tz");
    rewrite(&archive, &control, |e| e["filter"] = "QUATERNION".into());
    let failure = inspect(ValidationRequest::new(&control)).unwrap_err();
    assert_eq!(failure.category().0, "unsupported", "{failure}");
    assert!(
        failure.to_string().contains("filters other than NONE"),
        "{failure}"
    );
}
