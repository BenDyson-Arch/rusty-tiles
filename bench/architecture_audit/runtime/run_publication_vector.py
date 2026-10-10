#!/usr/bin/env python3
"""Standalone audit probes, deliberately excluded from the normal test suite."""
import argparse
import hashlib
import json
import os
import pathlib
import platform
import shutil
import subprocess
import tempfile
import zipfile

BASE = "8dfd74bd87dd23c86278e98d736c3f5912c246cf"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def execute(command, env=None):
    r = subprocess.run(list(map(str, command)), text=True, capture_output=True, env=env)
    return {"command": list(map(str, command)), "exit_code": r.returncode, "stdout": r.stdout, "stderr": r.stderr}


def archive_inventory(path):
    if not path.exists():
        return {"published": False}
    with zipfile.ZipFile(path) as archive:
        manifest = json.loads(archive.read("tileset.json"))
        report = json.loads(archive.read("conversion.json"))
        reports = [json.loads(line) for line in archive.read("geometry-reports.jsonl").splitlines()]
        references = set()

        def walk(node):
            for content in node.get("contents", []) + ([node["content"]] if "content" in node else []):
                references.add(content["uri"])
            for child in node.get("children", []):
                walk(child)

        walk(manifest["root"])
        members = set(archive.namelist())
        content = {name for name in members if name.startswith("t/")}
        return {"published": True, "archive_bytes": path.stat().st_size, "sha256": sha(path),
                "members": sorted(members), "referenced_contents": sorted(references),
                "unreferenced_content_members": sorted(content - references),
                "missing_referenced_contents": sorted(references - members),
                "root_extras": manifest["root"].get("extras", {}),
                "report": report, "geometry_reports": reports}


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--baseline-binary", type=pathlib.Path, required=True)
    p.add_argument("--injected-binary", type=pathlib.Path, required=True)
    p.add_argument("--publication-binary", type=pathlib.Path, required=True)
    p.add_argument("--evidence", type=pathlib.Path, required=True)
    p.add_argument("--work-parent", type=pathlib.Path, default=pathlib.Path.home() / ".cache")
    args = p.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=True)
    artifacts = args.evidence / "publication-vector-artifacts"
    artifacts.mkdir(exist_ok=True)
    env = os.environ.copy()
    env.pop("RT_AUDIT_ACCEPT_IO", None)
    env.pop("RT_AUDIT_DIRECTORY_RACE", None)
    evidence = {"baseline": BASE, "system": platform.platform(),
                "rustc": execute(["rustc", "--version"])["stdout"].strip(),
                "cargo": execute(["cargo", "--version"])["stdout"].strip(),
                "build": {"features": [], "jobs": 2, "dev_debug": 0},
                "binaries": {name: {"path": str(path), "sha256": sha(path)} for name, path in
                             [("baseline", args.baseline_binary), ("instrumented", args.injected_binary), ("publication", args.publication_binary)]},
                "probes": {}}
    with tempfile.TemporaryDirectory(prefix="rt-publication-vector-", dir=args.work_parent) as temporary:
        work = pathlib.Path(temporary)

        def fixture(name, features):
            path = work / f"{name}.geojson"
            path.write_text(json.dumps({"type": "FeatureCollection", "features": features}, separators=(",", ":")) + "\n")
            shutil.copyfile(path, artifacts / path.name)
            return path

        def point(identity, coordinates, properties):
            return {"type": "Feature", "id": identity, "properties": properties,
                    "geometry": {"type": "Point", "coordinates": coordinates}}

        def vector(binary, name, source, flags=(), inject=False):
            output = work / f"{name}.3tz"
            command = [binary, "--json", "vector", "-i", source, "-o", output, "--explicit",
                       "--reproducible", "--sourceCrs", "local", "--jobs", "1", "--lodLevels", "1", "--maxFeatures", "64", *flags]
            command_env = env.copy()
            if inject:
                command_env["RT_AUDIT_ACCEPT_IO"] = "1"
            result = execute(command, command_env)
            result["injected"] = inject
            result["source_sha256"] = sha(source)
            result["artifact"] = archive_inventory(output)
            if output.exists():
                shutil.copyfile(output, artifacts / output.name)
            return result

        # Control: all accepted content members remain referenced after cleanup.
        source = fixture("inventory", [point(i, [i % 5 * 10, i // 5 * 10, 0], {"value": i}) for i in range(20)])
        inventory = vector(args.baseline_binary, "inventory", source, ["--maxVertices", "4"])
        assert inventory["exit_code"] == 0
        assert inventory["artifact"]["unreferenced_content_members"] == []
        assert inventory["artifact"]["missing_referenced_contents"] == []
        evidence["probes"]["accepted_content_inventory_control"] = inventory

        # Natural encoded-byte rejection: root's real GLB exceeds 4096 bytes,
        # while two individually valid leaves remain. Finalization prunes candidates.
        source = fixture("encoded-budget", [point(i, [i * 100, 0, 0], {"payload": str(i) + "x" * 1500}) for i in range(2)])
        budget = vector(args.baseline_binary, "encoded-budget", source, ["--maxBytes", "4096"])
        assert budget["exit_code"] == 0
        assert budget["artifact"]["root_extras"]["routingReason"] == "bytes"
        assert len(budget["artifact"]["referenced_contents"]) == 2
        assert budget["artifact"]["unreferenced_content_members"] == []
        assert budget["artifact"]["missing_referenced_contents"] == []
        evidence["probes"]["rejected_encoded_candidate_cleanup_control"] = budget

        # Natural feature rollback: successful split side effects precede a failed fragment.
        polygon = {"type": "Feature", "id": "oversize", "properties": {"payload": "x" * 5000},
                   "geometry": {"type": "Polygon", "coordinates": [[[0, 0, 0], [10, 0, 0], [10, 10, 0], [0, 10, 0], [0, 0, 0]]]}}
        source = fixture("rollback", [polygon, point("valid", [20, 20, 0], {})])
        rollback = vector(args.baseline_binary, "rollback-skipped", source, ["--maxBytes", "4096", "--skipInvalid"])
        assert rollback["exit_code"] == 0
        report = rollback["artifact"]["report"]
        assert (report["features"], report["fragments"], report["skippedFeatures"], report["fragmentedPolygons"]) == (1, 1, 1, 1)
        assert len(rollback["artifact"]["geometry_reports"]) == 2
        evidence["probes"]["natural_feature_rollback_side_effects"] = rollback
        strict = vector(args.baseline_binary, "rollback-strict", source, ["--maxBytes", "4096"])
        assert strict["exit_code"] == 3 and not strict["artifact"]["published"]
        evidence["probes"]["natural_rollback_strict_control"] = strict

        # Synthetic ENOSPC returned from the real accept callback inside a savepoint.
        source = fixture("accept-io", [point("storage-failure", [0, 0, 0], {"audit_storage_failure": True}),
                                      point("valid", [20, 20, 0], {"audit_storage_failure": False})])
        control = vector(args.baseline_binary, "accept-io-control", source, ["--skipInvalid"])
        assert control["exit_code"] == 0 and control["artifact"]["report"]["features"] == 2
        evidence["probes"]["accept_io_baseline_control"] = control
        skipped = vector(args.injected_binary, "accept-io-skipped", source, ["--skipInvalid"], inject=True)
        assert skipped["exit_code"] == 0 and skipped["artifact"]["report"]["features"] == 1
        assert skipped["artifact"]["report"]["skippedFeatures"] == 1
        assert "No space left on device" in skipped["stderr"]
        evidence["probes"]["injected_accept_io_swallowed_by_skip_invalid"] = skipped
        strict = vector(args.injected_binary, "accept-io-strict", source, inject=True)
        assert strict["exit_code"] == 3 and not strict["artifact"]["published"]
        evidence["probes"]["injected_accept_io_reclassified_as_data"] = strict

        # Execute copied real private publisher, injecting competing mkdir after exists().
        directory = execute([args.publication_binary, "directory", work / "directory"])
        result = json.loads(directory["stdout"])
        assert directory["exit_code"] == 0 and result["success"]
        assert result["output_inode"] == result["staging_inode"]
        assert result["output_inode"] != int(result["competitor_inode"])
        assert result["output_contains_staged_tileset"] and not result["staging_exists_after"]
        directory["result"] = result
        evidence["probes"]["injected_directory_no_clobber_interleaving"] = directory
        archive = execute([args.publication_binary, "archive", work / "archive"])
        result = json.loads(archive["stdout"])
        assert archive["exit_code"] == 0 and not result["success"]
        assert result["error_category"] == "output_conflict"
        assert result["competitor_bytes_after"] == "competing archive writer"
        archive["result"] = result
        evidence["probes"]["archive_no_clobber_interleaving_control"] = archive

    destination = args.evidence / "publication-vector.json"
    destination.write_text(json.dumps(evidence, indent=2) + "\n")
    print(destination)


if __name__ == "__main__":
    main()
