#!/usr/bin/env python3
"""Additive `totalPayloadElements` ratio amendment of the C1 payload-control expectations (2026-10-30).

    python3 -I amend.py                          # write the amended bundles, lanes and record
    python3 -I amend.py --check                  # rebuild in memory, compare with the committed files
    python3 -I amend.py --ktx2 ARCHIVE_ROOT      # amend the KTX2 lane manifest in place
    python3 -I amend.py --ktx2 ARCHIVE_ROOT --check

Rule (docs/architecture/validator-limits-amendment.md, "measured basis (2026-10-30)" and its
additive budget, 2026-10-31): the `totalPayloadElements` budget is 16000000 + 105*S instead of
max(16000000, 16*S), still model 1. In every admitted report,
`limitDerivation.budgets.totalPayloadElements` becomes
{floor: 16000000, perStoredByte: [105, 1], source: "scaled" iff 105*S > 0, additive: true} and
`limits.totalPayloadElements` becomes 16000000 + 105*S. S is recomputed from each case archive
by the VS-C script's own ZIP central-directory parser (never by the validator) and must equal the
report's `storedBytes`; the old entry and value must be exactly the VS-C ratio's (16). Nothing else
in any report changes, and no category, exit code or refused case may change.

Inputs are the VS-C 2026-10-14 outputs, hash-verified and never modified (KTX2 is amended in
place, as VS-B and VS-C did). The shared helpers come from that script.
"""
import argparse, copy, importlib.util, json, os, shutil, sys, tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
CONTROLS = HERE.parents[1]
REL = "payload_element_ratio_amendment/2026-10-30"
spec = importlib.util.spec_from_file_location("vs_c_amend", CONTROLS / "vs_c_limits_amendment/2026-10-14/amend.py")
vsc = importlib.util.module_from_spec(spec); spec.loader.exec_module(vsc)
sha, Refused = vsc.sha, vsc.Refused
KEY = "totalPayloadElements"
FLOOR, OLD, NEW = 16000000, (16, 1), (105, 1)


def old_entry(s):
    p, q = OLD
    return {"floor": FLOOR, "perStoredByte": [p, q], "source": "scaled" if s * p // q > FLOOR else "floor"}


def new_entry(s):
    p, q = NEW
    return {"floor": FLOOR, "perStoredByte": [p, q], "source": "scaled" if s * p // q > 0 else "floor",
            "additive": True}


def new_limit(s):
    return FLOOR + s * NEW[0] // NEW[1]


def amend_report(name, report, s):
    """Amend one admitted expectedReport (a private copy) in place. Returns True if changed."""
    budgets, limits = report["limitDerivation"]["budgets"], report["limits"]
    if report["storedBytes"] != s:
        raise Refused("%s: storedBytes %r but the archive's S is %d" % (name, report["storedBytes"], s))
    if budgets[KEY] == new_entry(s) and limits[KEY] == new_limit(s):
        return False  # already amended (KTX2 in-place lane): verified only
    if budgets[KEY] != old_entry(s) or limits[KEY] != max(FLOOR, s * OLD[0] // OLD[1]):
        raise Refused("%s: %s is %r / %r, not the 16-per-byte rule at S=%d" % (name, KEY, budgets[KEY], limits[KEY], s))
    budgets[KEY] = new_entry(s)
    limits[KEY] = new_limit(s)
    return True


def amend_cases(cases, read_archive):
    """The ratio rule on every admitted case; read_archive(case) -> bytes. Used by the KTX2 runner."""
    stats = {"admitted": 0, "amended": 0, "refusedUnchanged": 0, "scaled": []}
    for case in cases:
        if case["expectedCategory"] != "admitted":
            if case.get("expectedReport") is not None:
                raise Refused("%s: refused case carries an expectedReport" % case["name"])
            stats["refusedUnchanged"] += 1
            continue
        raw = read_archive(case)
        if sha(raw) != case["sha256"]:
            raise Refused("%s: archive does not match its pinned sha256" % case["name"])
        _, s = vsc.stored_bytes(raw)
        report = case["expectedReport"] = copy.deepcopy(case["expectedReport"])
        stats["admitted"] += 1
        stats["amended"] += amend_report(case["name"], report, s)
        if s * OLD[0] // OLD[1] > FLOOR:
            stats["scaled"].append([case["name"], s])  # scaled under the former max rule too
    return cases, stats


def amend_manifest(manifest, read_archive, origin, origin_sha, manifest_sha):
    amended = copy.deepcopy(manifest)
    before = [(c["name"], c["expectedCategory"], c["expectedExitCode"]) for c in amended["cases"]]
    amended["cases"], stats = amend_cases(amended["cases"], read_archive)
    if before != [(c["name"], c["expectedCategory"], c["expectedExitCode"]) for c in amended["cases"]]:
        raise Refused("category or exit code changed")
    amended["payloadElementRatioAmendment"] = {
        "amends": origin, "inputSha256": origin_sha, "inputManifestSha256": manifest_sha,
        "rule": "admitted expectedReport: totalPayloadElements max(16000000, 16*S) -> 16000000 + 105*S "
                "(perStoredByte [105, 1], additive); nothing else changes",
        "specification": "docs/architecture/validator-limits-amendment.md (measured basis, 2026-10-30)",
        "generator": "bench/architecture_audit/c1_payload_integrity/production_controls/" + REL + "/amend.py",
    }
    return amended, stats


BUNDLES = [
    ("vs_c_limits_amendment/2026-10-14/corrected-205-vsc-fixtures.tar.gz",
     "229e97f7ce0e81b59cef5079106e47fec09320b4cdd63314098f4d845f4ca5e0",
     "3f248b5ed05d1b814dcbea71be6445066cabdaca0870345b21d446ed0c74e6ca", "corrected-205-ratio-fixtures.tar.gz"),
    ("vs_c_limits_amendment/2026-10-14/polygon-sensitive-5-vsc-fixtures.tar.gz",
     "f363386d920356499c823a9ded616e945061feb66f049cd5c600797b675478dc",
     "d0197d8501786535556ce806ec58523ba65d6a7a5a4ffdde176959f762827e67", "polygon-sensitive-5-ratio-fixtures.tar.gz"),
]
LANES = [
    ("vs_c_limits_amendment/2026-10-14/stride_controls",
     "38d693ea276bbf9941738ab5ea21589442373daf8161fec4798475dcbd9abe07", "stride_controls"),
    ("vs_c_limits_amendment/2026-10-14/meshopt_header",
     "22ff1456dba290926fc770ce624f9dbc8c6371c831b7622f2a32e4c4b70b24e9", "meshopt_header"),
]


def build(work):
    # The VS-C builders, with this rule and this directory (bundles and lanes keep their layout).
    vsc.amend_manifest, vsc.REL, vsc.HERE = amend_manifest, REL, HERE
    record = {"record": "totalPayloadElements ratio amendment of the C1 payload-control expectations "
                        "(additive; the VS-C inputs unchanged).", "bundles": [], "lanes": []}
    for source, tar_sha, manifest_sha, name in BUNDLES:
        record["bundles"].append(vsc.build_bundle(work, source, tar_sha, manifest_sha, name))
    for source, manifest_sha, dest in LANES:
        record["lanes"].append(vsc.build_lane(work, source, manifest_sha, dest))
    for lane in record["lanes"]:
        path = work / lane["amended"].rsplit("/", 1)[-1] / "integrity.json"
        integrity = json.loads(path.read_text())
        integrity["status"] = integrity["status"].replace("VS-C limits amendment", "Payload-element ratio amendment")
        path.write_text(json.dumps(integrity, indent=2) + "\n")
    record["ktx2"] = "ktx2_basisu_amendment/2026-10-13 is amended in place (--ktx2), as VS-B and VS-C did"
    return record


OUTPUTS = [n for *_, n in BUNDLES] + [d for *_, d in LANES] + ["amendment-manifest.json"]
KTX2 = CONTROLS / "ktx2_basisu_amendment/2026-10-13"


def ktx2(root, check):
    path = KTX2 / "fixtures/manifest.json"
    raw = path.read_bytes()
    manifest = json.loads(raw)
    manifest["cases"], stats = amend_cases(manifest["cases"], lambda case: (Path(root) / case["path"]).read_bytes())
    manifest["runnerSha256"] = sha((KTX2 / "runner.py").read_bytes())
    out = vsc.manifest_bytes(manifest)
    integrity_path = KTX2 / "integrity.json"
    integrity = json.loads(integrity_path.read_text())
    integrity["files"]["fixtures/manifest.json"] = {"bytes": len(out), "sha256": sha(out)}
    runner = (KTX2 / "runner.py").read_bytes()
    integrity["files"]["runner.py"] = {"bytes": len(runner), "sha256": sha(runner)}
    integrity_out = (json.dumps(integrity, indent=2) + "\n").encode()
    print(json.dumps({"admitted": stats["admitted"], "amended": stats["amended"], "scaled": stats["scaled"],
                      "manifestSha256": sha(out), "runnerSha256": sha(runner)}))
    if check:
        return 0 if (out == raw and integrity_out == integrity_path.read_bytes()) else 1
    path.write_bytes(out)
    integrity_path.write_bytes(integrity_out)
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", action="store_true", help="compare a fresh build with the committed outputs")
    parser.add_argument("--ktx2", metavar="ARCHIVE_ROOT", help="amend the KTX2 lane in place from a runner work dir")
    args = parser.parse_args()
    if args.ktx2:
        return ktx2(args.ktx2, args.check)
    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        try:
            record = build(work)
        except Refused as refusal:
            print("REFUSED; no outputs written:", refusal, file=sys.stderr)
            return 1
        (work / "amendment-manifest.json").write_bytes((json.dumps(record, indent=2) + "\n").encode())
        mismatches = []
        for name in OUTPUTS:
            fresh, committed = work / name, HERE / name
            if args.check:
                vsc.compare(fresh, committed, mismatches)
                continue
            if committed.is_dir():
                shutil.rmtree(committed)
            if fresh.is_dir():
                shutil.copytree(fresh, committed, symlinks=True)
            else:
                shutil.copyfile(fresh, committed)
        print(json.dumps({"bundles": {b["amended"]: [b["amendedSha256"], b["amendedManifestSha256"], b["admittedAmended"]]
                                      for b in record["bundles"]},
                          "lanes": {l["amended"]: [l["amendedManifestSha256"], l["admittedAmended"]] for l in record["lanes"]},
                          "check": ("identical" if not mismatches else mismatches) if args.check else None}, indent=1))
        return 1 if mismatches else 0


if __name__ == "__main__":
    sys.exit(main())
