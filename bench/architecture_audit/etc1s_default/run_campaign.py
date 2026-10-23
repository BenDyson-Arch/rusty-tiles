"""Bounded tex3/crop3 campaign; raw artifacts stay outside the repository.
Usage: python3 run_campaign.py LABEL BINARY EVIDENCE_DIR [profiles ...]
Run itself with chrt -i 0 nice -n 19. Slot 2 measurements are correctness/cost
observations under contention, not clean performance acceptance.
"""
import hashlib, json, os, struct, subprocess, sys, zipfile
from pathlib import Path
LABEL, BINARY, DEST = sys.argv[1:4]
DEST = Path(DEST).resolve(); DEST.mkdir(parents=True, exist_ok=True)
SCRATCH = Path("/home/bend/scratch/rusty-tiles-scratch/etc1s-default")
CROPS = Path("/home/bend/.cache/rusty-tiles-evidence/h17-20261013/crop")
PROFILES = sys.argv[4:] or ["fast", "balanced", "quality", "lossless"]
JOBS = os.environ.get("RT_CAMPAIGN_JOBS", "2")
FAMILIES = os.environ.get("RT_CAMPAIGN_FAMILIES", "tex3,crop3").split(",")
assert set(FAMILIES) <= {"tex3", "crop3"}
for family in FAMILIES:
    for profile in PROFILES:
        name = f"{LABEL}-{family}-{profile}"
        work = SCRATCH / name; work.mkdir(parents=True, exist_ok=True)
        dest = DEST / name; dest.mkdir(parents=True, exist_ok=True)
        if (dest / "result.json").exists():
            continue
        env = dict(os.environ, RAYON_NUM_THREADS="2", RUST_TEST_THREADS="2", RT_MEASURE_SLOT="2", RUSTY_TILES_STAGE_TIMING=str(dest / "timing.json"))
        if LABEL == "candidate":
            env["RT_CODEC_CALIBRATION"] = str(dest / "calibration.jsonl")
            (dest / "calibration.jsonl").unlink(missing_ok=True)
        cmd = ["chrt", "-i", "0", "nice", "-n", "19", "python3", "-I", "/home/bend/.cache/rt-scripts/measure.py", str(dest / "stdout.json"), BINARY, "--json", "mesh", "-i", str(CROPS / f"{family}.glb"), "-o", str(work / "out.3tz"), "--profile", profile, "--force", "--jobs", JOBS, "--anchor", "133.0584598837835", "-12.33625061681309", "100", "--scratchDir", str(work)]
        print("RUN", name, flush=True)
        with (dest / "measure.json").open("w") as out, (dest / "stderr.log").open("w") as err:
            subprocess.run(cmd, env=env, stdout=out, stderr=err, check=True)
        measured = json.loads((dest / "measure.json").read_text())
        if measured["exit"]:
            raise RuntimeError((name, measured))
        report = json.loads((dest / "stdout.json").read_text())
        per = {}; codecs = {}; pages = 0
        archive = work / "out.3tz"
        with zipfile.ZipFile(archive) as z:
            conversion = json.loads(z.read("conversion.json"))
            for entry in z.infolist():
                if not entry.filename.endswith(".glb"): continue
                b = z.read(entry); jl = struct.unpack_from("<I", b, 12)[0]; j = json.loads(b[20:20+jl])
                views = j.get("bufferViews", []); images = j.get("images", [])
                per[entry.filename] = sum(views[im["bufferView"]]["byteLength"] for im in images)
                binary_offset = 20 + jl + 8
                for im in images:
                    v = views[im["bufferView"]]; pos = binary_offset + v.get("byteOffset", 0)
                    if im["mimeType"] == "image/ktx2":
                        dfd = struct.unpack_from("<I", b, pos + 48)[0]
                        model = b[pos + dfd + 12]
                        codec = {163:"etc1s",166:"uastc"}.get(model,str(model))
                    else: codec = "png"
                    pages += 1; codecs[codec] = codecs.get(codec,0) + 1
        result = dict(label=LABEL, family=family, profile=profile, jobs=int(JOBS), binary_sha256=hashlib.sha256(Path(BINARY).read_bytes()).hexdigest(), archive_sha256=hashlib.sha256(archive.read_bytes()).hexdigest(), archive_bytes=archive.stat().st_size, texture_bytes=sum(per.values()), pages=pages, codecs=codecs, measurement=measured, leaf_escalations=conversion.get("escalated_pages"), leaf_png_fallbacks=conversion.get("lossless_fallback_pages"), views={})
        for w,h,r in [(1600,1000,"3"),(2560,1600,"0.75")]:
            sim_env = dict(os.environ, SIM_W=str(w), SIM_H=str(h), SIM_RANGE=r)
            data = subprocess.check_output(["chrt","-i","0","nice","-n","19","python3","-I","/tmp/simview/sim.py",str(archive),"--json"], env=sim_env, text=True)
            (dest / f"view-{w}-{h}-{r}.json").write_text(data)
            view = json.loads(data)[0]
            result["views"][f"{w}x{h}@{r}"] = dict(bytes=view["bytes"], texture_bytes=sum(per.get(n,0) for n in view["names"]), requests=view["requests"])
        timing = json.loads((dest / "timing.json").read_text())
        stages = timing["stages"]
        result["encode_worker_wall_s"] = sum(stages.get(n,{}).get("seconds",0) for n in ["basis_encode","png_encode"])
        result["encode_cpu_s"] = sum(stages.get(n,{}).get("cpu_seconds",0) or 0 for n in ["basis_encode","png_encode"]) if any("cpu_seconds" in stage for stage in stages.values()) else None
        (dest / "result.json").write_text(json.dumps(result,indent=2)+"\n")
        print("DONE", name, result["archive_bytes"], result["texture_bytes"],flush=True)
