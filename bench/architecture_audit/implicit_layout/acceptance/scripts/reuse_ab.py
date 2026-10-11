"""A-8 reuse timing: develop vs head, mf2-lod3 (--maxFeatures 2 --lodLevels 3), the design's small edit.
Each binary builds its own base archive from base/vector.geojson, then per round runs a fresh build and a
--reuse-tileset build of small-edit/vector.geojson. Binary order is ABBA across rounds; fresh/reuse order
alternates. Wall from a monotonic clock, CPU from os.times() of reaped children. Head reuse output is compared with head fresh member by member.
python3 -I reuse_ab.py OUT.json WORK CPUS ROUNDS"""
import hashlib, json, os, pathlib, subprocess, sys, time, zipfile
BINS = {"develop": "/home/bend/.cache/forest-acc/bin/rusty-tiles-develop-native", "head": "/home/bend/.cache/forest-acc/bin/rusty-tiles-head-native"}
OPTS = ["--reproducible", "--jobs", "2", "--maxFeatures", "2", "--lodLevels", "3"]
out, work, cpus, rounds = sys.argv[1], pathlib.Path(sys.argv[2]), sys.argv[3], int(sys.argv[4])
sha = lambda p: hashlib.sha256(open(p, "rb").read()).hexdigest()
def run(binary, args):
    cmd = ["/usr/bin/taskset", "-c", cpus, "/usr/bin/nice", "-n", "10", BINS[binary], "--json", "vector"] + args + OPTS
    t = time.monotonic(); p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env={"RAYON_NUM_THREADS": "2", "PATH": ""})
    o, e = p.communicate()
    wall = time.monotonic() - t
    return wall, p.returncode, o, e
def timed(binary, args):
    before = os.times(); wall, rc, o, e = run(binary, args); after = os.times()
    rec = {"binary": binary, "wall": round(wall, 3), "cpu": round(after.children_user - before.children_user + after.children_system - before.children_system, 3), "rc": rc}
    try:
        j = json.loads(o); rec["reuse"] = j.get("reuse")
    except ValueError:
        rec["stderr"] = e.decode()[-600:]
    return rec
def members(p):
    z = zipfile.ZipFile(p); return {n: hashlib.sha256(z.read(n)).hexdigest() for n in z.namelist()}
res = {"binaries": {k: sha(v) for k, v in BINS.items()}, "script": sha(__file__), "cpus": cpus,
       "inputs": {"base": sha(work / "base/vector.geojson"), "smallEdit": sha(work / "small-edit/vector.geojson")}, "runs": []}
for b in BINS:
    prev = work / f"prev-{b}.3tz"; prev.unlink(missing_ok=True)
    r = timed(b, ["-i", str(work / "base/vector.geojson"), "-o", str(prev)]); r["kind"] = "base"; res["runs"].append(r); print(json.dumps(r), flush=True)
keep = {}
for rnd in range(rounds):
    order = ["develop", "head"] if rnd % 2 == 0 else ["head", "develop"]
    for b in order:
        kinds = ["fresh", "reuse"] if rnd % 2 == 0 else ["reuse", "fresh"]
        for kind in kinds:
            target = work / f"{b}-{kind}-{rnd}.3tz"; target.unlink(missing_ok=True)
            args = ["-i", str(work / "small-edit/vector.geojson"), "-o", str(target)]
            if kind == "reuse": args += ["--reuse-tileset", str(work / f"prev-{b}.3tz")]
            r = timed(b, args); r.update(kind=kind, round=rnd, archiveBytes=target.stat().st_size if target.exists() else None)
            if target.exists(): r["sha256"] = sha(target)
            res["runs"].append(r); print(json.dumps({k: v for k, v in r.items() if k != "reuse"} | {"reuse": r.get("reuse")}), flush=True)
            if rnd == 0: keep[(b, kind)] = target
            else: target.unlink(missing_ok=True)
for b in BINS:
    f, u = members(keep[(b, "fresh")]), members(keep[(b, "reuse")])
    diff = sorted(n for n in set(f) | set(u) if f.get(n) != u.get(n))
    res[f"{b}ReuseVsFresh"] = {"differingMembers": diff, "unreferencedT": [n for n in u if n.startswith("t/")][:5]}
json.dump(res, open(out, "w"), indent=1)
