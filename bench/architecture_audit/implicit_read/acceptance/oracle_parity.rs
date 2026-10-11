//! I1 acceptance: production `inspect_selected` and the `implicit_tile_facts` seam against
//! every frozen O-implicit witness and control (coordinator-owned, R-helper). Expectations
//! come only from the oracle's committed manifest and reports; nothing here derives a
//! verdict from production code.
//!
//! Compared per case: the outcome kind (Accepted / InvalidInput / Unsupported /
//! ResourceLimit and its limit name), and for Accepted cases the canonical tile list
//! (`tilesSha256`, the oracle's digest form) plus the `tiles`, `implicitTiles` and
//! `contentReferences` counters (every content entry, external tilesets included). Writes a JSON summary to `I1_PARITY_OUT`; `I1_PROD_FACTS_DIR` receives the
//! production tile list of every Accepted case for offline diffing.
use crate::content_integrity::implicit::EffectiveVolume;
use crate::validate::{
    implicit_tile_facts, inspect_selected, ImplicitTileFact, ValidationFailure, ValidationLimits,
    ValidationRequest,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path, time::Instant};

const ROOT: &str = "bench/architecture_audit/implicit_read";

/// Disagreements diagnosed as O-side (fixture or stale production note), each pinned to the
/// exact production outcome observed. They are reported, never counted as agreement, and the test
/// fails if one changes. Production defects are never listed here. Z-payload was fixed by the
/// i1/oracle amendment `2ecbad1`; C-11b remains (below).
// C-11b was the only O-side difference; oracle amendment 4 (1d2b5e9) fixed its
// document-level geometricError, so production and oracle now agree on every case.
const O_SIDE: &[(&str, &str, &str)] = &[];

/// The oracle's frozen `DEFAULT_LIMITS` (`oracle/i1_oracle.py:29-39`, pre-#160 values). Parity
/// runs production under the same ceilings O decided with; `I1_PRODUCTION_DEFAULTS=1` instead
/// keeps production's own (#160) defaults for an informational run.
fn oracle_defaults() -> ValidationLimits {
    let mut l = ValidationLimits::default();
    if std::env::var_os("I1_PRODUCTION_DEFAULTS").is_none() {
        l.hierarchy_visits = 65_536;
        l.hierarchy_depth = 128;
        l.member_bytes = 64 << 20;
        l.total_bytes_read = 2 << 30;
        l.document_decoded_bytes = 64 << 20;
        l.references = 262_144;
        l.json_bytes = 8 << 20;
        l.json_depth = 64;
        l.document_items = 65_536;
    }
    l
}

fn limits(row: &Value) -> ValidationLimits {
    let mut l = oracle_defaults();
    for (key, value) in row["limits"].as_object().into_iter().flatten() {
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
            other => panic!("unknown oracle limit {other}"),
        }
    }
    l
}

fn hex(v: f64) -> String {
    format!("{:016x}", v.to_bits())
}

/// One production fact in the oracle's record form (README "Verdicts and record format").
fn record(f: &ImplicitTileFact) -> Value {
    use crate::content_integrity::implicit::EffectiveError;
    let join = |v: &[f64]| v.iter().map(|x| hex(*x)).collect::<Vec<_>>().join(",");
    let volume = match &f.volume {
        EffectiveVolume::Row(r) => format!("row:{}", join(r)),
        EffectiveVolume::DerivedRoot => "derived-root".into(),
        EffectiveVolume::Derived(c) => format!(
            "derived:{};{};{},{},{}",
            join(&c.root),
            c.level,
            c.coords[0],
            c.coords[1],
            c.coords[2]
        ),
    };
    let error = match f.error {
        EffectiveError::Row(e) | EffectiveError::Derived(e) => hex(e),
    };
    json!({
        "doc": f.doc, "level": f.level, "x": f.coords[0], "y": f.coords[1], "z": f.coords[2],
        "depth": f.depth, "parent": f.parent_level.map_or(-1, i64::from),
        "volume": volume, "error": error, "refine": f.refine,
        "contents": f.contents.iter().map(|(s, m)| json!([s, m])).collect::<Vec<_>>(),
    })
}

/// Python `json.dumps(v, sort_keys=True, separators=(",", ":"), ensure_ascii=True)`.
/// serde_json (no `preserve_order`) already sorts object keys; escape non-printable ASCII.
fn canonical(v: &Value) -> String {
    let s = serde_json::to_string(v).unwrap();
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if (' '..='~').contains(&ch) || matches!(ch, '\n' | '\r' | '\t') {
            out.push(ch);
        } else {
            let mut buf = [0u16; 2];
            for unit in ch.encode_utf16(&mut buf) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

/// Map a production ResourceLimit message to the `ValidationLimits` field it names.
/// `read_member`'s message is shared by `member_bytes` and the I1 live-path allowance
/// (`document_decoded_bytes`), whose remaining value is passed as the read maximum.
fn limit_name(m: &str) -> &'static str {
    let m = m.to_ascii_lowercase();
    if m.starts_with("member exceeds admitted byte limit") {
        "member_bytes|document_decoded_bytes"
    } else if m.contains("visits") || m.contains("traversal tasks") {
        "hierarchy_visits"
    } else if m.contains("depth limit") {
        "hierarchy_depth"
    } else if m.contains("reference") {
        "references"
    } else if m.contains("total_bytes_read") || m.contains("work limit") {
        "total_bytes_read"
    } else if m.contains("member exceeds") || m.contains("member_bytes") {
        "member_bytes"
    } else if m.contains("decoded") || m.contains("document_decoded_bytes") || m.contains("live") {
        "document_decoded_bytes"
    } else {
        "?"
    }
}

/// Production result in the oracle's vocabulary.
/// Production's outcome, with a panic recorded as its own outcome so that one case cannot abort
/// the run (production mutants may panic).
fn guarded(file: &Path, limits: ValidationLimits, dump: Option<(&Path, &str)>) -> Value {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        production(file, limits, dump)
    }))
    .unwrap_or_else(|panic| {
        let message = panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|m| (*m).to_owned()))
            .unwrap_or_default();
        json!({"kind": "Panic", "message": message})
    })
}

/// `I1_ONLY=ID[,ID…]` restricts both tests to those cases (single-case timing runs of the
/// production mutants); filtered cases are neither run nor reported as skipped.
fn selected(id: &str) -> bool {
    std::env::var("I1_ONLY").map_or(true, |only| only.split(',').any(|o| o == id))
}

fn production(file: &Path, limits: ValidationLimits, dump: Option<(&Path, &str)>) -> Value {
    let start = Instant::now();
    let report = inspect_selected(ValidationRequest::new(file), limits.clone());
    let wall = start.elapsed().as_secs_f64();
    match report {
        Ok(report) => {
            let facts = implicit_tile_facts(ValidationRequest::new(file), limits)
                .expect("the facts seam repeats an accepted inspection");
            let tiles: Vec<Value> = facts.iter().map(record).collect();
            let text = canonical(&Value::Array(tiles));
            if let Some((dir, id)) = dump {
                std::fs::write(dir.join(format!("{id}.json")), &text).unwrap();
            }
            let digest = if facts.is_empty() {
                Value::Null
            } else {
                json!(format!("{:x}", Sha256::digest(text.as_bytes())))
            };
            json!({"kind":"Accepted","tilesSha256":digest,"implicitTiles":facts.len(),
                "tiles":report.tiles,"contentReferences":report.content_references,
                "checks":report.checks,"wallSeconds":wall})
        }
        Err(f) => {
            let (kind, m) = match &f {
                ValidationFailure::InvalidInput(m) => ("InvalidInput", m.clone()),
                ValidationFailure::Unsupported(m) => ("Unsupported", m.clone()),
                ValidationFailure::ResourceLimit(m) => ("ResourceLimit", m.clone()),
                other => ("Other", other.to_string()),
            };
            let kind = if kind == "ResourceLimit" {
                format!("ResourceLimit:{}", limit_name(&m))
            } else {
                kind.to_owned()
            };
            json!({"kind":kind,"message":m,"wallSeconds":wall})
        }
    }
}

#[test]
fn production_matches_every_i1_oracle_witness_and_control() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(ROOT);
    let read = |p: &str| -> Value {
        serde_json::from_slice(&std::fs::read(root.join(p)).unwrap()).unwrap()
    };
    let mut oracle: BTreeMap<String, Value> = BTreeMap::new();
    for report in [
        "controls/oracle-results.json",
        "controls/oracle-results-large.json",
    ] {
        for r in read(report)["results"].as_array().unwrap() {
            oracle.insert(r["id"].as_str().unwrap().into(), r.clone());
        }
    }
    let dump = std::env::var_os("I1_PROD_FACTS_DIR").map(std::path::PathBuf::from);
    if let Some(dir) = &dump {
        std::fs::create_dir_all(dir).unwrap();
    }
    let (mut agree, mut skipped, mut disagree, mut rows) = (0, Vec::new(), Vec::new(), Vec::new());
    let mut o_side_rows = Vec::new();
    let manifest = read("controls/manifest.json");
    for case in manifest["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        if !selected(id) {
            continue;
        }
        let file = root.join("controls").join(case["file"].as_str().unwrap());
        if !file.exists() {
            skipped.push(id.to_owned());
            continue;
        }
        let o = &oracle[id];
        assert_eq!(
            o["oracle"], case["expected"],
            "{id}: committed oracle run disagrees"
        );
        // A recorded production expectation outside O's domain overrides O's verdict, except
        // R5's, which holds only under an injected allocation refusal not available here.
        let want = match case["production"].as_str() {
            Some(p) if id != "R5" => p.split(' ').next().unwrap().to_owned(),
            _ => case["expected"].as_str().unwrap().to_owned(),
        };
        let got = guarded(&file, limits(case), dump.as_deref().map(|d| (d, id)));
        let kind = got["kind"].as_str().unwrap();
        let mut problems = Vec::new();
        let kind_ok = match (want.split_once(':'), kind.split_once(':')) {
            (None, Some(("ResourceLimit", _))) => want == "ResourceLimit",
            (Some(("ResourceLimit", w)), Some(("ResourceLimit", g))) => {
                g.split('|').any(|g| g == w)
            }
            _ => kind == want,
        };
        if !kind_ok {
            problems.push(format!("kind {kind} != {want}"));
        }
        // Facts are compared wherever both sides accepted and O recorded them.
        if kind == "Accepted" && o["oracle"] == "Accepted" {
            if got["tilesSha256"] != o["tilesSha256"] {
                problems.push("tilesSha256".into());
            }
            let counts = &o["counts"];
            for k in ["tiles", "implicitTiles", "contentReferences"] {
                if got[k] != counts[k] {
                    problems.push(format!("{k} {} != {}", got[k], counts[k]));
                }
            }
        }
        let o_side = O_SIDE
            .iter()
            .find(|(k, g, _)| *k == id && *g == kind && !problems.is_empty())
            .map(|(_, _, why)| *why);
        let row = json!({"id":id,"oSide":o_side,"expected":case["expected"],"productionExpected":want,
            "question":case["question"],"production":got,
            "oracleTilesSha256":o["tilesSha256"],"oracleCounts":o["counts"],"problems":problems});
        if problems.is_empty() {
            agree += 1;
        } else if o_side.is_some() {
            o_side_rows.push(row.clone());
        } else {
            disagree.push(row.clone());
        }
        rows.push(row);
    }
    let summary = json!({"cases":manifest["cases"].as_array().unwrap().len(),"run":rows.len(),
        "agree":agree,"skipped":skipped,"oSide":o_side_rows,"disagree":disagree,"rows":rows});
    if let Some(out) = std::env::var_os("I1_PARITY_OUT") {
        std::fs::write(out, serde_json::to_vec_pretty(&summary).unwrap()).unwrap();
    }
    let brief = json!({"run":summary["run"],"agree":agree,"skipped":summary["skipped"],
        "disagree":summary["disagree"]});
    assert!(
        disagree.is_empty(),
        "{}",
        serde_json::to_string_pretty(&brief).unwrap()
    );
    assert_eq!(
        summary["oSide"].as_array().unwrap().len(),
        O_SIDE.len(),
        "an O-side disagreement changed"
    );
    // Only the gitignored `--large` controls may be absent.
    assert!(
        skipped.iter().all(|id| id.starts_with('R')),
        "unexpected missing fixtures {skipped:?}"
    );
}

/// W corpus (producer artifacts pinned by sha256 in `corpus/manifest.json`, oracle verdicts and
/// digests in `corpus/results.json`). Artifacts live outside the repository; a missing or
/// re-hashed file is skipped and listed, so CI runs only what is present.
#[test]
fn production_matches_the_oracle_on_the_w_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(ROOT);
    let read = |p: &str| -> Value {
        serde_json::from_slice(&std::fs::read(root.join(p)).unwrap()).unwrap()
    };
    let results: BTreeMap<String, Value> = read("corpus/results.json")["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["id"].as_str().unwrap().to_owned(), r.clone()))
        .collect();
    let (mut rows, mut skipped, mut disagree) = (Vec::new(), Vec::new(), Vec::new());
    for a in read("corpus/manifest.json")["artifacts"]
        .as_array()
        .unwrap()
    {
        let id = a["id"].as_str().unwrap();
        if !selected(id) {
            continue;
        }
        let path = Path::new(a["path"].as_str().unwrap());
        let present = std::fs::read(path)
            .ok()
            .is_some_and(|b| format!("{:x}", Sha256::digest(&b)) == a["sha256"].as_str().unwrap());
        if !present {
            skipped.push(id.to_owned());
            continue;
        }
        let o = &results[id];
        assert_eq!(
            o["oracle"], a["expected"],
            "{id}: committed oracle run disagrees"
        );
        let want = match a["production"].as_str() {
            Some(p) => p.split(' ').next().unwrap().to_owned(),
            None => a["expected"].as_str().unwrap().to_owned(),
        };
        let got = guarded(path, limits(a), None);
        let kind = got["kind"].as_str().unwrap();
        let mut problems = Vec::new();
        let kind_ok = match kind.split_once(':') {
            Some(("ResourceLimit", g)) => {
                want == "ResourceLimit"
                    || want
                        .split_once(':')
                        .is_some_and(|(_, w)| g.split('|').any(|g| g == w))
            }
            _ => kind == want,
        };
        if !kind_ok {
            problems.push(format!("kind {kind} != {want}"));
        }
        if kind == "Accepted" && o["oracle"] == "Accepted" {
            if got["tilesSha256"] != o["tilesSha256"] {
                problems.push("tilesSha256".into());
            }
            let c = &o["counts"];
            for k in ["tiles", "implicitTiles", "contentReferences"] {
                if got[k] != c[k] {
                    problems.push(format!("{k} {} != {}", got[k], c[k]));
                }
            }
        }
        let row = json!({"id":id,"expected":a["expected"],"productionExpected":want,"production":got,
            "oracleTilesSha256":o["tilesSha256"],"oracleCounts":o["counts"],"problems":problems});
        if !problems.is_empty() {
            disagree.push(row.clone());
        }
        rows.push(row);
    }
    let summary = json!({"artifacts":rows.len()+skipped.len(),"run":rows.len(),
        "agree":rows.len()-disagree.len(),"skipped":skipped,"disagree":disagree,"rows":rows});
    if let Some(out) = std::env::var_os("I1_CORPUS_PARITY_OUT") {
        std::fs::write(out, serde_json::to_vec_pretty(&summary).unwrap()).unwrap();
    }
    assert!(
        summary["disagree"].as_array().unwrap().is_empty(),
        "{}",
        serde_json::to_string_pretty(&json!({"disagree":summary["disagree"]})).unwrap()
    );
}
