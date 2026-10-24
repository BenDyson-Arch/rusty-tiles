//! The mesh guide, the CLI reference and the Python README promise only options that exist,
//! never advertise the hidden coarse-tile selector, and keep certified/proxy/ledger claims inside
//! the sections that describe the legacy path.
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
fn help_names_no_removed_option_and_no_hidden_one() {
    let mesh = help(&["mesh"]);
    for removed in REMOVED {
        assert!(!mesh.contains(removed), "{removed} in mesh --help");
    }
    for needed in ["--profile", "--jobs", "--scratchDir"] {
        assert!(mesh.contains(needed), "{needed} missing from mesh --help");
    }
    for hidden in ["--memory-budget", "--memoryBudget", "--lod"] {
        assert!(!mesh.contains(hidden), "hidden {hidden} in mesh --help");
    }
}

#[test]
fn guides_promise_only_existing_mesh_options() {
    let known = mesh_flags();
    let mesh_guide = read("docs/MESH.md");
    let cli = read("docs/CLI.md");
    let documents = [
        // The migration section and the legacy section may name removed options as removed.
        (
            "docs/MESH.md",
            section(
                &section(&mesh_guide, "## Migration from older mesh converters", true),
                "## Legacy H17 coarse tiles",
                true,
            ),
        ),
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
            // The Python README also names other commands' options only through their own
            // guides; every flag it prints must still be a real mesh or global flag.
            assert!(
                known.contains(flag),
                "{name} promises {flag}, which `mesh --help` does not offer"
            );
        }
    }
}

#[test]
fn the_hlod_selector_is_never_advertised() {
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
        ("docs/MESH.md", read("docs/MESH.md")),
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
        for hidden in [
            "--lod",
            "_lod",
            "TilerLod",
            "with_lod",
            "--memory-budget",
            "--memoryBudget",
            "memory_budget_bytes",
            "with_memory_budget",
        ] {
            assert!(!token(&text, hidden), "{name} advertises {hidden}");
        }
    }
    assert!(!help(&[]).contains("--lod"));
}

#[test]
fn certified_proxy_and_ledger_claims_stay_in_the_legacy_sections() {
    let guide = read("docs/MESH.md");
    let current = section(
        &section(
            &section(&guide, "## Legacy H17 coarse tiles", true),
            "## Migration from older mesh converters",
            true,
        ),
        "### Migrating from the legacy coarse tiles to HLOD",
        true,
    );
    // The status block quotes the legacy path by name; it is the one place outside them.
    let current: String = current
        .lines()
        .filter(|line| !line.starts_with('>'))
        .map(|line| format!("{line}\n"))
        .collect();
    let current = current
        .replace("not certified", "")
        .replace("not-certified", "")
        // A real report field whose name predates HLOD.
        .replace("unique_proxies", "")
        .replace("not certification", "");
    for forbidden in [
        "certified",
        "proxy_source",
        "ledger",
        "H27",
        "proxy",
        "proxies",
        "Proxy",
    ] {
        assert!(
            !current.contains(forbidden),
            "docs/MESH.md describes `{forbidden}` outside the legacy sections: {}",
            current
                .lines()
                .find(|line| line.contains(forbidden))
                .unwrap_or_default()
        );
    }
    // The legacy fence exists and says so.
    let legacy = section(&guide, "## Legacy H17 coarse tiles", false);
    assert!(legacy.contains("proxy_source") && legacy.contains("certified"));
    let status = guide
        .lines()
        .filter(|l| l.starts_with('>'))
        .collect::<String>();
    assert!(
        status.contains("legacy"),
        "status note must name the legacy path"
    );
}

#[test]
fn documented_budget_defaults_match_the_code() {
    let guide = read("docs/MESH.md");
    let budget = section(&guide, "### Memory budget", false);
    assert!(budget.contains("**4 GiB**") && budget.contains("2 GiB when headroom"));
    // The budget is documented only as planned, never as a current option.
    assert!(budget.contains("**Not a current option.**"), "{budget}");
    assert!(guide.contains("### Memory budget (coming with the HLOD default)"));
    // 4 GiB ceiling and 2 GiB undetected default are the constants the guide quotes.
    let resources = read("src/mesh_large/resources.rs");
    assert!(resources.contains("DEFAULT_CEILING_BYTES: u64 = 4 << 30"));
    assert!(resources.contains("UNDETECTED_BYTES: u64 = 2 << 30"));
}
