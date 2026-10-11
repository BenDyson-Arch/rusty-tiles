"""C2 corpus parity (coordinator R): production `validate --json` vs oracle corpus/manifest.json.
Usage: python3 -I corpus_parity.py BINARY CORPUS_RUN_DIR ORACLE_MANIFEST OUT_JSON"""
import json, subprocess, sys, os, hashlib
BIN, RUN, MAN, OUT = sys.argv[1:5]
rows = json.load(open(MAN))["rows"]; results = []
for r in rows:
    if not r.get("artifact"):
        results.append({"name": r["name"], "status": "no-artifact", "oracle": r.get("outcome")}); continue
    path = os.path.join(RUN, r["artifact"])
    assert hashlib.sha256(open(path, "rb").read()).hexdigest() == r["sha256"], r["name"]
    p = subprocess.run([BIN, "validate", path, "--json"], capture_output=True, text=True)
    doc = json.loads(p.stdout)
    rep = doc.get("report") or doc.get("validation") or doc
    checks = rep.get("checks") or []
    if p.returncode == 0 and "decodedContentContainment" in checks:
        got = {"outcome": "Contained"}
    elif p.returncode == 0:
        got = {"outcome": "NotInspected", "reasons": sorted({n.split(".", 1)[1] for n in rep.get("notInspected", []) if n.startswith("decodedContentBounds.")})}
    else:
        got = {"outcome": "error", "exit": p.returncode, "message": json.dumps(doc.get("error"))[:300]}
    want = {"outcome": r["outcome"], **({"reasons": sorted(r["reasons"])} if r.get("reasons") else {})}
    ok = got == want if want["outcome"] != "NotInspected" else (got.get("outcome") == "NotInspected" and got.get("reasons") == want["reasons"])
    results.append({"name": r["name"], "status": "agree" if ok else "DISAGREE", "oracle": want, "production": got})
summary = {"binary_sha256": hashlib.sha256(open(BIN, "rb").read()).hexdigest(), "agree": sum(x["status"] == "agree" for x in results),
           "disagree": [x for x in results if x["status"] == "DISAGREE"], "noArtifact": [x["name"] for x in results if x["status"] == "no-artifact"], "rows": results}
json.dump(summary, open(OUT, "w"), indent=1)
print(json.dumps({k: (v if not isinstance(v, list) else len(v)) for k, v in summary.items() if k != "rows"}))
for d in summary["disagree"]: print(json.dumps(d))
