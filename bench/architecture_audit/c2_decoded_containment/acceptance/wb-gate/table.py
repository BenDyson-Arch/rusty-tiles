#!/usr/bin/env python3
"""Summarize gate JSONL files (P and E) into the class-matrix table and tau.

    python3 -I table.py OUT.json GATE-P.jsonl [...] -- GATE-E.jsonl [...]
Later rows for the same case name replace earlier ones (re-runs).
"""
import json
import statistics
import sys


def load(paths):
    rows = {}
    for p in paths:
        for line in open(p):
            r = json.loads(line)
            rows[r["name"]] = r
    return rows


def clean_b(r):
    return [a["B"]["cpuTime"] for a in r["runs"] if a["A"]["clean"] and a["B"]["clean"]]


def main(argv):
    out = argv[0]
    i = argv.index("--")
    P, E = load(argv[1:i]), load(argv[i + 1:])
    names = list(dict.fromkeys(list(P) + list(E)))
    table = []
    for n in names:
        p, e = P.get(n), E.get(n)
        row = {"name": n, "units": (p or e)["units"]}
        for tag, r in (("P", p), ("E", e)):
            if r is None:
                row["status" + tag] = "not run"
                continue
            row["status" + tag] = r["status"]
            row["cleanPairs" + tag] = r["cleanPairs"]
            row["attempts" + tag] = r["attempts"]
            row["cpus" + tag] = sorted({x["cpu"] for x in r["picks"]})
            if r["status"] == "decided":
                row["ns" + tag] = r["nsPerUnit"]
                row["nsMin" + tag] = r["nsPerUnitMin"]
                row["stream" + tag] = r["medianStreamCpu"]
                row["medianB" + tag] = round(statistics.median(clean_b(r)), 5)
                runs = [x for a in r["runs"] for x in (a["A"], a["B"]) if x["clean"]]
                row["maxForeign" + tag] = max(x["foreignOwn"] for x in runs)
                row["maxNeighbour" + tag] = max(x["neighbour"] for x in runs)
        if "nsP" in row and "nsE" in row and row["nsP"] > 0:
            row["EoverP"] = round(row["nsE"] / row["nsP"], 2)
        table.append(row)
    dec = lambda t: [(r["ns" + t], r["name"]) for r in table if "ns" + t in r]
    summary = {"tauP": max(dec("P")), "tauE": max(dec("E")),
               "undecided": [r["name"] + "@" + t for r in table for t in "PE"
                             if r.get("status" + t) not in ("decided",)],
               "rows": table}
    ratios = sorted(r["EoverP"] for r in table if "EoverP" in r)
    if ratios:
        summary["EoverP"] = {"min": ratios[0], "median": statistics.median(ratios), "max": ratios[-1]}
    json.dump(summary, open(out, "w"), indent=1)
    print("tauP", summary["tauP"], "tauE", summary["tauE"], "undecided", summary["undecided"])
    for r in sorted(table, key=lambda r: -max(r.get("nsE", 0), r.get("nsP", 0))):
        print("%-44s %13d  P %6s  E %6s  E/P %s" % (r["name"], r["units"], r.get("nsP"), r.get("nsE"),
                                                   r.get("EoverP")))


if __name__ == "__main__":
    main(sys.argv[1:])
