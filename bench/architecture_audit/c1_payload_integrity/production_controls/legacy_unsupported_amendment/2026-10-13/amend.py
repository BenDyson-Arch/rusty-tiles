#!/usr/bin/env python3
"""Additive H7 amendment of the C2-WB-amended C1 payload-control expectations.

    python3 -I amend.py            # writes the amended bundle and manifest
    python3 -I amend.py --check    # rebuilds in memory and compares with the committed files

Input: c2wb_cap_amendment/2026-10-12/corrected-205-c2wb-fixtures.tar.gz, hash-verified and
never modified. The only rule (docs/architecture/c1-legacy-content-unsupported-amendment.md):
every control whose content member is a .b3dm is legacy content, so its expectation becomes
Unsupported (exit 2) with no report. The 12 controls are named here; each one's member set is
asserted to hold its .b3dm member. Any other case is copied unchanged, and a b3dm member in a
case not listed here is refused (nothing is written).

polygon-sensitive-5 and the stride lane hold no legacy content and are not amended.
"""

import argparse
import copy
import gzip
import hashlib
import io
import json
import sys
import tarfile
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
CONTROLS = HERE.parents[1]
SOURCE = CONTROLS / "c2wb_cap_amendment/2026-10-12"
INPUT = ("corrected-205-c2wb-fixtures.tar.gz",
         "7d844ecf1fc99719b0807f2859bbc6e988ee110283d5f001c25a0eee563fdb7e",
         "3ef63c08433e1b69573e83c9aef0fe00972f94b9160e7934b07974bad956ae35")
OUTPUT = "corrected-205-legacy-fixtures.tar.gz"
LEGACY = {
    # name: category before H7
    "b3dm-zero": "admitted", "b3dm-decimal": "admitted", "b3dm-exponent": "admitted",
    "b3dm-nonzero": "admitted", "b3dm-fraction": "invalid_input", "b3dm-u32-overflow": "invalid_input",
    "b3dm-huge": "invalid_input", "b3dm-null": "invalid_input", "b3dm-missing": "invalid_input",
    "b3dm-duplicate": "invalid_input", "b3dm-feature-unicode": "invalid_input",
    "b3dm-batch-unicode": "invalid_input",
}


class BehaviourChange(Exception):
    pass


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def manifest_bytes(manifest):
    return (json.dumps(manifest, indent=2) + "\n").encode()  # the input's serialization


def amend_manifest(manifest):
    amended = copy.deepcopy(manifest)
    seen, changes = [], []
    for case in amended["cases"]:
        legacy_members = [m for m in case.get("memberSha256", {}) if m.lower().endswith(".b3dm")]
        if case["name"] not in LEGACY:
            if legacy_members:
                raise BehaviourChange("%s holds %s but is not a named legacy control" % (case["name"], legacy_members))
            continue
        if not legacy_members:
            raise BehaviourChange("%s has no .b3dm member" % case["name"])
        before = LEGACY[case["name"]]
        if case["expectedCategory"] != before:
            raise BehaviourChange("%s: category is %r, expected %r" % (case["name"], case["expectedCategory"], before))
        changes.append({"case": case["name"], "member": legacy_members[0], "before": before, "after": "unsupported"})
        case["expectedCategory"] = "unsupported"
        case["expectedExitCode"] = 2
        case["expectedReport"] = None
        case.pop("c2OracleOutcome", None)
        seen.append(case["name"])
    if sorted(seen) != sorted(LEGACY):
        raise BehaviourChange("legacy controls missing from the input: %s" % sorted(set(LEGACY) - set(seen)))
    amended["legacyUnsupportedAmendment"] = {
        "amends": "c2wb_cap_amendment/2026-10-12/" + INPUT[0], "inputSha256": INPUT[1], "inputManifestSha256": INPUT[2],
        "rule": "controls whose content member is .b3dm expect Unsupported (exit 2, no report); nothing else changes",
        "changes": changes,
        "contract": "docs/architecture/c1-legacy-content-unsupported-amendment.md",
        "generator": "bench/architecture_audit/c1_payload_integrity/production_controls/"
                     "legacy_unsupported_amendment/2026-10-13/amend.py",
    }
    return amended, changes


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
    name, tar_sha, manifest_sha = INPUT
    raw = (SOURCE / name).read_bytes()
    assert sha(raw) == tar_sha, "input bundle %s does not match its pin" % name
    with tarfile.open(fileobj=io.BytesIO(raw), mode="r:gz") as bundle:
        manifest_raw = bundle.extractfile("manifest.json").read()
    assert sha(manifest_raw) == manifest_sha, "input manifest of %s does not match its pin" % name
    manifest = json.loads(manifest_raw)
    amended, changes = amend_manifest(manifest)
    new_manifest = manifest_bytes(amended)
    new_raw = rebuild_tar(raw, new_manifest)
    (work / OUTPUT).write_bytes(new_raw)
    return {"record": "H7 legacy-content amendment of the C2-WB-amended C1 payload-control expectations "
                      "(additive; input unchanged).",
            "input": name, "inputSha256": tar_sha, "inputManifestSha256": manifest_sha,
            "amended": OUTPUT, "amendedSha256": sha(new_raw), "amendedManifestSha256": sha(new_manifest),
            "cases": len(manifest["cases"]), "changes": changes}


OUTPUTS = [OUTPUT, "amendment-manifest.json"]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", action="store_true", help="compare a fresh build with the committed outputs")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        try:
            record = build(work)
        except BehaviourChange as error:
            print("REFUSED: %s; no outputs written" % error, file=sys.stderr)
            return 1
        (work / "amendment-manifest.json").write_bytes((json.dumps(record, indent=2) + "\n").encode())
        mismatches = []
        for name in OUTPUTS:
            fresh, committed = work / name, HERE / name
            if args.check:
                if not committed.is_file() or committed.read_bytes() != fresh.read_bytes():
                    mismatches.append(str(committed))
            else:
                committed.write_bytes(fresh.read_bytes())
        print(json.dumps({"changed": len(record["changes"]), "bundle": [record["amendedSha256"], record["amendedManifestSha256"]],
                          "check": ("identical" if not mismatches else mismatches) if args.check else None}, indent=1))
        return 1 if mismatches else 0


if __name__ == "__main__":
    sys.exit(main())
