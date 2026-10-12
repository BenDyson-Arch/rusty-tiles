#!/usr/bin/env python3
"""C2 slice 3: oracle outcomes and precheck totals of R's forest witnesses (contract 9.B), and the 9.D limit pairs.

    python3 -I corpus/slice3.py CORPUS_DIR [--only ID ...] [--no-pairs]

CORPUS_DIR holds R's producer archives (`W1.3tz`, `mf2-lod3.3tz`, ... and the `-explicit` twins). Each archive
is checked against its sha256 in WITNESSES before it is read. Writes `corpus/slice3-manifest.json`:

- per witness: outcome, reasons, Outside facts, the precheck totals and terms, the maximum span, the
  implicit counts and the oracle wall time (informative);
- the slice-3 coverage statement: which witnesses are `workLimit` at the default cap 2^29;
- the 9.D limit pairs, each value taken from these totals and confirmed by running the oracle at the value
  (expected Contained, or PrecheckPassed for the decoded-bytes pair) and at one less (expected NotInspected
  with the named reason). The precheck-only arm (`containmentBytesRead` = total - 1 on F-MF2) must stop at the
  last reference.
"""

import hashlib
import json
import os
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
TOP = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(TOP, "oracle"))
from c2o.implicit import SLICE3_LIMITS, ImplicitOracle  # noqa: E402

# Contract 9.B rows, b51f83d producer (probes/validate-corpus-b51f83d.json lists the same hashes).
WITNESSES = {
    "F-W1": ("W1", "572e664409802600da9827fb508e31fbb99d3517f37ae6032616135f693e343d"),
    "F-W2": ("W2", "29ac8851a677eb948dae6f75cb68d06219299f7d15c002f96116e34d23af8ae9"),
    "F-W3": ("W3-pc-deep", "3628339df5c680f56d045436e8133f157d804401d0c77b2d2fcd46cf3691a007"),
    "F-W4-points-lines": ("W4-points-lines", "f0d63396fa2a50c7072660bfe06391ed3fd3b2037b30beb5e8537dbdd960247d"),
    "F-W4-points-lines-quantize": ("W4-points-lines-quantize",
                                   "406e9eb91b160cb773ee4edde9aa1dd44a9d67a40e7641e40a9830d3686364b4"),
    "F-W4-points-lines-meshopt": ("W4-points-lines-meshopt",
                                  "5f5cfdfa199c748d581cb7411443489b4a3e4999715b55704b69d3d035891a0a"),
    "F-W4-polygons": ("W4-polygons", "d36e732390c0e4b30eda5cd0c84d7a6ef88de38e2d4f57c46b201b6880fa7244"),
    "F-W4-polygons-quantize": ("W4-polygons-quantize",
                               "55765e222eafd3fa651212a14ade5d5450adca3e77f0984fe3931ced7187c08b"),
    "F-W4-polygons-meshopt": ("W4-polygons-meshopt",
                              "91fed55b63913605c6c49423671da58fa0a1075888db499778de4130ec7fa69d"),
    "F-W5": ("W5-vec-perf", "57e2e66451bc77c8634941997bc5246ee339df88fafc2d08cb227512d1c112f7"),
    "F-W5-quantize": ("W5-vec-perf-quantize", "a3b4a32fcb6a55e0238a03c3fbf206d7d7a1e85a4457a56163822bfdd2cea81a"),
    "F-W5-meshopt": ("W5-vec-perf-meshopt", "07be93719b17fc3885e2fc26fb74d66954763fb082256813e5edab42ffca93fb"),
    "F-MF2": ("mf2-lod3", "41cd3a418dd901a530295c572aaa45a96d3e721b321e8e6c8cdcba164af5be4a"),
    "F-W6": ("W6", "751e6fbbe86cb18b1f7c71135edc72b307db55bb054aae93945de478f34187dd"),
    "F-W6d": ("W6d", "9af81d4242850b5dd3c4601d596a69cf60b6f36592e8b04030483d64f9d1cbe0"),
    "F-W6dn": ("W6d-nomeshopt", "b701588a3ad72929c93f5beed2de4d0b4811756757bcf0baa419fb738de16283"),
    "F-W7": ("W7", "1f8ec4df9356557b8663dfa5a085c77f290c4dbb876cfa0181f57a20ecd99ffe"),
    # --explicit twins (9.B "Twins")
    "T-W1": ("W1-explicit", "828e1010b881074ab3916577fef0c35d93d24b54da7764896ac0986388ee4b38"),
    "T-W2": ("W2-explicit", "20f20d5f1aaa9fe402320fc8a9edcb4e9d259f8767ecbbac9eeefe1508f7b5e5"),
    "T-W3": ("W3-pc-deep-explicit", "c985e3b5b9eef063fea4b2f9012ab4b06e8adef8bc099826ac44614db84d59a1"),
    "T-W4-points-lines": ("W4-points-lines-explicit",
                          "201854d13f826f587727bd1913fed27ec50ade1ca4a1b60ff16891be1fbeee22"),
    "T-W4-points-lines-quantize": ("W4-points-lines-quantize-explicit",
                                   "bdf02cebf88967aa3c18f4b01207c64a554871382084a567e7bc04f011eba38a"),
    "T-W4-points-lines-meshopt": ("W4-points-lines-meshopt-explicit",
                                  "782316fcec8b0447ca3b0389529990f1203252707fdca13bcde46f94cc62351d"),
    "T-W4-polygons": ("W4-polygons-explicit", "e0e08844b4d570bfdc6bf21f3b9ef0e5ac70345c0df6482830504b3f8272f0eb"),
    "T-W4-polygons-quantize": ("W4-polygons-quantize-explicit",
                               "14c7dcec40eb29666e6a09716229a1dbbd2e8413de345c018e5678cb4497e359"),
    "T-W4-polygons-meshopt": ("W4-polygons-meshopt-explicit",
                              "b0f935ca4878de52c586f14ce7f8c53f5edb396ab7b9ebad305727e62047f61d"),
    "T-W5": ("W5-vec-perf-explicit", "307ca4b2c2439dd60b3f2506669343dc0946f40d2870c986212da98367f73f11"),
    "T-W5-quantize": ("W5-vec-perf-quantize-explicit",
                      "64f6741dd98c4dd21100fb4f1b22653649f95cbfb00db1269275232357a1dfc0"),
    "T-W5-meshopt": ("W5-vec-perf-meshopt-explicit",
                     "dbbe891ac474ea1fa8ec7fa250a49a997e651f167f0be74e861d6aa7beec8947"),
    "T-MF2": ("mf2-lod3-explicit", "4881a0eed2d1aabbed48bf97efd99d52ac3c80d2e5bc231bbb4c18f526f9240b"),
    "T-W6": ("W6-explicit", "67d5f6e92e896f9a2dc0b10bf0f525befc7b5a510112a225c0417ad5813a0b41"),
    "T-W6d": ("W6d-explicit", "c2b6bfde9d50d651ee8a63f58cb519a4bbea9b090818441acf739e6a5fbc8193"),
    "T-W6dn": ("W6d-nomeshopt-explicit", "5d9bdd7c0e7707fd30c1ea79e27ee163d92d2940119cfeb61600822fc92c3de3"),
    "T-W7": ("W7-explicit", "51aded3b22301a34af545ab2aa8dcba4f9547357e6af4b4d1aa49b9e77b203f9"),
}

ORACLE_LIMITS = {"maxVertexLevelTests": 4_000_000_000}
REPEATED_URI = os.path.join(TOP, "controls", "slice3", "fixtures", "repeated-uri.base.3tz")


def run(path, limits=None, precheck_only=False):
    t0 = time.monotonic()
    report = ImplicitOracle(path, limits, ORACLE_LIMITS, None, precheck_only=precheck_only, detail=True).run()
    return report, round(time.monotonic() - t0, 3)


def max_span(report):
    best = 0
    for entry in (report.get("precheck") or {}).get("references", []):
        if "sInst" in entry:
            best = max([best, entry["sInst"]] + entry["levelSpans"])
    return best


def row_of(report, wall):
    facts = report.get("facts") or {}
    pre = report.get("precheck") or {}
    refs = pre.get("references", [])
    return {
        "outcome": report["outcome"], "reasons": report.get("reasons"), "why": report.get("why"),
        "phase": report.get("phase"),
        "facts": {k: facts[k] for k in ("volume", "depth", "member", "reference", "implicit", "why", "limit")
                  if k in facts} or None,
        "i1": report.get("i1", {}).get("verdict"),
        "implicit": report.get("implicit"), "references": report.get("references"),
        "totals": pre.get("totals"), "terms": pre.get("terms"), "maxSpan": max_span(report) if refs else None,
        "maxLevels": max((e["levels"] for e in refs), default=None), "oracleWallSeconds": wall,
    }


def pair(path, name, value, reason, at_outcome="Contained", precheck_only=False):
    """Run at `value` and at `value - 1`; return the record and whether both ends are as expected."""
    at, _ = run(path, {name: value}, precheck_only=precheck_only)
    under, _ = run(path, {name: value - 1}, precheck_only=True)
    ok = at["outcome"] == at_outcome and under["outcome"] == "NotInspected" and under.get("reasons") == [reason]
    return {"limit": name, "atValue": value, "atOutcome": at["outcome"], "underValue": value - 1,
            "underOutcome": under["outcome"], "underReasons": under.get("reasons"),
            "underStop": under.get("precheckStop"), "references": under.get("references"), "pass": ok}


def main(argv):
    if not argv or argv[0].startswith("--"):
        print(__doc__, file=sys.stderr)
        return 2
    corpus = argv[0]
    only = set(argv[argv.index("--only") + 1:]) if "--only" in argv else None
    out_path = os.path.join(HERE, "slice3-manifest.json")
    old = json.load(open(out_path)) if os.path.exists(out_path) else {}
    rows = dict(old.get("witnesses", {}))
    for wid, (name, digest) in WITNESSES.items():
        if only and wid not in only:
            continue
        path = os.path.join(corpus, name + ".3tz")
        if not os.path.exists(path):
            rows[wid] = {"archive": name + ".3tz", "sha256": digest, "status": "missing"}
            continue
        if hashlib.sha256(open(path, "rb").read()).hexdigest() != digest:
            raise SystemExit("%s: sha256 differs from the pinned value" % path)
        report, wall = run(path)
        rows[wid] = dict(row_of(report, wall), archive=name + ".3tz", sha256=digest)
        print(wid, rows[wid]["outcome"], (rows[wid]["totals"] or {}).get("units"), wall, flush=True)
    cap = SLICE3_LIMITS["containmentWorkUnits"]
    coverage = {wid: ("workLimit" if (r.get("totals") or {}).get("units", 0) > cap else r.get("outcome"))
                for wid, r in rows.items() if wid.startswith("F-")}
    result = {"generator": "corpus/slice3.py", "producer": "b51f83d (R's forest corpus)",
              "defaultLimits": SLICE3_LIMITS, "witnesses": dict(sorted(rows.items())),
              "coverageAtDefaultCap": {"cap": cap, "workLimit": sorted(k for k, v in coverage.items()
                                                                       if v == "workLimit"),
                                       "byWitness": dict(sorted(coverage.items()))},
              "pairs": old.get("pairs", {})}
    if "--no-pairs" not in argv and (not only or "F-MF2" in only):
        mf2 = rows["F-MF2"]
        path = os.path.join(corpus, WITNESSES["F-MF2"][0] + ".3tz")
        t = mf2["totals"]
        result["pairs"]["F-MF2.containmentWorkUnits"] = pair(path, "containmentWorkUnits", t["units"], "workLimit")
        result["pairs"]["F-MF2.containmentSpanBits"] = pair(path, "containmentSpanBits", mf2["maxSpan"],
                                                            "numericSpan", at_outcome="PrecheckPassed",
                                                            precheck_only=True)
        arm = pair(path, "containmentBytesRead", t["bytes"], "workLimit", at_outcome="PrecheckPassed",
                   precheck_only=True)
        arm["stopsAtLastReference"] = (arm["underStop"] or {}).get("reference") == mf2["references"] - 1
        arm["pass"] = arm["pass"] and arm["stopsAtLastReference"]
        result["pairs"]["F-MF2.containmentBytesRead (precheck-only arm)"] = arm
    if "--no-pairs" not in argv and (not only or "F-W6d" in only):
        w6d = rows["F-W6d"]
        path = os.path.join(corpus, WITNESSES["F-W6d"][0] + ".3tz")
        result["pairs"]["F-W6d.containmentDecodedBytes"] = pair(path, "containmentDecodedBytes",
                                                                w6d["totals"]["decoded"], "workLimit")
    if "--no-pairs" not in argv and not only:
        rep, _ = run(REPEATED_URI)
        units = rep["precheck"]["totals"]["units"]
        p = pair(REPEATED_URI, "containmentWorkUnits", units, "workLimit")
        p["fixture"] = "controls/slice3/fixtures/repeated-uri.base.3tz"
        p["sha256"] = hashlib.sha256(open(REPEATED_URI, "rb").read()).hexdigest()
        p["perReference"] = [{"member": e["member"], "units": e["units"]} for e in rep["precheck"]["references"]]
        result["pairs"]["repeated-uri.containmentWorkUnits"] = p
    with open(out_path, "w") as fh:
        json.dump(result, fh, indent=1, sort_keys=True)
        fh.write("\n")
    fails = [k for k, v in result["pairs"].items() if not v["pass"]]
    print(json.dumps({"witnesses": len(rows), "pairs": len(result["pairs"]), "failedPairs": fails,
                      "workLimitAtDefaultCap": result["coverageAtDefaultCap"]["workLimit"]}))
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
