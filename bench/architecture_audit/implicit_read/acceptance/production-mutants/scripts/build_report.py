"""Build production-mutants/report.json from a run_i1_mutants.sh results directory.

A control flips when production disagrees with the oracle, when a timed control misses its own
contract bound (§E), or, for R5b, when its allocation-probe clause fails. Bounds:
R7, R7a, R7b, R7c and R8 within 5 s; R8a within 2x R8a.nopad and within 60 s. The runner's
per-mutant timeouts (60 s, 300 s) only cap the run.
python3 -I build_report.py PATCH_DIR RESULTS_DIR"""
import hashlib, json, os, sys

PM, RES = sys.argv[1], sys.argv[2]
FIVE_SECONDS = {"R7", "R7a", "R7b", "R7c", "R8"}
# Contract clauses decided by an in-crate probe rather than the verdict.
PROBES = {"R5b": "content_integrity::implicit::availability::tests::visit_reservation_precedes_rank_table_and_tables"}
DIAGNOSIS = json.load(open(os.path.join(PM, "diagnoses.json")))


def load(name):
    path = os.path.join(RES, name)
    return json.load(open(path)) if os.path.exists(path) else None


def walls(parity):
    return {r["id"]: r["production"].get("wallSeconds") for r in parity["rows"]} if parity else {}


def breaches(w, timed_out_case=None):
    out = {}
    for case in FIVE_SECONDS:
        if case in w and w[case] is not None and w[case] > 5:
            out[case] = f"{w[case]:.2f} s > 5 s"
    if "R8a" in w and w["R8a"] is not None:
        nopad = w.get("R8a.nopad")
        if w["R8a"] > 60 or (nopad and w["R8a"] > 2 * nopad):
            out["R8a"] = f"{w['R8a']:.2f} s > min(60 s, 2 x R8a.nopad {nopad})"
    if timed_out_case:
        out[timed_out_case] = "exceeded the run timeout"
    return out


def matches(flip, paired):
    return flip == paired or (flip.startswith(paired) and not flip[len(paired)].isdigit())


sha = lambda p: hashlib.sha256(open(p, "rb").read()).hexdigest()
clean = load("clean.result.json")
assert clean and not clean["controlsFlipped"] and not clean["corpusFlipped"] and not clean["unitFailed"]
clean_bounds = breaches(walls(load("clean.parity.json")))
assert not clean_bounds, clean_bounds
rows = []
for m in json.load(open(os.path.join(PM, "mutants.json"))):
    r = load(m["name"] + ".result.json")
    parity = load(m["name"] + ".parity.json")
    w = walls(parity)
    bound = breaches(w, m["timingCase"] if r["timedOut"] else None)
    probes = [case for case, test in PROBES.items() if test in r["unitFailed"]]
    flipped = sorted(set(r["controlsFlipped"] or []) | set(bound) | {c + " (probe)" for c in probes})
    corpus = r["corpusFlipped"] or []
    paired = [p.strip().split(" ")[0] for p in m["paired"].split(",")]
    hit = sorted({p for p in paired for f in flipped + corpus if matches(f.removesuffix(" (probe)"), p)})
    rows.append({"name": m["name"], "patchSha256": sha(os.path.join(PM, m["name"] + ".patch")), "paired": m["paired"],
                 "timingCase": m["timingCase"], "timedOut": r["timedOut"],
                 "controlsFlipped": flipped, "corpusFlipped": corpus, "boundBreaches": bound,
                 "probeClausesFailed": probes, "pairedFlipped": hit,
                 "unitTestsFailed": r["unitFailed"], "diagnosis": DIAGNOSIS.get(m["name"])})
summary = {"total": len(rows),
           "pairedControlFlips": sum(bool(r["pairedFlipped"]) for r in rows),
           "pairedControlDoesNotFlip": [r["name"] for r in rows if not r["pairedFlipped"]],
           "flipsNoControl": [r["name"] for r in rows if not r["controlsFlipped"] and not r["corpusFlipped"]],
           "unitTestsDetect": sum(bool(r["unitTestsFailed"]) for r in rows)}
report = {"base": open(os.path.join(PM, "BASE")).read().strip(),
          "rules": {"fiveSecondBound": sorted(FIVE_SECONDS), "R8a": "<= 2x R8a.nopad and <= 60 s", "probes": PROBES},
          "clean": {"controls": clean["controlsFlipped"], "corpus": clean["corpusFlipped"], "unitFailed": clean["unitFailed"],
                    "timedWalls": {k: v for k, v in walls(load("clean.parity.json")).items() if k in FIVE_SECONDS | {"R8a", "R8a.nopad"}}},
          "mutants": rows, "summary": summary}
json.dump(report, open(os.path.join(PM, "report.json"), "w"), indent=1)
print(json.dumps(summary, indent=1))
