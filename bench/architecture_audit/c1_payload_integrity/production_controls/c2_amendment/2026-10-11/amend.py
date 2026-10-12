#!/usr/bin/env python3
"""Additive C2 amendment of the pinned C1 payload-control expectations.

    python3 -I amend.py            # writes the amended bundles, lane and manifest
    python3 -I amend.py --check    # rebuilds in memory and compares with the committed files

The 2026-10-10 bundles and lanes are read, hash-verified and never modified.
For every admitted case the independent C2 O-explicit oracle
(bench/architecture_audit/c2_decoded_containment/oracle) decides the C2 outcome
of the fixture archive, and the expected report is amended by the contract's
report rules (contract "Report and failure schema", R7.1/R7.4):

  * the C1 payload label "scene transforms, skins, animations, morph targets and
    rendered bounds" becomes "skins, animations, morph targets and rendered
    extent" (always) plus "scene transforms" (unless the outcome is Contained);
  * Contained: checks gains "decodedContentContainment" (appended after the C1
    checks) and notInspected loses "decodedContentBounds";
  * NotInspected(reasons): notInspected keeps "decodedContentBounds" and gains
    "decodedContentBounds.<reason>" per reason; sorted and deduplicated;
  * limits gains the five C2 ValidationLimits fields at their defaults.

Any admitted case that the oracle finds Outside, InvalidNode, InvalidInput or
cannot decide is a behaviour change (admitted -> failure), not a report delta:
it is reported loudly and nothing is written.
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
REPO = HERE.parents[5]
ORACLE_DIR = REPO / "bench/architecture_audit/c2_decoded_containment/oracle"
if not (ORACLE_DIR / "c2o").is_dir():
    sys.exit(
        f"error: the C2 O-explicit oracle ({ORACLE_DIR.relative_to(REPO)}) was moved out of the tree "
        "at e29cd19d2f23. Restore it with\n"
        "  git archive e29cd19d2f23 bench/architecture_audit/c2_decoded_containment/oracle | tar -x\n"
        "or from ~/.cache/rusty-tiles-evidence/architecture_audit/c2_decoded_containment.tar.zst "
        "(SHA-256 in bench/architecture_audit/c2_decoded_containment/README.md)."
    )
sys.path.insert(0, str(ORACLE_DIR))

from c2o.check import Oracle  # noqa: E402

OLD_LABEL = "scene transforms, skins, animations, morph targets and rendered bounds"
LABEL_ALWAYS = "skins, animations, morph targets and rendered extent"
LABEL_SCENE = "scene transforms"
# Contract "ValidationLimits (O-b)" serialized names; containmentWorkUnits per the
# coordinator's cap decision for PR #157 (2^28).
C2_LIMITS = {
    "containmentSpanBits": 4096,
    "containmentWorkUnits": 268435456,
    "containmentBytesRead": 2147483648,
    "containmentDecodedBytes": 2147483648,
    "containmentNodeDepth": 64,
}

POLYGON = CONTROLS / "polygon_correction/2026-10-10"
BUNDLES = [
    # (original bundle, original tar sha256, original manifest sha256, amended bundle name)
    ("corrected-205-fixtures.tar.gz",
     "c05406cdb264e386d1ad209abfded1ecc0b2b97da8c8b32ad8ff911263aa46a4",
     "40fae6bf9a0dec2fbdf6e900ac257e61a8732f0987fbdb9e1d84bd6e52510620",
     "corrected-205-c2-fixtures.tar.gz"),
    ("polygon-sensitive-5-fixtures.tar.gz",
     "add990418822a1118c9c3be2faff04fa0722ae926c2d97769353f90c928a6f2c",
     "a26782e50b63b5449652bcd3ccb1370723d1b2f3493e7678705b9f3fc7dc9a84",
     "polygon-sensitive-5-c2-fixtures.tar.gz"),
]
STRIDE = CONTROLS / "stride_controls/2026-10-10"
STRIDE_MANIFEST_SHA = "a3c07122e5fc902272bcf668a85e694601551a66556a7be3806132febd592aee"
STRIDE_INTEGRITY_SHA = "3662f04fc170af1d3869b6167ab22ba0ab5451c66edc0ff9ebd2ee2616c03b53"
STRIDE_LANE = "stride_controls"  # amended lane directory under HERE


class BehaviourChange(Exception):
    pass


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def manifest_bytes(manifest):
    return (json.dumps(manifest, indent=2) + "\n").encode()  # the originals' serialization


def amend_report(report, outcome):
    amended = copy.deepcopy(report)
    names = set(amended["notInspected"])
    had_label = OLD_LABEL in names
    names.discard(OLD_LABEL)
    if had_label:
        names.add(LABEL_ALWAYS)
    if outcome["outcome"] == "Contained":
        names.discard("decodedContentBounds")
        amended["checks"] = list(amended["checks"]) + ["decodedContentContainment"]
    elif outcome["outcome"] == "NotInspected":
        names.add("decodedContentBounds")
        if had_label:
            names.add(LABEL_SCENE)
        names.update("decodedContentBounds." + reason for reason in outcome["reasons"])
    else:
        raise BehaviourChange(outcome)
    amended["notInspected"] = sorted(names)
    amended["limits"] = dict(amended["limits"], **C2_LIMITS)
    return amended


def decide(archive_path):
    report = Oracle(str(archive_path)).run()
    outcome = {"outcome": report["outcome"]}
    for key in ("reasons", "facts", "why"):
        if report.get(key) is not None:
            outcome[key] = report[key]
    if report.get("c1Caveats"):
        outcome["c1Caveats"] = report["c1Caveats"]
    return outcome


def amend_manifest(manifest, fixtures_dir, provenance):
    """Amend every admitted case; return (manifest, per-case summary, behaviour changes)."""
    amended = copy.deepcopy(manifest)
    summary, changes = [], []
    for case in amended["cases"]:
        if case["expectedCategory"] != "admitted":
            continue
        outcome = decide(fixtures_dir / case["path"])
        summary.append({"name": case["name"], "c2": outcome})
        try:
            case["expectedReport"] = amend_report(case["expectedReport"], outcome)
        except BehaviourChange:
            changes.append({"name": case["name"], "c2": outcome})
            continue
        case["c2OracleOutcome"] = {k: outcome[k] for k in ("outcome", "reasons") if k in outcome}
    amended["c2Amendment"] = provenance
    return amended, summary, changes


def provenance(original, original_sha, manifest_sha):
    return {
        "amends": original, "originalSha256": original_sha, "originalManifestSha256": manifest_sha,
        "rules": "contract Report and failure schema (R7.1 label split, R7.4 checks/notInspected); "
                 "five C2 ValidationLimits fields at their defaults",
        "c2Oracle": "bench/architecture_audit/c2_decoded_containment/oracle (O-explicit, independent of src/)",
        "generator": "bench/architecture_audit/c1_payload_integrity/production_controls/c2_amendment/2026-10-11/amend.py",
    }


def rebuild_tar(original_raw, manifest_raw):
    """Same members, order and attributes as the original; only manifest.json differs."""
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
    """Build every output into `work`; return (record, behaviour changes)."""
    record = {"record": "C2 amendment of the C1 payload-control expectations (additive; originals unchanged).",
              "bundles": [], "strideLane": None}
    changes = []
    for original, tar_sha, manifest_sha, amended_name in BUNDLES:
        raw = (POLYGON / original).read_bytes()
        assert sha(raw) == tar_sha, "original bundle %s does not match its pin" % original
        fixtures = work / ("extract-" + original)
        with tarfile.open(fileobj=io.BytesIO(raw), mode="r:gz") as bundle:
            bundle.extractall(fixtures, filter="data")
        manifest_raw = (fixtures / "manifest.json").read_bytes()
        assert sha(manifest_raw) == manifest_sha, "original manifest of %s does not match its pin" % original
        manifest = json.loads(manifest_raw)
        amended, summary, bad = amend_manifest(manifest, fixtures,
                                               provenance("polygon_correction/2026-10-10/" + original, tar_sha,
                                                          manifest_sha))
        changes += [dict(item, bundle=original) for item in bad]
        new_manifest = manifest_bytes(amended)
        new_raw = rebuild_tar(raw, new_manifest)
        (work / amended_name).write_bytes(new_raw)
        record["bundles"].append({
            "original": original, "originalSha256": tar_sha, "originalManifestSha256": manifest_sha,
            "amended": amended_name, "amendedSha256": sha(new_raw), "amendedManifestSha256": sha(new_manifest),
            "cases": len(manifest["cases"]), "admitted": len(summary), "c2": summary})
    # The stride lane: the runner reads fixtures/ beside itself, so the amended lane is
    # a full copy with byte-identical runner, generator, primary references and fixtures,
    # except fixtures/manifest.json.
    assert sha((STRIDE / "fixtures/manifest.json").read_bytes()) == STRIDE_MANIFEST_SHA
    assert sha((STRIDE / "integrity.json").read_bytes()) == STRIDE_INTEGRITY_SHA
    original_integrity = json.loads((STRIDE / "integrity.json").read_text())
    for name, entry in original_integrity["files"].items():
        assert sha((STRIDE / name).read_bytes()) == entry["sha256"], name
    lane = work / STRIDE_LANE
    for name in ["runner.py", "generator.py"] + sorted(
            n for n in original_integrity["files"] if n.startswith(("primary/", "fixtures/"))):
        target = lane / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(STRIDE / name, target)
    manifest = json.loads((STRIDE / "fixtures/manifest.json").read_text())
    amended, summary, bad = amend_manifest(manifest, STRIDE / "fixtures",
                                           provenance("stride_controls/2026-10-10/fixtures/manifest.json",
                                                      STRIDE_MANIFEST_SHA, STRIDE_MANIFEST_SHA))
    changes += [dict(item, bundle="stride_controls/2026-10-10") for item in bad]
    (lane / "fixtures/manifest.json").write_bytes(manifest_bytes(amended))
    files = {}
    for path in sorted(p for p in lane.rglob("*") if p.is_file()):
        raw = path.read_bytes()
        files[str(path.relative_to(lane))] = {"bytes": len(raw), "sha256": sha(raw)}
    integrity = {"status": "C2 amendment of stride_controls/2026-10-10: only fixtures/manifest.json differs; "
                           "runner, generator, primary references and fixture bytes are copied unchanged",
                 "amends": {"integritySha256": STRIDE_INTEGRITY_SHA, "manifestSha256": STRIDE_MANIFEST_SHA},
                 "files": files}
    (lane / "integrity.json").write_bytes((json.dumps(integrity, indent=2) + "\n").encode())
    record["strideLane"] = {
        "original": "stride_controls/2026-10-10", "originalManifestSha256": STRIDE_MANIFEST_SHA,
        "amended": "c2_amendment/2026-10-11/" + STRIDE_LANE,
        "amendedManifestSha256": files["fixtures/manifest.json"]["sha256"],
        "cases": len(manifest["cases"]), "admitted": len(summary), "c2": summary}
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
            print("BEHAVIOUR CHANGE: admitted C1 controls that the C2 oracle does not admit "
                  "(admitted -> failure); no outputs written:", file=sys.stderr)
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
        counts = {}
        for item in record["bundles"] + [record["strideLane"]]:
            for case in item["c2"]:
                key = case["c2"]["outcome"] + "".join("(%s)" % r for r in case["c2"].get("reasons", []))
                counts[key] = counts.get(key, 0) + 1
        print(json.dumps({"admittedCases": sum(counts.values()), "c2Outcomes": counts,
                          "bundles": {b["amended"]: b["amendedSha256"] for b in record["bundles"]},
                          "strideManifest": record["strideLane"]["amendedManifestSha256"],
                          "check": ("identical" if not mismatches else mismatches) if args.check else None},
                         indent=1))
        return 1 if mismatches else 0


if __name__ == "__main__":
    sys.exit(main())
