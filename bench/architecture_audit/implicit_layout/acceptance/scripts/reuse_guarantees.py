"""Check the reuse guarantees on the head binary: unchanged-input reuse equals its base, and the perf-default
small edit's reuse output equals a fresh build, both excluding conversion.json, geometry-reports.jsonl and the
3tz index (the PV test's volatile set). Also records reuse counters.
python3 -I reuse_guarantees.py OUT.json WORK CPUS"""
import hashlib, json, os, subprocess, sys, zipfile
B = "/home/bend/.cache/forest-acc/bin/rusty-tiles-head-native"
out, work, cpus = sys.argv[1], sys.argv[2], sys.argv[3]
VOL = {"conversion.json", "geometry-reports.jsonl", "@3dtilesIndex1@"}
sha = lambda p: hashlib.sha256(open(p, "rb").read()).hexdigest()
def conv(inp, o, opts, prev=None):
    if os.path.exists(o): os.remove(o)
    cmd = ["/usr/bin/taskset", "-c", cpus, "/usr/bin/nice", "-n", "10", B, "--json", "vector", "-i", inp, "-o", o, "--reproducible", "--jobs", "2", *opts]
    if prev: cmd += ["--reuse-tileset", prev]
    p = subprocess.run(cmd, capture_output=True, text=True, env={"PATH": "", "RAYON_NUM_THREADS": "2"})
    assert p.returncode == 0, p.stderr[-500:]
    return json.loads(p.stdout).get("reuse")
def members(p):
    z = zipfile.ZipFile(p); return {n: hashlib.sha256(z.read(n)).hexdigest() for n in z.namelist() if n not in VOL}
def diff(a, b):
    ma, mb = members(a), members(b); return sorted(n for n in set(ma) | set(mb) if ma.get(n) != mb.get(n))
base_in, edit_in = f"{work}/base/vector.geojson", f"{work}/small-edit/vector.geojson"
res = {"binarySha256": sha(B), "script": sha(__file__), "inputs": {"base": sha(base_in), "smallEdit": sha(edit_in)}, "cases": []}
for cfg, opts in [("perf-default", []), ("mf2-lod3", ["--maxFeatures", "2", "--lodLevels", "3"])]:
    base = f"{work}/g-{cfg}-base.3tz"; conv(base_in, base, opts)
    same = f"{work}/g-{cfg}-same.3tz"; r_same = conv(base_in, same, opts, base)
    reused = f"{work}/g-{cfg}-reused.3tz"; r_edit = conv(edit_in, reused, opts, base)
    fresh = f"{work}/g-{cfg}-fresh.3tz"; conv(edit_in, fresh, opts)
    case = {"config": cfg, "unchanged": {"reuse": r_same, "differingMembers": diff(same, base)},
            "smallEdit": {"reuse": r_edit, "differingMembers": len(diff(reused, fresh)), "firstDiffering": diff(reused, fresh)[:8]}}
    res["cases"].append(case)
    print(cfg, "unchanged diff", len(case["unchanged"]["differingMembers"]), {k: r_same[k] for k in ("reusedContents", "rebuiltContents", "publishedContents")},
          "| small-edit diff", case["smallEdit"]["differingMembers"], {k: r_edit[k] for k in ("reusedContents", "rebuiltContents", "publishedContents")}, flush=True)
json.dump(res, open(out, "w"), indent=1)
