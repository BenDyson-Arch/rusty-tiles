#!/usr/bin/env python3
"""Run the oracle over the producer corpus handed over by R and write
corpus/manifest.json.

    python3 -I corpus/run.py CORPUS_DIR

CORPUS_DIR holds `manifest.json` (R's run manifest) and `artifacts/`. Every
artifact is checked against the sha256 in R's manifest before it is read. The
artifacts are data: nothing in CORPUS_DIR is executed or imported.
"""

import hashlib
import json
import os
import sys
from fractions import Fraction

HERE = os.path.dirname(os.path.abspath(__file__))
TOP = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(TOP, "oracle"))

from c2o.check import Oracle  # noqa: E402
from c2o.gltf import PayloadError  # noqa: E402
from c2o.numbers import f32_exact  # noqa: E402
from c2o.scene import quaternion_excess  # noqa: E402

PRODUCER_MANIFEST_SHA256 = "28e4bee6c457de66580b5fb0d125d77d062aca7aba2df34a4cba0940be0d37e4"

# producer-inventory.md (sha256 e391734c...) "v2 slice-1 coverage prediction" rows.
PREDICTIONS = {
    "pointcloud-implicit": ("Point cloud, default (implicit)", "NotInspected(implicitTiling)"),
    "pointcloud-explicit": ("Point cloud --explicit", "Contained; possible Outside on 1-ulp edge cases"),
    "vector-*-implicit": ("Vector, default (implicit)", "NotInspected(implicitTiling)"),
    "vector-*-explicit": ("Vector --explicit (plain, --quantize, --meshopt)",
                          "Contained, or Outside on 1-ulp edge cases"),
    "mesh-local-split*": ("F1c2/F1d mesh", "Contained"),
    "w1-*": ("W1 local / Wgs84", "Contained; NotInspected(numericSpan) beyond about 16 generic box levels"),
    "legacy-mesh-explicit": ("Legacy mesh-to-3tz, explicit", "Contained, or Outside on mixed magnitudes"),
    "legacy-mesh-small-explicit": ("Legacy small-model wrap", "Outside possible; NotInspected for unlisted "
                                   "source extensions, ambiguous scenes, skins and animation"),
    "legacy-mesh-*implicit": ("Legacy mesh-to-3tz, implicit", "NotInspected(implicitTiling)"),
    "convert-implicit-*": ("convert-to-implicit", "NotInspected(implicitTiling)"),
}


def prediction_for(name):
    import fnmatch
    for pattern, value in PREDICTIONS.items():
        if fnmatch.fnmatch(name, pattern):
            return value
    return (None, None)


def disposition(name, outcome, reasons):
    row, predicted = prediction_for(name)
    if predicted is None:
        return "no prediction"
    label = outcome + ("(%s)" % ",".join(reasons) if reasons else "")
    if predicted.startswith(label) or (outcome == "Contained" and predicted.startswith("Contained")):
        if name.startswith("w1-deep20"):
            return ("confirmed Contained; corrected: depth 20 generic rotated nodes stay within the "
                    "4096-bit cap (see spans), so 'numericSpan beyond about 16 box levels' does not hold "
                    "for this chain")
        return "confirmed"
    if outcome == "Contained" and "Contained" in predicted:
        return "confirmed (Contained; the predicted Outside/NotInspected alternative did not occur)"
    if outcome == "Contained" and "possible" in predicted:
        return "consistent: Contained (the predicted possible Outside did not occur for this input)"
    return "corrected: oracle reports %s" % label


def meshopt_minmax_check(oracle):
    """Decoded POSITION extrema equal the declared min/max (evidence for the
    attribute-tail baseline reading). Returns (accessors checked, mismatches)."""
    checked = mismatched = 0
    for facts in oracle.uri_facts.values():
        p = facts.payload
        views = p.doc.get("bufferViews", [])
        for ai, acc in enumerate(p.doc.get("accessors", [])):
            view = acc.get("bufferView")
            if view is None or "min" not in acc or acc.get("type") != "VEC3":
                continue
            if "EXT_meshopt_compression" not in (views[view].get("extensions") or {}):
                continue
            values = p.read_components(ai)
            for j in range(3):
                if acc["componentType"] == 5126:
                    col = [f32_exact(v[j]) for v in values]
                    lo, hi = Fraction(acc["min"][j]), Fraction(acc["max"][j])
                else:
                    col = [v[j] for v in values]
                    lo, hi = acc["min"][j], acc["max"][j]
                checked += 1
                if min(col) != lo or max(col) != hi:
                    mismatched += 1
    return checked, mismatched


def ulp(x):
    import math
    x = abs(float(x))
    return math.ulp(x) if x else 5e-324


def tightest_margin(oracle):
    """Informative: the smallest exact slack of any drawn vertex against any
    tested box face, absolute and in ulps of that face's binary64 position."""
    refs = oracle.traversal.references
    best = None
    for entry in oracle.trace():
        ref = refs[entry["reference"]]
        for level, coord in enumerate(entry["levels"]):
            for lv in ref.levels[level].volumes:
                vol = lv.volume
                if vol.kind != "box":
                    continue
                for j in range(3):
                    slack = vol.h[j] - abs(coord[j] - vol.c[j])
                    face = vol.c[j] + vol.h[j] if coord[j] >= vol.c[j] else vol.c[j] - vol.h[j]
                    in_ulps = float(slack / Fraction(ulp(face)))
                    if best is None or in_ulps < best["ulps"]:
                        best = {"ulps": in_ulps, "slack": float(slack), "level": level, "role": lv.role,
                                "row": j, "reference": entry["reference"], "vertex": entry["vertex"]}
    return best


def main(argv):
    if len(argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    corpus = argv[1]
    with open(os.path.join(corpus, "manifest.json"), "rb") as fh:
        raw = fh.read()
    if hashlib.sha256(raw).hexdigest() != PRODUCER_MANIFEST_SHA256:
        raise SystemExit("producer manifest does not match its pin")
    producer = json.loads(raw)
    rows = []
    for run in producer["runs"]:
        row = {"name": run["name"], "argv": run["argv"][2:], "producerExit": run["exit"]}
        mode = "explicit" if "--explicit" in run["argv"] else "implicit"
        if run["argv"][2] in ("glb-to-3tz", "mesh-local-to-3tz"):
            mode = "explicit (producer has no implicit mode)"
        row["mode"] = mode
        row["inventoryRow"], row["inventoryPrediction"] = prediction_for(run["name"])
        if "artifact" not in run or run["exit"] != 0:
            row["oracle"] = "no artifact (producer refused, exit %d)" % run["exit"]
            rows.append(row)
            continue
        path = os.path.join(corpus, run["artifact"])
        with open(path, "rb") as fh:
            digest = hashlib.sha256(fh.read()).hexdigest()
        if digest != run["sha256"]:
            raise SystemExit("artifact %s does not match R's manifest" % run["artifact"])
        oracle = Oracle(path)
        report = oracle.run()
        row.update({"artifact": run["artifact"], "sha256": digest, "bytes": run["bytes"],
                    "outcome": report["outcome"]})
        for key in ("reasons", "facts", "why"):
            if report.get(key) is not None:
                row[key] = report[key]
        if report.get("c1Caveats"):
            row["c1Caveats"] = report["c1Caveats"]
        if report.get("caveats"):
            row["caveats"] = report["caveats"][:3] + (["..."] if len(report["caveats"]) > 3 else [])
        refs = (report.get("precheck") or {}).get("references") or []
        if refs:
            row["references"] = len(refs)
            row["maxSpan"] = max(max([r.get("sInst", 0)] + r.get("levelSpans", [0])) for r in refs)
            row["maxLevels"] = max(r["levels"] for r in refs)
            row["precheckTotals"] = report["precheck"]["totals"]
        payloads = report.get("payloads") or []
        row["payloadKinds"] = sorted({p["kind"] for p in payloads})
        row["maxNodeDepth"] = max([p["maxNodeDepth"] for p in payloads] or [0])
        excess = []
        for facts in getattr(oracle, "uri_facts", {}).values():
            for node in facts.payload.doc.get("nodes", []):
                if "rotation" in node:
                    excess.append(quaternion_excess(node["rotation"]))
        if excess:
            row["maxQuaternionNormSqExcess"] = float(max(excess))
        if report.get("observations"):
            obs = report["observations"]
            row["polygonAllVertices"] = {
                "primitives": len(obs), "vertices": sum(o["vertices"] for o in obs),
                "outsideAnyVolume": sum(o["outsideAnyVolume"] for o in obs)}
        try:
            checked, mismatched = meshopt_minmax_check(oracle)
        except (PayloadError, AttributeError):
            checked, mismatched = 0, 0
        if checked:
            row["meshoptPositionMinMax"] = {"components": checked, "mismatches": mismatched}
        extensions = set()
        for facts in getattr(oracle, "uri_facts", {}).values():
            extensions.update(facts.payload.doc.get("extensionsUsed", []))
        row["gltfExtensionsUsed"] = sorted(extensions)
        if report["outcome"] == "Contained":
            row["tightestBoxMargin"] = tightest_margin(oracle)
        row["disposition"] = disposition(run["name"], report["outcome"], report.get("reasons") or [])
        rows.append(row)
    out = {
        "record": "C2 O-explicit oracle outcome per producer artifact (R corpus run2).",
        "producerManifest": {"path": "run2/manifest.json", "sha256": PRODUCER_MANIFEST_SHA256,
                             "sourceCommit": producer.get("source_commit"),
                             "binarySha256": producer.get("binary_sha256"),
                             "features": producer.get("features")},
        "inventory": {"file": "/tmp/rusty-tiles-c2-contract-20261011/producer-inventory.md",
                      "sha256": "e391734cc5ef6070528d92d4b913eba1fe27db035095f5cde0f5dcc9dec1e080"},
        "rows": rows,
    }
    with open(os.path.join(HERE, "manifest.json"), "w") as fh:
        json.dump(out, fh, indent=1, sort_keys=True)
    for row in rows:
        print("%-52s %-12s %s" % (row["name"], row.get("outcome", "-"), row.get("disposition", row.get("oracle"))))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
