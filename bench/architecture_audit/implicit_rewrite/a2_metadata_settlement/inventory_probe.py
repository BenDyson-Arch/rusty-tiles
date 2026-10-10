#!/usr/bin/env python3
"""Tiny independent candidate-inventory ownership model; no C1 execution.

The trusted expected plan is distinct from the candidate's untrusted report.
This does not model ZIP/index/CRC, payload semantic validation or final Rust IO.
"""
import copy
import hashlib
import json
from pathlib import Path
import sys

from probe import (CONTROL_NAMES, SOURCE_MANIFEST, SOURCE_REPORT, Refusal, authored,
                   canonical, glb_document, invalid, prepare, sha)


def build_plan(source):
    admitted = prepare(source)
    candidate = {name: data for name, data in source.items() if name not in CONTROL_NAMES}
    candidate[SOURCE_MANIFEST] = source["tileset.json"]
    if "conversion.json" in source:
        candidate[SOURCE_REPORT] = source["conversion.json"]
    # Skeleton placeholders here make no implicit-format/schema claim.
    candidate["tileset.json"] = canonical({"asset": admitted["live_asset"], "root": {}})
    candidate["conversion.json"] = canonical({"profile": admitted["profile"],
                                             "sourceProvenance": admitted["raw_sources"]})
    roles = {}
    for name in candidate:
        if name in admitted["payloads"]:
            roles[name] = "original-payload"
        elif name == SOURCE_MANIFEST:
            roles[name] = "source-manifest-snapshot"
        elif name == SOURCE_REPORT:
            roles[name] = "source-report-snapshot"
        elif name == "vector-build.json" and admitted["source_vector_binding"] is not None:
            roles[name] = "historical-vector-state"
        elif name in ("tileset.json", "conversion.json"):
            roles[name] = "generated-control"
        else:
            roles[name] = "retained-source-bytes"
    expected = {name: {"role": roles[name], "bytes": len(data), "sha256": sha(data)}
                for name, data in candidate.items()}
    return candidate, {"expected": expected, "source_edges": admitted["edges"],
                       "source_bindings": admitted["source_vector_binding"]}


def inspect_model(candidate, plan):
    expected = plan["expected"]
    if set(candidate) != set(expected):
        invalid("candidate inventory differs from admitted plan")
    used, payload_calls = set(), []
    for name, evidence in expected.items():
        data = candidate[name]
        if len(data) != evidence["bytes"] or sha(data) != evidence["sha256"]:
            invalid("candidate member differs from admitted bytes: " + name)
        role = evidence["role"]
        if role == "original-payload":
            # Deliberately only a header/document model; real integration must
            # invoke Check::payload and its validated resource/codec checks.
            glb_document(data)
            payload_calls.append(name)
        elif role == "source-manifest-snapshot":
            snapshot = json.loads(data)
            if not isinstance(snapshot.get("asset"), dict) or not isinstance(snapshot.get("root"), dict):
                invalid("source manifest snapshot shape")
        elif role in ("source-report-snapshot", "historical-vector-state", "generated-control"):
            if not isinstance(json.loads(data), dict):
                invalid("JSON role shape")
        elif role != "retained-source-bytes":
            invalid("unadmitted role")
        used.add(name)
    # Only edges settled by initial import are authority, never candidate report
    # labels or arbitrary strings inside historical JSON or opaque extras.
    for origin, target, edge_role in plan["source_edges"]:
        if target in CONTROL_NAMES:
            invalid("prepared source edge unexpectedly targets replaced control")
        if target not in expected:
            invalid("missing admitted source reference: " + target)
        used.add(target)
    if set(candidate) != used:
        invalid("unaccounted candidate member")
    return {"members_verified": len(used), "original_payload_model_calls": payload_calls,
            "historical_state_has_no_live_reuse_claim": True,
            "opaque_bytes_have_no_semantic_validation_claim": True}


def main():
    if len(sys.argv) != 2 or not __debug__:
        raise SystemExit("usage: python3 -B inventory_probe.py new-result.json (without -O)")
    result_path = Path(sys.argv[1])
    if result_path.exists():
        raise SystemExit("result must not exist")
    source = authored()
    source["orphan.bin"] = b"exact retained bytes; no semantic certificate"
    candidate, plan = build_plan(source)
    happy = inspect_model(candidate, plan)
    assert happy["original_payload_model_calls"] == ["nested/payload.glb"]
    rows = [{"case": "original_payload_and_snapshots_and_opaque_bytes", "outcome": "model_pass", "facts": happy}]
    controls = [
        ("original_payload_change", lambda c: c.__setitem__("nested/payload.glb", c["nested/payload.glb"] + b" ")),
        ("source_snapshot_same_value_different_bytes", lambda c: c.__setitem__(SOURCE_MANIFEST, c[SOURCE_MANIFEST] + b" ")),
        ("missing_source_report_snapshot", lambda c: c.pop(SOURCE_REPORT)),
        ("missing_known_geometry_report", lambda c: c.pop("geometry-reports.jsonl")),
        ("retained_orphan_bytes_change", lambda c: c.__setitem__("orphan.bin", b"changed")),
        ("extra_member_with_report_claim", lambda c: (c.__setitem__("private.dat", b"new"), c.__setitem__("conversion.json", canonical({"retainedContentUris": ["private.dat"]})))),
    ]
    for label, mutate in controls:
        changed = copy.deepcopy(candidate)
        mutate(changed)
        try:
            inspect_model(changed, plan)
        except Refusal as error:
            assert error.kind == "invalid_input"
            rows.append({"case": label, "outcome": "refused", "kind": error.kind, "reason": error.reason})
        else:
            raise AssertionError(label)
    # A report may describe retained resources, but cannot supply authority to
    # omit payload inspection. Existing legacy C1's operation string is no gate.
    caller_plan = copy.deepcopy(plan)
    caller_plan["expected"]["conversion.json"] = {"role": "generated-control", "bytes": 2, "sha256": sha(b"{}")}
    reportless = copy.deepcopy(candidate)
    reportless["conversion.json"] = b"{}"
    checked = inspect_model(reportless, caller_plan)
    assert checked["original_payload_model_calls"] == ["nested/payload.glb"]
    rows.append({"case": "original_payload_inspected_without_legacy_report_operation", "outcome": "model_pass"})
    bad_plan = copy.deepcopy(plan)
    bad_plan["source_edges"].append((SOURCE_REPORT, "missing.dat", "geometryReports"))
    try:
        inspect_model(candidate, bad_plan)
    except Refusal as error:
        assert error.kind == "invalid_input"
        rows.append({"case": "missing_expected_reference", "outcome": "refused", "kind": error.kind})
    else:
        raise AssertionError("missing_expected_reference")
    historical_source = copy.deepcopy(source)
    manifest = json.loads(historical_source["tileset.json"])
    manifest["asset"]["version"] = "1.1"
    historical_state = canonical({"version": 1, "manifestSha256": sha(canonical(manifest))})
    manifest["asset"]["extras"] = {"vectorBuildStateSha256": sha(historical_state)}
    historical_source["tileset.json"] = canonical(manifest)
    historical_source["vector-build.json"] = historical_state
    historical_candidate, historical_plan = build_plan(historical_source)
    assert historical_plan["expected"]["vector-build.json"]["role"] == "historical-vector-state"
    inspect_model(historical_candidate, historical_plan)
    rows.append({"case": "historical_state_accounted_without_live_marker", "outcome": "model_pass"})
    historical_candidate["vector-build.json"] += b" "
    try:
        inspect_model(historical_candidate, historical_plan)
    except Refusal as error:
        assert error.kind == "invalid_input"
        rows.append({"case": "historical_state_bytes_change", "outcome": "refused", "kind": error.kind})
    else:
        raise AssertionError("historical_state_bytes_change")
    root = Path(__file__).resolve().parents[4]
    c1 = root / "src/validate.rs"
    receipt = {"evidence": "independent candidate inventory model; no C1/Rust/producer execution",
               "driver_sha256": sha(Path(__file__).read_bytes()),
               "metadata_model_sha256": sha(Path(__file__).with_name("probe.py").read_bytes()),
               "inspected_c1_source_sha256": sha(c1.read_bytes()), "cases": rows,
               "proof_limits": "C1 private seam/report profile, ZIP/index/CRC, actual payload codecs, IO and lifecycle require production implementation/acceptance"}
    result_path.write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({"cases": len(rows), "driver_sha256": receipt["driver_sha256"]}))


if __name__ == "__main__":
    main()
