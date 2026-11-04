#!/usr/bin/env python3
"""Additive validator-caps amendment of the C2-amended C1 payload-control expectations.

    python3 -I amend.py            # writes the amended bundles, lane and manifest
    python3 -I amend.py --check    # rebuilds in memory and compares with the committed files

Inputs are the c2_amendment/2026-10-11 outputs, hash-verified and never modified.
The only rule: every admitted case's expectedReport.limits takes the raised
ValidationLimits defaults (bench/architecture_audit/validator_caps). Each old
value is asserted before it is replaced, and nothing else in a report changes.

Refused cases keep their expectations. Any resource_limit case whose fixture
could depend on a raised ceiling would be a behaviour change, not a report
delta: such a case is listed loudly and nothing is written. The four existing
resource_limit cases refuse u64-overflowing counts/lengths, which no finite
ceiling admits; actual executions confirm them (README).
"""

import argparse
import copy
import gzip
import hashlib
import io
import json
import shutil
import sys
import tarfile
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
CONTROLS = HERE.parents[1]
SOURCE = CONTROLS / "c2_amendment/2026-10-11"

# Serialized name: (old default, new default).
RAISED = {
    "archiveEntries": (65536, 524288),
    "hierarchyVisits": (65536, 262144),
    "references": (262144, 1048576),
    "centralDirectoryBytes": (16777216, 33554432),
    "documentItems": (65536, 131072),
    "memberBytes": (67108864, 134217728),
    "documentDecodedBytes": (67108864, 134217728),
}
# Refusals that cannot depend on a finite ceiling (u64 overflow of the declared value).
OVERFLOW_REFUSALS = {"exact-count-18446744073709551615", "exact-count-18446744073709551616",
                     "exact-count-1e400", "declared-length-overflow"}

BUNDLES = [
    # (input bundle, input sha256, input manifest sha256, amended bundle name)
    ("corrected-205-c2-fixtures.tar.gz",
     "ebf8437f1fc0711688fdef74f7b076939dd55f177bc663159a3527049e5b8481",
     "fd6004a4cd4074553d5ff08542fdd9b355d2db86c6035c695dc4513be5e3408c",
     "corrected-205-caps-fixtures.tar.gz"),
    ("polygon-sensitive-5-c2-fixtures.tar.gz",
     "94ae2cefc6214ac4ecf1ef972187ecbcd2275f2d44526152be13c46719e50189",
     "61237a250af51352098dbabeb282859aa38a0528aa9048c19f8aee82aaccca00",
     "polygon-sensitive-5-caps-fixtures.tar.gz"),
]
STRIDE = SOURCE / "stride_controls"
STRIDE_MANIFEST_SHA = "8516d51342c559d983e9f7fa83adeb3dc320078a21bf7ff9438c2fb9783e3557"
STRIDE_INTEGRITY_SHA = "dea02a0bd364b2e0f19629fc0e9ce42812ba3a78d027e409b901d3f3687a36f6"
STRIDE_LANE = "stride_controls"


class BehaviourChange(Exception):
    pass


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def manifest_bytes(manifest):
    return (json.dumps(manifest, indent=2) + "\n").encode()  # the inputs' serialization


def amend_manifest(manifest, origin, origin_sha, manifest_sha):
    amended = copy.deepcopy(manifest)
    admitted, changes = 0, []
    for case in amended["cases"]:
        if case["expectedCategory"] == "resource_limit" and case["name"] not in OVERFLOW_REFUSALS:
            changes.append(case["name"])
        if case["expectedCategory"] != "admitted":
            continue
        limits = case["expectedReport"]["limits"]
        for key, (old, new) in RAISED.items():
            if limits[key] != old:
                raise BehaviourChange("%s: %s is %r, expected %r" % (case["name"], key, limits[key], old))
            limits[key] = new
        admitted += 1
    amended["capsAmendment"] = {
        "amends": origin, "inputSha256": origin_sha, "inputManifestSha256": manifest_sha,
        "rule": "admitted expectedReport.limits take the raised ValidationLimits defaults; nothing else changes",
        "raised": {key: {"old": old, "new": new} for key, (old, new) in RAISED.items()},
        "evidence": "bench/architecture_audit/validator_caps",
        "generator": "bench/architecture_audit/c1_payload_integrity/production_controls/"
                     "validator_caps_amendment/2026-10-11/amend.py",
    }
    return amended, admitted, changes


def rebuild_tar(original_raw, manifest_raw):
    """Same members, order and attributes as the input; only manifest.json differs."""
    out = io.BytesIO()
    with tarfile.open(fileobj=io.BytesIO(original_raw), mode="r:gz") as source:
        with tarfile.open(fileobj=out, mode="w", format=tarfile.PAX_FORMAT) as target:
            for member in source.getmembers():
                data = manifest_raw if member.name == "manifest.json" else source.extractfile(member).read()
                info = tarfile.TarInfo(member.name)
                info.size = len(data)
                info.mode, info.uid, info.gid = member.mode, member.uid, member.gid
                info.uname, info.gname, info.mtime = member.uname, member.gname, member.mtime
                target.addfile(info, io.BytesIO(data))
    packed = io.BytesIO()
    with gzip.GzipFile(filename="", mode="wb", fileobj=packed, mtime=0, compresslevel=9) as stream:
        stream.write(out.getvalue())
    return packed.getvalue()


def build(work):
    record = {"record": "Validator-caps amendment of the C2-amended C1 payload-control expectations "
                        "(additive; inputs unchanged).",
              "raised": {key: {"old": old, "new": new} for key, (old, new) in RAISED.items()},
              "bundles": [], "strideLane": None}
    changes = []
    for original, tar_sha, manifest_sha, amended_name in BUNDLES:
        raw = (SOURCE / original).read_bytes()
        assert sha(raw) == tar_sha, "input bundle %s does not match its pin" % original
        with tarfile.open(fileobj=io.BytesIO(raw), mode="r:gz") as bundle:
            manifest_raw = bundle.extractfile("manifest.json").read()
        assert sha(manifest_raw) == manifest_sha, "input manifest of %s does not match its pin" % original
        manifest = json.loads(manifest_raw)
        amended, admitted, bad = amend_manifest(manifest, "c2_amendment/2026-10-11/" + original, tar_sha,
                                                manifest_sha)
        changes += [{"bundle": original, "case": name} for name in bad]
        new_manifest = manifest_bytes(amended)
        new_raw = rebuild_tar(raw, new_manifest)
        (work / amended_name).write_bytes(new_raw)
        record["bundles"].append({
            "input": original, "inputSha256": tar_sha, "inputManifestSha256": manifest_sha,
            "amended": amended_name, "amendedSha256": sha(new_raw), "amendedManifestSha256": sha(new_manifest),
            "cases": len(manifest["cases"]), "admittedAmended": admitted})
    assert sha((STRIDE / "fixtures/manifest.json").read_bytes()) == STRIDE_MANIFEST_SHA
    assert sha((STRIDE / "integrity.json").read_bytes()) == STRIDE_INTEGRITY_SHA
    source_integrity = json.loads((STRIDE / "integrity.json").read_text())
    for name, entry in source_integrity["files"].items():
        assert sha((STRIDE / name).read_bytes()) == entry["sha256"], name
    lane = work / STRIDE_LANE
    for name in sorted(source_integrity["files"]):
        if name == "fixtures/manifest.json":
            continue
        target = lane / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(STRIDE / name, target)
    manifest = json.loads((STRIDE / "fixtures/manifest.json").read_text())
    amended, admitted, bad = amend_manifest(manifest, "c2_amendment/2026-10-11/stride_controls/fixtures/manifest.json",
                                            STRIDE_MANIFEST_SHA, STRIDE_MANIFEST_SHA)
    changes += [{"bundle": "c2_amendment/2026-10-11/stride_controls", "case": name} for name in bad]
    (lane / "fixtures/manifest.json").write_bytes(manifest_bytes(amended))
    files = {}
    for path in sorted(p for p in lane.rglob("*") if p.is_file()):
        raw = path.read_bytes()
        files[str(path.relative_to(lane))] = {"bytes": len(raw), "sha256": sha(raw)}
    integrity = {"status": "Validator-caps amendment of c2_amendment/2026-10-11/stride_controls: only "
                           "fixtures/manifest.json differs; runner, generator, primary references and "
                           "fixture bytes are copied unchanged",
                 "amends": {"integritySha256": STRIDE_INTEGRITY_SHA, "manifestSha256": STRIDE_MANIFEST_SHA},
                 "files": files}
    (lane / "integrity.json").write_bytes((json.dumps(integrity, indent=2) + "\n").encode())
    record["strideLane"] = {
        "input": "c2_amendment/2026-10-11/stride_controls", "inputManifestSha256": STRIDE_MANIFEST_SHA,
        "inputIntegritySha256": STRIDE_INTEGRITY_SHA,
        "amended": "validator_caps_amendment/2026-10-11/" + STRIDE_LANE,
        "amendedManifestSha256": files["fixtures/manifest.json"]["sha256"],
        "cases": len(manifest["cases"]), "admittedAmended": admitted}
    record["behaviourChanges"] = changes
    return record, changes


OUTPUTS = [name for _, _, _, name in BUNDLES] + [STRIDE_LANE, "amendment-manifest.json"]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", action="store_true", help="compare a fresh build with the committed outputs")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        record, changes = build(work)
        (work / "amendment-manifest.json").write_bytes((json.dumps(record, indent=2) + "\n").encode())
        if changes:
            print("BEHAVIOUR CHANGE: resource_limit controls that may depend on a raised ceiling; "
                  "no outputs written:", file=sys.stderr)
            print(json.dumps(changes, indent=2), file=sys.stderr)
            return 1
        mismatches = []
        for name in OUTPUTS:
            fresh, committed = work / name, HERE / name
            if args.check:
                pairs = ([(p, committed / p.relative_to(fresh)) for p in fresh.rglob("*") if p.is_file()]
                         if fresh.is_dir() else [(fresh, committed)])
                mismatches += [str(c) for f, c in pairs if not c.is_file() or c.read_bytes() != f.read_bytes()]
            else:
                if committed.is_dir():
                    shutil.rmtree(committed)
                if fresh.is_dir():
                    shutil.copytree(fresh, committed)
                else:
                    shutil.copyfile(fresh, committed)
        print(json.dumps({"admittedAmended": sum(b["admittedAmended"] for b in record["bundles"])
                          + record["strideLane"]["admittedAmended"],
                          "bundles": {b["amended"]: [b["amendedSha256"], b["amendedManifestSha256"]]
                                      for b in record["bundles"]},
                          "strideManifest": record["strideLane"]["amendedManifestSha256"],
                          "check": ("identical" if not mismatches else mismatches) if args.check else None},
                         indent=1))
        return 1 if mismatches else 0


if __name__ == "__main__":
    sys.exit(main())
