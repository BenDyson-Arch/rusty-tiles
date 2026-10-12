//! C2 acceptance: production `inspect_selected` against every oracle witness and
//! control fixture (coordinator-owned). Expectations come only from the oracle's
//! committed reports; nothing here derives a verdict from production code.
use crate::validate::{
    containment_work_facts, containment_work_total, inspect_selected, ValidationFailure,
    ValidationLimits, ValidationRequest,
};
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
            let n = |k: &str| fields[k].parse::<u64>().unwrap();
            json!({"outcome":"Outside","facts":{
                "depth": n("depth"), "level": level, "role": role, "node": n("node"),
                "mesh": n("mesh"), "primitive": n("primitive"), "vertex": n("vertex")}})
        }
        Err(ValidationFailure::InvalidInput(m))
            if m.starts_with("child bounds escape parent") || m == "content bounds escape tile" =>
        {
            json!({"outcome":"C1Hierarchy","message":m})
        }
        Err(e) => json!({"outcome": e.category().0, "message": e.to_string()}),
    }
}

fn agrees(expected: &Value, oracle: &Value, got: &Value) -> bool {
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
        // Expected facts pin depth/level/role/node; the oracle's own computed
        // facts additionally pin the first escaping mesh, primitive and vertex.
        "Outside" => {
            got["outcome"] == "Outside"
                && ["depth", "level", "role", "node"].iter().all(|k| {
                    expected["facts"]
                        .get(*k)
                        .is_none_or(|v| got["facts"][*k] == *v)
                })
                && ["mesh", "primitive", "vertex"].iter().all(|k| {
                    oracle["facts"]
                        .get(*k)
                        .filter(|v| !v.is_null())
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
        if agrees(&row["expected"], &row["oracle"], &got) {
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
    // The recorded runs had no C1-precondition refusals; any new one is a change to review.
    assert!(
        summary["c1PreconditionRefused"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{}",
        serde_json::to_string_pretty(&summary).unwrap()
    );
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

/// C2-WB (contract draft v2.1, 9.B): production against O-WB's class fixtures.
/// CI rows: the at-cap pair (`units` -> Contained, `units - 1` -> workLimit), the
/// total and every per-URI fact. Matrix-size rows (`wb/large/`, gitignored): when
/// present, C1 must admit them and the precheck total and facts must equal O-WB's.
#[test]
fn production_matches_every_c2wb_class_fixture() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(ROOT);
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(root.join("wb/manifest.json")).unwrap()).unwrap();
    let cap = |units: u64| ValidationLimits {
        containment_work_units: units,
        ..ValidationLimits::default()
    };
    let (mut agree, mut skipped, mut disagree) = (Vec::new(), Vec::new(), Vec::new());
    for row in manifest["fixtures"].as_array().unwrap() {
        let id = format!(
            "{}@{}",
            row["id"].as_str().unwrap(),
            row["size"].as_str().unwrap()
        );
        let file = root.join(row["file"].as_str().unwrap());
        if !file.exists() {
            skipped.push(id);
            continue;
        }
        let mut problems = Vec::new();
        if row.get("limits").is_some() {
            let got = production(&file, limits(row));
            if !agrees(&row["expected"], &row["oracle"], &got) {
                problems.push(json!({"verdict": got}));
            }
        }
        if let Some(units) = row["units"].as_u64() {
            // Large rows exceed the default caps; facts are read with an open cap.
            let open = || cap(1 << 62);
            match containment_work_total(ValidationRequest::new(&file), open()) {
                Ok(total) if total == units => {}
                other => problems.push(json!({"units": units, "production": format!("{other:?}")})),
            }
            let facts =
                containment_work_facts(ValidationRequest::new(&file), open()).unwrap_or_default();
            for (uri, want) in row["payloadFacts"].as_object().into_iter().flatten() {
                let Some(got) = facts.iter().find(|f| f.uri == *uri) else {
                    problems.push(json!({"uri": uri, "production": "absent"}));
                    continue;
                };
                for (key, value) in [
                    ("T", got.t),
                    ("Idx", got.idx),
                    ("E", got.e),
                    ("Dec", got.dec),
                    ("B", got.b),
                    ("J", got.json_nodes),
                    ("Res", got.external_resources),
                    ("Prim", got.instance_primitives),
                ]
                .into_iter()
                .chain(got.nodes.map(|n| ("Nodes", n)))
                {
                    if want[key].as_u64() != Some(value) {
                        problems.push(json!({"uri": uri, "fact": key,
                            "oracle": want[key], "production": value}));
                    }
                }
            }
        }
        if let Some(pair) = row.get("capPair") {
            let units = pair["atCap"]["containmentWorkUnits"].as_u64().unwrap();
            let at = production(&file, cap(units));
            let under = production(&file, cap(units - 1));
            if at != json!({"outcome":"Contained"})
                || under != json!({"outcome":"NotInspected","reasons":["workLimit"]})
            {
                problems.push(json!({"atCap": at, "oneUnder": under}));
            }
        }
        if problems.is_empty() {
            agree.push(id);
        } else {
            disagree.push(json!({"id": id, "problems": problems}));
        }
    }
    let summary = json!({"fixtures": manifest["fixtures"].as_array().unwrap().len(),
        "agree": agree.len(), "skipped": skipped, "disagree": disagree});
    if let Some(out) = std::env::var_os("C2WB_PARITY_OUT") {
        std::fs::write(out, serde_json::to_vec_pretty(&summary).unwrap()).unwrap();
    }
    assert!(
        disagree.is_empty(),
        "{}",
        serde_json::to_string_pretty(&summary).unwrap()
    );
    // Only the uncommitted matrix-size rows may be absent.
    assert!(
        skipped.iter().all(|id| id.ends_with("@large")),
        "unexpected missing fixtures {skipped:?}"
    );
}
