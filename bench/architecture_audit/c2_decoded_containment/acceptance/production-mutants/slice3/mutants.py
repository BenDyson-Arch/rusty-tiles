"""C2 slice-3 production mutants (contract v3.1 9.E), coordinator R.

Usage (repository root of the integration branch, clean `src/`):
    python3 -I bench/architecture_audit/c2_decoded_containment/acceptance/production-mutants/slice3/mutants.py \
        OUT_DIR [--evidence DIR] [--forest DIR] [name ...]

Each mutant is a set of exact single-occurrence string edits of `src/`. The driver applies it,
saves `git diff -- src` as `<name>.patch` next to this file, runs the release parity tests
(`c2_oracle_parity`, `i1_oracle_parity`) with every summary written to OUT_DIR/<name>/, records
which cases each summary lists as disagreeing, and restores `src/`. A mutant is caught when its
paired control is among the disagreeing cases (or, for `compile`/`panic`, when the build or a
test aborts). `clean` (no edit) must flip nothing. The report is `report.json` next to this file.

Slice-3 mutants are the 9.E rows. The slice-1 production mutants
(`../scripts/mutants.py`, 19) are re-run on the same suites, so the slice-3 forest witnesses and
controls are among their controls; two are re-expressed for the slice-3 `build_levels`.
"""
import hashlib
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(os.path.dirname(HERE), "scripts"))
import mutants as slice1  # noqa: E402

V = "src/validate.rs"
LOCK = "/home/bend/.cache/rusty-tiles-cargo.lock"
TILE_FACT = (
    "            let index = self.c2.tile(TileFact {\n"
    "                transform: None,\n"
    "                volume: match &tile.volume {\n"
    "                    EffectiveVolume::Row(b) => Some(VolumeFacts::Box(*b)),\n"
    "                    EffectiveVolume::DerivedRoot => Some(volume_facts(&r.bounds)),\n"
    "                    EffectiveVolume::Derived(_) => None,\n"
    "                },\n"
    "                external: None,\n"
    "                parent: Some(parent_tile),\n"
    "                depth,\n"
    "                chain: 0,\n"
    "            })?;\n"
)
PATH_PARENT = "                    .map(|(volume, error, index)| (volume, *error, *index))\n"
C2_FACTS = "            // C2 facts: rows exactly (FLOAT32 already widened by I1), V_R for\n"


SLICE3 = [
    {"name": "keep-implicitTiling-exclusion", "paired": "F-W7",
     "row": "Keep the implicitTiling exclusion (F-W1; the reason now carries the derivedImplicitVolume bit)",
     "edits": [(V, "        let implicit = if declares_implicit(&node) {\n            Some(\n",
                "        let implicit = if declares_implicit(&node) {\n"
                "            self.c2.exclude(Reason::DerivedImplicitVolume);\n            Some(\n")]},
    {"name": "skip-I0-level", "paired": "S3-17.ancestor2",
     "row": "Skip the I_0 level (contract S3-4; O's reading 1: S3-17.ancestor2)",
     "edits": [(V, PATH_PARENT,
                "                    .map(|(volume, error, index)| {\n"
                "                        (volume, *error, if level == 1 { r.tile } else { *index })\n"
                "                    })\n")]},
    {"name": "merge-I0-into-R", "paired": "S3-17.ancestor3",
     "row": "Skip the V_R level: I_0 takes R's place (transform, external volume, parent) (S3-5)",
     "edits": [(V, TILE_FACT, TILE_FACT
                .replace("                transform: None,\n",
                         "                transform: if level == 0 { self.c2.tiles[r.tile].transform } else { None },\n")
                .replace("                external: None,\n",
                         "                external: if level == 0 { self.c2.tiles[r.tile].external } else { None },\n")
                .replace("                parent: Some(parent_tile),\n",
                         "                parent: if level == 0 { self.c2.tiles[r.tile].parent } else { Some(parent_tile) },\n"))]},
    {"name": "own-row-only", "paired": "S3-17.ancestor1",
     "row": "Test only the own row: every implicit tile's parent is R (S3-3)",
     "edits": [(V, PATH_PARENT, "                    .map(|(volume, error, _)| (volume, *error, r.tile))\n")]},
    {"name": "R-transform-before-V_R", "paired": "S3-8.syn",
     "row": "Apply R.transform before testing V_R (moved from R onto I_0's step)",
     "edits": [(V, C2_FACTS,
                "            let moved = if level == 0 { self.c2.tiles[r.tile].transform.take() } else { None };\n"
                + C2_FACTS),
               (V, TILE_FACT, TILE_FACT.replace("                transform: None,\n",
                                                "                transform: moved,\n"))]},
    {"name": "external-content-at-R-level", "paired": "S3-17.externalContent4",
     "row": "Test an external-root R's referencing content volume at R's level, before R.transform (S3-15)",
     "edits": [(V, C2_FACTS,
                "            let moved = if level == 0 { self.c2.tiles[r.tile].external.take() } else { None };\n"
                + C2_FACTS),
               (V, TILE_FACT, TILE_FACT.replace("                external: None,\n",
                                                "                external: moved,\n"))]},
    {"name": "derived-as-rounded-cell", "paired": "S3-9",
     "row": "Use Cell::rounded_for_hierarchy_bounds for derived tiles",
     "edits": [(V, TILE_FACT, TILE_FACT.replace(
         "                    EffectiveVolume::Derived(_) => None,\n",
         "                    EffectiveVolume::Derived(cell) => {\n"
         "                        Some(VolumeFacts::Box(cell.rounded_for_hierarchy_bounds()))\n"
         "                    }\n"))]},
    {"name": "archive-scoped-derived", "paired": "S3-10",
     "row": "Archive-scoped derived exclusion",
     "edits": [(V, C2_FACTS,
                "            if matches!(tile.volume, EffectiveVolume::Derived(_)) {\n"
                "                self.c2.exclude(Reason::DerivedImplicitVolume);\n            }\n" + C2_FACTS)]},
    {"name": "derived-detected-in-build-levels-only", "paired": "S3-20",
     "row": "Derived detected in build_levels only (no chain bit)",
     "edits": [(V, "            .map_or(Reason::DerivedImplicitVolume.bit(), shape_bit)\n",
                "            .map_or(0, shape_bit)\n"),
               (V, "        self.build_levels(tile, content)?;\n        self.precheck = self.precheck_step(&facts);\n",
                "        if self.build_levels(tile, content).is_err() {\n"
                "            self.exclude(Reason::DerivedImplicitVolume);\n            return Ok(());\n        }\n"
                "        self.precheck = self.precheck_step(&facts);\n")]},
    {"name": "float32-rows-via-shortest-decimal", "paired": "S3-14.syn.base",
     "row": "Read FLOAT32 rows rounded (each widened binary32 value through its shortest decimal, as f64)",
     "edits": [(V, TILE_FACT, TILE_FACT.replace(
         "                    EffectiveVolume::Row(b) => Some(VolumeFacts::Box(*b)),\n",
         "                    EffectiveVolume::Row(b) => Some(VolumeFacts::Box(b.map(|v| {\n"
         "                        if (v as f32) as f64 == v && v.is_finite() {\n"
         "                            format!(\"{}\", v as f32).parse().unwrap()\n"
         "                        } else {\n                            v\n                        }\n"
         "                    }))),\n"))]},
    {"name": "subtree-tileMetadata-is-tileMetadata", "paired": "F-W7",
     "row": "Treat subtree tileMetadata as tileMetadata (F-W1)",
     "edits": [(V, C2_FACTS,
                "            if matches!(tile.volume, EffectiveVolume::Row(_)) {\n"
                "                self.c2.exclude(Reason::TileMetadata);\n            }\n" + C2_FACTS)]},
    {"name": "ignore-header-metadata", "paired": "S3-12.header",
     "row": "Ignore header metadata (omit F step 7)",
     "edits": [(V, "                if header.get(\"metadata\").is_some() {\n"
                   "                    self.c2.exclude(Reason::TileMetadata);\n                }\n", "")]},
    {"name": "depth-is-chain-length-minus-1", "paired": "S3-17.own",
     "row": "depth = chain length - 1 (depth(R) + l + 1 for an implicit tile)",
     "edits": [(V, TILE_FACT, TILE_FACT.replace("                depth,\n", "                depth: depth + 1,\n"))]},
    {"name": "charging-reference-for-emitted-contents", "paired": "BQ.references-pair",
     "row": "Register emitted contents with the charging ArchiveResources::reference (I1 R9-shape references pair)",
     "edits": [(V, "                self.resources.reference_reserved(name)?;\n",
                "                self.resources.reference(name.clone())?;\n")]},
    {"name": "units-per-URI", "paired": "repeated-uri.cap-1",
     "row": "Units per URI instead of per reference (9.D repeated-URI pair)",
     "edits": [(V, "        if self.exclusions != 0 || self.precheck.is_some() {\n            return Ok(());\n",
                "        if self.exclusions != 0\n            || self.precheck.is_some()\n"
                "            || self.references.iter().filter(|r| r.uri == uri).count() > 1\n"
                "        {\n            return Ok(());\n")]},
    {"name": "fixed-chain-capacity", "paired": "S3-16",
     "row": "A fixed chain capacity (synthetic: no such capacity exists; 13 = S3-16's hierarchy_depth + 1, as O's twin)",
     "edits": [(V, "            try_push(&mut self.levels, fact)?;\n            current = parent;\n",
                "            if self.levels.len() >= 13 {\n"
                "                return Err(Error::limit(\"fixed chain capacity\"));\n            }\n"
                "            try_push(&mut self.levels, fact)?;\n            current = parent;\n")]},
]

# Slice-1 mutants whose edit no longer matches the slice-3 build_levels, re-expressed.
REEXPRESSED = {
    "skip-ancestors": [(V, "                    Some((Role::Ancestor(level), volume)),\n",
                        "                    None,\n")],
    "skip-content-volume": [(V, "                volumes: [Some((Role::Own, own)), content, None],\n",
                             "                volumes: [Some((Role::Own, own)), None, None],\n")],
}
SLICE1 = []
for m in slice1.MUTANTS:
    m = dict(m)
    m["edits"] = REEXPRESSED.get(m["name"], m["edits"])
    m["slice1"] = True
    m["paired"] = m.get("paired")
    m["name"] = "slice1-" + m["name"]
    SLICE1.append(m)

MUTANTS = [{"name": "clean", "edits": [], "paired": None, "row": "no edit: must flip nothing"}] + SLICE3 + SLICE1


def sh(cmd, **kw):
    return subprocess.run(cmd, check=True, capture_output=True, text=True, **kw).stdout


def run(name, out, evidence, forest):
    os.makedirs(out, exist_ok=True)
    for f in os.listdir(out):
        os.remove(os.path.join(out, f))
    env = dict(os.environ, CARGO_BUILD_JOBS="2", RUST_TEST_THREADS="2",
               CARGO_TARGET_DIR="/home/bend/.cache/rt-target-a",
               C2_PARITY_OUT=f"{out}/c2.json", C2WB_PARITY_OUT=f"{out}/c2wb.json",
               C2S3_PARITY_OUT=f"{out}/s3.json", C2S3_SWEEP_OUT=f"{out}/sweep.json",
               C2S3_FOREST_OUT=f"{out}/forest.json", I1_PARITY_OUT=f"{out}/i1.json",
               I1_CORPUS_PARITY_OUT=f"{out}/i1-corpus.json")
    if evidence:
        env["C2S3_EVIDENCE_DIR"] = evidence
    if forest:
        env["C2S3_FOREST_DIR"] = forest
    cmd = ["flock", LOCK, "nice", "-n", "10", "taskset", "-c", "0-3", "cargo", "test", "--locked", "--release",
           "--lib", "--", "c2_oracle_parity", "i1_oracle_parity"]
    p = subprocess.run(cmd, capture_output=True, text=True, env=env)
    with open(f"{out}/test.log", "w") as fh:
        fh.write(p.stdout + p.stderr)
    compiled = "error[E" not in p.stderr and "could not compile" not in p.stderr
    tests = {}
    for line in p.stdout.splitlines():
        if line.startswith("test ") and " ... " in line:
            t, status = line[5:].split(" ... ", 1)
            tests[t] = status.strip()
    flipped = {}
    for f in ("c2", "c2wb", "s3", "sweep", "forest", "i1", "i1-corpus"):
        path = f"{out}/{f}.json"
        if not os.path.exists(path):
            continue
        s = json.load(open(path))
        ids = [d.get("id") or d.get("pair") for d in s.get("disagree", [])]
        ids += [d.get("id") for d in s.get("oSide", [])]
        if ids:
            flipped[f] = ids
    return {"compiled": compiled, "exit": p.returncode, "tests": tests, "flipped": flipped}


def main(argv):
    out = argv[0]
    evidence = argv[argv.index("--evidence") + 1] if "--evidence" in argv else None
    forest = argv[argv.index("--forest") + 1] if "--forest" in argv else None
    skip = {i for i, a in enumerate(argv) if a in ("--evidence", "--forest")}
    names = [a for i, a in enumerate(argv[1:], 1) if not a.startswith("--") and i - 1 not in skip]
    if subprocess.run(["git", "diff", "--quiet", "--", "src"]).returncode:
        raise SystemExit("src/ has uncommitted changes")
    head = sh(["git", "rev-parse", "HEAD"]).strip()
    report_path = os.path.join(HERE, "report.json")
    report = json.load(open(report_path)) if os.path.exists(report_path) else {}
    report.update({"head": head, "suites": "cargo test --locked --release --lib -- c2_oracle_parity i1_oracle_parity"})
    rows = report.setdefault("mutants", {})
    for m in MUTANTS:
        if names and m["name"] not in names:
            continue
        try:
            for path, old, new in m["edits"]:
                text = open(path).read()
                if text.count(old) != 1:
                    raise SystemExit(f"{m['name']}: edit matches {text.count(old)} times in {path}")
                open(path, "w").write(text.replace(old, new))
            patch = sh(["git", "diff", "--", "src"])
            if m["edits"]:
                with open(os.path.join(HERE, m["name"] + ".patch"), "w") as fh:
                    fh.write(patch)
            res = run(m["name"], os.path.join(out, m["name"]), evidence, forest)
        finally:
            sh(["git", "checkout", "--", "src"])
        every = sorted({i for ids in res["flipped"].values() for i in ids})
        failed = sorted(t for t, s in res["tests"].items() if s != "ok" and not s.startswith("ignored"))
        slice3 = sorted({i for k in ("s3", "sweep", "forest") for i in res["flipped"].get(k, [])})
        if m["name"] == "clean":
            caught = not every and not failed and res["exit"] == 0
        elif m.get("slice1"):
            # Re-run of a slice-1 mutant: it must still flip, and slice-3 flips are recorded.
            caught = res["compiled"] and bool(every)
        else:
            caught = res["compiled"] and m["paired"] in every
        row = {"row": m.get("row") or m.get("what"), "paired": m["paired"],
               "patchSha256": hashlib.sha256(patch.encode()).hexdigest() if m["edits"] else None,
               "compiled": res["compiled"], "failedTests": failed, "flippedCount": len(every),
               "flipped": res["flipped"], "slice3Flips": slice3,
               "pairedFlips": m["paired"] in every if m["paired"] else None,
               "pass": caught}
        rows[m["name"]] = row
        print(json.dumps({"mutant": m["name"], "pass": caught, "flipped": len(every),
                          "pairedFlips": row["pairedFlips"], "slice3": len(slice3),
                          "failedTests": failed}), flush=True)
        with open(report_path, "w") as fh:
            json.dump(report, fh, indent=1, sort_keys=True)
            fh.write("\n")


if __name__ == "__main__":
    main(sys.argv[1:])
