#!/usr/bin/env python3
"""Run the oracle on the coordinator-supplied producer artifacts listed in manifest.json.

    python3 -I run.py [--only ID ...]

Each artifact is verified against its pinned sha256 before it is read; pending or missing artifacts are reported,
never guessed.  Writes results.json (verdicts, counts, tile-fact digests) and facts/<id>.json (the full canonical
tile list for Accepted artifacts, for parity with production's implicit_tile_facts seam).
"""
import hashlib
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ORACLE = os.path.join(os.path.dirname(HERE), "oracle", "i1_oracle.py")


def main(argv):
    only = set(argv[argv.index("--only") + 1:]) if "--only" in argv else None
    manifest = json.load(open(os.path.join(HERE, "manifest.json")))
    os.makedirs(os.path.join(HERE, "facts"), exist_ok=True)
    out, bad = [], 0
    for e in manifest["artifacts"]:
        if only and e["id"] not in only:
            continue
        if e.get("status") == "pending":
            out.append({"id": e["id"], "status": "pending"})
            continue
        if not os.path.exists(e["path"]):
            out.append({"id": e["id"], "status": "missing"})
            continue
        if hashlib.sha256(open(e["path"], "rb").read()).hexdigest() != e["sha256"]:
            out.append({"id": e["id"], "status": "hash-mismatch"})
            bad += 1
            continue
        cmd = [sys.executable, "-I", ORACLE]
        for k, v in sorted(e.get("limits", {}).items()):
            cmd += ["--limit", f"{k}={v}"]
        rec = json.loads(subprocess.run(cmd + [e["path"]], check=True, capture_output=True, text=True).stdout)
        v = rec["verdict"]
        got = v["kind"] + (f":{v['limit']}" if v["kind"] == "ResourceLimit" else "")
        agree = got == e["expected"]
        bad += not agree
        if rec.get("tiles") is not None and not e.get("limits"):
            with open(os.path.join(HERE, "facts", e["id"] + ".json"), "w") as f:
                json.dump({"archiveSha256": rec["archiveSha256"], "tilesSha256": rec["tilesSha256"],
                           "tiles": rec["tiles"]}, f, sort_keys=True, separators=(",", ":"))
                f.write("\n")
        out.append({"id": e["id"], "expected": e["expected"], "oracle": got, "agree": agree, "reason": v["reason"],
                    "stage": v["stage"], "counts": rec["counts"], "tilesSha256": rec.get("tilesSha256"),
                    "unreferencedMembers": rec.get("unreferencedMembers")})
    if not only:
        with open(os.path.join(HERE, "results.json"), "w") as f:
            json.dump({"disagreements": bad, "results": out}, f, indent=1, sort_keys=True)
            f.write("\n")
    for r in out:
        print(json.dumps({k: r.get(k) for k in ("id", "status", "expected", "oracle", "agree")}))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
