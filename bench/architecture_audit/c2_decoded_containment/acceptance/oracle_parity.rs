//! C2 acceptance: production `inspect_selected` against every oracle witness and
//! control fixture (coordinator-owned). Expectations come only from the oracle's
//! committed reports; nothing here derives a verdict from production code.
use crate::validate::{inspect_selected, ValidationFailure, ValidationLimits, ValidationRequest};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path};

const ROOT: &str = "bench/architecture_audit/c2_decoded_containment";
const OUTSIDE: &str = "decoded content outside bounding volume: ";

fn limits(row: &Value) -> ValidationLimits {
    let mut l = ValidationLimits::default();
    for (key, value) in row["limits"].as_object().into_iter().flatten() {
        let v = value.as_u64().expect("oracle limit is u64");
        match key.as_str() {
            "containmentSpanBits" => l.containment_span_bits = v,
            "containmentWorkUnits" => l.containment_work_units = v,
            "containmentBytesRead" => l.containment_bytes_read = v,
            "containmentDecodedBytes" => l.containment_decoded_bytes = v,
            "containmentNodeDepth" => l.containment_node_depth = v,
            other => panic!("unknown oracle limit {other}"),
        }
    }
    l
}

/// Production result in the oracle's vocabulary.
fn production(file: &Path, limits: ValidationLimits) -> Value {
    match inspect_selected(ValidationRequest::new(file), limits) {
        Ok(report)
            if report
                .checks
                .iter()
                .any(|c| c == "decodedContentContainment") =>
        {
            json!({"outcome":"Contained"})
        }
        Ok(report) => {
            let mut reasons: Vec<&str> = report
                .not_inspected
                .iter()
                .filter_map(|n| n.strip_prefix("decodedContentBounds."))
                .collect();
            reasons.sort_unstable();
            reasons.dedup();
            json!({"outcome":"NotInspected","reasons":reasons})
        }
        Err(ValidationFailure::InvalidInput(m)) if m.starts_with(OUTSIDE) => {
            let fields: BTreeMap<&str, &str> = m[OUTSIDE.len()..]
                .split(' ')
                .filter_map(|f| f.split_once('='))
                .collect();
            let (role, level) = match fields["volume"].split_once(':') {
                Some((role, k)) => (role, k.parse::<u64>().unwrap()),
                None => (fields["volume"], 0),
            };
            json!({"outcome":"Outside","facts":{
                "depth": fields["depth"].parse::<u64>().unwrap(),
                "level": level, "role": role,
                "node": fields["node"].parse::<u64>().unwrap()}})
        }
        Err(ValidationFailure::InvalidInput(m))
            if m.starts_with("child bounds escape parent") || m == "content bounds escape tile" =>
        {
            json!({"outcome":"C1Hierarchy","message":m})
        }
        Err(e) => json!({"outcome": e.category().0, "message": e.to_string()}),
    }
}

fn agrees(expected: &Value, got: &Value) -> bool {
    let outcome = expected["outcome"].as_str().unwrap();
    match outcome {
        "Contained" => got["outcome"] == "Contained",
        "NotInspected" => {
            let mut want: Vec<&str> = expected["reasons"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r.as_str().unwrap())
                .collect();
            want.sort_unstable();
            want.dedup();
            got["outcome"] == "NotInspected" && got["reasons"] == json!(want)
        }
        "Outside" => {
            got["outcome"] == "Outside"
                && ["depth", "level", "role", "node"].iter().all(|k| {
                    expected["facts"]
                        .get(*k)
                        .is_none_or(|v| got["facts"][*k] == *v)
                })
        }
        // The oracle's InvalidNode is production's C1/C2 node admission InvalidInput.
        "InvalidInput" | "InvalidNode" => got["outcome"] == "invalid_input",
        // C1 refuses before C2 runs (C2 requires C1 success); only the kind is compared.
        "C1Refused" => match expected["kind"].as_str().unwrap() {
            "invalid_input" => got["outcome"] == "invalid_input" || got["outcome"] == "C1Hierarchy",
            kind => got["outcome"] == kind,
        },
        other => panic!("unknown oracle outcome {other}"),
    }
}

#[test]
fn production_matches_every_oracle_witness_and_control() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let read = |p: &str| -> Value {
        serde_json::from_slice(&std::fs::read(root.join(ROOT).join(p)).unwrap()).unwrap()
    };
    let mut rows: BTreeMap<String, Value> = BTreeMap::new();
    for row in read("witnesses/report.json")["rows"].as_array().unwrap() {
        rows.insert(row["id"].as_str().unwrap().into(), row.clone());
    }
    for pair in read("controls/manifest.json")["pairs"].as_array().unwrap() {
        for side in ["base", "mutated"] {
            let row = &pair[side];
            rows.entry(row["id"].as_str().unwrap().into())
                .or_insert_with(|| row.clone());
        }
    }
    let (mut agree, mut c1_allowed, mut skipped) = (0, Vec::new(), Vec::new());
    let mut disagree = Vec::new();
    for (id, row) in &rows {
        let file = root.join(ROOT).join(row["file"].as_str().unwrap());
        if !file.exists() {
            skipped.push(id.clone());
            continue;
        }
        let got = production(&file, limits(row));
        if agrees(&row["expected"], &got) {
            agree += 1;
        } else if got["outcome"] == "C1Hierarchy"
            && row["c1Precondition"]["judgement"]
                .as_str()
                .is_some_and(|j| j.starts_with("likely") || j.starts_with("possible"))
        {
            c1_allowed.push(id.clone());
        } else {
            disagree.push(json!({"id":id,"expected":row["expected"],"production":got}));
        }
    }
    let summary = json!({"cases":rows.len(),"agree":agree,"c1PreconditionRefused":c1_allowed,
        "skipped":skipped,"disagree":disagree});
    if let Some(out) = std::env::var_os("C2_PARITY_OUT") {
        std::fs::write(out, serde_json::to_vec_pretty(&summary).unwrap()).unwrap();
    }
    assert!(
        summary["disagree"].as_array().unwrap().is_empty(),
        "{}",
        serde_json::to_string_pretty(&summary).unwrap()
    );
    // Only the uncommitted `--large` repeated-index pair may be absent.
    assert!(
        skipped
            .iter()
            .all(|id| id.starts_with("repeated-index-three-payloads")),
        "unexpected missing fixtures {skipped:?}"
    );
}
