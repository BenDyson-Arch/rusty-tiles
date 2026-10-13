#!/usr/bin/env python3
"""Additive VS-C limits amendment of the frozen C1 payload-control expectations (2026-10-14).

    python3 -I amend.py                          # write the amended bundles, lanes and record
    python3 -I amend.py --check                  # rebuild in memory, compare with the committed files
    python3 -I amend.py --ktx2 ARCHIVE_ROOT      # amend the KTX2 lane manifest in place (like VS-B)
    python3 -I amend.py --ktx2 ARCHIVE_ROOT --check

Rule (docs/architecture/validator-limits-amendment.md, VS-C-1 and VS-C-4): every successful
report gains `storedBytes` (S, the sum of the stored sizes of every central-directory entry,
the `@3dtilesIndex1@` index included) and `limitDerivation` (model 1: for each of the seven
budgets its floor, ratio and source "floor" or "scaled"); in `limits` the four Class B caps
(sourceArchiveBytes, archiveEntries, centralDirectoryBytes, archiveStoredBytes) are null and
liveDocumentBytes (268435456) is added. S is computed here from each case archive's ZIP
central directory, by a small parser written for this script and cross-checked against
zipfile; never from the validator. The old values are asserted before they are replaced, and
each existing budget must equal max(floor, S*p//q) (consistency with VS-B).

Inputs (hash-verified, never modified): the H7 legacy corrected-205 bundle, the C2-WB
polygon-sensitive-5 bundle, the C2-WB stride lane and the 2026-10-12 meshopt header lane. The
parent1 lane (stride_parent_control/2026-10-10) has no admitted case and its one refusal
message is not a limit refusal, so it is unchanged and not copied.

Behaviour changes are refused: no case may change expectedCategory or expectedExitCode (this
script never writes either) and a refused case must carry no expectedReport; anything else is
listed and nothing is written. Whether the new binary changes a refused case's verdict is
decided by running the oracles. No lane compares a refusal message except parent1, whose
message is not a cap or budget refusal.
"""

import argparse
import copy
import gzip
import hashlib
import io
import json
import os
import shutil
import struct
import sys
import tarfile
import tempfile
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
CONTROLS = HERE.parents[1]
REL = "vs_c_limits_amendment/2026-10-14"

MODEL = 1
# Class C budget: (floor, p, q). Effective default = max(floor, S*p//q).
RULES = {
    "hierarchyVisits": (262144, 1, 8),
    "references": (1048576, 1, 8),
    "totalBytesRead": (2147483648, 4, 1),
    "totalPayloadElements": (16000000, 16, 1),
    "containmentWorkUnits": (536870912, 128, 1),
    "containmentBytesRead": (2147483648, 4, 1),
    "containmentDecodedBytes": (2147483648, 16, 1),
}
# Class B caps: old default -> null.
OLD_CAPS = {"sourceArchiveBytes": 1073741824, "archiveEntries": 524288,
            "centralDirectoryBytes": 33554432, "archiveStoredBytes": 1073741824}
LIVE_DOCUMENT_BYTES = 268435456
NEW_TOP = ("storedBytes", "limitDerivation")


class Refused(Exception):
    pass


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def manifest_bytes(manifest):
    return (json.dumps(manifest, indent=2) + "\n").encode()  # the inputs' serialization


# ---- S, independently ------------------------------------------------------------------

def directory_stored_bytes(raw):
    """(entries, sum of stored sizes) from the ZIP central directory; no ZIP64 (fixtures are tiny)."""
    tail = raw[max(0, len(raw) - 65557):]
    at = tail.rfind(b"PK\x05\x06")
    assert at >= 0, "no end of central directory"
    eocd = tail[at:]
    entries, size, offset = struct.unpack_from("<HII", eocd, 10)
    assert 0xFFFF not in (entries,) and 0xFFFFFFFF not in (size, offset), "ZIP64 fixture not expected"
    total, cursor = 0, offset
    for _ in range(entries):
        assert raw[cursor:cursor + 4] == b"PK\x01\x02", "bad central directory record"
        compressed = struct.unpack_from("<I", raw, cursor + 20)[0]
        name, extra, comment = struct.unpack_from("<HHH", raw, cursor + 28)
        total += compressed
        cursor += 46 + name + extra + comment
    assert cursor == offset + size, "central directory size mismatch"
    return entries, total


def stored_bytes(raw):
    entries, total = directory_stored_bytes(raw)
    with zipfile.ZipFile(io.BytesIO(raw)) as archive:
        infos = archive.infolist()
        assert len(infos) == entries and sum(i.compress_size for i in infos) == total
        assert all(i.compress_type == zipfile.ZIP_STORED for i in infos)
    assert any(i.filename == "@3dtilesIndex1@" for i in infos), "index entry not in directory"
    return entries, total


def derivation(s):
    budgets = {}
    for key in sorted(RULES):
        floor, p, q = RULES[key]
        budgets[key] = {"floor": floor, "perStoredByte": [p, q],
                        "source": "scaled" if s * p // q > floor else "floor"}
    return {"model": MODEL, "budgets": budgets}


# ---- the rule --------------------------------------------------------------------------

def amend_report(name, report, entries, s):
    """Amend one admitted expectedReport (a private copy) in place. Returns True if changed."""
    limits = report["limits"]
    if "storedBytes" in report:  # already amended (KTX2 in-place lane): verify only
        assert report["storedBytes"] == s and report["limitDerivation"] == derivation(s), name
        assert all(limits[k] is None for k in OLD_CAPS) and limits["liveDocumentBytes"] == LIVE_DOCUMENT_BYTES, name
        return False
    assert "limitDerivation" not in report and "liveDocumentBytes" not in limits, name
    assert report["entries"] == entries, (name, "report entries", report["entries"], "directory", entries)
    for key, old in OLD_CAPS.items():
        if limits[key] != old:
            raise Refused("%s: %s is %r, expected the old cap %r" % (name, key, limits[key], old))
    for key, (floor, p, q) in RULES.items():
        want = max(floor, s * p // q)
        if limits[key] != want:
            raise Refused("%s: %s is %r, expected max(floor, S*p//q) = %r (S=%d)" % (name, key, limits[key], want, s))
    for key in OLD_CAPS:
        limits[key] = None
    limits["liveDocumentBytes"] = LIVE_DOCUMENT_BYTES
    report["storedBytes"] = s
    report["limitDerivation"] = derivation(s)
    return True


def amend_cases(cases, read_archive):
    """The VS-C rule on every admitted case; read_archive(case) -> bytes. Returns (cases, stats).

    Used by this script and by the KTX2 runner on its regenerated cases. Raises Refused on a
    refused case that carries a report or an admitted case that does not match the old state."""
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
        entries, s = stored_bytes(raw)
        # The generator may share one report object between cases: amend a deep copy.
        report = case["expectedReport"] = copy.deepcopy(case["expectedReport"])
        stats["admitted"] += 1
        stats["amended"] += amend_report(case["name"], report, entries, s)
        for key, entry in report["limitDerivation"]["budgets"].items():
            if entry["source"] == "scaled":
                stats["scaled"].append([case["name"], key, s])
    return cases, stats


def amend_manifest(manifest, read_archive, origin, origin_sha, manifest_sha):
    amended = copy.deepcopy(manifest)
    before = [(c["name"], c["expectedCategory"], c["expectedExitCode"]) for c in amended["cases"]]
    amended["cases"], stats = amend_cases(amended["cases"], read_archive)
    after = [(c["name"], c["expectedCategory"], c["expectedExitCode"]) for c in amended["cases"]]
    if before != after:
        raise Refused("category or exit code changed: %r" % sorted(set(before) ^ set(after)))
    amended["vsCLimitsAmendment"] = {
        "amends": origin, "inputSha256": origin_sha, "inputManifestSha256": manifest_sha,
        "rule": "admitted expectedReport gains storedBytes and limitDerivation; limits: the four Class B caps "
                "become null and liveDocumentBytes is added; nothing else changes",
        "model": MODEL, "oldCaps": OLD_CAPS, "liveDocumentBytes": LIVE_DOCUMENT_BYTES,
        "specification": "docs/architecture/validator-limits-amendment.md (VS-C-1, VS-C-4)",
        "generator": "bench/architecture_audit/c1_payload_integrity/production_controls/" + REL + "/amend.py",
    }
    return amended, stats


# ---- bundles and lanes -----------------------------------------------------------------

BUNDLES = [
    # (input bundle relative to CONTROLS, input sha256, input manifest sha256, amended bundle name)
    ("legacy_unsupported_amendment/2026-10-13/corrected-205-legacy-fixtures.tar.gz",
     "7fe07fd1341d8fdbc743be9cb7cd0b1fa063731b5678621d652e799ad0101b8d",
     "f4ce9c7bd60e7788ca9ec7d6af5c7edf20f08e2c2cc8f25ced32a2d2c9a01815",
     "corrected-205-vsc-fixtures.tar.gz"),
    ("c2wb_cap_amendment/2026-10-12/polygon-sensitive-5-c2wb-fixtures.tar.gz",
     "d64eaf9ade5bb7b3dd5d73a5b2398a12083c2bc3ae948a3a250a07139971b25f",
     "2d5d56c03526d50ecc864da16ae744ffa44efaa10e683c9fa4f7bdd40cda6ef8",
     "polygon-sensitive-5-vsc-fixtures.tar.gz"),
]
LANES = [
    # (source lane relative to CONTROLS, manifest sha256, integrity sha256 or None, amended lane dir)
    ("c2wb_cap_amendment/2026-10-12/stride_controls",
     "83ca40eb831ad802e1ce1fbbfb1b4ede45905761c305dd91821c8b58bd08d345", None, "stride_controls"),
    ("meshopt_header_amendment/2026-10-12", None, None, "meshopt_header"),
]


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


def build_bundle(work, source, tar_sha, manifest_sha, name):
    raw = (CONTROLS / source).read_bytes()
    assert sha(raw) == tar_sha, "input bundle %s does not match its pin" % source
    with tarfile.open(fileobj=io.BytesIO(raw), mode="r:gz") as bundle:
        manifest_raw = bundle.extractfile("manifest.json").read()
        assert sha(manifest_raw) == manifest_sha, "input manifest of %s does not match its pin" % source

        def read_archive(case):
            return bundle.extractfile(case["path"]).read()

        amended, stats = amend_manifest(json.loads(manifest_raw), read_archive, source, tar_sha, manifest_sha)
    new_manifest = manifest_bytes(amended)
    new_raw = rebuild_tar(raw, new_manifest)
    (work / name).write_bytes(new_raw)
    return {"input": source, "inputSha256": tar_sha, "inputManifestSha256": manifest_sha, "amended": name,
            "amendedSha256": sha(new_raw), "amendedManifestSha256": sha(new_manifest),
            "cases": len(amended["cases"]), "admittedAmended": stats["amended"],
            "refusedUnchanged": stats["refusedUnchanged"], "scaledBudgets": stats["scaled"]}


def build_lane(work, source, manifest_sha, dest):
    src = CONTROLS / source
    source_integrity_raw = (src / "integrity.json").read_bytes()
    source_integrity = json.loads(source_integrity_raw)
    manifest_raw = (src / "fixtures/manifest.json").read_bytes()
    if manifest_sha is not None:
        assert sha(manifest_raw) == manifest_sha, "lane manifest of %s does not match its pin" % source
    for name, entry in source_integrity["files"].items():
        data = (src / name).read_bytes()
        assert sha(data) == entry["sha256"] and len(data) == entry["bytes"], name
    lane = work / dest
    # Unchanged lane files are relative symlinks to their final targets (no duplicated bytes in
    # the tree), except runner.py (copied: it resolves its own directory); fixtures/manifest.json
    # and integrity.json are new files.
    for name in sorted(source_integrity["files"]):
        if name == "fixtures/manifest.json":
            continue
        target = lane / name
        target.parent.mkdir(parents=True, exist_ok=True)
        if name == "runner.py":
            shutil.copyfile(src / name, target)
            continue
        placed = HERE / dest / name
        os.symlink(os.path.relpath(os.path.realpath(src / name), placed.parent), target)

    def read_archive(case):
        return (src / "fixtures" / case["path"]).read_bytes()

    amended, stats = amend_manifest(json.loads(manifest_raw), read_archive, source + "/fixtures/manifest.json",
                                    sha(source_integrity_raw), sha(manifest_raw))
    (lane / "fixtures").mkdir(exist_ok=True)
    new_manifest = manifest_bytes(amended)
    (lane / "fixtures/manifest.json").write_bytes(new_manifest)
    files = {}
    for name in sorted(source_integrity["files"]):
        data = new_manifest if name == "fixtures/manifest.json" else (src / name).read_bytes()
        files[name] = {"bytes": len(data), "sha256": sha(data)}
    integrity = {"status": "VS-C limits amendment of %s: only fixtures/manifest.json differs; the runner is copied, "
                           "and every other file is a relative symlink to its unchanged target" % source,
                 "amends": {"integritySha256": sha(source_integrity_raw), "manifestSha256": sha(manifest_raw)},
                 "files": files}
    (lane / "integrity.json").write_bytes((json.dumps(integrity, indent=2) + "\n").encode())
    return {"input": source, "inputManifestSha256": sha(manifest_raw), "inputIntegritySha256": sha(source_integrity_raw),
            "amended": REL + "/" + dest, "amendedManifestSha256": files["fixtures/manifest.json"]["sha256"],
            "cases": len(amended["cases"]), "admittedAmended": stats["amended"],
            "refusedUnchanged": stats["refusedUnchanged"], "scaledBudgets": stats["scaled"]}


def build(work):
    record = {"record": "VS-C limits amendment of the C1 payload-control expectations (additive; inputs unchanged).",
              "model": MODEL, "bundles": [], "lanes": [],
              "parent1": "stride_parent_control/2026-10-10: no admitted case; its one refusal message "
                         "('meshopt and bufferView stride disagree') is not a limit refusal; unchanged"}
    for source, tar_sha, manifest_sha, name in BUNDLES:
        record["bundles"].append(build_bundle(work, source, tar_sha, manifest_sha, name))
    for source, manifest_sha, _, dest in LANES:
        record["lanes"].append(build_lane(work, source, manifest_sha, dest))
    record["ktx2"] = "ktx2_basisu_amendment/2026-10-13 is amended in place (--ktx2), as VS-B did; see README"
    return record


OUTPUTS = [n for *_, n in BUNDLES] + [d for *_, d in LANES] + ["amendment-manifest.json"]


def compare(fresh, committed, mismatches):
    pairs = ([(p, committed / p.relative_to(fresh)) for p in sorted(fresh.rglob("*")) if p.is_symlink() or p.is_file()]
             if fresh.is_dir() else [(fresh, committed)])
    for f, c in pairs:
        if f.is_symlink():
            same = c.is_symlink() and os.readlink(c) == os.readlink(f)
        else:
            same = c.is_file() and not c.is_symlink() and c.read_bytes() == f.read_bytes()
        if not same:
            mismatches.append(str(c))


# ---- KTX2 lane, in place ---------------------------------------------------------------

KTX2 = CONTROLS / "ktx2_basisu_amendment/2026-10-13"


def ktx2(root, check):
    path = KTX2 / "fixtures/manifest.json"
    raw = path.read_bytes()
    manifest = json.loads(raw)
    root = Path(root)
    manifest["cases"], stats = amend_cases(manifest["cases"], lambda case: (root / case["path"]).read_bytes())
    manifest["runnerSha256"] = sha((KTX2 / "runner.py").read_bytes())
    out = manifest_bytes(manifest)
    integrity_path = KTX2 / "integrity.json"
    integrity = json.loads(integrity_path.read_text())
    integrity["files"]["fixtures/manifest.json"] = {"bytes": len(out), "sha256": sha(out)}
    runner = (KTX2 / "runner.py").read_bytes()
    integrity["files"]["runner.py"] = {"bytes": len(runner), "sha256": sha(runner)}
    integrity_out = (json.dumps(integrity, indent=2) + "\n").encode()
    print(json.dumps({"admitted": stats["admitted"], "amended": stats["amended"],
                      "refusedUnchanged": stats["refusedUnchanged"], "scaledBudgets": stats["scaled"],
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
            print("REFUSED (behaviour change or unexpected old state); no outputs written:", file=sys.stderr)
            print(refusal, file=sys.stderr)
            return 1
        (work / "amendment-manifest.json").write_bytes((json.dumps(record, indent=2) + "\n").encode())
        mismatches = []
        for name in OUTPUTS:
            fresh, committed = work / name, HERE / name
            if args.check:
                compare(fresh, committed, mismatches)
            else:
                if committed.is_dir():
                    shutil.rmtree(committed)
                if fresh.is_dir():
                    shutil.copytree(fresh, committed, symlinks=True)
                else:
                    shutil.copyfile(fresh, committed)
        print(json.dumps({"admittedAmended": {b["amended"]: b["admittedAmended"] for b in record["bundles"]}
                          | {l["amended"]: l["admittedAmended"] for l in record["lanes"]},
                          "bundles": {b["amended"]: [b["amendedSha256"], b["amendedManifestSha256"]]
                                      for b in record["bundles"]},
                          "laneManifests": {l["amended"]: l["amendedManifestSha256"] for l in record["lanes"]},
                          "check": ("identical" if not mismatches else mismatches) if args.check else None}, indent=1))
        return 1 if mismatches else 0


if __name__ == "__main__":
    sys.exit(main())
