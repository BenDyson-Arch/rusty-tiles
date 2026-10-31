#!/usr/bin/env python3
"""Independent fourteen-case literal-token primary controls, not a production oracle.

Only runs a supplied frozen CLI; no Cargo, installation or production imports.
Raw fixtures and stdout/stderr are retained in a fresh caller-owned external tree.
"""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import time
import zipfile


def sha(data):
    return hashlib.sha256(data).hexdigest()


BASE = '{"asset":{"version":"2.0"},"buffers":[{"byteLength":12}],"bufferViews":[{"buffer":0,"byteLength":12}],"accessors":[{"bufferView":0,"componentType":5126,"count":1,"type":"VEC3","min":[0,0,0],"max":[0,0,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":0}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0}'
VIEWLESS = '{"asset":{"version":"2.0"},"accessors":[{"componentType":5126,"count":1,"type":"VEC3","min":[0,0,0],"max":[0,0,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":0}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0}'
ARBITRARY = '"min":[17,-4,8],"max":[17,-4,8]'
ZERO_BOUNDS = '"min":[0,0,0],"max":[0,0,0]'
# Literal substitutions retain authored decimal/exponent tokens exactly.
CASES = [
    ("stored-zero-control", BASE, "glb", "admitted", "Core stored f32 zero POINTS control."),
    ("count-decimal", BASE.replace('"count":1', '"count":1.0'), "glb", "admitted", "Integer count encoded as zero-fraction decimal."),
    ("component-exponent", BASE.replace('"componentType":5126', '"componentType":5.126e3'), "glb", "admitted", "Integer componentType encoded as exponent."),
    ("position-reference-exponent", BASE.replace('"POSITION":0', '"POSITION":0e0'), "glb", "admitted", "Nonnegative integer accessor index encoded as exponent."),
    ("buffer-length-decimal", BASE.replace('"byteLength":12}', '"byteLength":12.0}', 1), "glb", "admitted", "Integer buffer byteLength encoded as zero-fraction decimal."),
    ("local-gltf-integer-representations", BASE.replace('"byteLength":12', '"byteLength":12e0').replace('"buffer":0', '"buffer":0.0').replace('"bufferView":0', '"bufferView":0e0').replace('"componentType":5126', '"componentType":5126.0').replace('"count":1', '"count":1e0').replace('"POSITION":0', '"POSITION":0.0').replace('"mesh":0', '"mesh":0e0').replace('"scene":0', '"scene":0.0').replace('"nodes":[0]', '"nodes":[0.0]').replace('"mode":0', '"mode":0e0').replace('"byteLength":12e0}', '"byteLength":12e0,"uri":"zero.bin"}', 1), "gltf", "admitted", "Archive-local glTF control with literal integer-valued decimal/exponent properties."),
    ("count-fractional", BASE.replace('"count":1', '"count":1.5'), "glb", "invalid_input", "Nonzero fractional integer count is forbidden."),
    ("position-reference-fractional", BASE.replace('"POSITION":0', '"POSITION":0.5'), "glb", "invalid_input", "Nonzero fractional accessor index is forbidden."),
    ("component-fractional", BASE.replace('"componentType":5126', '"componentType":5126.5'), "glb", "invalid_input", "Nonzero fractional componentType is forbidden."),
    ("count-fraction-rounds-to-integer", BASE.replace('"count":1', '"count":1.0000000000000001'), "glb", "invalid_input", "Exact nonzero fractional count is forbidden even when f64 rounds it to 1."),
    ("count-fraction-exponent-rounds-to-integer", BASE.replace('"count":1', '"count":10000000000000001e-16'), "glb", "invalid_input", "Exact exponent count is 1.0000000000000001 and remains fractional despite f64 rounding."),
    ("viewless-zero-control", VIEWLESS, "glb", "admitted", "Viewless non-sparse accessor initializes one f32 VEC3 to zero."),
    ("viewless-arbitrary-bounds", VIEWLESS.replace(ZERO_BOUNDS, ARBITRARY), "glb", "admitted", "No sparse/view: primary explicitly permits arbitrary min/max independently of initialized zero values."),
    ("stored-zero-arbitrary-bounds", BASE.replace(ZERO_BOUNDS, ARBITRARY), "glb", "invalid_input", "Same arbitrary bounds on stored zero bytes violate actual binary extrema agreement."),
]


def glb(text, binary):
    raw = text.encode("utf-8")
    raw += b" " * (-len(raw) % 4)
    chunks = struct.pack("<II", len(raw), 0x4E4F534A) + raw
    if binary is not None:
        chunks += struct.pack("<II", len(binary), 0x004E4942) + binary
    return struct.pack("<4sII", b"glTF", 2, 12 + len(chunks)) + chunks


def archive(members):
    stream = io.BytesIO()
    with zipfile.ZipFile(stream, "w", compression=zipfile.ZIP_STORED) as out:
        offsets = []
        for name, raw in members:
            offsets.append((hashlib.md5(name.encode()).digest(), stream.tell()))
            info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_STORED
            out.writestr(info, raw)
        # 3TZ hash ordering is by little-endian low/high u64, not byte order.
        offsets.sort(key=lambda pair: struct.unpack("<QQ", pair[0]))
        index = b"".join(digest + struct.pack("<Q", offset) for digest, offset in offsets)
        info = zipfile.ZipInfo("@3dtilesIndex1@", (1980, 1, 1, 0, 0, 0))
        info.compress_type = zipfile.ZIP_STORED
        out.writestr(info, index)
    return stream.getvalue()


def snapshot(root):
    return {str(path.relative_to(root)): sha(path.read_bytes())
            for path in sorted(root.rglob("*")) if path.is_file()}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--work", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--source-pin", type=Path, required=True)
    args = parser.parse_args()
    expected_binary = "3328487c2920e07b291eebec8072954f11b0d20a39181ccbc0c4287e2f765524"
    expected_source_pin = "913e15540cfe4b387905fdaf00aff270614285ba88d9e12acfe4ffccaa18aa13"
    assert sha(args.binary.read_bytes()) == expected_binary
    assert sha(args.source_pin.read_bytes()) == expected_source_pin
    fixtures = args.work / "fixtures"
    raw = args.work / "raw"
    fixtures.mkdir(exist_ok=False)
    raw.mkdir(exist_ok=False)
    launcher = args.work / "controlled-launch.py"
    launcher.write_text('''import json, os, sys
os.setpriority(os.PRIO_PROCESS, 0, 10)
available = sorted(os.sched_getaffinity(0))
os.sched_setaffinity(0, set(available[:2]))
os.environ.update(RAYON_NUM_THREADS="2", RUST_TEST_THREADS="2", CARGO_BUILD_JOBS="2")
with open(sys.argv[1], "w") as record:
    json.dump({"nice":os.getpriority(os.PRIO_PROCESS,0), "affinity":sorted(os.sched_getaffinity(0)), "rayon":"2"}, record)
os.execvpe(sys.argv[2], sys.argv[2:], os.environ)
''')
    receipts = []
    assert len(CASES) == 14
    for label, text, kind, expectation, rationale in CASES:
        case = fixtures / label
        case.mkdir()
        (case / "literal-source.json").write_bytes(text.encode())
        json.loads(text)  # syntax sanity only; never reserializes authored tokens
        content_name = "point." + kind
        members = [("tileset.json", ('{"asset":{"version":"1.1"},"geometricError":0,"root":{"boundingVolume":{"box":[0,0,0,10,0,0,0,10,0,0,0,10]},"geometricError":0,"content":{"uri":"' + content_name + '"}}}').encode())]
        if kind == "gltf":
            members += [(content_name, text.encode()), ("zero.bin", b"\0" * 12)]
        else:
            members += [(content_name, glb(text, None if label.startswith("viewless") else b"\0" * 12))]
        for name, data in members:
            (case / name).write_bytes(data)
        data = archive(members)
        path = case / "input.3tz"
        path.write_bytes(data)
        # Validate ZIP structure/CRCs separately from target; primary truth remains
        # literal declarations + analytical zero values, not parser agreement.
        with zipfile.ZipFile(io.BytesIO(data)) as check:
            assert check.testzip() is None
        before = snapshot(fixtures)
        runtime_file = raw / (label + ".runtime.json")
        command = [sys.executable, str(launcher), str(runtime_file), str(args.binary), "--json", "validate", str(path)]
        started = time.monotonic()
        result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10, check=False)
        elapsed = time.monotonic() - started
        (raw / (label + ".stdout")).write_bytes(result.stdout)
        (raw / (label + ".stderr")).write_bytes(result.stderr)
        after = snapshot(fixtures)
        try:
            response = json.loads(result.stdout)
        except (ValueError, UnicodeError):
            response = None
        observed = "admitted" if result.returncode == 0 else (response or {}).get("error", {}).get("code", "unknown")
        receipt = {
            "label": label, "expectation": expectation, "rationale": rationale,
            "observed": observed, "exitCode": result.returncode, "elapsedSeconds": elapsed,
            "command": command, "execution": json.loads(runtime_file.read_text()),
            "readOnly": before == after, "archiveSha256": sha(data), "archiveBytes": len(data),
            "memberSha256": {name: sha(value) for name, value in members},
            "literalSourceSha256": sha(text.encode()),
            "stdoutSha256": sha(result.stdout), "stderrSha256": sha(result.stderr), "response": response,
            "independentDecodedPosition": {"values": [[0.0, 0.0, 0.0]], "method": "Viewless core zero initialization" if label.startswith("viewless") else "Little-endian f32 zero bytes", "storedBytes": None if label.startswith("viewless") else "000000000000000000000000", "declaredBoundsAreSeparate": True},
        }
        receipts.append(receipt)
        print(json.dumps({"label": label, "observed": observed, "expected": expectation, "readOnly": receipt["readOnly"]}), flush=True)
    report = {
        "schemaVersion": 1, "scope": "Finite independent primary controls; no full C1, A2, release or universal glTF conformance acceptance.",
        "sourcePinSha256": expected_source_pin, "sourcePin": json.loads(args.source_pin.read_text()),
        "binarySha256": expected_binary, "driverSha256": sha(Path(__file__).read_bytes()),
        "launcherSha256": sha(launcher.read_bytes()), "primaryPin": json.loads((args.work / "primary-pin.json").read_text()),
        "executions": len(receipts), "receipts": receipts,
        "fixtureSnapshot": snapshot(fixtures), "rawSnapshot": snapshot(raw),
    }
    (args.work / "receipt.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
