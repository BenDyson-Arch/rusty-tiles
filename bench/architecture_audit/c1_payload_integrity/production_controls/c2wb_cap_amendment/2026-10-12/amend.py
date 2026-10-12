#!/usr/bin/env python3
"""Additive C2-WB cap amendment of the validator-caps-amended C1 payload-control expectations.

    python3 -I amend.py            # writes the amended bundles, lane and manifest
    python3 -I amend.py --check    # rebuilds in memory and compares with the committed files

Inputs are the validator_caps_amendment/2026-10-11 outputs, hash-verified and never
modified. The only rule: every admitted case's expectedReport.limits takes the C2-WB
default containmentWorkUnits (2^29, contract C2-WB WB-6; evidence in
bench/architecture_audit/c2_decoded_containment/acceptance/README.md, "C2-WB gate"). The old value is
asserted before it is replaced, and nothing else in a report changes.

Refused cases keep their expectations (C2 never refuses). An admitted case whose
expected report says decodedContentBounds.workLimit could change verdict under a
larger cap: that would be a behaviour change, not a report delta, so it is listed
loudly and nothing is written. There are none.
"""

import argparse
import copy
import gzip
import hashlib
import io
import json
import os
import shutil
import sys
import tarfile
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
CONTROLS = HERE.parents[1]
SOURCE = CONTROLS / "validator_caps_amendment/2026-10-11"

# Serialized name: (old default, new default).
RAISED = {"containmentWorkUnits": (268435456, 536870912)}

BUNDLES = [
    # (input bundle, input sha256, input manifest sha256, amended bundle name)
    ("corrected-205-caps-fixtures.tar.gz",
     "b8b0499e456dbc48af4d071d95b12a9765c88add3231c6a73b3fbd07a03035c3",
     "55057d9df744d977044d909875f1cbed6c981867ff2d30e7e22085796a4c039e",
     "corrected-205-c2wb-fixtures.tar.gz"),
    ("polygon-sensitive-5-caps-fixtures.tar.gz",
     "7f5488810f3bc69dbdcc49df8e2c420315af7b24b6b0c8f44d4ee5d6c5e95ef4",
     "3f956fb18f8a7e3e9a42070706a8a105ad197242c5d82f6d13fef5826bcbe9bc",
     "polygon-sensitive-5-c2wb-fixtures.tar.gz"),
]
STRIDE = SOURCE / "stride_controls"
STRIDE_MANIFEST_SHA = "a934bc207fe17670cb095f0b4023842dd4241ef1b65c9556bbd65aff4bde7040"
STRIDE_INTEGRITY_SHA = "925429a64023bf5aa0ed8e1e5e037b694609ef97d593cba30ae8b6ae02010bbc"
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
        if case["expectedCategory"] != "admitted":
            continue
        if "decodedContentBounds.workLimit" in case["expectedReport"]["notInspected"]:
            changes.append(case["name"])
        limits = case["expectedReport"]["limits"]
        for key, (old, new) in RAISED.items():
            if limits[key] != old:
                raise BehaviourChange("%s: %s is %r, expected %r" % (case["name"], key, limits[key], old))
            limits[key] = new
        admitted += 1
    amended["c2wbCapAmendment"] = {
        "amends": origin, "inputSha256": origin_sha, "inputManifestSha256": manifest_sha,
        "rule": "admitted expectedReport.limits take the C2-WB containmentWorkUnits default; nothing else changes",
        "raised": {key: {"old": old, "new": new} for key, (old, new) in RAISED.items()},
        "evidence": "bench/architecture_audit/c2_decoded_containment/acceptance/README.md (C2-WB gate)",
        "generator": "bench/architecture_audit/c1_payload_integrity/production_controls/"
                     "c2wb_cap_amendment/2026-10-12/amend.py",
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
    record = {"record": "C2-WB cap amendment of the validator-caps-amended C1 payload-control "
                        "expectations (additive; inputs unchanged).",
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
        amended, admitted, bad = amend_manifest(manifest, "validator_caps_amendment/2026-10-11/" + original, tar_sha,
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
    # Unchanged lane files are relative symlinks to the validator-caps lane (no duplicated
    # bytes in the tree), except runner.py (copied: it resolves its own directory);
    # fixtures/manifest.json and integrity.json are new files.
    for name in sorted(source_integrity["files"]):
        if name == "fixtures/manifest.json":
            continue
        target = lane / name
        target.parent.mkdir(parents=True, exist_ok=True)
        if name == "runner.py":  # the runner locates its lane from its own resolved path
            shutil.copyfile(STRIDE / name, target)
            continue
        placed = HERE / STRIDE_LANE / name
        os.symlink(os.path.relpath(STRIDE / name, placed.parent), target)
    manifest = json.loads((STRIDE / "fixtures/manifest.json").read_text())
    amended, admitted, bad = amend_manifest(manifest, "validator_caps_amendment/2026-10-11/stride_controls/fixtures/manifest.json",
                                            STRIDE_MANIFEST_SHA, STRIDE_MANIFEST_SHA)
    changes += [{"bundle": "validator_caps_amendment/2026-10-11/stride_controls", "case": name} for name in bad]
    (lane / "fixtures/manifest.json").write_bytes(manifest_bytes(amended))
    files = {}
    for name in sorted(source_integrity["files"]):
        raw = (lane / name).read_bytes() if name == "fixtures/manifest.json" else (STRIDE / name).read_bytes()
        files[name] = {"bytes": len(raw), "sha256": sha(raw)}
    integrity = {"status": "C2-WB cap amendment of validator_caps_amendment/2026-10-11/stride_controls: only "
                           "fixtures/manifest.json differs; the runner is copied, and the generator, primary "
                           "references and fixture files are relative symlinks to that lane",
                 "amends": {"integritySha256": STRIDE_INTEGRITY_SHA, "manifestSha256": STRIDE_MANIFEST_SHA},
                 "files": files}
    (lane / "integrity.json").write_bytes((json.dumps(integrity, indent=2) + "\n").encode())
    record["strideLane"] = {
        "input": "validator_caps_amendment/2026-10-11/stride_controls", "inputManifestSha256": STRIDE_MANIFEST_SHA,
        "inputIntegritySha256": STRIDE_INTEGRITY_SHA,
        "amended": "c2wb_cap_amendment/2026-10-12/" + STRIDE_LANE,
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
            print("BEHAVIOUR CHANGE: admitted controls whose C2 verdict is workLimit; "
                  "no outputs written:", file=sys.stderr)
            print(json.dumps(changes, indent=2), file=sys.stderr)
            return 1
        mismatches = []
        for name in OUTPUTS:
            fresh, committed = work / name, HERE / name
            if args.check:
                pairs = ([(p, committed / p.relative_to(fresh)) for p in fresh.rglob("*")
                          if p.is_symlink() or p.is_file()] if fresh.is_dir() else [(fresh, committed)])
                for f, c in pairs:
                    if f.is_symlink():
                        same = c.is_symlink() and os.readlink(c) == os.readlink(f)
                    else:
                        same = c.is_file() and not c.is_symlink() and c.read_bytes() == f.read_bytes()
                    if not same:
                        mismatches.append(str(c))
            else:
                if committed.is_dir():
                    shutil.rmtree(committed)
                if fresh.is_dir():
                    shutil.copytree(fresh, committed, symlinks=True)
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
