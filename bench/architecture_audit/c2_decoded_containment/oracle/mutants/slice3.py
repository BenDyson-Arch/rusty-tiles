#!/usr/bin/env python3
"""Slice-3 oracle mutants (contract 9.E, oracle side).

    python3 -I oracle/mutants/slice3.py [--evidence DIR]

Each mutant is a textual patch of a copy of `c2o/` (mostly `c2o/implicit.py`). The runner writes the
unified diff to `oracle/mutants/slice3/<name>.patch` (hashed in `slice3-report.json`), runs the mutated
oracle over every slice-3 control in `controls/slice3/manifest.json` (committed fixtures, plus evidence files
when present) and records which results flip against the unmutated oracle. A mutant that flips nothing, or
does not flip its named control, fails.
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
SLICE3 = os.path.join(TOP, "controls", "slice3")
DEFAULT_EVIDENCE = "/home/bend/.cache/c2s3-evidence/slice3"
I = "c2o/implicit.py"

# (name, [(file, old, new)], named control, contract 9.E row)
MUTANTS = [
    ("keep-implicitTiling-exclusion",
     [(I, "                self._walk_implicit(tile, member, where, path)\n",
       "                self.exclude(\"implicitTiling\", where)\n"
       "                self._walk_implicit(tile, member, where, path)\n")],
     "F-W7", "Keep the implicitTiling exclusion (F-W1 when the evidence is present)"),
    ("skip-I0-level",
     [(I, "            chain = path + stack\n",
       "            chain = path + (stack[1:] if level else stack)\n")],
     "S3-17.ancestor2", "Skip the I_0 level"),
    ("merge-I0-into-R",
     [(I, "            chain = path + stack\n", "            chain = path[:-1] + stack\n")],
     "S3-17.ancestor3", "Skip the V_R level (merge I_0 into R)"),
    ("own-row-only",
     [(I, "            chain = path + stack\n", "            chain = path + stack[-1:]\n")],
     "S3-17.ancestor1", "Test only the own row (skip implicit ancestors); also S3-3"),
    ("R-transform-before-V_R",
     [(I, "            chain = path + stack\n",
       "            r_frame = TS._Frame(path[-1].tile, path[-1].where, path[-1].volume, None, None)\n"
       "            i0 = TS._Frame(None, stack[0].where, stack[0].volume, path[-1].transform, None)\n"
       "            chain = path[:-1] + [r_frame, i0] + stack[1:]\n")],
     "S3-8.syn.base", "Apply R.transform before testing V_R"),
    ("external-content-at-R-level",
     [(I, "            chain = path + stack\n",
       "            parent = path[-2] if len(path) > 1 else None\n"
       "            if parent is not None and parent.external_content is not None:\n"
       "                p2 = TS._Frame(parent.tile, parent.where, parent.volume, parent.transform, None)\n"
       "                r2 = TS._Frame(path[-1].tile, path[-1].where, path[-1].volume, path[-1].transform,\n"
       "                               parent.external_content)\n"
       "                chain = path[:-2] + [p2, r2] + stack\n"
       "            else:\n"
       "                chain = path + stack\n")],
     "S3-17.externalContent4", "Test an external-root R's referencing content volume at R's level, "
                               "before R.transform"),
    ("derived-as-root-volume",
     [(I, "        return DerivedVolume(vol)\n", "        return root_volume\n")],
     "S3-9", "Use an approximate box for derived tiles (Cell::rounded_for_hierarchy_bounds stand-in: V_R)"),
    ("archive-scoped-derived",
     [(I, "            vol = effective_volume(rec, root_frame.volume, twhere)\n",
       "            vol = effective_volume(rec, root_frame.volume, twhere)\n"
       "            if isinstance(vol, DerivedVolume):\n"
       "                self.exclude(DERIVED_REASON, twhere)\n")],
     "S3-10", "Archive-scoped derived exclusion"),
    ("derived-detected-at-precheck-only",
     [(I, "    def exclusion(self):\n        return DERIVED_REASON\n",
       "    def exclusion(self):\n        return None\n")],
     "S3-20", "Derived detected in build_levels only (no chain bit): the precheck stops first"),
    ("float32-rows-via-shortest-decimal",
     [(I, "        return parse_volume({\"box\": values}, where)\n",
       "        f32 = lambda v: struct.unpack(\"<f\", struct.pack(\"<f\", v))[0]\n"
       "        values = [next(float(t) for t in (\"%.*g\" % (p, v) for p in range(1, 18)) if f32(float(t)) == v)\n"
       "                  if abs(v) < 3e38 and f32(v) == v else v for v in values]\n"
       "        return parse_volume({\"box\": values}, where)\n")],
     "S3-14.syn.base", "Read FLOAT32 rows rounded (through their shortest decimal)"),
    ("subtree-tileMetadata-is-tileMetadata",
     [(I, "            vol = effective_volume(rec, root_frame.volume, twhere)\n",
       "            vol = effective_volume(rec, root_frame.volume, twhere)\n"
       "            if rec[\"volume\"].startswith(\"row:\"):\n"
       "                self.exclude(\"tileMetadata\", twhere)\n")],
     "F-W7", "Treat subtree tileMetadata as tileMetadata (F-W1 when the evidence is present)"),
    ("ignore-header-metadata",
     [(I, "                if \"metadata\" in content:\n                    # I6",
       "                if \"metadata\" in content and not implicit:\n                    # I6")],
     "S3-12.header", "Ignore header metadata (omit F step 7)"),
    ("depth-is-chain-length-minus-1",
     [(I, "                ref.depth = rec[\"depth\"]  # section 3: depth(R) + l, not the chain length - 1\n", "")],
     "S3-17.own", "depth = chain length - 1"),
    ("units-per-URI",
     [("c2o/check.py",
       "        level_cache = {}  # (id(levels), member) -> (levels, spans, V, rows); levels may be shared\n"
       "        for ref in references:\n",
       "        level_cache = {}  # (id(levels), member) -> (levels, spans, V, rows); levels may be shared\n"
       "        charged = set()\n"
       "        for ref in references:\n"
       "            if ref.member in charged:\n"
       "                continue\n"
       "            charged.add(ref.member)\n")],
     "repeated-uri.cap-1", "Units per URI instead of per reference"),
    ("fixed-chain-capacity (synthetic)",
     [(I, "                self.max_chain = max(self.max_chain, len(ref.levels))\n",
       "                self.max_chain = max(self.max_chain, len(ref.levels))\n"
       "                if len(ref.levels) > 13:  # hierarchy_depth + 1 at S3-16's hierarchy_depth 12\n"
       "                    raise OracleUndecided(\"fixed chain capacity exceeded\")\n")],
     "S3-16", "A fixed 129-level chain capacity (synthetic; here hierarchy_depth + 1 = 13)"),
]

RUNNER = r"""
import json, sys
sys.path.insert(0, sys.argv[1])
from c2o.implicit import ImplicitOracle
out = {}
for item in json.load(sys.stdin):
    try:
        r = ImplicitOracle(item["file"], item["limits"], {"maxVertexLevelTests": 4000000000}, item["i1Limits"],
                           detail=False).run()
    except Exception as err:  # a mutant may crash; that is a flip, not a runner failure
        out[item["id"]] = ["Crash", type(err).__name__, str(err)[:120]]
        continue
    f = r.get("facts") or {}
    out[item["id"]] = [r["outcome"], sorted(r.get("reasons") or []), f.get("volume"), f.get("depth"),
                       f.get("vertex"), f.get("reference"), f.get("limit")]
json.dump(out, sys.stdout)
"""


def verdicts(oracle_dir, fixtures):
    proc = subprocess.run([sys.executable, "-I", "-c", RUNNER, oracle_dir], input=json.dumps(fixtures),
                          capture_output=True, text=True, check=True)
    return json.loads(proc.stdout)


def main(argv):
    evidence = argv[argv.index("--evidence") + 1] if "--evidence" in argv else DEFAULT_EVIDENCE
    manifest = json.load(open(os.path.join(SLICE3, "manifest.json")))
    fixtures = []
    for c in manifest["cases"]:
        f = c["file"]
        path = os.path.join(SLICE3, f) if f.startswith("fixtures/") else os.path.join(evidence, f.split(":", 1)[1])
        if os.path.exists(path):
            fixtures.append({"id": c["id"], "file": path, "limits": c["limits"], "i1Limits": c["i1Limits"]})
    base = verdicts(ORACLE, fixtures)
    if any(v[0] == "Crash" for v in base.values()):
        raise SystemExit("the unmutated oracle crashes: %s" % [k for k, v in base.items() if v[0] == "Crash"])
    patches = os.path.join(HERE, "slice3")
    os.makedirs(patches, exist_ok=True)
    results = []
    for name, edits, target, row in [("unmutated copy (runner sanity)", [], None, "must flip nothing")] + MUTANTS:
        with tempfile.TemporaryDirectory() as tmp:
            # Same relative layout as the repository, so c2o/implicit.py finds O-implicit.
            oracle_copy = os.path.join(tmp, "c2_decoded_containment", "oracle")
            shutil.copytree(os.path.join(ORACLE, "c2o"), os.path.join(oracle_copy, "c2o"),
                            ignore=shutil.ignore_patterns("__pycache__"))
            os.makedirs(os.path.join(tmp, "implicit_read", "oracle"))
            shutil.copy(os.path.join(TOP, "..", "implicit_read", "oracle", "i1_oracle.py"),
                        os.path.join(tmp, "implicit_read", "oracle", "i1_oracle.py"))
            diff = []
            for rel, old, new in edits:
                full = os.path.join(oracle_copy, rel)
                text = open(full).read()
                if text.count(old) != 1:
                    raise SystemExit("mutant %s: anchor not unique in %s" % (name, rel))
                mutated = text.replace(old, new)
                open(full, "w").write(mutated)
                diff += difflib.unified_diff(text.splitlines(True), mutated.splitlines(True),
                                             "a/oracle/" + rel, "b/oracle/" + rel)
            patch = "".join(diff)
            fname = name.split(" ")[0]
            if edits:
                with open(os.path.join(patches, fname + ".patch"), "w") as fh:
                    fh.write(patch)
            got = verdicts(oracle_copy, fixtures)
        flipped = sorted(k for k in base if got.get(k) != base[k])
        crashed = sorted(k for k in flipped if got[k][0] == "Crash")
        ok = (target in flipped and target not in crashed) if target else not flipped
        results.append({"mutant": name, "contractRow": row, "crashed": len(crashed),
                        "patch": "oracle/mutants/slice3/%s.patch" % fname if edits else None,
                        "patchSha256": hashlib.sha256(patch.encode()).hexdigest(), "namedControl": target,
                        "namedControlFlips": ok, "flipped": len(flipped), "flippedExamples": flipped[:16],
                        "namedControlBase": base.get(target), "namedControlMutant": got.get(target), "pass": ok})
        print("%-40s %-4s flipped %3d crashed %2d  %s" % (name, "PASS" if ok else "FAIL", len(flipped), len(crashed),
                                                        target), flush=True)
    with open(os.path.join(HERE, "slice3-report.json"), "w") as fh:
        json.dump({"fixtures": len(fixtures), "mutants": results,
                   "notModelled": ["Register emitted contents with the charging ArchiveResources::reference: "
                                   "C1 reference accounting is production-only; O-implicit's references "
                                   "verdicts cover it (I1 R9 pair)"]}, fh, indent=1, sort_keys=True)
        fh.write("\n")
    return 0 if all(r["pass"] for r in results) else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
