"""E2: peak RSS (os.wait4 ru_maxrss) of `validate --json` on R3.L8, R3.L32 and W1, release binary.
python3 -I r3_rss.py BINARY OUT.json CPUS REPEAT"""
import hashlib, json, os, subprocess, sys, time
B, out, cpus, n = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
F = "/tmp/rusty-tiles-forest/bench/architecture_audit/implicit_read/controls/fixtures"
cases = {"W1": "/home/bend/.cache/forest-acc/work/head-native/W1.3tz", "R3.L8": f"{F}/R3.L8.3tz", "R3.L32": f"{F}/R3.L32.3tz"}
sha = lambda p: hashlib.sha256(open(p, "rb").read()).hexdigest()
res = {"binary": B, "binarySha256": sha(B), "cpus": cpus, "cases": {}}
for rnd in range(n):
    for name, path in (cases.items() if rnd % 2 == 0 else reversed(cases.items())):
        t = time.monotonic()
        with open("/home/bend/.cache/forest-acc/tmp/r3.out", "w+b") as fo:
            # taskset execs the binary in place, so wait4 reports the validator's own peak RSS.
            p = subprocess.Popen(["/usr/bin/taskset", "-c", cpus, B, "--json", "validate", path], stdout=fo, stderr=subprocess.STDOUT, env={"PATH": ""})
            _, status, usage = os.wait4(p.pid, 0)
            wall = time.monotonic() - t
            fo.seek(0); d = json.loads(fo.read())
        rec = res["cases"].setdefault(name, {"sha256": sha(path), "runs": []})
        rec["runs"].append({"exit": os.waitstatus_to_exitcode(status), "wall": round(wall, 4), "peakRssKiB": usage.ru_maxrss,
                            "ok": d.get("ok"), "error": (d.get("error") or {}).get("message")})
    print(rnd, flush=True)
for name, rec in res["cases"].items():
    rec["maxPeakRssMiB"] = round(max(r["peakRssKiB"] for r in rec["runs"]) / 1024, 2)
w1 = res["cases"]["W1"]["maxPeakRssMiB"]
res["bound"] = "R3 peak RSS within 16 MiB of W1 (contract §E R3)"
res["deltaToW1MiB"] = {k: round(v["maxPeakRssMiB"] - w1, 2) for k, v in res["cases"].items()}
res["pass"] = all(abs(d) <= 16 for d in res["deltaToW1MiB"].values())
json.dump(res, open(out, "w"), indent=1)
print(json.dumps({k: (v["maxPeakRssMiB"], [r["error"] for r in v["runs"]][:1]) for k, v in res["cases"].items()}), res["deltaToW1MiB"], res["pass"])
