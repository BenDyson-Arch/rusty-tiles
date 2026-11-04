"""Assemble production-mutants/report.json from the driver outputs."""
import hashlib
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import mutants as M  # noqa: E402

ORACLE = os.path.join(M.WT, "bench/architecture_audit/c2_decoded_containment/oracle/mutants/report.json")
oracle = {m["mutant"]: m for m in json.load(open(ORACLE))["mutants"]}
base = json.load(open(os.path.join(M.OUT, "baseline.json")))

rows = []
for m in M.MUTANTS:
    name = m["name"]
    patch = os.path.join(M.WT, M.ACC, name + ".patch")
    out = os.path.join(M.OUT, name + ".json")
    row = {
        "name": name,
        "kind": "oracle-mirror" if m["oracle"] else "production-specific",
        "what": m["what"],
        "patch": "production-mutants/" + name + ".patch",
        "patchSha256": hashlib.sha256(open(patch, "rb").read()).hexdigest(),
    }
    if not os.path.exists(out):
        row["status"] = "no parity summary (build or test harness failure; see log)"
        rows.append(row)
        continue
    s = json.load(open(out))
    flipped = sorted(d["id"] for d in s["disagree"])
    newly_c1 = sorted(set(s["c1PreconditionRefused"]) - set(base["c1PreconditionRefused"]))
    paired = oracle[name]["namedFixture"] if m["oracle"] else m["paired"]
    row.update({
        "status": "ok",
        "cases": s["cases"],
        "agree": s["agree"],
        "flipped": len(flipped),
        "flippedIds": flipped[:20],
        "flippedIdsTruncated": max(0, len(flipped) - 20),
        "productionOutcomes": sorted({json.dumps(d["production"].get("outcome")) for d in s["disagree"]}),
        "newlyC1PreconditionRefused": newly_c1,
        "pairedFixture": paired,
        "pairedFixtureSource": "oracle namedFixture of the same-named oracle mutant" if m["oracle"]
        else "chosen for this production-specific mutant (no oracle counterpart)",
        "pairedFixtureFlipped": paired in flipped,
    })
    if m["oracle"]:
        row["oracleFlipped"] = oracle[name]["flipped"]
    rows.append(row)

report = {
    "sourceCommit": "1c9783b",
    "acceptanceTest": "bench/architecture_audit/c2_decoded_containment/acceptance/oracle_parity.rs (--lib c2_oracle_parity)",
    "baseline": {"cases": base["cases"], "agree": base["agree"], "disagree": len(base["disagree"])},
    "mutants": rows,
    "undetected": [r["name"] for r in rows if r.get("flipped") == 0],
    "findings": {
        "volumes-in-slot-order": {
            "diagnosis": "acceptance corpus gap, not a no-op",
            "reasoning": "Level l>0 slots are [Ancestor(l), None, ExternalContent(l)], so slot order tests the "
                         "ancestor before externalContent; level 0 slots [Own, Content] already match Role order. "
                         "The order only changes a verdict when a vertex is outside BOTH the ancestor and the "
                         "externalContent volume at the same level. No fixture does that: "
                         "external-content-volume-excludes is outside only the shrunk content volume (its tile "
                         "volume still contains the vertex), and each ~ulp-L*-ancestor / ~ulp-L*-externalContent "
                         "variant shrinks only one volume. The unit test "
                         "validate::containment::tests::volumes_are_tested_in_role_order does fail on this patch.",
            "missingFixture": "external-content-volume-excludes with the referencing tile's boundingVolume also "
                              "shrunk to exclude the vertex (content volume kept inside the tile for C1): "
                              "expected Outside role=externalContent level=1; slot order reports ancestor:1.",
        },
        "charge-repeated-indices": {
            "diagnosis": "acceptance corpus gap, not a no-op",
            "reasoning": "The patch charges T by index occurrences (Idx) instead of distinct POSITION vertices. "
                         "Units change only where Idx != T. In work-base-meshopt (and all work-*-at-sum / "
                         "sum-minus-1 copies) indices are [0,1,2] over 3 vertices, so Idx == T and the patch is "
                         "inert there. In repeated-index-three-payloads, T rises from 3 to 3,999,990 per payload "
                         "(vertex units about 200M each, about 600M total), which is still under the default "
                         "containmentWorkUnits 2^32, so the base stays Contained. The units-minus-1 copy is already "
                         "workLimit. The validate unit suite also passes: its work-caps test uses non-indexed "
                         "POINTS (T == Idx == 1).",
            "missingFixture": "repeated-index-three-payloads with containmentWorkUnits set to exactly the archive "
                              "sum (24000420): expected Contained; this patch gives workLimit. Alternatively, "
                              "a work-*-at-sum variant whose payload repeats indices.",
        },
    },
}
dst = os.path.join(M.WT, M.ACC, "report.json")
open(dst, "w").write(json.dumps(report, indent=1) + "\n")
for r in rows:
    print(r["name"], r.get("flipped"), r.get("pairedFixtureFlipped"), r.get("oracleFlipped"), r.get("status"))
