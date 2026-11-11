#!/usr/bin/env python3
"""Run the slice-3 oracle on every slice-3 control and on every I1 control (S3-18), and record the results.

    python3 -I controls/slice3/check.py [--evidence DIR] [--only ID ...] [--skip-i1]

- `manifest.json` cases: the oracle must agree with each designed expectation (outcome, reasons, and the
  Outside `volume` and `depth`, or the I1 `limit`). S3-19 rows carry no designed expectation; the oracle's
  result is recorded as the expectation.
- S3-18: every case of `implicit_read/controls/manifest.json` (after amendment 6), at its manifest limits, in
  the categories of contract 9.C.
Writes `results.json` (oracle facts per case, including the I1 verdict and the precheck totals). Exit 1 on any
disagreement, missing file or hash mismatch. Missing evidence-only files are reported, not failed, unless
`--require-evidence`.
"""

import hashlib
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TOP = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.join(TOP, "oracle"))
from c2o.implicit import ImplicitOracle, i1_module  # noqa: E402

I1_ROOT = os.path.normpath(os.path.join(TOP, "..", "implicit_read"))
DEFAULT_EVIDENCE = "/home/bend/.cache/c2s3-evidence/slice3"


def summary(report):
    facts = report.get("facts") or {}
    keep = ("volume", "depth", "member", "reference", "implicit", "vertex", "limit", "why")
    precheck = report.get("precheck") or {}
    return {
        "outcome": report["outcome"],
        "reasons": report.get("reasons"),
        "why": report.get("why"),
        "phase": report.get("phase"),
        "facts": {k: facts[k] for k in keep if k in facts} or None,
        "precheckStop": report.get("precheckStop"),
        "totals": precheck.get("totals"),
        "i1": {"kind": report["i1"]["verdict"]["kind"], "reason": report["i1"]["verdict"]["reason"],
               "limit": report["i1"]["verdict"]["limit"]} if "i1" in report else None,
        "implicit": report.get("implicit"),
    }


def agrees(expected, got):
    if expected is None:
        return True
    if expected["outcome"] != got["outcome"]:
        return False
    if "reasons" in expected and expected["reasons"] != got["reasons"]:
        return False
    if "limit" in expected and (got["facts"] or {}).get("limit") != expected["limit"]:
        return False
    for k, v in (expected.get("facts") or {}).items():
        if (got["facts"] or {}).get(k) != v:
            return False
    return True


def category(case, got):
    """Contract 9.C S3-18 categories."""
    if case.get("production"):
        return "production:" + case["production"].split(" ")[0]
    if got["i1"] is None or got["i1"]["kind"] != "Accepted":
        return "I1-refused"
    if got["reasons"] == ["derivedImplicitVolume"]:
        return "derived"
    if got["reasons"] == ["tileMetadata"]:
        return "tileMetadata"
    return got["outcome"]


def run(path, limits, i1_limits):
    return summary(ImplicitOracle(path, limits, None, i1_limits, detail=False).run())


def dumps_rows(obj):
    """Indented JSON with each element of a "rows" list on one line (compact, diffable)."""
    def enc(value, indent):
        pad = " " * indent
        if isinstance(value, dict):
            items = []
            for k in sorted(value):
                v = value[k]
                if k == "rows" and isinstance(v, list):
                    body = (",\n" + pad + "  ").join(json.dumps(r, sort_keys=True, separators=(",", ":")) for r in v)
                    items.append('%s "%s": [\n%s  %s\n%s ]' % (pad, k, pad, body, pad))
                else:
                    items.append('%s "%s": %s' % (pad, k, enc(v, indent + 1)))
            return "{\n" + ",\n".join(items) + "\n" + pad + "}"
        return json.dumps(value, sort_keys=True)
    return enc(obj, 0) + "\n"


def main(argv):
    evidence = argv[argv.index("--evidence") + 1] if "--evidence" in argv else DEFAULT_EVIDENCE
    only = set(argv[argv.index("--only") + 1:]) if "--only" in argv else None
    require = "--require-evidence" in argv
    manifest = json.load(open(os.path.join(HERE, "manifest.json")))
    rows, bad, missing = [], 0, []
    for case in manifest["cases"]:
        if only and case["id"] not in only:
            continue
        f = case["file"]
        path = os.path.join(HERE, f) if f.startswith("fixtures/") else os.path.join(evidence, f.split(":", 1)[1])
        if not os.path.exists(path):
            missing.append(case["id"])
            bad += require or f.startswith("fixtures/")
            continue
        if hashlib.sha256(open(path, "rb").read()).hexdigest() != case["sha256"]:
            rows.append({"id": case["id"], "status": "hash-mismatch"})
            bad += 1
            continue
        got = run(path, case["limits"], case["i1Limits"])
        ok = agrees(case["expected"], got)
        bad += not ok
        rows.append({"id": case["id"], "control": case["control"], "role": case["role"],
                     "expected": case["expected"], "oracle": got, "agree": ok})
        if not ok:
            print(json.dumps(rows[-1], sort_keys=True))
    i1_rows = []
    if "--skip-i1" not in argv and not only:
        i1 = json.load(open(os.path.join(I1_ROOT, "controls", "manifest.json")))
        for case in i1["cases"]:
            path = os.path.join(I1_ROOT, "controls", case["file"])
            if not os.path.exists(path):
                i1_rows.append({"id": case["id"], "status": "missing (large; generate.py --large)"})
                continue
            if hashlib.sha256(open(path, "rb").read()).hexdigest() != case["sha256"]:
                i1_rows.append({"id": case["id"], "status": "hash-mismatch"})
                bad += 1
                continue
            # Contract 9.A: I1 controls run at their manifest limits over O-implicit's own defaults, as in
            # the I1 parity test (oracle_defaults), not at the #160 defaults.
            got = run(path, {}, dict(i1_module().DEFAULT_LIMITS, **case["limits"]))
            i1_kind = case["expected"].split(":")[0]
            consistent = got["i1"] is not None and got["i1"]["kind"] == i1_kind
            bad += not consistent
            facts = got["facts"] or {}
            compact = {"outcome": got["outcome"], "reasons": got["reasons"], "volume": facts.get("volume"),
                       "depth": facts.get("depth"), "i1": [got["i1"]["kind"], got["i1"]["limit"]] if got["i1"] else None,
                       "units": (got["totals"] or {}).get("units")}
            i1_rows.append({"id": case["id"], "i1Expected": case["expected"], "production": case.get("production"),
                            "category": category(case, got), "oracle": compact, "i1Consistent": consistent})
    result = {"cases": len(rows), "disagreements": bad, "missingEvidence": missing, "rows": rows}
    if i1_rows:
        cats = {}
        for r in i1_rows:
            cats.setdefault(r.get("category", r.get("status")), []).append(r["id"])
        result["s3_18"] = {"cases": len(i1_rows), "categories": {k: sorted(v) for k, v in sorted(cats.items())},
                           "rows": i1_rows}
    if not only:
        with open(os.path.join(HERE, "results.json"), "w") as fh:
            fh.write(dumps_rows(result))
    print(json.dumps({"cases": len(rows), "s3_18": len(i1_rows), "disagreements": bad,
                      "missingEvidence": len(missing)}))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
