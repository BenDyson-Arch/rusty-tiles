#!/usr/bin/env python3
"""Observe #113 API contract differences; never assert the undesirable behavior.

Uses only Python's standard library and the repository's locked Rust dependencies.
Compiles CLI, Python binding and a tiny library driver from the current checkout.
Produces a JSON observation report even if a conversion changes its behavior.
Build/fixture/driver errors still fail the process because they invalidate the audit.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import struct
import subprocess
import sys
import tempfile
import zipfile
import zlib


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
PRODUCTION_PATHS = ["src", "bindings", "build.rs", "Cargo.toml", "Cargo.lock"]


def run(command, *, env=None, capture=True):
    return subprocess.run(
        [str(value) for value in command], cwd=ROOT, env=env,
        text=True, capture_output=capture, check=True,
    )


def png():
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    pixels = b"".join(
        b"\0" + bytes(channel for x in range(4) for channel in (x * 70, y * 70, (x + y) * 30))
        for y in range(4)
    )
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 4, 4, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(pixels)) + chunk(b"IEND", b""))


def glb(points, textured=False):
    binary = struct.pack("<9f", *(value for point in points for value in point))
    document = {
        "asset": {"version": "2.0", "generator": "rusty-tiles architecture audit"},
        "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
        "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                       "min": [min(p[i] for p in points) for i in range(3)],
                       "max": [max(p[i] for p in points) for i in range(3)]}],
        "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": len(binary), "target": 34962}],
    }
    if textured:
        uv = struct.pack("<6f", 0, 0, 1, 0, 0, 1)
        document["bufferViews"].append({"buffer": 0, "byteOffset": len(binary), "byteLength": len(uv), "target": 34962})
        binary += uv
        document["accessors"].append({"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC2"})
        primitive = document["meshes"][0]["primitives"][0]
        primitive["attributes"]["TEXCOORD_0"] = 1
        primitive["material"] = 0
        image = png()
        document["bufferViews"].append({"buffer": 0, "byteOffset": len(binary), "byteLength": len(image)})
        binary += image
        document["images"] = [{"bufferView": 2, "mimeType": "image/png"}]
        document["textures"] = [{"source": 0}]
        document["materials"] = [{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}}]
    document["buffers"] = [{"byteLength": len(binary)}]
    encoded = json.dumps(document, separators=(",", ":")).encode()
    encoded += b" " * (-len(encoded) % 4)
    binary += b"\0" * (-len(binary) % 4)
    return (struct.pack("<4sII", b"glTF", 2, 28 + len(encoded) + len(binary))
            + struct.pack("<II", len(encoded), 0x4E4F534A) + encoded
            + struct.pack("<II", len(binary), 0x004E4942) + binary)


def archive(path):
    if not path.exists():
        return {"exists": False}
    result = {"exists": True, "bytes": path.stat().st_size}
    try:
        with zipfile.ZipFile(path) as contents:
            members = {name: contents.read(name) for name in sorted(contents.namelist())}
        result["zip_readable"] = True
        # Hash uncompressed members, including their names. ZIP timestamps/headers
        # do not influence the comparison; differing archive names also do not.
        digest = hashlib.sha256()
        for name, data in members.items():
            digest.update(name.encode() + b"\0" + struct.pack("<Q", len(data)) + data)
        result["logical_payload_sha256"] = digest.hexdigest()
        result["members"] = list(members)
        tileset = json.loads(members["tileset.json"])
        result["root_transform"] = tileset["root"].get("transform")
        codecs = []
        for name, data in members.items():
            if name.endswith(".glb"):
                size = struct.unpack_from("<I", data, 12)[0]
                doc = json.loads(data[20:20 + size])
                codecs.extend(image.get("mimeType", "unspecified") for image in doc.get("images", []))
        result["image_mime_types"] = sorted(set(codecs))
    except (OSError, KeyError, ValueError, struct.error, zipfile.BadZipFile) as error:
        result["inspection_error"] = f"{type(error).__name__}: {error}"
    return result


def python_conversion(function, source, output, **options):
    try:
        result = function(source, output, **options)
        observed = {"status": "returned_success", "archive": result.archive}
    except Exception as error:
        observed = {"status": "raised_exception", "exception": type(error).__name__, "message": str(error)}
    observed["output"] = archive(output)
    return observed


def cli_conversion(cli, command, source, output, *options):
    arguments = [str(cli), "--json", command, "-i", str(source), "-o", str(output), *options]
    completed = subprocess.run(arguments, cwd=ROOT, text=True, capture_output=True)
    return {"arguments": ["<built-cli>", *arguments[1:]], "exit_code": completed.returncode,
            "stdout": completed.stdout.strip(), "stderr": completed.stderr.strip(), "output": archive(output)}


def compare_pair(records, control, variant):
    left, right = records[control]["output"], records[variant]["output"]
    # Missing hashes must not make two failed conversions appear identical.
    comparable = "logical_payload_sha256" in left and "logical_payload_sha256" in right
    return {"control": control, "variant": variant, "comparable": comparable,
            "identical_logical_payload": (left["logical_payload_sha256"] == right["logical_payload_sha256"]) if comparable else None}


def production_snapshot(baseline):
    files = run(["git", "ls-files", "-z", "--", *PRODUCTION_PATHS]).stdout.split("\0")
    files = sorted(name for name in files if name)
    digest = hashlib.sha256()
    for name in files:
        data = (ROOT / name).read_bytes()
        digest.update(name.encode() + b"\0" + struct.pack("<Q", len(data)) + data)
    return {
        "paths": PRODUCTION_PATHS, "tracked_file_count": len(files),
        "working_files_sha256": digest.hexdigest(),
        "baseline_git_tree_sha256": hashlib.sha256(run(["git", "ls-tree", "-r", baseline, "--", *PRODUCTION_PATHS]).stdout.encode()).hexdigest(),
        "checkout_git_tree_sha256": hashlib.sha256(run(["git", "ls-tree", "-r", "HEAD", "--", *PRODUCTION_PATHS]).stdout.encode()).hexdigest(),
        "baseline_worktree_diff_files": run(["git", "diff", "--name-only", baseline, "--", *PRODUCTION_PATHS]).stdout.splitlines(),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target-dir", type=Path, default=Path.home() / ".cache/rusty-tiles-113-api-target")
    parser.add_argument("--results", type=Path, default=HERE / "results.json")
    args = parser.parse_args()
    target = args.target_dir.resolve()
    target.mkdir(parents=True, exist_ok=True)
    environment = os.environ.copy()
    build_environment = {"CARGO_TARGET_DIR": str(target), "CARGO_BUILD_JOBS": "2",
                         "CARGO_PROFILE_DEV_DEBUG": "0", "CARGO_PROFILE_DEV_INCREMENTAL": "false"}
    environment.update(build_environment)
    build_command = ["cargo", "build", "--locked", "--offline", "-p", "rusty-tiles", "-p", "rusty-tiles-python", "--features", "pyo3/extension-module"]
    run(build_command, env=environment, capture=False)
    serde = sorted((target / "debug/deps").glob("libserde_json-*.rlib"))
    if len(serde) != 1:
        raise RuntimeError(f"Expected one serde_json rlib in this dedicated target, found {len(serde)}")
    driver = target / "api-core-probe"
    driver_command = ["rustc", "--edition=2021", "-C", "debuginfo=0", HERE / "core_probe.rs",
                      "--extern", f"rusty_tiles={target / 'debug/librusty_tiles.rlib'}",
                      "--extern", f"serde_json={serde[0]}", "-L", f"dependency={target / 'debug/deps'}", "-o", driver]
    run(driver_command)
    binding = target / "debug/librusty_tiles.so"
    spec = importlib.util.spec_from_file_location("rusty_tiles", binding)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    records = {}
    with tempfile.TemporaryDirectory(prefix="api-probe-", dir=target) as temporary:
        work = Path(temporary)
        fixtures = work / "fixtures"
        fixtures.mkdir()
        fixture_bytes = {
            "local.glb": glb([[0, 0, 0], [1, 0, 0], [0, 1, 0]]),
            "geographic.glb": glb([[153, 10, 27], [153.0001, 10, 27], [153, 11, 27.0001]]),
            "textured.glb": glb([[0, 0, 0], [1, 0, 0], [0, 1, 0]], textured=True),
        }
        for name, data in fixture_bytes.items():
            (fixtures / name).write_bytes(data)
        core = json.loads(run([driver, fixtures, work]).stdout)
        for name, value in core.items():
            value["output"] = archive(work / f"core_{name}.3tz")
            records[f"core_{name}"] = value
        for name, offset in [("geographic_control", None), ("geographic_offset", [1000, 2000, 300])]:
            records[f"python_{name}"] = python_conversion(
                module.mesh_to_3tz, fixtures / "geographic.glb", work / f"python_{name}.3tz",
                source_crs="geographic", source_offset=offset, explicit=True, meshopt=False,
            )
        records["cli_geographic_offset"] = cli_conversion(
            target / "debug/rusty-tiles", "meshTo3tz", fixtures / "geographic.glb", work / "cli_geographic_offset.3tz",
            "--sourceCrs", "geographic", "--sourceOffset", "1000", "2000", "300",
        )
        for api in ["mesh", "glb"]:
            function = module.mesh_to_3tz if api == "mesh" else module.glb_to_3tz
            for name, rotation in [("rotation_control", None), ("rotation_only", [37, 11, 5])]:
                key = f"python_{api}_{name}"
                extra = {"explicit": True} if api == "mesh" else {}
                records[key] = python_conversion(function, fixtures / "local.glb", work / f"{key}.3tz", rotation=rotation, **extra)
            key = f"cli_{api}_rotation_only"
            records[key] = cli_conversion(
                target / "debug/rusty-tiles", "meshTo3tz" if api == "mesh" else "glbTo3tz",
                fixtures / "local.glb", work / f"{key}.3tz", "--rotationDegrees", "37", "11", "5",
            )
        records["python_default_codec"] = python_conversion(
            module.mesh_to_3tz, fixtures / "textured.glb", work / "python_default_codec.3tz",
            max_bytes=1, tile_size=64, explicit=True, meshopt=False,
        )
        records["cli_default_codec"] = cli_conversion(
            target / "debug/rusty-tiles", "meshTo3tz", fixtures / "textured.glb", work / "cli_default_codec.3tz",
            "--maxBytes", "1", "--tileSize", "64", "--explicit", "--noMeshopt",
        )
        for phase in ["first_event", "after_publication"]:
            key = f"python_callback_{phase}"
            output = work / f"{key}.3tz"
            events = []
            failure = RuntimeError(f"audit callback failure: {phase}")
            def callback(event):
                snapshot = {"event": event, "output_exists_during_callback": output.exists()}
                events.append(snapshot)
                if phase == "first_event" or str(event.get("message", "")).startswith("mesh-to-3tz: done "):
                    snapshot["raised"] = True
                    raise failure
            records[key] = python_conversion(
                module.mesh_to_3tz, fixtures / "textured.glb", output,
                max_bytes=1, tile_size=64, explicit=True, meshopt=False, callback=callback,
            )
            records[key]["events"] = events
            records[key]["validates_after_exception"] = None
            if output.exists():
                validation = subprocess.run([str(target / "debug/rusty-tiles"), "--json", "validate", str(output)], text=True, capture_output=True)
                records[key]["validates_after_exception"] = validation.returncode == 0
                records[key]["validation_stdout"] = validation.stdout.strip()
                records[key]["validation_stderr"] = validation.stderr.strip()
        comparisons = [compare_pair(records, f"{frontend}_geographic_control", f"{frontend}_geographic_offset") for frontend in ["core", "python"]]
        comparisons += [compare_pair(records, f"{frontend}_{api}_rotation_control", f"{frontend}_{api}_rotation_only") for frontend in ["core", "python"] for api in ["mesh", "glb"]]
        # Avoid persisting non-replayable temporary path names in recorded calls.
        for value in records.values():
            for field in ["arguments", "stdout", "stderr", "validation_stdout", "validation_stderr"]:
                if field in value:
                    if isinstance(value[field], list):
                        value[field] = [part.replace(str(work), "<probe-work>") for part in value[field]]
                    else:
                        value[field] = value[field].replace(str(work), "<probe-work>")
    baseline = run(["git", "log", "-1", "--format=%H", "--", *PRODUCTION_PATHS]).stdout.strip()
    report = {
        "schema_version": 1, "issue": 113,
        "baseline_commit": baseline,
        "checkout_commit": run(["git", "rev-parse", "HEAD"]).stdout.strip(),
        "cargo_lock_sha256": hashlib.sha256((ROOT / "Cargo.lock").read_bytes()).hexdigest(),
        "production_changes_in_worktree": run(["git", "diff", "--name-only", "HEAD", "--", *PRODUCTION_PATHS]).stdout.splitlines(),
        "production_snapshot": production_snapshot(baseline),
        "toolchain": {"rustc": run(["rustc", "-Vv"]).stdout.strip(), "cargo": run(["cargo", "-V"]).stdout.strip(),
                      "python": sys.version, "platform": platform.platform()},
        "build": {"cwd": str(ROOT), "environment": build_environment, "command": build_command,
                  "driver_command": [str(part) for part in driver_command], "binding_path": str(binding)},
        "built_artifact_sha256": {name: hashlib.sha256(path.read_bytes()).hexdigest() for name, path in {
            "cli": target / "debug/rusty-tiles", "python_binding": binding, "core_driver": driver,
        }.items()},
        "fixtures": {name: {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()} for name, data in fixture_bytes.items()},
        "desired_contracts": {
            "geographic_offset": "Uniformly reject an unsupported offset or apply documented offset semantics; do not silently discard it.",
            "rotation_without_placement": "Uniformly reject a rotation that cannot be applied without placement, or document and implement placement-independent rotation.",
            "default_codec": "Choose an intentional shared default or explicitly document frontend-specific fidelity and codec defaults.",
            "callback_failure": "Define whether callback failure cancels conversion or is an observer failure; expose publication status unambiguously to callers.",
        },
        "records": records, "comparisons": comparisons,
        "not_tested": ["Geographic general-CRS adapter with explicit source_axes/height_offset", "NaN/infinite placement", "Force overwrite of an existing output on callback failure", "Signal cancellation", "Windows/macOS bindings", "Native geospatial/JPEG builds"],
    }
    args.results.parent.mkdir(parents=True, exist_ok=True)
    args.results.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(f"Wrote {args.results}")
    for comparison in comparisons:
        print(f"{comparison['variant']}: identical logical payload={comparison['identical_logical_payload']}")
    for frontend in ["core", "python", "cli"]:
        print(f"{frontend} default MIME types: {records[f'{frontend}_default_codec']['output'].get('image_mime_types')}")
    for phase in ["first_event", "after_publication"]:
        item = records[f"python_callback_{phase}"]
        print(f"callback {phase}: {item['status']}; output exists={item['output']['exists']}; validates={item['validates_after_exception']}")


if __name__ == "__main__":
    main()
