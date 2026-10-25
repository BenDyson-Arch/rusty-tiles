//! S5: named archive checks plus a separate source-aware acceptance oracle.
use rusty_tiles::{
    package::{package, PackageRequest},
    validate::{inspect, LimitKey, ValidationRequest},
    RunControl,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, process::Command};

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
                ) {
                    vec!["--binary", env!("CARGO_BIN_EXE_rusty-tiles")]
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
    let revision_hashes: Value =
        serde_json::from_slice(&fs::read(oracle.join("revision-controls-sha256.json")).unwrap())
            .unwrap();
    for (name, expected) in revision_hashes.as_object().unwrap() {
        let bytes = fs::read(work.path().join(name)).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            expected.as_str().unwrap(),
            "{name}"
        );
    }
    let hashes: Value =
        serde_json::from_slice(&fs::read(oracle.join("controls-sha256.json")).unwrap()).unwrap();
    for (name, expected) in hashes.as_object().unwrap() {
        let bytes = fs::read(work.path().join(name)).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            expected.as_str().unwrap(),
            "{name}"
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
        let result = inspect(ValidationRequest::new(&archive));
        let cli = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
            .arg("validate")
            .arg(&archive)
            .arg("--json")
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
