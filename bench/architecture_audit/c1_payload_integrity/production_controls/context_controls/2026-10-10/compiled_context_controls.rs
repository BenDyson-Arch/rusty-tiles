//! Independent actual-C1 selected-context/aggregate controls; root owns builds.
//! Every equality/one-less pair uses identical supplied archive bytes.
use crate::validate::{ValidationFailure, ValidationLimits, ValidationRequest};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const MANIFEST: &str = include_str!("fixtures/manifest.json");
const FIXTURE_ROOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/bench/architecture_audit/c1_payload_integrity/production_controls/context_controls/2026-10-10/"
);

fn selected_limits() -> ValidationLimits {
    // Fifteen explicit actual-C1 inputs. No default or format-owner bypass.
    ValidationLimits {
        json_bytes: 65536,
        json_depth: 32,
        member_bytes: 262144,
        archive_stored_bytes: 1048576,
        archive_entries: 256,
        accessor_elements: 4096,
        document_decoded_bytes: 262144,
        hierarchy_visits: 128,
        hierarchy_depth: 32,
        references: 1024,
        total_payload_elements: 16,
        total_bytes_read: 4194304,
        source_archive_bytes: 1048576,
        central_directory_bytes: 65536,
        document_items: 8192,
    }
}

fn declared_limits_json(limits: &ValidationLimits) -> Value {
    // Expected serialized mapping is declared from the selected request, never
    // copied from a target-produced report or inferred from default constants.
    json!({
        "jsonBytes": limits.json_bytes,
        "jsonDepth": limits.json_depth,
        "memberBytes": limits.member_bytes,
        "archiveStoredBytes": limits.archive_stored_bytes,
        "archiveEntries": limits.archive_entries,
        "accessorElements": limits.accessor_elements,
        "documentDecodedBytes": limits.document_decoded_bytes,
        "hierarchyVisits": limits.hierarchy_visits,
        "hierarchyDepth": limits.hierarchy_depth,
        "references": limits.references,
        "totalPayloadElements": limits.total_payload_elements,
        "totalBytesRead": limits.total_bytes_read,
        "sourceArchiveBytes": limits.source_archive_bytes,
        "centralDirectoryBytes": limits.central_directory_bytes,
        "documentItems": limits.document_items,
    })
}

fn fixture(name: &str) -> Value {
    let manifest: Value = serde_json::from_str(MANIFEST).unwrap();
    manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == name)
        .unwrap()
        .clone()
}

fn path_for(case: &Value) -> PathBuf {
    Path::new(FIXTURE_ROOT).join(case["path"].as_str().unwrap())
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn execution(case: &Value, fact: &str, limits: ValidationLimits, admitted: bool) {
    let path = path_for(case);
    let before = std::fs::read(&path).unwrap();
    assert_eq!(hash(&before), case["sha256"].as_str().unwrap());
    let selected = declared_limits_json(&limits);
    let result = crate::validate::inspect_selected(ValidationRequest::new(&path), limits);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "real C1 modified immutable archive"
    );
    let mut actual_report_hash = Value::Null;
    if admitted {
        let report = result.unwrap_or_else(|error| {
            panic!("{} {fact} exact equality refused: {error:?}", case["name"])
        });
        let actual = serde_json::to_value(report).unwrap();
        let mut expected = case["expectedReport"].clone();
        expected["archive"] = json!(path);
        expected["limits"] = selected.clone();
        assert_eq!(
            actual, expected,
            "{} {fact}: independently declared full report",
            case["name"]
        );
        actual_report_hash = json!(hash(&serde_json::to_vec(&actual).unwrap()));
    } else {
        let error = result.unwrap_err();
        assert!(
            matches!(error, ValidationFailure::ResourceLimit(_)),
            "{} {fact} expected typed ResourceLimit, observed {error:?}",
            case["name"]
        );
    }
    eprintln!(
        "{}",
        json!({
            "independentContextControl": case["name"], "fact": fact,
            "outcome": if admitted { "admitted" } else { "resource_limit" },
            "archiveSha256": case["sha256"], "readOnly": true,
            "selectedLimits": selected, "actualReportSha256": actual_report_hash
        })
    );
}

fn context(name: &str) {
    let case = fixture(name);
    assert_eq!(case["target"], name);
    let target = &case["metrics"][name];
    for fact in ["bytes", "depth", "nodes"] {
        let exact = target[fact].as_u64().unwrap();
        assert!(exact > 0);
        // Independent manifest establishes that another consumer cannot mask
        // this target: all other interpreted documents are strictly smaller.
        for (other, metrics) in case["metrics"].as_object().unwrap() {
            if other != name {
                assert!(
                    metrics[fact].as_u64().unwrap() < exact,
                    "target dominance lost"
                );
            }
        }
        for admitted in [true, false] {
            let ceiling = exact - u64::from(!admitted);
            let mut limits = selected_limits();
            match fact {
                "bytes" => limits.json_bytes = ceiling,
                "depth" => limits.json_depth = ceiling,
                "nodes" => limits.document_items = ceiling,
                _ => unreachable!(),
            }
            execution(&case, fact, limits, admitted);
        }
    }
}

macro_rules! context_test {
    ($function:ident, $name:literal) => {
        #[test]
        fn $function() {
            context($name);
        }
    };
}

context_test!(actual_c1_root_json_selected_limits, "root-tileset");
context_test!(actual_c1_external_json_selected_limits, "external-tileset");
context_test!(
    actual_c1_metadata_schema_root_selected_limits,
    "metadata-schema-root"
);
context_test!(
    actual_c1_metadata_schema_payload_selected_limits,
    "metadata-schema-payload"
);
context_test!(
    actual_c1_conversion_report_selected_limits,
    "conversion-report"
);
context_test!(actual_c1_local_gltf_selected_limits, "local-gltf");
context_test!(actual_c1_glb_json_selected_limits, "glb-json");
context_test!(
    actual_c1_feature_table_selected_limits,
    "b3dm-feature-table"
);
context_test!(actual_c1_batch_table_selected_limits, "b3dm-batch-table");
context_test!(actual_c1_embedded_glb_selected_limits, "b3dm-embedded-glb");
context_test!(actual_c1_subtree_json_selected_limits, "subtree-json");
context_test!(
    actual_c1_nested_serialized_extras_selected_limits,
    "nested-metadata-extras"
);

#[test]
fn actual_c1_aggregate_cache_and_shared_accessor_components() {
    for name in [
        "same-uri-cache",
        "resolved-alias-cache",
        "distinct-byte-identical-members",
        "shared-position-primitives",
        "unused-viewless-accessor",
    ] {
        let case = fixture(name);
        let equality = case["aggregateEquality"].as_u64().unwrap();
        assert_eq!(
            equality,
            case["successfulUniqueComponents"].as_u64().unwrap()
        );
        for admitted in [true, false] {
            let mut limits = selected_limits();
            limits.total_payload_elements = equality - u64::from(!admitted);
            execution(&case, "totalPayloadElements", limits, admitted);
        }
    }
}
