#!/usr/bin/env python3
"""Regenerate the seven W-corpus archives that were built by the pre-forest producer f2238cd.

    python3 -I regenerate.py [--out DIR] [--producer PATH]

Writes pc-perf-2000.3tz, pc-deep.3tz, vec-perf.3tz and a2fix/{octree,quadtree}-{none,rename-class}.3tz
under DIR (default ~/.cache/rusty-tiles-w-corpus/samples), never /tmp (a RAM tmpfs that a reboot empties).
Each archive's sha256 is checked against manifest.json; the bytes depend on the f2238cd binary
(`git worktree add ... f2238cd`, release build; sha256 of the retained build 369c3c81...), so a
mismatch means the wrong producer, except vec-perf (W5u), which embeds the PROJ version and is
marked verdictOnly in the manifest. Inputs come from bench/perf_lane/corpus.py (pinned digests, generated
deterministically) plus one external dependency: cloud.las (sha256 353b9130...), which has no generator and
is copied from ~/.cache/c2-corpus/run2/inputs/cloud.las (pc-deep cannot be rebuilt without it).  Existing archives with the right digest are kept.  Run `run.py` afterwards.
"""
import base64, hashlib, json, os, pathlib, struct, subprocess, sys, tempfile

HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parents[3]
HOME_CACHE = pathlib.Path.home() / ".cache"
DEFAULT_OUT = HOME_CACHE / "rusty-tiles-w-corpus" / "samples"
DEFAULT_PRODUCER = HOME_CACHE / "c2-corpus" / "rusty-tiles-f2238cd"
PRODUCER_SHA = "369c3c815f95a64018e49f360b605f84740c3c4f9f145c5fde33dbd4148749b9"
CLOUD_SHA = "353b913082fec2a2062664b6e44c3b8d2258687592bf62ecffd4b017f9f2eec9"
A2 = REPO / "bench" / "architecture_audit" / "implicit_rewrite" / "a2_semantics"


def sha(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()


def materialize(fixture, out, rename_class):
    """Fixture JSON -> tileset directory (the retired probes/materialize_a2_fixture.py, same bytes)."""
    members = {k: base64.b64decode(v) for k, v in json.load(open(fixture))["members_base64"].items()}
    members["tileset.json"] = members.pop("n0.json")
    for name in list(members):
        if name.endswith(".json"):
            doc = json.loads(members[name])
            if isinstance(doc.get("schema"), dict) and "id" not in doc["schema"]:
                doc["schema"]["id"] = "a2"
                members[name] = json.dumps(doc, separators=(",", ":")).encode()
    if rename_class:
        for name in list(members):
            if name.endswith(".json"):
                members[name] = members[name].replace(b'"a2Tile"', b'"rustyTile"')
            if name.endswith(".subtree"):
                data = members[name]
                jlen = int.from_bytes(data[8:16], "little")
                js = data[24:24 + jlen].replace(b'"a2Tile"', b'"rustyTile"')
                js = js + b" " * (jlen - len(js))
                members[name] = data[:24] + js + data[24 + jlen:]
    for name, data in members.items():
        path = out / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)


def main(argv):
    out = pathlib.Path(argv[argv.index("--out") + 1]) if "--out" in argv else DEFAULT_OUT
    producer = pathlib.Path(argv[argv.index("--producer") + 1]) if "--producer" in argv else DEFAULT_PRODUCER
    if str(out.resolve()).startswith("/tmp/"):
        sys.exit("refusing /tmp: it is RAM-backed and lost at reboot")
    want, loose = {}, set()
    for a in json.load(open(HERE / "manifest.json"))["artifacts"]:
        if "/rusty-tiles-w-corpus/samples/" in a["path"]:
            rel = pathlib.Path(a["path"].split("/samples/", 1)[1])
            want[rel] = a["sha256"]
            if a.get("verdictOnly"):
                loose.add(rel)
    todo = {rel: h for rel, h in want.items()
            if not (out / rel).exists() or (rel not in loose and sha(out / rel) != h)}
    if not todo:
        print("all seven archives present with pinned digests")
        return 0
    if not producer.exists() or sha(producer) != PRODUCER_SHA:
        sys.exit(f"{producer}: need the f2238cd release binary (sha256 {PRODUCER_SHA})")
    out.mkdir(parents=True, exist_ok=True)
    work = out.parent / "inputs"
    work.mkdir(exist_ok=True)
    sys.path.insert(0, str(REPO / "bench" / "perf_lane"))
    import corpus as perf
    perf.build(work)  # points.las and vector.geojson, digests pinned by the perf lane
    cloud = work / "cloud.las"
    src = HOME_CACHE / "c2-corpus" / "run2" / "inputs" / "cloud.las"
    if not cloud.exists():
        if not src.exists():
            sys.exit(f"cloud.las ({CLOUD_SHA}) is not reproducible by script; expected at {src}")
        cloud.write_bytes(src.read_bytes())
    if sha(cloud) != CLOUD_SHA:
        sys.exit(f"{cloud}: wrong digest")
    run = lambda *a, **k: subprocess.run([str(producer), "--json", *map(str, a)], check=True, capture_output=True, text=True, **k)
    jobs = {
        "pc-perf-2000.3tz": ["point-cloud", "-i", work / "points.las", "--sourceCrs", "EPSG:32756", "--heightOffset", "0", "--maxPoints", "2000"],
        "pc-deep.3tz": ["point-cloud", "-i", cloud, "--sourceCrs", "local", "--maxPoints", "40"],
        "vec-perf.3tz": ["vector", "-i", work / "vector.geojson", "--reproducible", "--jobs", "2"],
    }
    for rel, args in jobs.items():
        if pathlib.Path(rel) in todo:
            run(*args, "-o", out / rel)
    for shape in ("quadtree", "octree"):
        for mutation in ("none", "rename-class"):
            rel = pathlib.Path("a2fix") / f"{shape}-{mutation}.3tz"
            if rel in todo:
                with tempfile.TemporaryDirectory(dir=out.parent) as d:
                    materialize(A2 / f"{shape}.json", pathlib.Path(d) / "t", mutation == "rename-class")
                    (out / "a2fix").mkdir(exist_ok=True)
                    run("convert", "-i", pathlib.Path(d) / "t", "-o", out / rel)
    bad = [str(r) for r, h in todo.items() if r not in loose and sha(out / r) != h]
    for r in todo:
        print(("MISMATCH " if str(r) in bad else "ok       ") + str(r))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
