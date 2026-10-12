#!/usr/bin/env python3
"""C2-WB formula mutants (contract C2-WB 9.C, the unit-arithmetic subset) against the
oracle, scored on the CI-size at-cap pairs of wb/manifest.json.

    python3 -I wb/mutants.py

Each mutant is a textual edit of one oracle source file, applied to a temporary
copy of c2o. A mutant passes when it flips at least one at-cap pair and flips its
named pair (Contained at the cap, or NotInspected(workLimit) one under).
The unified diffs are hashed into wb/mutants-report.json.
"""

import difflib
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
TOP = os.path.dirname(HERE)
ORACLE = os.path.join(TOP, "oracle")

UNITS = ("        units = (base * max(FLOOR_BASE, q) + step * max(FLOOR_STEP, q)\n"
         "                 + boxes * max(FLOOR_BOX, q) + spheres * max(FLOOR_SPHERE, q))\n")
MUTANTS = [
    ("frozen-vertex-weights", "c2o/check.py", UNITS, "        units = q\n", "box-chain-L16"),
    ("drop-step-term", "c2o/check.py", "step * max(FLOOR_STEP, q)", "0 * max(FLOOR_STEP, q)", "box-step-L16"),
    ("sphere-floor-equals-box-floor", "c2o/check.py", "FLOOR_SPHERE = 32", "FLOOR_SPHERE = 25",
     "terms-content-sphere"),
    ("drop-level-0-base", "c2o/check.py", "(base * max(FLOOR_BASE, q)", "(0 * max(FLOOR_BASE, q)", "terms-base"),
    ("drop-R0-1152", "c2o/check.py", "REFERENCE_FIXED = 1152", "REFERENCE_FIXED = 0", "tiny-refs-L4"),
    ("drop-B", "c2o/check.py", "return (facts.Dec + facts.B + ", "return (facts.Dec + ", "json-flat-string-glb"),
    ("drop-Dec", "c2o/check.py", "return (facts.Dec + facts.B + ", "return (facts.B + ", "meshopt-attributes-raw"),
    ("drop-J", "c2o/check.py", "PER_JSON_NODE = 16", "PER_JSON_NODE = 0", "unselected-nodes"),
    ("drop-Res", "c2o/check.py", "PER_RESOURCE = 160", "PER_RESOURCE = 0", "extbuf-20"),
    ("drop-Prim", "c2o/check.py", "PER_INSTANCE_PRIMITIVE = 4", "PER_INSTANCE_PRIMITIVE = 0",
     "shared-accessor-160x20"),
    ("node-floor-128", "c2o/scene.py", "NODE_FLOOR = 256", "NODE_FLOOR = 128", "terms-node"),
    ("drop-8-per-level", "c2o/check.py", "PER_LEVEL = 8  ", "PER_LEVEL = 0  ", "tiny-refs-L103"),
    ("J-without-b3dm-batch-table", "c2o/gltf.py",
     '        nodes += value_nodes(_table_json(data[bt_start:bt_start + btj], "batch table", where))',
     '        _table_json(data[bt_start:bt_start + btj], "batch table", where)', "json-flat-numbers-b3dm"),
    ("Res-counts-per-distinct-member", "c2o/gltf.py",
     "                data = self._read_resource(uri)\n                self.external_resources += 1",
     "                seen = len(self._resolved)\n                data = self._read_resource(uri)\n"
     "                self.external_resources += len(self._resolved) - seen", "reading-res-duplicate-uri"),
]

RUNNER = r"""
import json, sys
sys.path.insert(0, sys.argv[1])
from c2o.check import Oracle
out = {}
for item in json.load(sys.stdin):
    r = Oracle(item["file"], item["limits"]).run()
    out[item["key"]] = [r["outcome"], sorted(r.get("reasons") or []), (r.get("precheck") or {}).get("totals", {}).get("units")]
json.dump(out, sys.stdout)
"""


def verdicts(oracle_dir, items):
    proc = subprocess.run([sys.executable, "-I", "-c", RUNNER, oracle_dir], input=json.dumps(items),
                          capture_output=True, text=True, check=True)
    return json.loads(proc.stdout)


def main():
    with open(os.path.join(HERE, "manifest.json")) as fh:
        rows = [r for r in json.load(fh)["fixtures"] if "capPair" in r and r["size"] == "ci"]
    items = []
    for r in rows:
        for label in ("atCap", "oneUnder"):
            limits = dict(r.get("limits") or {}, containmentWorkUnits=r["capPair"][label]["containmentWorkUnits"])
            items.append({"key": r["id"] + ":" + label, "file": os.path.join(TOP, r["file"]), "limits": limits})
    base = verdicts(ORACLE, items)
    results = []
    for name, rel, old, new, target in MUTANTS:
        with tempfile.TemporaryDirectory() as tmp:
            shutil.copytree(os.path.join(ORACLE, "c2o"), os.path.join(tmp, "c2o"),
                            ignore=shutil.ignore_patterns("__pycache__"))
            full = os.path.join(tmp, rel)
            with open(full) as fh:
                text = fh.read()
            if text.count(old) != 1:
                raise SystemExit("mutant %s: anchor not unique in %s" % (name, rel))
            mutated = text.replace(old, new)
            with open(full, "w") as fh:
                fh.write(mutated)
            patch = "".join(difflib.unified_diff(text.splitlines(True), mutated.splitlines(True),
                                                 "a/oracle/" + rel, "b/oracle/" + rel))
            got = verdicts(tmp, items)
        flipped = sorted({k.split(":")[0] for k in base if got[k][:2] != base[k][:2]})
        ok = bool(flipped) and (target is None or target in flipped)
        results.append({"mutant": name, "file": rel, "patchSha256": hashlib.sha256(patch.encode()).hexdigest(),
                        "patch": patch, "namedPair": target, "namedPairFlips": (target in flipped) if target else None,
                        "pairsFlipped": len(flipped), "flipped": flipped, "pass": ok})
        print("%-34s %-5s flips %2d pairs%s" % (name, "PASS" if ok else "FAIL", len(flipped),
                                                "" if target is None else " (named: %s)" % target))
    with open(os.path.join(HERE, "mutants-report.json"), "w") as fh:
        json.dump({"pairs": len(rows), "mutants": results}, fh, indent=1, sort_keys=True)
        fh.write("\n")
    return 0 if all(r["pass"] for r in results) else 1


if __name__ == "__main__":
    sys.exit(main())
