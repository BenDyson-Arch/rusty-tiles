#!/usr/bin/env python3
"""Build the C2-WB gate cases: each wb/large fixture (O-WB units from wb/manifest.json) and each
b51f83d explicit producer archive (O-WB oracle units), with its gltfUpAxis = "Z" twin.

    python3 -I cases.py REPO OUTDIR
"""
import importlib.util
import json
import os
import sys
import zipfile

spec = importlib.util.spec_from_file_location("wbgen", "/tmp/rusty-tiles-c2wb-contract-20261012/probes/wbgen.py")
G = importlib.util.module_from_spec(spec)
spec.loader.exec_module(G)
FOREST = "/home/bend/.cache/forest-acc/work/head-b51f83d"
UNITS = "/home/bend/.cache/c2wb-int/producer-units"
PRODUCER = ["W1-explicit", "W2-explicit", "W3-pc-deep-explicit", "W5-vec-perf-explicit",
            "W5-vec-perf-quantize-explicit", "W5-vec-perf-meshopt-explicit", "W6d-explicit",
            "W6d-nomeshopt-explicit", "mf2-lod3-explicit"]


def twin(src, dst):
    with zipfile.ZipFile(src) as z:
        m = {i.filename: z.read(i) for i in z.infolist() if i.filename != G.INDEX}
    G.write_3tz(G.twin(m), dst)


def main(repo, out):
    os.makedirs(out, exist_ok=True)
    root = os.path.join(repo, "bench/architecture_audit/c2_decoded_containment")
    manifest = json.load(open(os.path.join(root, "wb/manifest.json")))
    cases = []
    for row in manifest["fixtures"]:
        if row["size"] != "large":
            continue
        a = os.path.join(root, row["file"])
        b = os.path.join(out, row["id"] + "-Z.3tz")
        twin(a, b)
        cases.append({"name": "wb:" + row["id"], "matrix": row["matrix"], "a": a, "b": b, "units": row["units"],
                      "expectA": "contained", "expectB": "upAxisOverride"})
    for name in PRODUCER:
        a = os.path.join(FOREST, name + ".3tz")
        rep = json.load(open(os.path.join(UNITS, name + ".json")))
        units = rep["precheck"]["totals"]["units"]
        b = os.path.join(out, name + "-Z.3tz")
        twin(a, b)
        cases.append({"name": "producer:" + name, "matrix": "producer twin (b51f83d)", "a": a, "b": b,
                      "units": units, "oracleOutcome": rep["outcome"], "expectA": "contained",
                      "expectB": "upAxisOverride"})
    with open(os.path.join(out, "cases.json"), "w") as fh:
        json.dump(cases, fh, indent=1)
    print(len(cases), "cases")


if __name__ == "__main__":
    main(*sys.argv[1:3])
