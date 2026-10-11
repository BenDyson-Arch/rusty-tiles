"""E3: absolute validate timings on a >= 0.4 s forest archive. Per run: wall (monotonic), CPU and peak
RSS (os.wait4), and foreign busy time on the pinned CPUs (/proc/stat busy minus own CPU), as process.py.
python3 -I e3_validate_perf.py OUT.json CPUS REPEAT NAME=BINARY=ARCHIVE ..."""
import hashlib, json, os, subprocess, sys, time
out, cpus, n = sys.argv[1], sys.argv[2], int(sys.argv[3])
cases = [c.split("=", 2) for c in sys.argv[4:]]
pinned = [int(c) for c in cpus.split(",")]
def sha(path):
    # Streamed: on Linux ru_maxrss of an exec'd child includes the parent's high-water mark at
    # spawn, so the parent must never hold an archive in memory.
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()
tick = os.sysconf("SC_CLK_TCK")
def busy():
    total = 0
    for line in open("/proc/stat"):
        p = line.split()
        if p[0][3:].isdigit() and int(p[0][3:]) in pinned:
            v = list(map(int, p[1:])); total += sum(v) - v[3] - v[4]
    return total / tick
res = {"cpus": cpus, "repeat": n, "cases": {name: {"binary": b, "binarySha256": sha(b), "archive": a, "archiveSha256": sha(a), "runs": []} for name, b, a in cases}}
for rnd in range(n):
    for name, b, a in (cases if rnd % 2 == 0 else list(reversed(cases))):
        b0, t = busy(), time.monotonic()
        with open("/home/bend/.cache/forest-acc/tmp/e3.out", "w+b") as fo:
            p = subprocess.Popen(["/usr/bin/taskset", "-c", cpus, "/usr/bin/nice", "-n", "10", b, "--json", "validate", a], stdout=fo, stderr=subprocess.STDOUT, env={"PATH": ""})
            _, status, u = os.wait4(p.pid, 0)
            wall = time.monotonic() - t
            fo.seek(0); d = json.loads(fo.read())
        cpu = u.ru_utime + u.ru_stime
        res["cases"][name]["runs"].append({"exit": os.waitstatus_to_exitcode(status), "ok": d.get("ok"), "tiles": d.get("tiles"),
            "wallSeconds": round(wall, 4), "cpuSeconds": round(cpu, 4), "peakRssMiB": round(u.ru_maxrss / 1024, 2),
            "foreignCpuSeconds": round(max(0.0, busy() - b0 - cpu), 3)})
for name, c in res["cases"].items():
    r = c["runs"]; best = min(r, key=lambda x: x["wallSeconds"])
    c["summary"] = {"bestWallSeconds": best["wallSeconds"], "foreignCpuSecondsAtBestWall": best["foreignCpuSeconds"],
                    "bestCpuSeconds": min(x["cpuSeconds"] for x in r), "peakRssMiB": max(x["peakRssMiB"] for x in r),
                    "medianWallSeconds": sorted(x["wallSeconds"] for x in r)[len(r) // 2], "allOk": all(x["ok"] for x in r)}
    print(name, json.dumps(c["summary"]))
import resource
res["parentPeakRssMiB"] = round(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024, 2)
json.dump(res, open(out, "w"), indent=1)
print("parent peak RSS MiB", res["parentPeakRssMiB"])
