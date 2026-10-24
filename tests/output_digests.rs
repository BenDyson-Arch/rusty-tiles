//! Byte-identity guard: run fixed recipes through the built binary twice.
//!
//! Every recipe must produce identical bytes on both runs. Recipes that do
//! not touch GDAL/PROJ (mesh, glb-to-3tz, createTilesetJson, convert)
//! are also compared with the per-entry sha256 digests committed in
//! `tests/fixtures/output_digests.json` on the platform recorded there. Codec
//! output (and hence archive offsets) can differ across CPU architectures.
//! Every platform still checks repeatability for every enabled recipe.
//! Geospatial recipes are not compared with committed digests: their
//! bytes legitimately change with the GDAL/PROJ release (resampling, CRS
//! transforms and the recorded `gdalVersion`), so cross-commit identity for
//! them is checked on one machine with `scripts/compare_outputs.sh`.
//!
//! Regenerate after an intended output change with
//! `UPDATE_OUTPUT_DIGESTS=1 cargo test --test output_digests`.
mod support;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io::Read, path::Path};
use support::{digests, enabled_recipes, recipes, run, write_inputs};

type Digests = BTreeMap<String, BTreeMap<String, String>>;

#[test]
fn fingerprint_normalisation_keeps_other_metadata() {
    let input = json!({
        "asset":{"extras":{"vectorBuildStateSha256":"volatile", "encoder":"user metadata"}},
        "config":{"encoder":"volatile", "signature":"setting"},
        "records":{"0":{"signature":"volatile", "survey":"volatile", "encoder":"record metadata"}},
        "properties":{"signature":"feature attribute", "vectorBuildStateSha256":"feature attribute"}
    });
    let output: Value = serde_json::from_slice(&support::normalise_vector_json(
        &serde_json::to_vec(&input).unwrap(),
    ))
    .unwrap();
    assert_eq!(output["properties"], input["properties"]);
    assert_eq!(output["asset"]["extras"]["encoder"], "user metadata");
    assert_eq!(output["config"]["signature"], "setting");
    assert_eq!(output["records"]["0"]["encoder"], "record metadata");
    assert_eq!(
        output["asset"]["extras"]["vectorBuildStateSha256"],
        "<fingerprint>"
    );
    assert_eq!(output["config"]["encoder"], "<fingerprint>");
    assert_eq!(output["records"]["0"]["signature"], "<fingerprint>");
    assert_eq!(output["records"]["0"]["survey"], "<fingerprint>");
}

fn committed_path() -> std::path::PathBuf {
    support::repo_root().join("tests/fixtures/output_digests.json")
}

fn assert_etc1s_colour(path: &Path) {
    let mut archive = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut pages = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        if !entry.name().ends_with(".glb") {
            continue;
        }
        let mut glb = Vec::new();
        entry.read_to_end(&mut glb).unwrap();
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let document: Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
        for image in document["images"].as_array().into_iter().flatten() {
            assert_eq!(image["mimeType"], "image/ktx2");
            let view = &document["bufferViews"][image["bufferView"].as_u64().unwrap() as usize];
            let start = 28 + json_len + view["byteOffset"].as_u64().unwrap_or(0) as usize;
            let ktx = &glb[start..];
            let dfd = u32::from_le_bytes(ktx[48..52].try_into().unwrap()) as usize;
            assert_eq!(ktx[dfd + 12], 163, "balanced golden must exercise ETC1S");
            pages += 1;
        }
    }
    assert!(pages > 0, "balanced golden must contain lossy colour pages");
}

fn produce(inputs: &Path, outputs: &Path) -> Digests {
    fs::create_dir_all(outputs).unwrap();
    let mut all = Digests::new();
    for recipe in enabled_recipes() {
        let result = run(&recipe, inputs, outputs, &[]);
        assert!(
            result.status.success(),
            "recipe {} failed: {}",
            recipe.name,
            String::from_utf8_lossy(&result.stderr)
        );
        let path = outputs.join(recipe.output);
        let mut entries = digests(&path, recipe.vector);
        if recipe.name == "mesh-balanced" {
            assert_etc1s_colour(&path);
            // One complete default-lossy archive is pinned in addition to its
            // entry digests/order. The recipe deliberately omits --profile.
            entries.insert(
                "#archive".into(),
                format!("{:x}", Sha256::digest(fs::read(path).unwrap())),
            );
        }
        all.insert(recipe.name.into(), entries);
    }
    all
}

/// The first difference between two digest maps of one recipe.
fn first_difference(
    name: &str,
    want: &BTreeMap<String, String>,
    got: &BTreeMap<String, String>,
) -> Option<String> {
    for (entry, digest) in want {
        match got.get(entry) {
            None => return Some(format!("{name}: entry {entry} is no longer written")),
            Some(actual) if actual != digest => {
                return Some(format!(
                    "{name}: {entry} changed (committed {digest}, got {actual})"
                ))
            }
            _ => {}
        }
    }
    got.keys()
        .find(|entry| !want.contains_key(*entry))
        .map(|entry| format!("{name}: new entry {entry}"))
}

#[test]
fn outputs_match_committed_digests() {
    let work = tempfile::tempdir().unwrap();
    let inputs = work.path().join("inputs");
    write_inputs(&inputs);
    let produced = produce(&inputs, &work.path().join("first"));
    let again = produce(&inputs, &work.path().join("second"));
    let unstable: Vec<String> = produced
        .iter()
        .filter_map(|(name, want)| first_difference(name, want, &again[name]))
        .collect();
    assert!(
        unstable.is_empty(),
        "nondeterministic output:\n{}",
        unstable.join("\n")
    );
    let portable: Vec<&str> = recipes()
        .iter()
        .filter(|r| !r.native)
        .map(|r| r.name)
        .collect();
    let produced: Digests = produced
        .into_iter()
        .filter(|(name, _)| portable.contains(&name.as_str()))
        .collect();
    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    if std::env::var_os("UPDATE_OUTPUT_DIGESTS").is_some_and(|v| v == "1") {
        let file = json!({
            "about": "sha256 per output entry of the GDAL-independent recipes in tests/support/mod.rs ('#order' hashes entry order; '#archive' pins the complete default balanced mesh archive). Regenerate with UPDATE_OUTPUT_DIGESTS=1 cargo test --test output_digests.",
            "platform": platform,
            "recipes": produced,
        });
        let mut text = serde_json::to_string_pretty(&file).unwrap();
        text.push('\n');
        fs::write(committed_path(), text).unwrap();
        eprintln!("updated {}", committed_path().display());
        return;
    }
    let committed: Value = serde_json::from_slice(&fs::read(committed_path()).unwrap()).unwrap();
    let baseline_platform = committed["platform"]
        .as_str()
        .expect("committed output digests must identify their baseline platform");
    if baseline_platform != platform {
        eprintln!(
            "repeatability passed on {platform}; committed digests apply to {baseline_platform}"
        );
        return;
    }
    let stored: Digests = serde_json::from_value(committed["recipes"].clone()).unwrap();
    let mut failures = Vec::new();
    for (name, got) in &produced {
        match stored.get(name) {
            None => failures.push(format!(
                "{name}: no committed digests (run with UPDATE_OUTPUT_DIGESTS=1)"
            )),
            Some(want) => failures.extend(first_difference(name, want, got)),
        }
    }
    failures.extend(
        stored
            .keys()
            .filter(|name| !produced.contains_key(*name))
            .map(|name| format!("{name}: committed but no longer a recipe")),
    );
    assert!(
        failures.is_empty(),
        "output digests differ from tests/fixtures/output_digests.json; if the change is \
         intended, regenerate with UPDATE_OUTPUT_DIGESTS=1:\n{}",
        failures.join("\n")
    );
}

/// Export the generated inputs and the recipe list for
/// `scripts/compare_outputs.sh` (`RUSTY_TILES_DIGEST_EXPORT=<dir>`).
#[test]
#[ignore = "used by scripts/compare_outputs.sh"]
fn export_recipes() {
    let Some(dir) = std::env::var_os("RUSTY_TILES_DIGEST_EXPORT") else {
        panic!("set RUSTY_TILES_DIGEST_EXPORT to the export directory");
    };
    let dir = Path::new(&dir);
    write_inputs(&dir.join("inputs"));
    // name, subcommand, output, vector flag, then one argument per field.
    let lines: Vec<String> = enabled_recipes()
        .iter()
        .map(|r| {
            let mut fields = vec![
                r.name.to_string(),
                r.command.to_string(),
                r.output.to_string(),
                (r.vector as u8).to_string(),
            ];
            fields.extend(r.args.iter().map(|a| a.to_string()));
            fields.join("\t")
        })
        .collect();
    fs::write(dir.join("recipes.tsv"), lines.join("\n") + "\n").unwrap();
}
