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
    apply_c2_limits(&mut l, &row["limits"]);
    l
}

fn apply_c2_limits(l: &mut ValidationLimits, limits: &Value) {
    for (key, value) in limits.as_object().into_iter().flatten() {
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
                "depth": n("depth"), "level": level, "role": role, "member": fields["content"],
                "node": n("node"),
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

// ---- C2 slice 3 (contract v3.1 §9.B–D): implicit archives ----

/// Apply the I1 (C1) limit overrides of an oracle row (`ValidationLimits` field names).
fn apply_i1_limits(l: &mut ValidationLimits, limits: &Value) {
    for (key, value) in limits.as_object().into_iter().flatten() {
        let v = value.as_u64().expect("oracle limit is u64");
        match key.as_str() {
            "hierarchy_visits" => l.hierarchy_visits = v,
            "hierarchy_depth" => l.hierarchy_depth = v,
            "references" => l.references = v,
            "total_bytes_read" => l.total_bytes_read = v,
            "document_decoded_bytes" => l.document_decoded_bytes = v,
            "member_bytes" => l.member_bytes = v,
            "document_items" => l.document_items = v,
            "json_bytes" => l.json_bytes = v,
            "json_depth" => l.json_depth = v,
            other => panic!("unknown I1 limit {other}"),
        }
    }
}

/// O-implicit's own pre-#160 defaults (`implicit_read/oracle/i1_oracle.py`), under which the
/// I1 controls run at their manifest limits (contract 9.A; as the I1 parity test does).
fn i1_oracle_defaults() -> ValidationLimits {
    ValidationLimits {
        hierarchy_visits: 65_536,
        hierarchy_depth: 128,
        member_bytes: 64 << 20,
        total_bytes_read: 2 << 30,
        document_decoded_bytes: 64 << 20,
        references: 262_144,
        json_bytes: 8 << 20,
        json_depth: 64,
        document_items: 65_536,
        ..ValidationLimits::default()
    }
}

fn sha256_file(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    std::fs::read(path)
        .ok()
        .map(|b| format!("{:x}", Sha256::digest(&b)))
}

/// Production against one slice-3 oracle result (`controls/slice3/results.json` `oracle`, or a
/// corpus witness row): outcome, sorted reasons, and for Outside the §3 facts the oracle
/// reports (volume = role[:k], depth, member, vertex); for ResourceLimit the limit's kind.
fn slice3_agrees(oracle: &Value, got: &Value) -> bool {
    let sorted = |v: &Value| {
        let mut r: Vec<String> = v
            .as_array()
            .into_iter()
            .flatten()
            .map(|x| x.as_str().unwrap().to_owned())
            .collect();
        r.sort_unstable();
        r.dedup();
        json!(r)
    };
    match oracle["outcome"].as_str().unwrap() {
        "Contained" => got["outcome"] == "Contained",
        "NotInspected" => {
            got["outcome"] == "NotInspected" && got["reasons"] == sorted(&oracle["reasons"])
        }
        "Outside" => {
            let f = &got["facts"];
            let volume = match f["level"].as_u64() {
                Some(0) => f["role"].as_str().unwrap().to_owned(),
                Some(k) => format!("{}:{k}", f["role"].as_str().unwrap()),
                None => return false,
            };
            let o = &oracle["facts"];
            got["outcome"] == "Outside"
                && o["volume"] == json!(volume)
                && o["depth"] == f["depth"]
                && o["member"] == f["member"]
                && o["vertex"] == f["vertex"]
        }
        "ResourceLimit" => {
            got["outcome"] == "resource_limit"
                && match oracle["facts"]["limit"].as_str() {
                    Some("hierarchy_depth") => got["message"]
                        .as_str()
                        .is_some_and(|m| m.contains("depth limit")),
                    Some("document_items") => got["message"]
                        .as_str()
                        .is_some_and(|m| m.contains("value-node limit")),
                    _ => true,
                }
        }
        other => panic!("unknown slice-3 oracle outcome {other}"),
    }
}

/// Production's precheck total equals the oracle's `units` for a Contained archive.
fn slice3_units(file: &Path, limits: ValidationLimits, oracle_units: &Value) -> Option<Value> {
    let want = oracle_units.as_u64()?;
    match containment_work_total(ValidationRequest::new(file), limits) {
        Ok(got) if got == want => None,
        other => Some(json!({"units": want, "production": format!("{other:?}")})),
    }
}

/// Production outcome names for an oracle failure kind.
fn kind(k: &str) -> Vec<&str> {
    match k {
        "InvalidInput" => vec!["invalid_input", "C1Hierarchy"],
        "Unsupported" => vec!["unsupported"],
        "ResourceLimit" => vec!["resource_limit"],
        other => vec![other],
    }
}

fn read_root(p: &str) -> Value {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    serde_json::from_slice(&std::fs::read(root.join(p)).unwrap()).unwrap()
}

/// Contract 9.C/9.D: production against every slice-3 control of O-implicit-C2
/// (`controls/slice3/manifest.json`, oracle results `results.json`): verdict, Outside facts and,
/// for Contained cases, the precheck total. Forest-derived controls above 120 kB live outside the
/// repository (`evidence:` rows); they run only when `C2S3_EVIDENCE_DIR` names their directory,
/// and are otherwise listed as skipped. `C2S3_PARITY_OUT` receives the summary.
#[test]
fn production_matches_every_slice3_oracle_control() {
    let dir = format!("{ROOT}/controls/slice3");
    let manifest = read_root(&format!("{dir}/manifest.json"));
    let results = read_root(&format!("{dir}/results.json"));
    let oracle: BTreeMap<&str, &Value> = results["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["id"].as_str().unwrap(), r))
        .collect();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(&dir);
    let evidence = std::env::var_os("C2S3_EVIDENCE_DIR").map(std::path::PathBuf::from);
    let (mut agree, mut skipped, mut disagree) = (Vec::new(), Vec::new(), Vec::new());
    for case in manifest["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let file = case["file"].as_str().unwrap();
        let path = match file.strip_prefix("evidence:") {
            Some(name) => match &evidence {
                Some(d) => d.join(name),
                None => {
                    skipped.push(id.to_owned());
                    continue;
                }
            },
            None => root.join(file),
        };
        assert_eq!(
            sha256_file(&path).as_deref(),
            case["sha256"].as_str(),
            "{id}: fixture missing or re-hashed"
        );
        let o = oracle[id];
        assert_eq!(o["agree"], true, "{id}: committed oracle run disagrees");
        let mut l = ValidationLimits::default();
        apply_i1_limits(&mut l, &case["i1Limits"]);
        apply_c2_limits(&mut l, &case["limits"]);
        let got = production(&path, l.clone());
        let mut problems = Vec::new();
        if !slice3_agrees(&o["oracle"], &got) {
            problems.push(json!({"verdict": got}));
        }
        if o["oracle"]["outcome"] == "Contained" {
            problems.extend(slice3_units(&path, l, &o["oracle"]["totals"]["units"]));
        }
        if problems.is_empty() {
            agree.push(id.to_owned());
        } else {
            disagree.push(json!({"id": id, "oracle": o["oracle"], "problems": problems}));
        }
    }
    let summary = json!({"cases": manifest["cases"].as_array().unwrap().len(),
        "agree": agree.len(), "skipped": skipped, "disagree": disagree});
    if let Some(out) = std::env::var_os("C2S3_PARITY_OUT") {
        std::fs::write(out, serde_json::to_vec_pretty(&summary).unwrap()).unwrap();
    }
    assert!(
        disagree.is_empty(),
        "{}",
        serde_json::to_string_pretty(&summary).unwrap()
    );
}

/// Contract 9.C S3-18: production's C2 outcome on every I1 control after amendment 6, at its
/// manifest limits, against O's sweep (`controls/slice3/results.json` `s3_18`). I1-refused
/// cases must keep their I1 failure kind (no C2 verdict); C2 cases compare verdict, reasons,
/// Outside volume/depth and, when Contained, the precheck total. The cases production refuses
/// in C1 although O-implicit accepts them (C-10, R1b.full) keep production's recorded kind;
/// R5 holds only under an injected allocation refusal and is listed, not run. Only the
/// gitignored `--large` I1 controls may be absent. `C2S3_SWEEP_OUT` receives the summary.
#[test]
fn production_matches_the_slice3_sweep_of_every_i1_control() {
    let i1 = "bench/architecture_audit/implicit_read/controls";
    let manifest = read_root(&format!("{i1}/manifest.json"));
    let sweep = read_root(&format!("{ROOT}/controls/slice3/results.json"));
    let rows: BTreeMap<&str, &Value> = sweep["s3_18"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["id"].as_str().unwrap(), r))
        .collect();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(i1);
    let (mut agree, mut skipped, mut injected, mut disagree) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut categories: BTreeMap<String, usize> = BTreeMap::new();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(rows.len(), cases.len(), "S3-18 covers every I1 control");
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let row = rows[id];
        let path = root.join(case["file"].as_str().unwrap());
        if !path.exists() {
            skipped.push(id.to_owned());
            continue;
        }
        assert_eq!(
            sha256_file(&path).as_deref(),
            case["sha256"].as_str(),
            "{id}: re-hashed"
        );
        let category = row["category"].as_str().unwrap();
        if id == "R5" {
            injected.push(id.to_owned());
            continue;
        }
        *categories.entry(category.to_owned()).or_default() += 1;
        let mut l = i1_oracle_defaults();
        apply_i1_limits(&mut l, &case["limits"]);
        let got = production(&path, l.clone());
        let o = &row["oracle"];
        let mut problems = Vec::new();
        match category {
            "I1-refused" => {
                let want = row["i1Expected"]
                    .as_str()
                    .unwrap()
                    .split(':')
                    .next()
                    .unwrap();
                if !kind(want).contains(&got["outcome"].as_str().unwrap()) {
                    problems.push(json!({"i1": want, "production": got}));
                }
            }
            "production:InvalidInput" => {
                if !kind("InvalidInput").contains(&got["outcome"].as_str().unwrap()) {
                    problems.push(json!({"production": got}));
                }
            }
            _ => {
                let facts = json!({"volume": o["volume"], "depth": o["depth"]});
                let oracle = json!({"outcome": o["outcome"], "reasons": o["reasons"],
                    "facts": facts});
                let same = match o["outcome"].as_str().unwrap() {
                    // S3-18 records volume and depth only; compare those.
                    "Outside" => {
                        got["outcome"] == "Outside"
                            && got["facts"]["depth"] == o["depth"]
                            && slice3_agrees(
                                &json!({"outcome":"Outside","facts":{"volume":o["volume"],
                                "depth":o["depth"],"member":got["facts"]["member"],
                                "vertex":got["facts"]["vertex"]}}),
                                &got,
                            )
                    }
                    _ => slice3_agrees(&oracle, &got),
                };
                if !same {
                    problems.push(json!({"oracle": oracle, "production": got}));
                }
                if o["outcome"] == "Contained" {
                    problems.extend(slice3_units(&path, l, &o["units"]));
                }
            }
        }
        if problems.is_empty() {
            agree.push(id.to_owned());
        } else {
            disagree.push(json!({"id": id, "category": category, "problems": problems}));
        }
    }
    // Contract 9.E "charging reference" pairs. `references` keeps its C1 meaning (I1 §2a step 10):
    // one per actual caller resource visit (C1 contract, "C1 reads/references"). O-implicit's
    // `referencesCharged` (implicit_read/oracle/README.md, "Accounting") counts contents,
    // external-tileset links, subtree members, external buffers and reserved `E`, but not the
    // read of each tileset document, which C1 charges through `reference()` (the root
    // `tileset.json` and every external tileset). So the boundary is O's `referencesCharged` plus
    // O's `tilesets`: that total admits the archive and one less is ResourceLimit. BQ has one
    // tileset document; C-11 has an external implicit-root tileset (two). Emitted contents are
    // reserved once and never charged again (the 9.E mutant flips both pairs).
    let counts: BTreeMap<String, Value> = read_root(&format!("{i1}/oracle-results.json"))
        ["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["id"].as_str().unwrap().to_owned(), r["counts"].clone()))
        .collect();
    for id in ["BQ", "C-11"] {
        let c = &counts[id];
        let total = c["referencesCharged"].as_u64().unwrap() + c["tilesets"].as_u64().unwrap();
        let case = cases.iter().find(|c| c["id"] == id).unwrap();
        let pair = |references: u64| {
            let mut l = i1_oracle_defaults();
            apply_i1_limits(&mut l, &case["limits"]);
            l.references = references;
            production(&root.join(case["file"].as_str().unwrap()), l)
        };
        let (at, under) = (pair(total), pair(total - 1));
        let under_ok = under["outcome"] == "resource_limit"
            && under["message"]
                .as_str()
                .is_some_and(|m| m.contains("reference"));
        let name = format!("{id}.references-pair");
        if at == json!({"outcome":"Contained"}) && under_ok {
            agree.push(name);
        } else {
            disagree.push(json!({"id": name, "references": total, "at": at, "under": under}));
        }
    }
    let summary = json!({"cases": cases.len(), "agree": agree.len(), "categories": categories,
        "skipped": skipped, "injectedOnly": injected, "disagree": disagree});
    if let Some(out) = std::env::var_os("C2S3_SWEEP_OUT") {
        std::fs::write(out, serde_json::to_vec_pretty(&summary).unwrap()).unwrap();
    }
    assert!(
        disagree.is_empty(),
        "{}",
        serde_json::to_string_pretty(&summary).unwrap()
    );
    assert!(
        skipped.iter().all(|id| id.starts_with('R')),
        "unexpected missing fixtures {skipped:?}"
    );
}

/// Contract 9.B and 9.D: production against O's forest witnesses (`corpus/slice3-manifest.json`:
/// verdict and precheck total of every forest archive and `--explicit` twin) and the limit pairs,
/// each value from O's totals (cap → Contained, one less → the named reason). Producer archives
/// live outside the repository: forest rows run only when `C2S3_FOREST_DIR` names R's corpus
/// directory (archives pinned by sha256; a missing or re-hashed archive fails). The in-tree
/// repeated-URI pair always runs. `C2S3_FOREST_OUT` receives the summary.
#[test]
fn production_matches_the_slice3_forest_witnesses_and_limit_pairs() {
    let corpus = read_root(&format!("{ROOT}/corpus/slice3-manifest.json"));
    let forest = std::env::var_os("C2S3_FOREST_DIR").map(std::path::PathBuf::from);
    let top = Path::new(env!("CARGO_MANIFEST_DIR")).join(ROOT);
    let (mut agree, mut skipped, mut disagree) = (Vec::new(), Vec::new(), Vec::new());
    let witness = |id: &str| -> Option<std::path::PathBuf> {
        let row = &corpus["witnesses"][id];
        let path = forest.as_ref()?.join(row["archive"].as_str().unwrap());
        assert_eq!(
            sha256_file(&path).as_deref(),
            row["sha256"].as_str(),
            "{id}: forest archive missing or re-hashed"
        );
        Some(path)
    };
    for (id, row) in corpus["witnesses"].as_object().unwrap() {
        let Some(path) = witness(id) else {
            skipped.push(id.clone());
            continue;
        };
        let got = production(&path, ValidationLimits::default());
        let mut problems = Vec::new();
        if !slice3_agrees(row, &got) {
            problems.push(json!({"verdict": got}));
        }
        if row["outcome"] == "Contained" {
            problems.extend(slice3_units(
                &path,
                ValidationLimits::default(),
                &row["totals"]["units"],
            ));
        }
        if problems.is_empty() {
            agree.push(id.clone());
        } else {
            disagree.push(json!({"id": id, "problems": problems}));
        }
    }
    let mut pairs = Vec::new();
    for (name, pair) in corpus["pairs"].as_object().unwrap() {
        let path = match pair["fixture"].as_str() {
            Some(f) => {
                let p = top.join(f.trim_start_matches(&format!("{ROOT}/")));
                assert_eq!(
                    sha256_file(&p).as_deref(),
                    pair["sha256"].as_str(),
                    "{name}"
                );
                Some(p)
            }
            None => witness(name.split('.').next().unwrap()),
        };
        let Some(path) = path else {
            skipped.push(name.clone());
            continue;
        };
        let at_value = pair["atValue"].as_u64().unwrap();
        let mut at = ValidationLimits::default();
        apply_c2_limits(&mut at, &json!({pair["limit"].as_str().unwrap(): at_value}));
        let mut under = at.clone();
        apply_c2_limits(
            &mut under,
            &json!({pair["limit"].as_str().unwrap(): at_value - 1}),
        );
        let (got_at, got_under) = (production(&path, at), production(&path, under));
        // At the value the precheck passes, so production streams the (Contained) archive.
        let want_under = json!({"outcome":"NotInspected","reasons":pair["underReasons"]});
        let ok = got_at == json!({"outcome":"Contained"}) && got_under == want_under;
        let record = json!({"pair": name, "atValue": at_value, "at": got_at, "under": got_under});
        if ok {
            pairs.push(record);
        } else {
            disagree.push(record);
        }
    }
    let summary = json!({"witnesses": corpus["witnesses"].as_object().unwrap().len(),
        "agree": agree.len(), "pairs": pairs, "skipped": skipped, "disagree": disagree});
    if let Some(out) = std::env::var_os("C2S3_FOREST_OUT") {
        std::fs::write(out, serde_json::to_vec_pretty(&summary).unwrap()).unwrap();
    }
    assert!(
        disagree.is_empty(),
        "{}",
        serde_json::to_string_pretty(&summary).unwrap()
    );
    if forest.is_none() {
        assert!(
            skipped
                .iter()
                .all(|id| id.starts_with("F-") || id.starts_with("T-")),
            "{skipped:?}"
        );
    }
}

/// Contract §10 item 2 (precheck-only arm), a timing helper, not an acceptance check: one
/// `inspect_selected` of `C2S3_ARM_FILE`, with `containmentBytesRead` = `C2S3_ARM_BYTES` when set
/// (O's total − 1 on F-MF2: the precheck runs to the last reference, then `workLimit`, no streaming),
/// printing production's outcome. R times the test process against the same run of the
/// `gltfUpAxis` twin (no precheck, no streaming).
#[test]
#[ignore = "perf helper: run explicitly with C2S3_ARM_FILE"]
fn slice3_precheck_only_arm() {
    let Some(file) = std::env::var_os("C2S3_ARM_FILE") else {
        return;
    };
    let mut l = ValidationLimits::default();
    if let Ok(bytes) = std::env::var("C2S3_ARM_BYTES") {
        l.containment_bytes_read = bytes.parse().unwrap();
    }
    println!("{}", production(Path::new(&file), l));
}

// ---- H11: corner fast-path controls (oracle-decided, `h11/generate.py`) ----

/// Every H11 fixture against the oracle report recorded beside it. The pairs are built so that a
/// broken fast path (wrong corners, inward corners, skipped fallback, declared bounds, chunk offset,
/// first-vertex report) flips them; production must agree on every one, including the first
/// escaping vertex.
#[test]
fn production_matches_every_h11_fast_path_control() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(ROOT);
    let report: Value =
        serde_json::from_slice(&std::fs::read(root.join("h11/report.json")).unwrap()).unwrap();
    let (mut agree, mut disagree) = (0, Vec::new());
    let rows = report["rows"].as_array().unwrap();
    for row in rows {
        let file = root.join(row["file"].as_str().unwrap());
        let got = production(&file, ValidationLimits::default());
        if agrees(&row["expected"], &row["oracle"], &got) {
            agree += 1;
        } else {
            disagree.push(json!({"id": row["id"], "expected": row["expected"], "production": got}));
        }
    }
    let summary = json!({"cases": rows.len(), "agree": agree, "disagree": disagree});
    if let Some(out) = std::env::var_os("C2H11_PARITY_OUT") {
        std::fs::write(out, serde_json::to_vec_pretty(&summary).unwrap()).unwrap();
    }
    assert!(
        disagree.is_empty() && agree == 14,
        "{}",
        serde_json::to_string_pretty(&summary).unwrap()
    );
}
