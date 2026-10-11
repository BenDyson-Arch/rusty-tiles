#!/usr/bin/env python3
"""Oracle implementation mutants (contract acceptance plan, item 3).

    python3 -I oracle/mutants/run.py

Each mutant is a textual patch of one oracle source file. The runner writes
the unified diff to oracle/mutants/patches/<name>.patch (hashed in the
report), runs the mutated oracle over every committed witness/control
fixture (with its limits) and records which verdicts flip against the
unmutated oracle. A mutant that flips nothing, or that does not flip its
named fixture, is a failed control.
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
ORACLE = os.path.dirname(HERE)
TOP = os.path.dirname(ORACLE)

MUTANTS = [
    ("omit-U", "c2o/check.py", "U = [[1, 0, 0], [0, 0, -1], [0, 1, 0]]",
     "U = [[1, 0, 0], [0, 1, 0], [0, 0, 1]]", "W12-TRS-half-turn-order"),
    ("double-U", "c2o/check.py", "U = [[1, 0, 0], [0, 0, -1], [0, 1, 0]]",
     "U = [[1, 0, 0], [0, -1, 0], [0, 0, -1]]", "W12-TRS-half-turn-order"),
    ("reverse-tile-order", "c2o/tileset.py", "child = path[len(path) - distance]",
     "child = path[distance - 1]", "W01-noRTC-variant"),
    ("test-undrawn-vertices", "c2o/check.py", "marks |= chunk", "marks.update(range(count))",
     "W04-unreferenced-indexed-outlier"),
    ("rounded-normalized-quotient", "c2o/gltf.py",
     "            scale = Fraction(1, k)\n",
     "            scale = Fraction(1, 2 ** 1100)\n", "W06-quantized-U16-normalized-node-dequant"),
    ("add-1e-6-slack", "c2o/volumes.py",
     "self.bounds = [int(vol.h[j] * dens) for j in range(3)]",
     "self.bounds = [int((vol.h[j] + Fraction(1, 10 ** 6)) * dens) for j in range(3)]",
     "W12-TRS-half-turn-order~ulp-L0-own-r0"),
    ("rounded-f64-point-as-exact", "c2o/volumes.py",
     "            for (a, beta), bound in zip(self.rows, self.bounds):\n"
     "                d = a[0] * v0 + a[1] * v1 + a[2] * v2 + beta\n",
     "            for j, ((a, beta), bound) in enumerate(zip(self.rows, self.bounds)):\n"
     "                d = a[0] * v0 + a[1] * v1 + a[2] * v2 + beta\n"
     "                c = self.vol.c[j]\n"
     "                d = (Fraction(float(Fraction(d, self.dens) + c)) - c) * self.dens\n",
     "W13-valid-local-TRS-cumulative-shear"),
    ("skip-ancestors", "c2o/tileset.py",
     '                level.volumes.append(LevelVolume("ancestor", distance, frame.volume, frame.where))',
     "                pass", "W11-external-tileset-root-in-chain~ulp-L1-ancestor-r0"),
    ("skip-external-root-transform", "c2o/tileset.py", "                if child.transform is not None:",
     "                if child.transform is not None and not child.external_root:",
     "W11-external-tileset-root-in-chain~ulp-L2-ancestor-r0"),
    ("reuse-URI-cache-for-verdicts", "c2o/check.py",
     "        outside = None\n        for ref in references:\n",
     "        outside = None\n        seen = set()\n        for ref in references:\n"
     "            if ref.member in seen:\n                continue\n            seen.add(ref.member)\n",
     "same-URI-other-reference-outside"),
    ("default-to-scene-0", "c2o/scene.py", "if isinstance(scenes, list) and len(scenes) == 1:",
     "if isinstance(scenes, list) and len(scenes) >= 1:", "ambiguous-scene-no-default"),
    ("raw-quaternion-polynomial", "c2o/scene.py", "L[r][c] = H[r][c] / n * s[c]",
     "L[r][c] = H[r][c] * s[c]", "W13-valid-local-TRS-cumulative-shear"),
]

# The rounded-normalized mutant also needs the decoded words rescaled.
EXTRA = {
    "rounded-normalized-quotient": [(
        "c2o/gltf.py",
        "                elif normalized and word in (\"i8\", \"i16\"):\n"
        "                    v = tuple(max(c, -k) for c in comps)\n",
        "                elif normalized:\n"
        "                    v = tuple(int(Fraction(max(c, -k) / k) * 2 ** 1100) for c in comps)\n")],
}

RUNNER = r"""
import json, sys
sys.path.insert(0, sys.argv[1])
from c2o.check import Oracle
out = {}
for item in json.load(sys.stdin):
    r = Oracle(item["file"], item["limits"]).run()
    f = r.get("facts") or {}
    out[item["id"]] = [r["outcome"], sorted(r.get("reasons") or []), f.get("level"), f.get("role"), f.get("vertex"), f.get("node")]
json.dump(out, sys.stdout)
"""


def verdicts(oracle_dir, fixtures):
    proc = subprocess.run([sys.executable, "-I", "-c", RUNNER, oracle_dir], input=json.dumps(fixtures),
                          capture_output=True, text=True, check=True)
    return json.loads(proc.stdout)


def main():
    with open(os.path.join(TOP, "witnesses", "report.json")) as fh:
        rows = json.load(fh)["rows"]
    fixtures = [{"id": r["id"], "file": os.path.join(TOP, r["file"]), "limits": r["limits"]}
                for r in rows if "file" in r and os.path.exists(os.path.join(TOP, r["file"]))
                and "/large/" not in r["file"]]
    base = verdicts(ORACLE, fixtures)
    patches = os.path.join(HERE, "patches")
    os.makedirs(patches, exist_ok=True)
    results = []
    for name, rel, old, new, target in MUTANTS:
        with tempfile.TemporaryDirectory() as tmp:
            shutil.copytree(os.path.join(ORACLE, "c2o"), os.path.join(tmp, "c2o"),
                            ignore=shutil.ignore_patterns("__pycache__"))
            diff = []
            for path, a, b in [(rel, old, new)] + EXTRA.get(name, []):
                full = os.path.join(tmp, path)
                with open(full) as fh:
                    text = fh.read()
                if text.count(a) != 1:
                    raise SystemExit("mutant %s: anchor not unique in %s" % (name, path))
                mutated = text.replace(a, b)
                with open(full, "w") as fh:
                    fh.write(mutated)
                diff += difflib.unified_diff(text.splitlines(True), mutated.splitlines(True),
                                             "a/oracle/" + path, "b/oracle/" + path)
            patch = "".join(diff)
            with open(os.path.join(patches, name + ".patch"), "w") as fh:
                fh.write(patch)
            got = verdicts(tmp, fixtures)
        flipped = sorted(k for k in base if got.get(k) != base[k])
        ok = bool(flipped) and (target is None or target in flipped)
        results.append({"mutant": name, "patch": "oracle/mutants/patches/%s.patch" % name,
                        "patchSha256": hashlib.sha256(patch.encode()).hexdigest(),
                        "namedFixture": target, "namedFixtureFlips": target in flipped if target else None,
                        "flipped": len(flipped), "flippedExamples": flipped[:12], "pass": ok})
        print("%-30s %-5s flipped %d" % (name, "PASS" if ok else "FAIL", len(flipped)))
    with open(os.path.join(HERE, "report.json"), "w") as fh:
        json.dump({"fixtures": len(fixtures), "mutants": results}, fh, indent=1, sort_keys=True)
    return 0 if all(r["pass"] for r in results) else 1


if __name__ == "__main__":
    sys.exit(main())
