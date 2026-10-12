#!/usr/bin/env python3
"""Run the oracle on every materialized case in ../controls/manifest.json and compare with the recorded expectation.

    python3 -I check.py [--large] [--only ID ...]

Each case is run in its own `python3 -I` subprocess (single-threaded; no shared state).  Results go to
../controls/oracle-results.json (small) or ../controls/oracle-results-large.json (large).  Exit status 1 on any
disagreement, missing file or hash mismatch.
"""
import hashlib
import json
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
ORACLE = os.path.join(ROOT, "oracle", "i1_oracle.py")


def run(path, limits):
    cmd = [sys.executable, "-I", ORACLE, "--summary"]
    for k, v in sorted(limits.items()):
        cmd += ["--limit", f"{k}={v}"]
    t0 = time.monotonic()
    out = subprocess.run(cmd + [path], check=True, capture_output=True, text=True).stdout
    return json.loads(out), time.monotonic() - t0


def main(argv):
    large = "--large" in argv
    only = set(argv[argv.index("--only") + 1:]) if "--only" in argv else None
    manifest = json.load(open(os.path.join(ROOT, "controls", "manifest.json")))
    cases = [c for c in manifest["cases"] if c["file"].startswith("large/") == large]
    if only:
        cases = [c for c in cases if c["id"] in only]
    digests = {}
    results = []
    bad = 0
    for c in cases:
        path = os.path.join(ROOT, "controls", c["file"])
        if not os.path.exists(path):
            results.append({"id": c["id"], "status": "missing"})
            bad += 1
            continue
        sha = hashlib.sha256(open(path, "rb").read()).hexdigest()
        if sha != c["sha256"]:
            results.append({"id": c["id"], "status": "hash-mismatch"})
            bad += 1
            continue
        rec, wall = run(path, c["limits"])
        v = rec["verdict"]
        got = v["kind"] + (f":{v['limit']}" if v["kind"] == "ResourceLimit" else "")
        digests[c["id"]] = rec.get("tilesSha256")
        ok = got == c["expected"]
        results.append({"id": c["id"], "expected": c["expected"], "oracle": got, "reason": v["reason"],
                        "stage": v["stage"], "agree": ok, "tilesSha256": rec.get("tilesSha256"),
                        "counts": rec["counts"], "wallSeconds": round(wall, 3)})
        if not ok:
            bad += 1
    by_id = {r["id"]: r for r in results}
    for c in cases:
        if "facts" not in c or c["id"] not in by_id or "agree" not in by_id[c["id"]]:
            continue
        rel, other = c["facts"]
        mine, theirs = digests.get(c["id"]), digests.get(other)
        if theirs is None:
            other_rec = None
            for fname in ("oracle-results.json", "oracle-results-large.json"):
                p = os.path.join(ROOT, "controls", fname)
                if os.path.exists(p):
                    for r in json.load(open(p))["results"]:
                        if r["id"] == other:
                            other_rec = r
            theirs = other_rec and other_rec.get("tilesSha256")
        holds = mine is not None and theirs is not None and ((mine == theirs) == (rel == "equal"))
        by_id[c["id"]]["facts"] = {"relation": rel, "to": other, "holds": holds}
        if not holds:
            by_id[c["id"]]["agree"] = False
            bad += 1
    name = "oracle-results-large.json" if large else "oracle-results.json"
    if not only:
        with open(os.path.join(ROOT, "controls", name), "w") as f:
            json.dump({"manifestCases": len(cases), "disagreements": bad, "results": results}, f, indent=1,
                      sort_keys=True)
            f.write("\n")
    for r in results:
        if not r.get("agree"):
            print(json.dumps(r, sort_keys=True))
    print(json.dumps({"cases": len(cases), "disagreements": bad}))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
