//! The mesh guide, the CLI reference and the Python README promise only options and report
//! fields that exist, and name the removed H17/H27 surface only in the migration section.
use std::{collections::BTreeSet, fs, path::Path, process::Command};

fn read(relative: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)).unwrap()
}

fn help(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(args)
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap()
}

/// Every `--long-flag` token in `text`.
fn flags(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        let starts = i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'-');
        if starts && &bytes[i..i + 2] == b"--" && bytes[i + 2].is_ascii_alphabetic() {
            let end = bytes[i + 2..]
                .iter()
                .position(|b| !(b.is_ascii_alphanumeric() || *b == b'-'))
                .map_or(bytes.len(), |n| i + 2 + n);
            found.insert(text[i..end].trim_end_matches('-').to_owned());
            i = end;
        } else {
            i += 1;
        }
    }
    found
}

/// Flags a document promises: a removed option may be named only in a paragraph that says it
/// does not exist.
fn promised_flags(text: &str) -> BTreeSet<String> {
    let mut promised = BTreeSet::new();
    for paragraph in text.split("\n\n") {
        let denies = ["There is no", "no `", "removed", "gone"]
            .iter()
            .any(|word| paragraph.contains(word));
        for flag in flags(paragraph) {
            if !(denies && REMOVED.contains(&flag.as_str())) {
                promised.insert(flag);
            }
        }
    }
    promised
}

/// The text of the section whose heading line starts with `heading` (through the next heading
/// of the same or a higher level), or everything else when `invert` is set.
fn section(text: &str, heading: &str, invert: bool) -> String {
    let level = heading.bytes().take_while(|b| *b == b'#').count();
    let (mut inside, mut kept) = (false, String::new());
    for line in text.lines() {
        if line.starts_with('#') && line.trim_start_matches('#').starts_with(' ') {
            let depth = line.bytes().take_while(|b| *b == b'#').count();
            if line.starts_with(heading) {
                inside = true;
            } else if inside && depth <= level {
                inside = false;
            }
        }
        if inside != invert {
            kept.push_str(line);
            kept.push('\n');
        }
    }
    kept
}

const REMOVED: [&str; 10] = [
    "--leaf-triangles",
    "--root-proxy-triangles",
    "--max-proxy-error-metres",
    "--texture-format",
    "--explicit",
    "--node-features",
    "--no-meshopt",
    "--basisu",
    "--cartographic-position-degrees",
    "--rotation-degrees",
];
// Flags of other commands or the global options that the mesh guide may name.
const OTHER: [&str; 6] = [
    "--command",
    "--json",
    "--progress",
    "--help",
    "--version",
    "--force",
];

fn mesh_flags() -> BTreeSet<String> {
    let mut known = flags(&help(&["mesh"]));
    known.extend(OTHER.map(String::from));
    known
}

#[test]
fn help_names_the_budget_and_no_removed_option() {
    let mesh = help(&["mesh"]);
    for removed in REMOVED {
        assert!(!mesh.contains(removed), "{removed} in mesh --help");
    }
    for needed in [
        "--profile",
        "--jobs",
        "--scratchDir",
        "--memory-budget",
        "--memoryBudget",
    ] {
        assert!(mesh.contains(needed), "{needed} missing from mesh --help");
    }
    for gone in ["--lod", "H17", "h17"] {
        assert!(!mesh.contains(gone), "{gone} in mesh --help");
    }
    let validate = help(&["validate"]);
    assert!(validate.contains("--expectProfile") && validate.contains("--expect-profile"));
}

/// The guide with its migration section (which names removed things as removed) cut out.
fn current_guide(guide: &str) -> String {
    section(guide, "## Migration from older mesh converters", true)
}

#[test]
fn guides_promise_only_existing_mesh_options() {
    let known = mesh_flags();
    let cli = read("docs/CLI.md");
    let documents = [
        // The migration section may name removed options as removed.
        ("docs/MESH.md", current_guide(&read("docs/MESH.md"))),
        ("docs/CLI.md (mesh)", section(&cli, "### mesh", false)),
        (
            "bindings/python/README.md (mesh)",
            section(
                &read("bindings/python/README.md"),
                "### Streamed local mesh tiler",
                false,
            ),
        ),
    ];
    for (name, text) in documents {
        let named = promised_flags(&text);
        if !name.contains("README") {
            assert!(named.contains("--jobs"), "{name}: expected mesh options");
        }
        for flag in &named {
            // Every flag a guide prints must be a real mesh or global flag.
            assert!(
                known.contains(flag),
                "{name} promises {flag}, which `mesh --help` does not offer"
            );
        }
    }
    // The validate profile option is documented where validation is.
    let validate = section(&cli, "### validate", false);
    assert!(validate.contains("--expectProfile") && validate.contains("--expect-profile"));
}

#[test]
fn the_budget_is_documented_on_every_surface() {
    let guide = read("docs/MESH.md");
    for needed in [
        "--memory-budget BYTES",
        "memory_budget_bytes",
        "with_memory_budget",
    ] {
        assert!(guide.contains(needed), "docs/MESH.md lacks {needed}");
    }
    assert!(section(&read("docs/CLI.md"), "### mesh", false).contains("--memory-budget"));
    let python = read("bindings/python/README.md");
    assert!(python.contains("memory_budget_bytes=None"));
    assert!(python.contains("expect_profile"));
}

#[test]
fn removed_surface_is_named_only_in_the_migration_section() {
    // Other converters have their own `--lod`-style options, so match whole tokens, and read
    // only the mesh section of the CLI reference.
    let token = |text: &str, word: &str| {
        text.match_indices(word).any(|(at, _)| {
            let before = text[..at].chars().next_back();
            let after = text[at + word.len()..].chars().next();
            let word_char = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
            !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                && !after.is_some_and(word_char)
        })
    };
    for (name, text) in [
        ("docs/MESH.md", current_guide(&read("docs/MESH.md"))),
        (
            "docs/CLI.md (mesh)",
            section(&read("docs/CLI.md"), "### mesh", false),
        ),
        ("docs/CONVERTERS.md", read("docs/CONVERTERS.md")),
        (
            "bindings/python/README.md",
            read("bindings/python/README.md"),
        ),
    ] {
        for removed in [
            "--lod",
            "_lod",
            "TilerLod",
            "with_lod",
            "_memory_budget_bytes",
            "proxy_bake",
            "proxy_source",
            "lod_prepass",
            "lod_simplify",
            "seam_resolution",
            "lod_finish",
            "lod_vanished",
            "lod_filled",
        ] {
            assert!(!token(&text, removed), "{name} promises removed {removed}");
        }
    }
    assert!(!help(&[]).contains("--lod"));
}

#[test]
fn certified_proxy_and_ledger_claims_are_gone_from_the_guide() {
    let current = current_guide(&read("docs/MESH.md"))
        .replace("not certified", "")
        .replace("not-certified", "")
        .replace("not certification", "")
        // D11 certifies source-parameter border displacement, not surface error.
        .replace("displacement certified against those arcs", "")
        // A real report field whose name predates HLOD.
        .replace("unique_proxies", "")
        // D11's public omission count, not the removed proxy-bake surface.
        .replace("omitted_proxies", "");
    for forbidden in [
        "certified",
        "ledger",
        "H17",
        "H27",
        "proxy",
        "proxies",
        "Proxy",
        "legacy path",
        "Legacy H17",
    ] {
        assert!(
            !current.contains(forbidden),
            "docs/MESH.md describes `{forbidden}` outside the migration section: {}",
            current
                .lines()
                .find(|line| line.contains(forbidden))
                .unwrap_or_default()
        );
    }
}

#[test]
fn documented_phases_are_the_reported_phases() {
    // The `phase_seconds` fields of the run diagnostics, read from their definition.
    let phases = read("src/mesh_large/phases.rs");
    let start = phases.find("pub struct PhaseSeconds {").unwrap();
    let fields: BTreeSet<String> = phases[start..]
        .lines()
        .skip(1)
        .take_while(|line| !line.starts_with('}'))
        .filter_map(|line| line.trim().strip_prefix("pub ")?.split(':').next())
        .map(String::from)
        .collect();
    assert_eq!(fields.len(), 5, "{fields:?}");
    let guide = read("docs/MESH.md");
    let sentence_start = guide
        .find("`phase_seconds` holds the wall seconds of")
        .unwrap();
    let sentence = &guide[sentence_start..];
    let sentence = &sentence[..sentence.find(". ").unwrap()];
    let documented: BTreeSet<String> = sentence
        .split('`')
        .skip(3)
        .step_by(2)
        .map(String::from)
        .collect();
    assert_eq!(documented, fields);
}

#[test]
fn documented_budget_defaults_match_the_code() {
    let guide = read("docs/MESH.md");
    let budget = section(&guide, "### Memory budget", false);
    assert!(budget.contains("**4 GiB**") && budget.contains("2 GiB when headroom"));
    assert!(!budget.contains("Not a current option") && !budget.contains("hidden"));
    // 4 GiB ceiling and 2 GiB undetected default are the constants the guide quotes.
    let resources = read("src/mesh_large/resources.rs");
    assert!(resources.contains("DEFAULT_CEILING_BYTES: u64 = 4 << 30"));
    assert!(resources.contains("UNDETECTED_BYTES: u64 = 2 << 30"));
}

#[test]
fn policy_statements_match_the_implementation() {
    let guide = current_guide(&read("docs/MESH.md"));
    // A parent that cannot fit the cap is omitted, never refused (amended D1).
    let omission = section(&guide, "### Hierarchy and cracks", false);
    assert!(omission.contains("**Omission, not refusal.**"));
    assert!(omission.contains("omitted"), "omission must be described");
    for field in [
        "diagnostics.hlod",
        "omitted_proxies",
        "omission_sources",
        "omitted_by_depth",
        "borders_by_depth",
        "including zeros",
    ] {
        assert!(
            omission.contains(field),
            "missing public omission summary: {field}"
        );
    }
    assert!(!omission.contains("not in `tilerReport` or `diagnostics`"));
    assert!(
        !omission.contains("the run is `unsupported`"),
        "a cap-infeasible parent is omitted, not refused"
    );
    assert!(
        read("src/mesh_large/hlod/geometry.rs").contains("capped frontier"),
        "the guide describes omission; the geometry stage must implement it"
    );
    // Coarse pages publish their own atlas samplers and always have the PNG fallback.
    let appearance = section(&guide, "### Coarse appearance", false);
    assert!(!appearance.contains("samplers are kept"));
    assert!(appearance.contains("newly\nauthored atlas samplers"));
    assert!(appearance.contains("never fails the run"));
    assert!(read("src/mesh_large/hlod/appearance.rs")
        .contains("mipmapped: delivered.mime == \"image/ktx2\""));
    // D8: output and intermediates on memory-backed storage are charged and measured.
    let budget = section(&guide, "### Memory budget", false);
    assert!(!budget.contains("**Not charged:**"));
    assert!(budget.contains("**Memory-backed storage.**") && budget.contains("tmpfs"));
    assert!(budget.contains("`memory_backed_storage`"));
    // Containers: the fail-safe charge and its documented declaration.
    assert!(budget.contains("**Containers.**") && budget.contains("`memory_backed_assumed`"));
    assert!(budget.contains("`RUSTY_TILES_STORAGE_DISK=1`") && budget.contains("`disk_declared`"));
    assert!(read("src/spill/volume.rs").contains("\"RUSTY_TILES_STORAGE_DISK\""));
    assert!(read("docs/CLI.md").contains("`RUSTY_TILES_STORAGE_DISK=1`"));
    let pipeline = read("src/mesh_large/pipeline.rs");
    assert!(pipeline.contains("fn storage_charges") && pipeline.contains("charges.archive"));
    // The producer declares HLOD, so validation needs no flag.
    assert!(guide.contains("`asset.extras.rustyTiles.lod`"));
    assert!(read("src/mesh_large/delivery.rs").contains(".declaring_hlod()"));
}
