#!/usr/bin/env python3
"""C2-WB section 9.D performance gate: production `validate --json`, archive A against its
excluded twin B (asset.gltfUpAxis = "Z"), on P-cores or E-cores.

Run rules (contract draft v2.1, 9.D):
* A run counts only if, over its duration, the pinned CPU's foreign busy share is <= 5% and the
  contention neighbour is <= 10% busy (P-core: the SMT sibling; E-core: the busiest other E-core of
  the shared-L2 cluster 16-19). Busy time is read from /proc/schedstat (rq_cpu_time, ns) and the
  child's own CPU (wait4 rusage) is subtracted on the pinned CPU.
* A pair is one A run then one B run, both clean. Each case needs K = 7 clean pairs within 30
  attempts; the case figure is the median of the 7 clean (A - B) CPU differences, minimum beside it.
  Fewer than 7 clean pairs: the case is "undecided".
* Core choice: before a case (and after 3 consecutive dirty attempts) busy time is sampled for 2 s;
  P: the SMT pair (2k, 2k+1), k = 0..7, with the lowest max busy share, child pinned to the idler
  sibling; E: the idlest of 16-19. Child runs at nice 10. This harness pins itself to CPU HOUSE.

    python3 -I gate.py BINARY CASES.json OUT.jsonl {P|E}
CASES.json: [{"name", "a", "b", "units", "expectA": "contained", "expectB": "upAxisOverride"}]
"""
import json
import os
import statistics
import subprocess
import sys
import time

K, MAX_ATTEMPTS = 7, 30
OWN_MAX, NEIGHBOUR_MAX = 0.05, 0.10
PAIRS = [(2 * k, 2 * k + 1) for k in range(8)]
ECORES = [16, 17, 18, 19]


def rq():
    out = {}
    with open("/proc/schedstat") as fh:
        for line in fh:
            if line.startswith("cpu"):
                f = line.split()
                out[int(f[0][3:])] = int(f[7])
    return out


def pick_once(core_type):
    a, t0 = rq(), time.perf_counter()
    time.sleep(2.0)
    b, dt = rq(), time.perf_counter() - t0
    load = {c: (b[c] - a[c]) / 1e9 / dt for c in a}
    if core_type == "E":
        cpu = min(ECORES, key=lambda c: load[c])
    else:
        house = int(os.environ.get("HOUSE", "1"))
        pair = min((p for p in PAIRS if house not in p), key=lambda p: max(load[p[0]], load[p[1]]))
        cpu = min(pair, key=lambda c: load[c])
    quiet = load[cpu] <= OWN_MAX and max(load[c] for c in neighbours(core_type, cpu)) <= NEIGHBOUR_MAX
    return cpu, {str(c): round(v, 3) for c, v in sorted(load.items())}, quiet


def pick(core_type, wait=float(os.environ.get("QUIET_WAIT", "900"))):
    """The idlest core; waits (sampling every 2 s, up to `wait` s) until it and its contention
    neighbour look quiet. Only the per-run rule decides which runs count."""
    t0 = time.time()
    while True:
        cpu, load, quiet = pick_once(core_type)
        if quiet or time.time() - t0 > wait:
            return cpu, dict(load, waited=round(time.time() - t0, 1), quiet=quiet)


def neighbours(core_type, cpu):
    return [c for c in ECORES if c != cpu] if core_type == "E" else [cpu ^ 1]


def outcome(rep):
    if "error" in rep and not rep.get("checks"):
        return "error:" + str(rep["error"].get("code", rep["error"]))
    if "decodedContentContainment" in rep.get("checks", []):
        return "contained"
    rs = [r.split(".", 1)[1] for r in rep.get("notInspected", []) if r.startswith("decodedContentBounds.")]
    return ",".join(sorted(set(rs))) or "?"


def run_once(binary, path, cpu, core_type, scratch):
    a = rq()
    t0 = time.perf_counter()
    with open(os.devnull, "wb") as null, open(scratch, "wb") as out:
        p = subprocess.Popen(["nice", "-n", "10", binary, "validate", "--json", path], stdout=out,
                             stderr=null, preexec_fn=lambda: os.sched_setaffinity(0, {cpu}))
        _, status, ru = os.wait4(p.pid, 0)
    wall = time.perf_counter() - t0
    b = rq()
    child = ru.ru_utime + ru.ru_stime
    own = (b[cpu] - a[cpu]) / 1e9
    foreign = max(0.0, own - child) / wall
    nb = max((b[c] - a[c]) / 1e9 / wall for c in neighbours(core_type, cpu))
    with open(scratch, "rb") as fh:
        rep = json.loads(fh.read() or b"{}")
    return {"cpu": cpu, "wall": round(wall, 5), "cpuTime": round(child, 5), "foreignOwn": round(foreign, 4),
            "neighbour": round(nb, 4), "clean": foreign <= OWN_MAX and nb <= NEIGHBOUR_MAX,
            "outcome": outcome(rep), "exit": os.waitstatus_to_exitcode(status)}


def measure(binary, case, core_type, scratch):
    cpu, load = pick(core_type)
    picks = [{"cpu": cpu, "loadBefore": load}]
    attempts, pairs, dirty_streak = [], [], 0
    for _ in range(MAX_ATTEMPTS):
        ra = run_once(binary, case["a"], cpu, core_type, scratch)
        rb = run_once(binary, case["b"], cpu, core_type, scratch)
        attempts.append({"A": ra, "B": rb})
        bad = [(k, r["outcome"]) for k, r in (("A", ra), ("B", rb))
               if case.get("expect" + k) and r["outcome"] != case["expect" + k]]
        if bad:
            return {"name": case["name"], "coreType": core_type, "units": case["units"], "picks": picks,
                    "status": "unexpectedOutcome", "outcomes": bad, "attempts": len(attempts), "cleanPairs": 0,
                    "runs": attempts}
        if ra["clean"] and rb["clean"]:
            pairs.append(ra["cpuTime"] - rb["cpuTime"])
            dirty_streak = 0
            if len(pairs) == K:
                break
        else:
            dirty_streak += 1
            if dirty_streak == 3:
                cpu, load = pick(core_type)
                picks.append({"cpu": cpu, "loadBefore": load, "afterAttempt": len(attempts)})
                dirty_streak = 0
    row = {"name": case["name"], "coreType": core_type, "units": case["units"], "picks": picks,
           "attempts": len(attempts), "cleanPairs": len(pairs), "pairs": [round(p, 5) for p in pairs],
           "outcomeA": attempts[-1]["A"]["outcome"], "outcomeB": attempts[-1]["B"]["outcome"],
           "runs": attempts}
    if len(pairs) == K:
        med = statistics.median(pairs)
        row.update({"status": "decided", "medianStreamCpu": round(med, 5), "minStreamCpu": round(min(pairs), 5),
                    "medianCpuA": round(statistics.median(
                        a["A"]["cpuTime"] for a in attempts if a["A"]["clean"] and a["B"]["clean"]), 5),
                    "nsPerUnit": round(med * 1e9 / case["units"], 3),
                    "nsPerUnitMin": round(min(pairs) * 1e9 / case["units"], 3)})
    else:
        row["status"] = "undecided"
    return row


def main(argv):
    binary, cases_path, out_path, core_type = argv[:4]
    only = set(argv[argv.index("--only") + 1].split(",")) if "--only" in argv else None
    os.sched_setaffinity(0, {int(os.environ.get("HOUSE", "1"))})
    scratch = out_path + ".last.json"
    with open(cases_path) as fh:
        cases = json.load(fh)
    done = set()
    if os.path.exists(out_path):  # resume: decided or refused cases are not re-run
        for line in open(out_path):
            r = json.loads(line)
            if r["status"] in ("decided", "unexpectedOutcome"):
                done.add(r["name"])
    with open(out_path, "a") as out:
        for case in cases:
            if (only and case["name"] not in only) or case["name"] in done:
                continue
            row = measure(binary, case, core_type, scratch)
            row["loadavg"] = open("/proc/loadavg").read().split()[:3]
            row["time"] = time.strftime("%Y-%m-%dT%H:%M:%S")
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
            print(json.dumps({k: row.get(k) for k in ("name", "coreType", "status", "cleanPairs", "attempts",
                                                       "nsPerUnit", "nsPerUnitMin", "medianStreamCpu",
                                                       "outcomeA", "outcomeB")}), flush=True)


if __name__ == "__main__":
    main(sys.argv[1:])
