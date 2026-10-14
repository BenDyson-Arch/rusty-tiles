"""Production CLI validate on every corpus artifact (head native + portable), and the P A-3
checks/notInspected comparison against develop 766b92b on its own default (implicit) archive.
python3 -I validate_cli.py OUT.json"""
import hashlib, json, subprocess, sys, time
W = "/home/bend/.cache/forest-acc/work"; B = "/home/bend/.cache/forest-acc/bin/rusty-tiles-"
IDS = ["W1", "W2", "W3-pc-deep", "W4-points-lines", "W4-points-lines-quantize", "W4-points-lines-meshopt", "W4-polygons",
       "W4-polygons-quantize", "W4-polygons-meshopt", "W5-vec-perf", "W5-vec-perf-quantize", "W5-vec-perf-meshopt",
       "mf2-lod3", "W6", "W6d", "W7"]
def val(binary, path):
    t = time.monotonic()
    p = subprocess.run(["/usr/bin/taskset", "-c", "4,5", "/usr/bin/nice", "-n", "10", B + binary, "--json", "validate", path],
                       capture_output=True, text=True)
    w = time.monotonic() - t
    try: d = json.loads(p.stdout or p.stderr)
    except ValueError: d = {"raw": (p.stdout + p.stderr)[-400:]}
    d.pop("payloads", None); d.pop("limits", None)
    return {"exit": p.returncode, "wallSeconds": round(w, 3), **d}
def a3(head, dev):
    want_checks = []
    for c in dev["checks"]:
        want_checks.append(c)
        if c == "hierarchyBounds": want_checks.append("implicitAddressingAndAvailability")
    want_ni = sorted((set(dev["notInspected"]) - {"implicitAddressingAndAvailability"}) | {"recordedBudgets.implicitTileExtras"})
    must = {"decodedContentBounds", "decodedContentBounds.implicitTiling", "metadataSemantics"}
    return {"checksEqual": head["checks"] == want_checks, "notInspectedEqual": sorted(head["notInspected"]) == want_ni,
            "mustContain": sorted(must - set(head["notInspected"])) == [],
            "checksDiff": [c for c in head["checks"] if c not in want_checks] + ["-" + c for c in want_checks if c not in head["checks"]],
            "notInspectedDiff": sorted(set(head["notInspected"]) ^ set(want_ni))}
out = {"binaries": {b: hashlib.sha256(open(B + b, "rb").read()).hexdigest() for b in ("head-native", "head-portable", "develop-native")}, "cases": []}
for k in IDS:
    r = {"id": k, "sha256": hashlib.sha256(open(f"{W}/head-native/{k}.3tz", "rb").read()).hexdigest()}
    r["headNative"] = val("head-native", f"{W}/head-native/{k}.3tz")
    r["headPortableOnPortableArchive"] = val("head-portable", f"{W}/head-portable/{k}.3tz") if k != "mf2-lod3" else None
    r["developOnDevelopDefault"] = val("develop-native", f"{W}/develop-native/{k}.3tz")
    r["developOnDevelopExplicit"] = val("develop-native", f"{W}/develop-native/{k}-explicit.3tz")
    h = r["headNative"]
    if h.get("ok") and r["developOnDevelopDefault"].get("ok"):
        r["A3"] = a3(h, r["developOnDevelopDefault"])
    print(json.dumps({"id": k, "head": (h.get("ok"), h.get("tiles"), h.get("contentReferences"), h.get("wallSeconds")),
                      "portable": (r["headPortableOnPortableArchive"] or {}).get("ok"),
                      "devDefault": (r["developOnDevelopDefault"].get("ok"), r["developOnDevelopDefault"].get("tiles"), r["developOnDevelopDefault"].get("contentReferences")),
                      "A3": r.get("A3")}), flush=True)
    out["cases"].append(r)
for extra in ["W8s-convert-implicit", "W8m-vector-flat-rounded-implicit"]:
    out["cases"].append({"id": extra, "headNative": val("head-native", f"{W}/inputs/{extra}.3tz")})
    print(extra, json.dumps(out["cases"][-1]["headNative"])[:300])
json.dump(out, open(sys.argv[1], "w"), indent=1)
