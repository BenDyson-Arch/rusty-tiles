"""Summarize measured level/channel distributions without hiding tail errors.
Usage: python3 summarize_calibration.py EVIDENCE_DIR
"""
import collections, json, math, sys
from pathlib import Path
def band(texels):
    return "tiny" if texels < 64 else "middle" if texels < 1024 else "large"
root = Path(sys.argv[1]); groups = collections.defaultdict(list)
fixture_log = root / "fixture-measurement.log"
if fixture_log.exists():
    for line in fixture_log.read_text().splitlines():
        if line.startswith("etc1s-measure "):
            row = json.loads(line.split(" ",1)[1])
            key = ("fixture", row["fixture"], row["profile"], band(row["texels"]), row["channel"])
            groups[key].append(row)
smooth_log = root / "smooth-photo-measurement.log"
if smooth_log.exists():
    for line in smooth_log.read_text().splitlines():
        if line.startswith("etc1s-photo-measure "):
            row = json.loads(line.split(" ",1)[1])
            key = ("smooth-fixture", row["fixture"], row["profile"], band(row["texels"]), row["channel"])
            groups[key].append(row)
semantic = root / "semantic-fixtures.jsonl"
if semantic.exists():
    for line in semantic.read_text().splitlines():
        row = json.loads(line)
        if row["kind"] == "level":
            key = ("semantic-fixture",row["page"],row["profile"],band(row["texels"]),row["channel"])
            groups[key].append(row)
for path in sorted(root.glob("candidate-tex3-*/calibration.jsonl")):
    for line in path.read_text().splitlines():
        row = json.loads(line)
        if row["kind"] == "level" and row["step"] == "profile":
            key = ("tex3", row["slot"], row["profile"], band(row["texels"]), row["channel"])
            groups[key].append(row)
        elif row["kind"] == "sampler":
            key = ("tex3-sampler", row["slot"], row["profile"], "all", row["channel"])
            groups[key].append(row)
def stats(values):
    values = sorted(values)
    def q(f): return values[max(0,math.ceil(len(values)*f)-1)]
    return dict(min=values[0],p50=q(.5),p90=q(.9),p99=q(.99),max=values[-1])
result = []
for key, rows in sorted(groups.items()):
    entry = dict(zip(["corpus","family","profile","size","channel"],key))
    entry["samples"] = len(rows)
    entry["exact_samples"] = sum(r.get("max") == 0 for r in rows)
    for name in ["psnr","p999","max","blockRmse"]:
        values = [r[name] for r in rows if r.get(name) is not None and (name != "psnr" or r.get("max") != 0)]
        if values: entry[name] = stats(values)
    result.append(entry)
(root / "distribution.json").write_text(json.dumps(result,indent=2)+"\n")
print("groups",len(result),"observations",sum(e["samples"] for e in result))
for e in result:
    if e["corpus"] == "tex3" and e["channel"] < 3:
        print(e["profile"],e["size"],e["channel"],"PSNRmin",round(e.get("psnr",{}).get("min",99),3),"p999max",e["p999"]["max"],"blockmax",round(e["blockRmse"]["max"],3),"n",e["samples"])
