#!/usr/bin/env python3
"""Read existing records only; print normalized timings. Never invoke a converter.

Usage: chrt -i 0 nice -n 19 python3 bench/architecture_audit/bake_time_history/summarize.py
Optional --evidence and --scratch relocate the two read-only evidence roots.
CPU means user + system rusage. Worker timers are elapsed time, not CPU.
"""

import argparse
import json
from pathlib import Path


def read(path):
    return json.loads(path.read_text())


def measurement(path, triangles):
    d = read(path)
    wall = d.get("wall_s", d.get("wall_seconds"))
    user = d.get("user_s", d.get("user_seconds"))
    system = d.get("sys_s", d.get("sys_seconds", d.get("system_seconds")))
    cpu = user + system if user is not None and system is not None else None
    rss = d.get("max_rss_gib")
    if rss is None and "peak_rss_bytes" in d:
        rss = d["peak_rss_bytes"] / 2**30
    return dict(
        path=str(path), exit=d.get("exit"), triangles=triangles,
        wall_s=wall, user_s=user, sys_s=system, cpu_s=cpu,
        cpu_s_per_mtri=cpu * 1e6 / triangles if cpu is not None else None,
        cpu_s_per_wall_s=cpu / wall if cpu is not None else None,
        peak_rss_gib=rss, cpus=d.get("cpus"),
        load_start=d.get("load_at_start", d.get("loadavg_start")),
        scratch_peak_sampled_bytes=d.get("scratch_peak_sampled_bytes"),
    )


def cpu_segments(path):
    """CPU stamps exist only in the October 24 p251 logs, not ordinary progress."""
    points = {}
    for number, line in enumerate(path.read_text().splitlines(), 1):
        parts = line.split(" ", 3)
        if len(parts) != 4:
            continue
        try:
            wall, user, system = map(float, parts[:3])
            event = json.loads(parts[3])
        except ValueError:
            continue
        phase = event.get("phase")
        if phase in ("capture", "leaf_build", "proxy_bake") and event.get("done") == 0:
            points[phase + "_start"] = (wall, user + system, number)
        for phase in ("capture", "leaf_build", "proxy_bake"):
            if event.get("message", "").startswith("mesh phase " + phase + ":"):
                points[phase + "_end"] = (wall, user + system, number)
    result = {}
    for label, start, end in (
        ("capture", "capture_start", "capture_end"),
        ("leaf_build", "leaf_build_start", "leaf_build_end"),
        ("lod_and_handoff", "leaf_build_end", "proxy_bake_start"),
        ("proxy_bake", "proxy_bake_start", "proxy_bake_end"),
    ):
        if start in points and end in points:
            a, b = points[start], points[end]
            result[label] = dict(wall_s=b[0] - a[0], cpu_s=b[1] - a[1], lines=[a[2], b[2]])
    return result


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--evidence", type=Path, default=Path.home() / ".cache/rusty-tiles-evidence")
    ap.add_argument("--scratch", type=Path, default=Path("/home/bend/scratch/rusty-tiles-scratch"))
    args = ap.parse_args()
    e, s = args.evidence, args.scratch
    runs = [
        (e / "h8-perf-20261013/runs/v6-full2048-balanced/run.json", 9999999),
        (e / "h8-perf-20261013/runs/v8-full2048-balanced/run.json", 9999999),
        (e / "h16-20261013/out/balanced.run.json", 9999999),
        (e / "h17-20261013/real/prev-128321f/default.run.json", 9999999),
        (e / "h17-20261013/real/default.run.json", 9999999),
        (e / "h17-20261013/real/det2.run.json", 9999999),
        (e / "vs-c-20261014/real/run.run.json", 9999999),
        (e / "mesh-scratch-20261014/mgal-develop.measure.json", 9999999),
        (e / "mesh-scratch-20261014/mgal-B.measure.json", 9999999),
        (e / "h27-int-20261020/crop-21815e51-jobs8/measure.json", 788425),
        (e / "h27-int-20261020/crop-bc6b3710-jobs8/measure.json", 788425),
        (e / "h28-20261021/measure.json", 788425),
        (s / "mgal-diag/measure.json", 9999999),
        (e / "proxy-donor-20261022/m2048-fix3/measure.json", 9999999),
        (e / "proxy-donor-20261022/m2048-fix4/measure.json", 9999999),
        (e / "bake-bounded-20261022/m2048-final/measure.json", 9999999),
        (e / "proxy-density-20261023/m2048/measure-after.json", 9999999),
    ]
    out = {"runs": [measurement(p, n) for p, n in runs], "phase_reports": {}, "p251_cpu": [], "legacy_lane": []}
    for p in (s / "bake-bounded/m2048-final/report.json", e / "proxy-density-20261023/m2048/report-after.json"):
        d = read(p)
        diag = d["diagnostics"]
        out["phase_reports"][str(p)] = {k: diag.get(k) for k in ("requested_jobs", "phase_seconds", "proxy_bake")}
        out["phase_reports"][str(p)]["leaf_escalation"] = {
            k: d["tilerReport"].get(k) for k in ("escalated_pages", "escalation_reasons", "lossless_fallback_pages")
        }
    for p in sorted((e / "p251-cpu-20261024/runs").glob("*/measure.json")):
        row = measurement(p, 788425)
        row["phases"] = cpu_segments(p.with_name("progress.log"))
        out["p251_cpu"].append(row)
    out["scratch_samples"] = {}
    for name in ("mgal-develop", "mgal-B"):
        p = e / "mesh-scratch-20261014" / (name + ".split.samples")
        samples = []
        for number, line in enumerate(p.read_text().splitlines(), 1):
            if line.startswith("done"):
                continue
            elapsed, raw = line.split(" ", 1)
            # runner.py: first element is logical st_size, second allocated bytes.
            groups = json.loads(raw)
            samples.append((groups.get("spill", [0])[0], number, float(elapsed)))
        size, number, elapsed = max(samples)
        out["scratch_samples"][str(p)] = dict(
            spill_peak_bytes=size, line=number, elapsed_s=elapsed,
            boundary="spill only; excludes work/staged output",
        )
    for p in sorted((e / "perf").glob("2026-*/process.json")):
        d = read(p)
        env = d["environment"]
        for case in d["cases"]:
            if case["case"] != "mesh":
                continue
            out["legacy_lane"].append(dict(
                path=str(p), cpus=env["cpuAffinity"], load=env["loadAverageBefore"],
                binaries=env["binaries"], argv=case["argv"],
                sides={side: {k: row.get(k) for k in ("wallSeconds", "cpuSeconds", "peakRssMiB", "outputBytes")}
                       for side, row in case["sides"].items()},
            ))
    print(json.dumps(out, indent=2))


if __name__ == "__main__":
    main()
