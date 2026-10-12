#!/usr/bin/env python3
"""Independent small-input H1 source/hierarchy reference and sensitive controls.

Truth comes from generated glTF inputs and design v6.1 section 2.4, rather than
the producer's sorted records or streaming stack. The reference intentionally
holds the complete tiny mesh and recursively partitions it in memory. Production
uses bounded streams; this program is not a memory/performance implementation.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import heapq
import json
import math
import os
from pathlib import Path
import random
import struct
import subprocess
import tempfile
import unittest


KEY_DEPTH = 21


def f32(value):
    rounded = struct.unpack("<f", struct.pack("<f", value))[0]
    return 0.0 if rounded == 0 else rounded


def bits(point):
    return tuple(struct.unpack("<I", struct.pack("<f", value))[0] for value in point)


def bounds(triangles):
    positions = [p for t in triangles for p in t["positions"]]
    assert positions, "empty geometry has no bounds"
    return {"min": [min(p[i] for p in positions) for i in range(3)],
            "max": [max(p[i] for p in positions) for i in range(3)]}


def morton(point, origin, side):
    if side == 0:
        return 0
    cells = [min((1 << KEY_DEPTH) - 1,
                 max(0, math.floor((point[a] - origin[a]) / side * (1 << KEY_DEPTH))))
             for a in range(3)]
    # Direct bit interleaving is independent of the producer's spreading masks.
    return sum(((cells[a] >> bit) & 1) << (3 * bit + a)
               for bit in range(KEY_DEPTH) for a in range(3))


def sorted_source(source):
    box = bounds(source)
    origin = box["min"]
    side = max(box["max"][a] - origin[a] for a in range(3))
    result = copy.deepcopy(source)
    for triangle in result:
        centroid = [sum(p[a] for p in triangle["positions"]) / 3 for a in range(3)]
        triangle["key"] = morton(centroid, origin, side)
        triangle["degenerate"] = len({bits(p) for p in triangle["positions"]}) < 3
    return sorted(result, key=lambda t: (t["key"], tuple(t["source"])))


def hierarchy(source, leaf_triangles, tile_bytes):
    """Recursive specification: maximal admissible cells, packed leaf octants.

    A tree is represented by nodes with explicit children, so recursive structure
    establishes parentage independently of the producer's emitted rank records.
    """
    ordered = sorted_source(source)

    def fits(first, end):
        return end - first <= leaf_triangles and sum(t["bytes"] for t in ordered[first:end]) <= tile_bytes

    def node(first, end, identity, children=None, last_octant=None):
        depth, cell, path_len, path = identity
        return {"identity": list(identity), "depth": depth + path_len,
                "kind": "internal" if children else "leaf",
                "first": first, "end": end,
                "bytes": sum(t["bytes"] for t in ordered[first:end]),
                "bounds": bounds(ordered[first:end]),
                "last_octant": last_octant if last_octant is not None else
                    ((path & 7) if path_len else (cell & 7)),
                "children": children or []}

    def runs(first, end, depth, cell):
        ranges = []
        cursor = first
        while cursor < end:
            stop = cursor + 1
            assert fits(cursor, stop), "single triangle exceeds tile budget"
            while stop < end and fits(cursor, stop + 1):
                stop += 1
            ranges.append((cursor, stop))
            cursor = stop
        height = 1
        while 8 ** height < len(ranges):
            height += 1

        def group(lo, hi, level, path):
            identity = (depth, cell, level, path)
            if level == height:
                assert hi == lo + 1
                return node(*ranges[lo], identity)
            block = 8 ** (height - level - 1)
            children = [group(start, min(start + block, hi), level + 1,
                              path * 8 + slot)
                        for slot, start in enumerate(range(lo, hi, block))]
            return node(ranges[lo][0], ranges[hi - 1][1], identity, children)
        return group(0, len(ranges), 0, 0)

    def cell(first, end, depth, prefix):
        identity = (depth, prefix, 0, 0)
        if fits(first, end):
            return node(first, end, identity)
        if depth == KEY_DEPTH:
            return runs(first, end, depth, prefix)
        shift = 3 * (KEY_DEPTH - depth - 1)
        children = []
        cursor = first
        while cursor < end:
            slot = (ordered[cursor]["key"] >> shift) & 7
            stop = cursor + 1
            while stop < end and (ordered[stop]["key"] >> shift) & 7 == slot:
                stop += 1
            children.append(cell(cursor, stop, depth + 1, prefix * 8 + slot))
            cursor = stop
        packed = []
        for child in children:
            if (packed and packed[-1]["kind"] == child["kind"] == "leaf"
                    and fits(packed[-1]["first"], child["end"])):
                previous = packed.pop()
                packed.append(node(previous["first"], child["end"],
                                   tuple(previous["identity"]),
                                   last_octant=child["last_octant"]))
            else:
                packed.append(child)
        return node(first, end, identity, packed)

    root = cell(0, len(ordered), 0, 0)
    flat = []

    def visit(n):
        children = n["children"]
        row = {k: copy.deepcopy(v) for k, v in n.items() if k != "children"}
        row["children"] = len(children)
        flat.append(row)
        for child in children:
            visit(child)
    visit(root)
    vertex_depth = {}
    depth_boxes = {}
    maximum_extents = {}
    for n in flat:
        points = [n["bounds"]["min"], n["bounds"]["max"]]
        existing = depth_boxes.setdefault(n["depth"], {"min": list(points[0]), "max": list(points[1])})
        maximum_extents[n["depth"]] = max(maximum_extents.get(n["depth"], 0),
                    max(points[1][a] - points[0][a] for a in range(3)))
        for a in range(3):
            existing["min"][a] = min(existing["min"][a], points[0][a])
            existing["max"][a] = max(existing["max"][a], points[1][a])
        if n["kind"] == "leaf":
            for triangle in ordered[n["first"]:n["end"]]:
                for point in triangle["positions"]:
                    key = bits(point)
                    vertex_depth[key] = min(vertex_depth.get(key, n["depth"]), n["depth"])
    return {"triangles": ordered, "nodes": flat,
            "vertex_depth": [[*key, depth] for key, depth in sorted(vertex_depth.items())],
            "depth_bounds": [[depth, box, maximum_extents[depth]] for depth, box in sorted(depth_boxes.items())]}


def compare(expected, actual):
    # Separate assertions identify the owning invariant. No candidate field is
    # used to derive source truth, tree shape, rank assignments or depth minima.
    assert actual["triangles"] == expected["triangles"], "source provenance/positions/Morton order mismatch"
    assert actual["nodes"] == expected["nodes"], "cell tree identity/packing/ranges/boxes mismatch"
    assert actual["vertex_depth"] == expected["vertex_depth"], "exact all-corner welded min depth mismatch"
    assert actual["depth_bounds"] == expected["depth_bounds"], "depth union bounds mismatch"
    leaves = [n for n in actual["nodes"] if n["kind"] == "leaf"]
    ranks = [rank for n in leaves for rank in range(n["first"], n["end"])]
    assert sorted(ranks) == list(range(len(expected["triangles"]))), "source rank must occur in exactly one leaf"
    ids = [tuple(n["identity"]) for n in actual["nodes"]]
    assert len(ids) == len(set(ids)), "duplicate node/run identity"


def fixed_records(path, format_string):
    codec = struct.Struct(format_string)
    raw = Path(path).read_bytes()
    assert len(raw) % codec.size == 0, f"partial diagnostic record in {path}"
    return codec.iter_unpack(raw)


def decode_dump(directory):
    """Decode the declared diagnostic wire interface, never producer helpers."""
    directory = Path(directory)
    triangles = []
    for row in fixed_records(directory / "triangles.bin", "<QIIIQ3Q9fIB"):
        assert row[18] in (0, 1), "invalid degenerate flag"
        triangles.append({"key": row[0], "source": list(row[1:5]),
                          "vertices": list(row[5:8]),
                          "positions": [list(row[a:a + 3]) for a in (8, 11, 14)],
                          "bytes": row[17], "degenerate": bool(row[18])})
    nodes = []
    for row in fixed_records(directory / "nodes.bin", "<BQBQBBBQQQ6f3x"):
        assert row[5] in (0, 1), "invalid node kind"
        nodes.append({"identity": list(row[:4]), "depth": row[0] + row[2],
                      "last_octant": row[4], "kind": ("internal", "leaf")[row[5]],
                      "children": row[6], "first": row[7], "end": row[8],
                      "bytes": row[9],
                      "bounds": {"min": list(row[10:13]), "max": list(row[13:16])}})
    vertex_depth = [list(row) for row in fixed_records(directory / "vertex_depth.bin", "<IIIB")]
    info = json.loads((directory / "info.json").read_text())
    per_depth = info.get("per_depth", info.get("tree", {}).get("per_depth"))
    assert per_depth is not None, "info.json must expose tree per_depth summaries"
    return {"triangles": triangles, "nodes": nodes, "vertex_depth": vertex_depth,
            "depth_bounds": [[row["depth"], row["bounds"], row["maximum_extent"]] for row in per_depth]}


def verify_fixture(truth, directory, leaf_triangles, tile_bytes):
    expected = hierarchy(json.loads(Path(truth).read_text()), leaf_triangles, tile_bytes)
    actual = decode_dump(directory)
    compare(expected, actual)
    summary = {"triangles": len(expected["triangles"]), "nodes": len(expected["nodes"]),
            "leaves": sum(n["kind"] == "leaf" for n in expected["nodes"]),
            "height": max(n["depth"] for n in expected["nodes"]) + 1,
            "welded_vertices": len(expected["vertex_depth"])}
    info = json.loads((Path(directory) / "info.json").read_text())
    for key, value in summary.items():
        assert info["tree"][key] == value, f"independent {key} count mismatch"
    assert info["tree"]["internal"] == summary["nodes"] - summary["leaves"], "internal count mismatch"
    assert info["tree"]["degenerate_cells"] == sum(n["identity"][0] == 21 and
                n["identity"][2] == 0 and n["kind"] == "internal" for n in expected["nodes"]), "degenerate cell count mismatch"
    source_bounds = bounds(expected["triangles"])
    assert info["source_bounds"] == source_bounds, "source box mismatch"
    assert info["cube"] == {"origin": source_bounds["min"],
                "side": max(source_bounds["max"][a] - source_bounds["min"][a] for a in range(3))}, "source cube mismatch"
    assert info["geometry_bytes"] == sum(t["bytes"] for t in expected["triangles"]), "geometry charge mismatch"
    return summary


def projected_geometry_bytes(normal=False, tangent=False, uv_sets=0, color=False):
    # H1 geometry-only nonindexed projected member share. Texture pages and
    # material/projected UV footprint estimates are downstream obligations.
    return 3 * (12 + 12 * normal + 16 * tangent + 8 * uv_sets + 16 * color) + 20


def primitive_triangles(positions, indices=None, node=0, mesh=0, primitive=0,
                        matrix=None, projected=None):
    matrix = matrix or [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]
    # Column-major glTF matrix, applied in f64 then rounded once to stored f32.
    def transform(p):
        return [f32(sum(matrix[a + 4 * b] * p[b] for b in range(3)) + matrix[a + 12])
                for a in range(3)]
    a = [[matrix[row + 4 * col] for col in range(3)] for row in range(3)]
    det = sum(a[0][col] * (a[1][(col + 1) % 3] * a[2][(col + 2) % 3]
              - a[1][(col + 2) % 3] * a[2][(col + 1) % 3]) for col in range(3))
    indices = indices if indices is not None else list(range(len(positions)))
    truth = []
    for tri in range(len(indices) // 3):
        vertices = indices[tri * 3:tri * 3 + 3]
        if det < 0:
            vertices = [vertices[0], vertices[2], vertices[1]]
        truth.append({"source": [node, mesh, primitive, tri],
                      "vertices": vertices, "positions": [transform(positions[v]) for v in vertices],
                      "bytes": projected if projected is not None else projected_geometry_bytes()})
    return truth


def write_fixture(directory, name, positions, indices=None, component=5125,
                  stride=12, matrix=None, instances=1, glb=False, full_attributes=False):
    directory = Path(directory) / name
    directory.mkdir(parents=True, exist_ok=True)
    positions = [[f32(value) for value in p] for p in positions]
    payload = bytearray()
    for p in positions:
        payload.extend(struct.pack("<fff", *p))
        payload.extend(bytes(stride - 12))
    views = [{"buffer": 0, "byteOffset": 0, "byteLength": len(payload), "byteStride": stride}]
    accessors = [{"bufferView": 0, "componentType": 5126, "count": len(positions),
                  "type": "VEC3", "min": [min(p[a] for p in positions) for a in range(3)],
                  "max": [max(p[a] for p in positions) for a in range(3)]}]
    primitive = {"attributes": {"POSITION": 0}, "mode": 4}
    if indices is not None:
        while len(payload) % 4:
            payload.append(0)
        start = len(payload)
        fmt = {5121: "B", 5123: "H", 5125: "I"}[component]
        payload.extend(struct.pack("<" + fmt * len(indices), *indices))
        views.append({"buffer": 0, "byteOffset": start, "byteLength": len(payload) - start})
        accessors.append({"bufferView": 1, "componentType": component,
                          "count": len(indices), "type": "SCALAR"})
        primitive["indices"] = 1
    if full_attributes:
        attributes = {"NORMAL": ("VEC3", [0, 0, 1]), "TANGENT": ("VEC4", [1, 0, 0, 1]),
                      "TEXCOORD_0": ("VEC2", [0, 0]), "TEXCOORD_1": ("VEC2", [.25, .5]),
                      "COLOR_0": ("VEC4", [.2, .3, .4, 1])}
        for name, (shape, values) in attributes.items():
            while len(payload) % 4:
                payload.append(0)
            start = len(payload)
            payload.extend(struct.pack("<" + "f" * len(values), *values) * len(positions))
            primitive["attributes"][name] = len(accessors)
            accessors.append({"bufferView": len(views), "componentType": 5126,
                              "count": len(positions), "type": shape})
            views.append({"buffer": 0, "byteOffset": start, "byteLength": len(payload) - start})
    doc = {"asset": {"version": "2.0"}, "scene": 0,
           "scenes": [{"nodes": list(range(instances))}],
           "nodes": [{"mesh": 0, **({"matrix": matrix} if matrix else {})} for _ in range(instances)],
           "meshes": [{"primitives": [primitive]}],
           "buffers": [{"byteLength": len(payload)}], "bufferViews": views, "accessors": accessors}
    if glb:
        raw = json.dumps(doc, separators=(",", ":")).encode()
        raw += b" " * (-len(raw) % 4)
        padded = payload + bytes(-len(payload) % 4)
        envelope = struct.pack("<III", 0x46546C67, 2, 12 + 8 + len(raw) + 8 + len(padded))
        path = directory / "source.glb"
        path.write_bytes(envelope + struct.pack("<II", len(raw), 0x4E4F534A) + raw
                         + struct.pack("<II", len(padded), 0x004E4942) + padded)
    else:
        doc["buffers"][0]["uri"] = "payload.bin"
        (directory / "payload.bin").write_bytes(payload)
        path = directory / "source.gltf"
        path.write_text(json.dumps(doc, separators=(",", ":")) + "\n")
    truth = []
    for instance in range(instances):
        truth.extend(primitive_triangles(positions, indices, node=instance, matrix=matrix,
                        projected=projected_geometry_bytes(True, True, 2, True) if full_attributes else None))
    (directory / "truth.json").write_text(json.dumps(truth, separators=(",", ":")) + "\n")
    return path, truth


def corpus(directory):
    cases = []
    def add(name, positions, **kwargs):
        path, truth = write_fixture(directory, name, positions, **kwargs)
        cases.append({"name": name, "input": str(path), "truth": str(path.parent / "truth.json"),
                      "triangles": len(truth)})
    triangle = [[0, 0, 0], [1, 0, 0], [0, 1, 0]]
    add("single", triangle)
    for component in (5121, 5123, 5125):
        add(f"indexed-{component}", triangle, indices=[2, 0, 1] * 19, component=component,
            stride=20, glb=component == 5123)
    add("reflected-instances", triangle, indices=[0, 1, 2] * 7, instances=3,
        matrix=[-2, 0, 0, 0, 0, 3, 0, 0, 0, 0, 1, 0, 13, -5, 7, 1])
    add("attribute-byte-charge", triangle, indices=[0, 1, 2] * 7, full_attributes=True)
    add("coincident-runs", triangle, indices=[0, 1, 2] * 137)
    add("degenerates", [[0, 0, -0.0], [0, 0, 0], [1, 0, 0],
                        [0, 0, 0], [1, 0, 0], [2, 0, 0]])
    add("f32-collapse", triangle,
        matrix=[1e-9, 0, 0, 0, 0, 1e-9, 0, 0, 0, 0, 1e-9, 0, 1, 1, 1, 1])
    # Independent analytic parent-child composition, plus an unselected mesh
    # instance. Source tuple must use the actual selected node index.
    path, _ = write_fixture(directory, "nested-selected", triangle)
    doc = json.loads(path.read_text())
    doc["nodes"] = [{"translation": [8, -4, 3], "children": [1]},
                    {"mesh": 0, "scale": [-2, 3, 1]}, {"mesh": 0}]
    doc["scenes"] = [{"nodes": [2]}, {"nodes": [0]}]
    doc["scene"] = 1
    path.write_text(json.dumps(doc, separators=(",", ":")) + "\n")
    truth = primitive_triangles(triangle, node=1,
                matrix=[-2, 0, 0, 0, 0, 3, 0, 0, 0, 0, 1, 0, 8, -4, 3, 1])
    (path.parent / "truth.json").write_text(json.dumps(truth, separators=(",", ":")) + "\n")
    cases.append({"name": "nested-selected", "input": str(path),
                  "truth": str(path.parent / "truth.json"), "triangles": len(truth)})
    rng = random.Random(651203)
    positions = []
    for _ in range(173):
        x, y, z = [rng.randrange(0, 32) for _ in range(3)]
        positions.extend([[x, y, z], [x + 0.25, y, z], [x, y + 0.25, z + 0.125]])
    add("octants", positions)
    # Shared origin has far more than sixteen incidents. Different triangle
    # extents place those corners' containing leaves at different depths.
    positions = []
    for index in range(39):
        extent = 0.125 if index < 38 else 128
        positions.extend([[0, 0, 0], [extent, 0, 0], [0, extent, 0]])
    add("all-incidents-min", positions)
    (Path(directory) / "corpus.json").write_text(json.dumps(cases, indent=2) + "\n")
    return cases


def grid_template(cells=32):
    positions = [[float(x), float(y), 0.] for y in range(cells + 1) for x in range(cells + 1)]
    indices = []
    for y in range(cells):
        for x in range(cells):
            a = y * (cells + 1) + x
            b, c, d = a + 1, a + cells + 2, a + cells + 1
            indices.extend([a, b, c, a, c, d])
    return positions, indices


def generate_scale(directory, triangles=10_000_000):
    """Write only bounded template blocks; no input-sized Python structures."""
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    positions, indices = grid_template()
    template_count = len(indices) // 3
    with (directory / "payload.bin").open("wb") as output:
        output.write(b"".join(struct.pack("<fff", *p) for p in positions))
        block = struct.pack("<" + "I" * len(indices), *indices)
        whole, tail = divmod(triangles, template_count)
        for _ in range(whole):
            output.write(block)
        output.write(block[:tail * 12])
    position_bytes = len(positions) * 12
    doc = {"asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}],
           "nodes": [{"mesh": 0}], "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
           "buffers": [{"uri": "payload.bin", "byteLength": position_bytes + triangles * 12}],
           "bufferViews": [{"buffer": 0, "byteLength": position_bytes},
                           {"buffer": 0, "byteOffset": position_bytes, "byteLength": triangles * 12}],
           "accessors": [{"bufferView": 0, "componentType": 5126, "type": "VEC3", "count": len(positions),
                          "min": [0, 0, 0], "max": [32, 32, 0]},
                         {"bufferView": 1, "componentType": 5125, "type": "SCALAR", "count": triangles * 3}]}
    path = directory / "source.gltf"
    path.write_text(json.dumps(doc, separators=(",", ":")) + "\n")
    (directory / "scale-truth.json").write_text(json.dumps({"triangles": triangles, "cells": 32,
                        "template_triangles": template_count, "positions": len(positions)}, indent=2) + "\n")
    return path


def generate_dense(directory, nodes=65_536):
    path, _ = write_fixture(directory, "dense-plan", [[0, 0, 0], [1, 0, 0], [0, 1, 0]])
    doc = json.loads(path.read_text())
    doc["nodes"] = [{"mesh": 0}] + [{} for _ in range(nodes - 1)]
    path.write_text(json.dumps(doc, separators=(",", ":")) + "\n")
    return path


def digest(path):
    result = hashlib.sha256()
    with Path(path).open("rb") as source:
        while raw := source.read(1 << 20):
            result.update(raw)
    return result.hexdigest()


def run_corpus(binary, directory, label):
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    cases = corpus(directory / "corpus")
    scratch = [directory / "scratch-a", directory / "scratch-b"]
    for parent in scratch:
        parent.mkdir(exist_ok=True)
    configurations = [(1, 1024, 2, 17, 0, 5, 280), (2, 2048, 3, 31, 0, 5, 280),
                      (1, 8192, 8, 257, 1, 5, 280), (2, 256, 2, 17, 1, 5, 280),
                      (1, 1024, 2, 17, 0, 5, 1_000_000),
                      (1, 1024, 2, 17, 0, 1_000, 112)]
    receipts = {"label": label, "binary": str(Path(binary).resolve()), "binary_sha256": digest(binary),
                "oracle_sha256": digest(__file__), "runs": []}
    cpus = sorted(os.sched_getaffinity(0))[:2] if hasattr(os, "sched_getaffinity") else []
    prefix = ["nice", "-n", "10"]
    if cpus:
        prefix += ["taskset", "-c", ",".join(map(str, cpus))]
    for case in cases:
        previous = {}
        for variant, (jobs, run_bytes, fan_in, buffer_bytes, scratch_index, leaf_triangles, tile_bytes) in enumerate(configurations):
            if variant == 5:
                tile_bytes = 2 * max(t["bytes"] for t in json.loads(Path(case["truth"]).read_text()))
            output = directory / f"{case['name']}-{variant}"
            env = dict(os.environ, H1_INPUT=case["input"], H1_OUTPUT=str(output),
                       H1_SCRATCH=str(scratch[scratch_index]), H1_JOBS=str(jobs),
                       H1_RUN_BYTES=str(run_bytes), H1_FAN_IN=str(fan_in),
                       H1_BUFFER_BYTES=str(buffer_bytes), H1_PROFILE="quality",
                       H1_LEAF_TRIANGLES=str(leaf_triangles), H1_TILE_BYTES=str(tile_bytes))
            result = subprocess.run(prefix + [str(binary), "--exact", "mesh_large::tests::external_driver",
                                              "--ignored", "--nocapture"], env=env, capture_output=True, timeout=90)
            output.mkdir(exist_ok=True)
            (output / "driver.log").write_bytes(result.stdout + result.stderr)
            assert result.returncode == 0, f"driver failed for {case['name']} variant {variant}: {result.stdout.decode()} {result.stderr.decode()}"
            summary = verify_fixture(case["truth"], output, leaf_triangles, tile_bytes)
            hashes = {name: digest(output / name) for name in ("triangles.bin", "nodes.bin", "vertex_depth.bin", "info.json")}
            budget = (leaf_triangles, tile_bytes)
            if budget in previous:
                assert hashes == previous[budget], f"configuration-dependent dump for {case['name']}"
            previous[budget] = hashes
            receipts["runs"].append({"case": case["name"], "variant": variant, "config": {key: value for key, value in env.items() if key.startswith("H1_")},
                                      "summary": summary, "hashes": hashes})
        print(f"accepted {case['name']}: {summary}", flush=True)
    (directory / "receipt.json").write_text(json.dumps(receipts, indent=2) + "\n")
    return len(receipts["runs"])


def run_refusals(binary, directory, label):
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    single, _ = write_fixture(directory, "single", [[0, 0, 0], [1, 0, 0], [0, 1, 0]])
    coincident, _ = write_fixture(directory, "coincident", [[0, 0, 0], [1, 0, 0], [0, 1, 0]], indices=[0, 1, 2] * 137)
    missing, _ = write_fixture(directory, "missing-resource", [[0, 0, 0], [1, 0, 0], [0, 1, 0]])
    (missing.parent / "payload.bin").unlink()
    invalid, _ = write_fixture(directory, "bad-extrema", [[0, 0, 0], [1, 0, 0], [0, 1, 0]])
    doc = json.loads(invalid.read_text())
    doc["accessors"][0]["max"][0] = 2
    invalid.write_text(json.dumps(doc, separators=(",", ":")) + "\n")
    cases = [("single-byte-refusal", single, {"H1_TILE_BYTES": "55"}, "Unsupported", "single triangle"),
             ("height-refusal", coincident, {"H1_HEIGHT_CAP": "22"}, "Unsupported", "hierarchy height 24 > 22"),
             ("lower-member-refusal", coincident, {"H1_MEMBER_CEILING": "33"}, "Unsupported", "3TZ member ceiling"),
             ("exact-member-refusal", coincident, {"H1_MEMBER_CEILING": "58"}, "Unsupported", "3TZ member ceiling"),
             ("zero-budget", single, {"H1_LEAF_TRIANGLES": "0"}, "InvalidRequest", "budgets must be positive"),
             ("invalid-fan-in", single, {"H1_FAN_IN": "1"}, "InvalidRequest", "fan-in"),
             ("missing-resource", missing, {}, "Io", "No such file"),
             ("bad-extrema", invalid, {}, "InvalidInput", "extrema")]
    scratch = directory / "scratch"
    scratch.mkdir(exist_ok=True)
    receipts = {"label": label, "binary_sha256": digest(binary), "oracle_sha256": digest(__file__), "runs": []}
    for name, source, overrides, kind, message in cases:
        output = directory / name
        env = dict(os.environ, H1_INPUT=str(source), H1_OUTPUT=str(output), H1_SCRATCH=str(scratch),
                   H1_JOBS="1", H1_RUN_BYTES="1024", H1_FAN_IN="2", H1_BUFFER_BYTES="17",
                   H1_PROFILE="quality", H1_LEAF_TRIANGLES="5", H1_TILE_BYTES="280")
        env.update(overrides)
        result = subprocess.run(["nice", "-n", "10", str(binary), "--exact", "mesh_large::tests::external_driver",
                                 "--ignored", "--nocapture"], env=env, capture_output=True, timeout=90)
        assert result.returncode != 0, f"negative control accepted: {name}"
        error = json.loads((output / "error.json").read_text())
        assert error["kind"] == kind and message.lower() in error["message"].lower(), f"wrong refusal {name}: {error}"
        assert error["secondary"] == [] and error["retained_paths"] == [], f"refusal did not clean up: {name}"
        assert list(scratch.iterdir()) == [], f"refusal left private scratch: {name}"
        assert not (output / "triangles.bin").exists(), f"refused input exposed diagnostic geometry: {name}"
        (output / "driver.log").write_bytes(result.stdout + result.stderr)
        receipts["runs"].append({"name": name, "error": error, "overrides": overrides})
    (directory / "receipt.json").write_text(json.dumps(receipts, indent=2) + "\n")
    return len(cases)


def streamed_records(path, format_string):
    codec = struct.Struct(format_string)
    with Path(path).open("rb") as source:
        while raw := source.read(codec.size):
            assert len(raw) == codec.size, f"partial diagnostic record in {path}"
            yield codec.unpack(raw)


def expected_grid_stream(triangle_count):
    positions, indices = grid_template()
    template = sorted_source(primitive_triangles(positions, indices))
    by_key = {}
    for t in template:
        by_key.setdefault(t["key"], []).append(t)
    period = len(template)
    for key in sorted(by_key):
        lookup = {t["source"][3]: t for t in by_key[key]}
        pending = [(residue, residue) for residue in lookup if residue < triangle_count]
        heapq.heapify(pending)
        while pending:
            ordinal, residue = heapq.heappop(pending)
            yield ordinal, lookup[residue]
            if ordinal + period < triangle_count:
                heapq.heappush(pending, (ordinal + period, residue))


def verify_scale(truth, directory, leaf_triangles, tile_bytes):
    """Exact source/provenance replay plus O(height + template vertices) audit.

    The production-sized triangle/node files are never loaded in memory. A
    separately generated arithmetic progression supplies every expected source
    ordinal in Morton order, making exact-once coverage stronger than a digest
    or triangle-count-only check.
    """
    truth = json.loads(Path(truth).read_text())
    count = truth["triangles"]
    assert truth["cells"] == 32 and count >= truth["template_triangles"]
    directory = Path(directory)
    info = json.loads((directory / "info.json").read_text())
    node_stream = iter(streamed_records(directory / "nodes.bin", "<BQBQBBBQQQ6f3x"))
    next_node = next(node_stream, None)
    stack = []
    vertices = {}
    last_geometry_pair = None
    summary = {"triangles": 0, "nodes": 0, "leaves": 0, "internal": 0, "height": 0, "degenerate_cells": 0}
    per_depth = {}
    expected_stream = iter(expected_grid_stream(count))
    for rank, record in enumerate(streamed_records(directory / "triangles.bin", "<QIIIQ3Q9fIB")):
        expected = next(expected_stream, None)
        assert expected is not None, "extra scale triangle"
        ordinal, triangle = expected
        assert record[0] == triangle["key"], "scale Morton key/order mismatch"
        assert tuple(record[1:5]) == (0, 0, 0, ordinal), "scale provenance must replay exactly once"
        assert tuple(record[5:8]) == tuple(triangle["vertices"]), "scale authored vertex reference mismatch"
        assert tuple(record[8:17]) == tuple(v for p in triangle["positions"] for v in p), "scale f32 position mismatch"
        assert record[17:] == (56, 0), "scale geometry charge/degenerate mismatch"
        while next_node is not None and next_node[7] == rank:
            n = next_node
            depth = n[0] + n[2]
            assert 0 <= depth < 32 and n[7] < n[8] <= count, "invalid scale node interval/depth"
            assert record[0] >> (3 * (KEY_DEPTH - n[0])) == n[1], "scale cell identity differs from first source key"
            assert n[9] == 56 * (n[8] - n[7]), "scale node geometry bytes mismatch"
            if stack:
                parent = stack[-1]
                p = parent["wire"]
                identity = (n[0], n[1], n[2] - 1, n[3] >> 3) if n[2] else (n[0] - 1, n[1] >> 3, 0, 0)
                assert identity == p[:4], "scale parent identity mismatch"
                slot = n[3] & 7 if n[2] else n[1] & 7
                assert slot > parent["last_slot"], "scale duplicate/unordered sibling slot"
                assert n[7] == parent["next_child_rank"] and n[8] <= p[8], "scale sibling range coverage mismatch"
                parent["last_slot"] = slot
                parent["next_child_rank"] = n[8]
                parent["seen_children"] += 1
            else:
                assert n[:4] == (0, 0, 0, 0) and n[7:9] == (0, count), "scale tree must have one full-range root"
            assert n[5] in (0, 1), "invalid scale node kind"
            if n[5] == 1:
                assert n[6] == 0 and n[8] - n[7] <= leaf_triangles and n[9] <= tile_bytes, "scale leaf exceeds budget"
            else:
                assert 1 <= n[6] <= 8, "scale internal child count outside 1..8"
            summary["nodes"] += 1
            summary["leaves" if n[5] else "internal"] += 1
            summary["height"] = max(summary["height"], depth + 1)
            summary["degenerate_cells"] += n[0] == 21 and n[2] == 0 and n[5] == 0
            stack.append({"wire": n, "min": [math.inf] * 3, "max": [-math.inf] * 3,
                          "seen_children": 0, "last_slot": -1, "next_child_rank": n[7]})
            next_node = next(node_stream, None)
        assert stack and stack[-1]["wire"][5] == 1, "scale triangle has no containing leaf"
        leaf = stack[-1]["wire"]
        leaf_depth = leaf[0] + leaf[2]
        cell_prefix = record[0] >> (3 * (KEY_DEPTH - leaf[0]))
        assert leaf[1] <= cell_prefix <= (leaf[1] if leaf[2] else (leaf[1] >> 3) * 8 + leaf[4]), "scale leaf crosses its cell/packed octants"
        # Repeated source ordinals of one template triangle have the same
        # geometry. Audit each row's exact payload above, but update geometric
        # unions/minima once per (template, leaf), independently of repetition N.
        pair = (triangle["source"][3], stack[-1]["wire"][:4])
        if pair != last_geometry_pair:
            for point in triangle["positions"]:
                key = bits(point)
                vertices[key] = min(vertices.get(key, leaf_depth), leaf_depth)
                frame = stack[-1]
                for a in range(3):
                    frame["min"][a] = min(frame["min"][a], point[a])
                    frame["max"][a] = max(frame["max"][a], point[a])
            last_geometry_pair = pair
        while stack and stack[-1]["wire"][8] == rank + 1:
            frame = stack.pop()
            n = frame["wire"]
            assert tuple(frame["min"] + frame["max"]) == n[10:16], "scale node box differs from source triangles"
            assert frame["seen_children"] == n[6], "scale declared child count mismatch"
            if n[5] == 0:
                assert frame["next_child_rank"] == n[8], "scale children do not cover internal node"
            if stack:
                parent = stack[-1]
                for a in range(3):
                    parent["min"][a] = min(parent["min"][a], frame["min"][a])
                    parent["max"][a] = max(parent["max"][a], frame["max"][a])
            d = n[0] + n[2]
            extent = max(frame["max"][a] - frame["min"][a] for a in range(3))
            per_depth.setdefault(d, {"min": [math.inf] * 3, "max": [-math.inf] * 3, "maximum_extent": 0})
            p = per_depth[d]
            for a in range(3):
                p["min"][a] = min(p["min"][a], frame["min"][a])
                p["max"][a] = max(p["max"][a], frame["max"][a])
            p["maximum_extent"] = max(p["maximum_extent"], extent)
        summary["triangles"] += 1
    assert summary["triangles"] == count and next(expected_stream, None) is None, "missing scale triangles"
    assert not stack and next_node is None, "scale node ranks must all be consumed"
    weld = list(streamed_records(directory / "vertex_depth.bin", "<IIIB"))
    assert weld == [(*key, value) for key, value in sorted(vertices.items())], "scale exact welded minimum mismatch"
    summary["welded_vertices"] = len(vertices)
    assert len(vertices) == truth["positions"], "scale source vertex coverage mismatch"
    for key, value in summary.items():
        assert info["tree"][key] == value, f"scale {key} summary mismatch"
    expected_depth = [{"depth": d, "bounds": {"min": p["min"], "max": p["max"]},
                       "maximum_extent": p["maximum_extent"]} for d, p in sorted(per_depth.items())]
    assert info["tree"]["per_depth"] == expected_depth, "scale depth summary mismatch"
    assert info["source_bounds"] == {"min": [0, 0, 0], "max": [32, 32, 0]}, "scale source bounds mismatch"
    assert info["cube"] == {"origin": [0, 0, 0], "side": 32}, "scale cube mismatch"
    assert info["geometry_bytes"] == 56 * count, "scale geometry sum mismatch"
    return summary


class ReferenceControls(unittest.TestCase):
    def setUp(self):
        rng = random.Random(40012)
        source = []
        for index in range(73):
            x, y, z = [rng.randrange(8) for _ in range(3)]
            source.extend(primitive_triangles([[x, y, z], [x + .125, y, z], [x, y + .125, z]],
                                              node=index))
        self.expected = hierarchy(source, 5, 5 * projected_geometry_bytes())

    def test_self_compare(self):
        compare(self.expected, self.expected)

    def test_controls(self):
        controls = {
            "lost_source": lambda r: r["triangles"].pop(),
            "duplicate_source": lambda r: r["triangles"].append(r["triangles"][0]),
            "wrong_provenance": lambda r: r["triangles"][0]["source"].__setitem__(0, 999),
            "wrong_frame": lambda r: r["triangles"][0]["positions"][0].__setitem__(1, 999),
            "wrong_sort": lambda r: r["triangles"].reverse(),
            "wrong_identity": lambda r: r["nodes"][1].update(identity=r["nodes"][0]["identity"]),
            "wrong_pack_end": lambda r: r["nodes"][1].update(last_octant=99),
            "wrong_leaf_range": lambda r: next(n for n in r["nodes"] if n["kind"] == "leaf").update(end=999),
            "wrong_bounds": lambda r: r["nodes"][0]["bounds"]["max"].__setitem__(0, -1),
            "wrong_weld_depth": lambda r: r["vertex_depth"][0].__setitem__(3, 99),
            "wrong_depth_union": lambda r: r["depth_bounds"][0][1]["min"].__setitem__(0, -999),
        }
        for name, mutate in controls.items():
            with self.subTest(control=name):
                damaged = copy.deepcopy(self.expected)
                mutate(damaged)
                with self.assertRaises(AssertionError):
                    compare(self.expected, damaged)

    def test_degenerate_paths(self):
        source = primitive_triangles([[0, 0, 0], [1, 0, 0], [0, 1, 0]], [0, 1, 2] * 137)
        expected = hierarchy(source, 1, 100)
        nodes = expected["nodes"]
        self.assertEqual(max(n["depth"] for n in nodes), 24)
        self.assertEqual(len({tuple(n["identity"]) for n in nodes}), len(nodes))
        self.assertTrue(all(n["children"] <= 8 for n in nodes))
        compare(expected, expected)

    def test_empty_gaps_pack(self):
        positions = [[0, 0, 0], [.125, 0, 0], [0, .125, 0],
                     [0, 4, 0], [.125, 4, 0], [0, 4.125, 0],
                     [4, 4, 4], [4.125, 4, 4], [4, 4.125, 4]]
        expected = hierarchy(primitive_triangles(positions), 2, 1000)
        leaves = [n for n in expected["nodes"] if n["kind"] == "leaf"]
        self.assertEqual([n["end"] - n["first"] for n in leaves], [2, 1])

    def test_minimum_after_sixteenth_incident(self):
        positions = []
        for index in range(39):
            extent = .125 if index < 38 else 128
            positions.extend([[0, 0, 0], [extent, 0, 0], [0, extent, 0]])
        expected = hierarchy(primitive_triangles(positions), 5, 10000)
        incidents = []
        origin_bits = bits([0, 0, 0])
        for leaf in (n for n in expected["nodes"] if n["kind"] == "leaf"):
            for t in expected["triangles"][leaf["first"]:leaf["end"]]:
                if any(bits(p) == origin_bits for p in t["positions"]):
                    incidents.append(leaf["depth"])
        self.assertGreater(len(incidents), 16)
        self.assertGreater(min(incidents[:16]), min(incidents))
        damaged = copy.deepcopy(expected)
        row = next(r for r in damaged["vertex_depth"] if tuple(r[:3]) == origin_bits)
        row[3] = min(incidents[:16])
        with self.assertRaisesRegex(AssertionError, "all-corner"):
            compare(expected, damaged)

    def test_streamed_scale_reference_and_wrong_provenance(self):
        positions, template_indices = grid_template()
        source = primitive_triangles(positions, template_indices * 2)
        expected = hierarchy(source, 256, 100000)
        with tempfile.TemporaryDirectory(prefix="h1-oracle-control-") as temporary:
            directory = Path(temporary)
            (directory / "scale-truth.json").write_text(json.dumps({"triangles": 4096, "cells": 32,
                    "template_triangles": 2048, "positions": 1089}))
            raw = bytearray()
            for t in expected["triangles"]:
                raw.extend(struct.pack("<QIIIQ3Q9fIB", t["key"], *t["source"], *t["vertices"],
                                       *(v for p in t["positions"] for v in p), t["bytes"], t["degenerate"]))
            (directory / "triangles.bin").write_bytes(raw)
            with (directory / "nodes.bin").open("wb") as output:
                for n in expected["nodes"]:
                    output.write(struct.pack("<BQBQBBBQQQ6f3x", *n["identity"], n["last_octant"],
                            n["kind"] == "leaf", n["children"], n["first"], n["end"], n["bytes"],
                            *n["bounds"]["min"], *n["bounds"]["max"]))
            (directory / "vertex_depth.bin").write_bytes(b"".join(struct.pack("<IIIB", *row)
                                                        for row in expected["vertex_depth"]))
            leaves = sum(n["kind"] == "leaf" for n in expected["nodes"])
            info = {"tree": {"triangles": 4096, "nodes": len(expected["nodes"]), "leaves": leaves,
                    "internal": len(expected["nodes"]) - leaves,
                    "height": max(n["depth"] for n in expected["nodes"]) + 1,
                    "degenerate_cells": 0, "welded_vertices": 1089,
                    "per_depth": [{"depth": d, "bounds": box, "maximum_extent": extent}
                                  for d, box, extent in expected["depth_bounds"]]},
                    "source_bounds": {"min": [0, 0, 0], "max": [32, 32, 0]},
                    "cube": {"origin": [0, 0, 0], "side": 32}, "geometry_bytes": 56 * 4096}
            (directory / "info.json").write_text(json.dumps(info))
            verify_scale(directory / "scale-truth.json", directory, 256, 100000)
            struct.pack_into("<Q", raw, 20, 4095)
            (directory / "triangles.bin").write_bytes(raw)
            with self.assertRaisesRegex(AssertionError, "provenance"):
                verify_scale(directory / "scale-truth.json", directory, 256, 100000)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("self-test")
    fixture = sub.add_parser("generate")
    fixture.add_argument("directory", type=Path)
    reference = sub.add_parser("reference")
    reference.add_argument("truth", type=Path)
    reference.add_argument("output", type=Path)
    reference.add_argument("--leaf-triangles", type=int, required=True)
    reference.add_argument("--tile-bytes", type=int, required=True)
    verify = sub.add_parser("verify")
    verify.add_argument("truth", type=Path)
    verify.add_argument("directory", type=Path)
    verify.add_argument("--leaf-triangles", type=int, required=True)
    verify.add_argument("--tile-bytes", type=int, required=True)
    scale = sub.add_parser("generate-scale")
    scale.add_argument("directory", type=Path)
    scale.add_argument("--triangles", type=int, default=10_000_000)
    scale_verify = sub.add_parser("verify-scale")
    scale_verify.add_argument("truth", type=Path)
    scale_verify.add_argument("directory", type=Path)
    scale_verify.add_argument("--leaf-triangles", type=int, required=True)
    scale_verify.add_argument("--tile-bytes", type=int, required=True)
    dense = sub.add_parser("generate-dense")
    dense.add_argument("directory", type=Path)
    dense.add_argument("--nodes", type=int, default=65_536)
    run = sub.add_parser("run-corpus")
    run.add_argument("binary", type=Path)
    run.add_argument("directory", type=Path)
    run.add_argument("--label", required=True)
    refusal = sub.add_parser("run-refusals")
    refusal.add_argument("binary", type=Path)
    refusal.add_argument("directory", type=Path)
    refusal.add_argument("--label", required=True)
    args = parser.parse_args()
    if args.command == "self-test":
        result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(ReferenceControls))
        raise SystemExit(not result.wasSuccessful())
    if args.command == "generate":
        print(json.dumps(corpus(args.directory), indent=2))
    if args.command == "reference":
        result = hierarchy(json.loads(args.truth.read_text()), args.leaf_triangles, args.tile_bytes)
        raw = (json.dumps(result, separators=(",", ":")) + "\n").encode()
        args.output.write_bytes(raw)
        print(hashlib.sha256(raw).hexdigest())
    if args.command == "verify":
        print(json.dumps(verify_fixture(args.truth, args.directory, args.leaf_triangles, args.tile_bytes)))
    if args.command == "generate-scale":
        print(generate_scale(args.directory, args.triangles))
    if args.command == "verify-scale":
        print(json.dumps(verify_scale(args.truth, args.directory, args.leaf_triangles, args.tile_bytes)))
    if args.command == "generate-dense":
        print(generate_dense(args.directory, args.nodes))
    if args.command == "run-corpus":
        print(f"{run_corpus(args.binary, args.directory, args.label)} independently checked executions")
    if args.command == "run-refusals":
        print(f"{run_refusals(args.binary, args.directory, args.label)} independently checked refusals")


if __name__ == "__main__":
    main()
