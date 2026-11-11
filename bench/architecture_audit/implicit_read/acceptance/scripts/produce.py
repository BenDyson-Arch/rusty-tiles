"""Run the I1/P acceptance producer corpus with one binary.
python3 -I produce.py BIN LABEL OUTDIR [--only id,...] [--cpus 2,3]"""
import hashlib, json, os, subprocess, sys, time
BIN, LABEL, OUT = sys.argv[1:4]
only = set(sys.argv[sys.argv.index("--only") + 1].split(",")) if "--only" in sys.argv else None
cpus = sys.argv[sys.argv.index("--cpus") + 1] if "--cpus" in sys.argv else "2,3"
I = "/home/bend/.cache/forest-acc/work/inputs"
V = ["--reproducible", "--jobs", "2"]
cases = {
  "W1": ["point-cloud", "-i", f"{I}/cloud.las", "--sourceCrs", "local", "--maxPoints", "1000"],
  "W2": ["point-cloud", "-i", f"{I}/points.las", "--sourceCrs", "EPSG:32756", "--heightOffset", "0", "--maxPoints", "2000"],
  "W3-pc-deep": ["point-cloud", "-i", f"{I}/cloud.las", "--sourceCrs", "local", "--maxPoints", "40"],
  "W4-points-lines": ["vector", "-i", f"{I}/vector.geojson", *V],
  "W4-points-lines-quantize": ["vector", "-i", f"{I}/vector.geojson", *V, "--quantize"],
  "W4-points-lines-meshopt": ["vector", "-i", f"{I}/vector.geojson", *V, "--meshopt"],
  "W4-polygons": ["vector", "-i", f"{I}/countries-source.geojson", *V],
  "W4-polygons-quantize": ["vector", "-i", f"{I}/countries-source.geojson", *V, "--quantize"],
  "W4-polygons-meshopt": ["vector", "-i", f"{I}/countries-source.geojson", *V, "--meshopt"],
  "W5-vec-perf": ["vector", "-i", f"{I}/perf-vector.geojson", *V],
  "W5-vec-perf-quantize": ["vector", "-i", f"{I}/perf-vector.geojson", *V, "--quantize"],
  "W5-vec-perf-meshopt": ["vector", "-i", f"{I}/perf-vector.geojson", *V, "--meshopt"],
  "mf2-lod3": ["vector", "-i", f"{I}/perf-vector.geojson", *V, "--maxFeatures", "2", "--lodLevels", "3"],
  "W6": ["mesh-to-3tz", "-i", f"{I}/split.gltf", "--maxBytes", "0", "--maxTriangles", "512"],
  "W6d": ["mesh-to-3tz", "-i", f"{I}/deep-mesh.glb", "--maxTriangles", "2", "--maxBytes", "0", "--tileSize", "64"],
  "W7": ["mesh-to-3tz", "-i", f"{I}/example.gltf"],
}
glb = {"glb-plain": ["glb-to-3tz", "-i", f"{I}/plain.gltf"], "glb-deep20": ["glb-to-3tz", "-i", f"{I}/deep20.gltf"]}
explicit_ok = set(cases)
jobs = []
for k, a in cases.items():
    jobs.append((k, a)); jobs.append((k + "-explicit", a + ["--explicit"]))
jobs += list(glb.items())
os.makedirs(OUT, exist_ok=True)
env = {"PATH": "", "RAYON_NUM_THREADS": "2", "PROJ_NETWORK": "OFF", "HOME": os.environ.get("HOME", "")}
bsha = hashlib.sha256(open(BIN, "rb").read()).hexdigest()
recs = []
for k, a in jobs:
    if only and k not in only and k.removesuffix("-explicit") not in only:
        continue
    out = os.path.join(OUT, k + ".3tz")
    if os.path.exists(out): os.remove(out)
    argv = ["/usr/bin/taskset", "-c", cpus, "/usr/bin/nice", "-n", "10", BIN, "--json", *a, "-o", out]
    t = time.monotonic(); p = subprocess.run(argv, capture_output=True, text=True, env=env); w = time.monotonic() - t
    r = {"id": k, "argv": [BIN] + ["--json", *a, "-o", out], "exit": p.returncode, "wallSeconds": round(w, 3)}
    if p.returncode == 0 and os.path.exists(out):
        r["sha256"] = hashlib.sha256(open(out, "rb").read()).hexdigest(); r["bytes"] = os.path.getsize(out)
    else:
        r["stderr"] = p.stderr[-600:]
    recs.append(r); print(json.dumps({x: r.get(x) for x in ("id", "exit", "bytes", "wallSeconds", "stderr")}), flush=True)
json.dump({"label": LABEL, "binary": BIN, "binarySha256": bsha, "inputs": open(f"{I}/SHA256SUMS").read(), "runs": recs},
          open(os.path.join(OUT, "produce.json"), "w"), indent=1)
