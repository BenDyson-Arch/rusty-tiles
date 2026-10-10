#!/usr/bin/env python3
"""Read small independently pinned real-source archives from sibling evidence.

No producer is run. The originating receipt owns actual execution provenance.
This script checks case/member identities and counts archive/document envelopes.
"""
import hashlib
import json
from pathlib import Path
import sys
import zipfile

from probe import admit_zip


def sha(blob):
    return hashlib.sha256(blob).hexdigest()


def count_values(value):
    if isinstance(value, dict):
        return 1 + sum(map(count_values, value.values()))
    if isinstance(value, list):
        return 1 + sum(map(count_values, value))
    return 1


def main():
    if len(sys.argv) != 3 or not __debug__:
        raise SystemExit("usage: python3 -B count_source_envelopes.py sibling-artifacts new-result.json")
    root, output = Path(sys.argv[1]), Path(sys.argv[2])
    if output.exists():
        raise SystemExit("result must not exist")
    receipt_path = root / "receipt.json"
    receipt_bytes = receipt_path.read_bytes()
    receipt = json.loads(receipt_bytes)
    cases = []
    for case in receipt["cases"]:
        path = root / (case["label"] + ".3tz")
        blob = path.read_bytes()
        assert sha(blob) == case["source_sha256"]
        admitted = admit_zip(blob)
        with zipfile.ZipFile(path) as archive:
            members = archive.infolist()
            actual = {m.filename: sha(archive.read(m.filename)) for m in members if m.filename != "@3dtilesIndex1@"}
            assert actual == case["source_members"]
            manifest_bytes = archive.read("tileset.json")
            manifest = json.loads(manifest_bytes)
            json_members = [m for m in members if m.filename.endswith(".json")]
            values = sum(count_values(json.loads(archive.read(m.filename))) for m in json_members)
            header = {k: v for k, v in manifest.items() if k != "root"}
            header_bytes = len(json.dumps(header, separators=(",", ":")).encode())
            reference_count = sum(n["contents"] for n in case["nodes"])
            facts = dict(label=case["label"], source_sha256=sha(blob), archive_bytes=len(blob),
                         **admitted, max_member_bytes=max(m.file_size for m in members),
                         manifest_bytes=len(manifest_bytes), distinct_json_bytes=sum(m.file_size for m in json_members),
                         distinct_json_values=values, compact_original_header_bytes=header_bytes,
                         source_nodes=len(case["nodes"]), content_references=reference_count,
                         max_content_slots=max(n["contents"] for n in case["nodes"]),
                         source_member_hashes=actual)
        candidate = root / (case["label"] + "-implicit.3tz")
        if candidate.exists() and case.get("output_sha256"):
            assert sha(candidate.read_bytes()) == case["output_sha256"]
            with zipfile.ZipFile(candidate) as archive:
                aliases = [m for m in archive.infolist() if Path(m.filename).name.startswith("owned-")]
                facts["legacy_alias_bytes"] = sum(m.file_size for m in aliases)
                facts["legacy_alias_count"] = len(aliases)
                facts["legacy_candidate_sha256"] = case["output_sha256"]
        cases.append(facts)
    numeric = ("archive_bytes", "entries", "member_bytes", "directory_bytes", "max_member_bytes",
               "manifest_bytes", "distinct_json_bytes", "distinct_json_values", "compact_original_header_bytes",
               "source_nodes", "content_references", "max_content_slots")
    result = dict(evidence="Read-only independent counting of small real producer archives; no new production execution or stress",
                  originating_receipt_sha256=sha(receipt_bytes), originating_execution_pins=receipt["pins"],
                  driver_sha256=sha(Path(__file__).read_bytes()), admission_model_sha256=sha(Path(__file__).with_name("probe.py").read_bytes()),
                  cases=cases, maxima={field: max(c[field] for c in cases) for field in numeric},
                  proof_limits="Small fixtures establish observed envelope fit only; production resource ceilings remain unmeasured/unaccepted")
    output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"cases": len(cases), "maxima": result["maxima"]}))


if __name__ == "__main__":
    main()
