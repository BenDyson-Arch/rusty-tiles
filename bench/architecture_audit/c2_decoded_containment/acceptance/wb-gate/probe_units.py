#!/usr/bin/env python3
"""O-WB oracle (c2o) precheck-only units for every probe-matrix A archive; writes gate cases.

    python3 -I probe_units.py PROBE_CASES.json OUT_CASES.json
"""
import json
import os
import subprocess
import sys

ORACLE = "/tmp/rusty-tiles-c2wb/bench/architecture_audit/c2_decoded_containment/oracle/c2_oracle.py"


def main(src, dst):
    cases = json.load(open(src))
    out = []
    for c in cases:
        p = subprocess.run([sys.executable, "-I", ORACLE, c["a"], "--json", "--precheck-only", "--no-detail",
                            "--limit", "containmentWorkUnits=4611686018427387904"], capture_output=True, text=True)
        rep = json.loads(p.stdout)
        units = ((rep.get("precheck") or {}).get("totals") or {}).get("units")
        row = {"name": "probe:" + c["name"], "matrix": "R probe matrix (C2-WB v2 calibration)", "a": c["a"],
               "b": c["b"], "units": units, "oracleOutcome": rep["outcome"], "expectA": "contained",
               "expectB": "upAxisOverride"}
        print(json.dumps({k: row[k] for k in ("name", "units", "oracleOutcome")}), flush=True)
        out.append(row)
    json.dump(out, open(dst, "w"), indent=1)


if __name__ == "__main__":
    main(*sys.argv[1:3])
