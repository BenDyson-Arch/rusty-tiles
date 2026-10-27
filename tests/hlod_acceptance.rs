//! S5: named archive checks plus a separate source-aware acceptance oracle.
use rusty_tiles::{
    package::{package, PackageRequest},
    validate::{inspect, ExpectedProfile, LimitKey, ValidationRequest},
    RunControl,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Work-root directories outside the `base` family: the other digested families
/// and run outputs (winding probes, omission producer-report inputs, D11 border
/// controls, whose producer-baked sheet is not digested).
const OTHER: [&str; 8] = [
    "lattice",
    "native",
    "nonmanifold",
    "exact",
    "omission",
    "winding",
    "omission-reports",
    "border",
];

/// Sorted `relative-path\tsha256\n` lines of a family's generated members.
fn family_lines(work: &Path, family: &str) -> Vec<String> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.push(path);
            }
        }
    }
    let root = if family == "base" {
        work.to_path_buf()
    } else {
        work.join(family)
    };
    let mut files = Vec::new();
    walk(&root, &mut files);
    let mut lines: Vec<String> = files
        .into_iter()
        .filter_map(|path| {
            let relative = path
                .strip_prefix(work)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            let first = relative.split('/').next().unwrap();
            let member = matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("json" | "glb" | "bin" | "subtree")
            );
            (member && (family != "base" || !OTHER.contains(&first))).then(|| {
                format!(
                    "{relative}\t{:x}\n",
                    Sha256::digest(fs::read(&path).unwrap())
                )
            })
        })
        .collect();
    lines.sort();
    lines
}

#[test]
fn sampled_hlod_archives_and_named_mutants() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let oracle = repo.join("bench/architecture_audit/mesh_hlod/s5");
    let work = tempfile::tempdir().unwrap();
    let generated = Command::new("python3")
        .arg(oracle.join("controls.py"))
        .arg(work.path())
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    for name in [
        "lattice_controls.py",
        "winding_controls.py",
        "native_controls.py",
        "nonmanifold_controls.py",
        "exact_controls.py",
        "omission_controls.py",
        "seam_diagnostic_controls.py",
        "border_controls.py",
    ] {
        let probes = work.path().join(name.trim_end_matches("_controls.py"));
        let result = Command::new("python3")
            .arg(oracle.join(name))
            .args(if name == "seam_diagnostic_controls.py" {
                vec![]
            } else {
                vec![probes.as_os_str()]
            })
            .args(
                if matches!(
                    name,
                    "native_controls.py"
                        | "nonmanifold_controls.py"
                        | "exact_controls.py"
                        | "omission_controls.py"
                        | "border_controls.py"
                ) {
                    let mut args = vec!["--binary", env!("CARGO_BIN_EXE_rusty-tiles")];
                    // The producer sheet's exact audit dominates this test: 80 is the
                    // smallest sheet with contracted borders (182 arcs); evidence runs use 120.
                    if name == "border_controls.py" {
                        args.extend(["--sheet", "80"]);
                    }
                    args
                } else {
                    vec![]
                },
            )
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    // One digest per generated family over its sorted `path\tsha256` lines;
    // per-file manifests live in the evidence cache (bench/MOVED_EVIDENCE.md).
    let digests: Value =
        serde_json::from_slice(&fs::read(oracle.join("controls-digest.json")).unwrap()).unwrap();
    for (family, expected) in digests.as_object().unwrap() {
        let lines = family_lines(work.path(), family);
        assert_eq!(
            lines.len() as u64,
            expected["files"].as_u64().unwrap(),
            "{family}"
        );
        let joined: String = lines.concat();
        assert_eq!(
            format!("{:x}", Sha256::digest(joined.as_bytes())),
            expected["sha256"].as_str().unwrap(),
            "{family} members:\n{joined}"
        );
    }
    let cases: Value =
        serde_json::from_slice(&fs::read(work.path().join("cases.json")).unwrap()).unwrap();
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let archive = work.path().join(format!("{name}.3tz"));
        package(
            PackageRequest::directory(work.path().join(name), &archive),
            &RunControl::default(),
        )
        .unwrap();
        // HLOD acceptance always states its expectation (review r12).
        let result =
            inspect(ValidationRequest::new(&archive).expect_profile(ExpectedProfile::Hlod));
        let cli = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
            .arg("validate")
            .arg(&archive)
            .args(["--json", "--expect-profile", "hlod"])
            .output()
            .unwrap();
        let cli_report: Value = serde_json::from_slice(&cli.stdout).unwrap();
        if let Some(check) = case["archiveCheck"].as_str() {
            let error = result.expect_err(name).to_string();
            assert!(error.contains(check), "{name} must fail {check}: {error}");
            assert!(!cli.status.success(), "{name}: {cli_report}");
            assert!(
                cli_report["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains(check),
                "{name}: {cli_report}"
            );
        } else if case["oracleCheck"].is_null() && case["archiveCheck"].is_null() {
            if name == "valid" {
                let error = inspect(
                    ValidationRequest::new(&archive)
                        .expect_profile(ExpectedProfile::Hlod)
                        .max_volume(LimitKey::TotalPayloadElements, 200),
                )
                .unwrap_err();
                assert!(error.to_string().contains("hlodFeatureIds"), "{error}");
                let refusal = error.exceeded().unwrap();
                assert_eq!(refusal.limit, LimitKey::TotalPayloadElements);
                assert_eq!(refusal.value, 200);
                assert!(refusal.observed > 200);
            }
            let report = result.unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(cli.status.success(), "{name}: {cli_report}");
            let census = report
                .interface_census
                .as_ref()
                .expect("identity-bearing positive must run the decoded incidence census");
            assert!(census.max_payload_triangles <= 8192, "{name}: hard cap");
            assert!(
                census.frontier_tiles > 0 && census.frontier_leaves > 0,
                "{name}: the first renderable cover must be derived"
            );
            assert_eq!(
                census.omitted_by_depth.iter().sum::<u64>(),
                census.omitted_cells,
                "{name}: omission by depth"
            );
            assert!(census.omission_sources <= census.omitted_cells, "{name}");
            assert_eq!(census.basis, "decoded leaf canonical f32 keys", "{name}");
            assert!(
                census.source_open_edges_with_shared_endpoints <= census.source_open_edges,
                "{name}: open-edge subset must be counted consistently"
            );
            assert!(
                report.checks.iter().any(|s| s == "hlodFeatureIds"),
                "{name}: identity must run"
            );
            assert!(
                report
                    .checks
                    .iter()
                    .any(|s| s == "replaceContentContinuity"),
                "{name}: continuity must run"
            );
            assert!(
                !report
                    .not_inspected
                    .iter()
                    .any(|s| s == "EXT_mesh_features" || s == "metadataSemantics"),
                "{name}: checked identity must not be uninspected"
            );
            assert!(report
                .not_inspected
                .iter()
                .any(|s| s == "geometricErrorAccuracy"));
            assert_eq!(serde_json::to_value(report).unwrap(), cli_report);
        }
    }
    // Review r12: every identity label stripped, through the public API. The expectation
    // comes from the caller or the archive's declaration, never from the labels.
    for (name, declared) in [
        ("stripped-undeclared", false),
        ("stripped-declared-asset", true),
        ("stripped-declared-report", true),
    ] {
        let archive = work.path().join(format!("omission/{name}-api.3tz"));
        package(
            PackageRequest::directory(work.path().join("omission").join(name), &archive),
            &RunControl::default(),
        )
        .unwrap();
        let flagged =
            inspect(ValidationRequest::new(&archive).expect_profile(ExpectedProfile::Hlod))
                .unwrap_err()
                .to_string();
        assert!(flagged.contains("hlodFeatureIds"), "{name}: {flagged}");
        let plain = inspect(ValidationRequest::new(&archive));
        if declared {
            let error = plain.unwrap_err().to_string();
            assert!(error.contains("hlodFeatureIds"), "{name}: {error}");
        } else {
            let report = plain.unwrap_or_else(|e| panic!("{name}: generic tileset: {e}"));
            assert!(report.interface_census.is_none(), "{name}");
            assert!(
                !report.checks.iter().any(|c| c == "hlodFeatureIds"),
                "{name}"
            );
        }
    }
    // I1's independently authored A-1: co-adjusted missing tile/content bit,
    // metadata row and payload, rather than an obviously dangling URI.
    let incomplete = repo.join("bench/architecture_audit/implicit_read/controls/fixtures/A-1.3tz");
    let error = inspect(ValidationRequest::new(&incomplete))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("unavailable parent"),
        "implicitAddressingAndAvailability: {error}"
    );
}
