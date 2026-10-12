#!/usr/bin/env python3
"""Materialize the C2 slice-1 witnesses and controls as `.3tz` fixtures, run the
oracle on each and write the expected-versus-actual report.

    python3 -I witnesses/generate.py [--large]

Inputs (pinned by sha256, copied under witnesses/inputs/):
  witnesses.json, slice1-witness-expectations.json, w13-normalized.json.
Outputs:
  witnesses/fixtures/*.3tz, witnesses/report.json,
  controls/fixtures/*.3tz, controls/manifest.json,
  controls/large/*.3tz (only with --large; not committed, hashes pinned).

Expected verdicts come from the expectation file and from exact membership of
independently known points (literal witness values, or points computed by
build.py's own exact helpers). The oracle's verdict is then compared with them.
"""

import argparse
import ast
import copy
import hashlib
import json
import math
import os
import random
import shutil
import struct
import sys
from fractions import Fraction

HERE = os.path.dirname(os.path.abspath(__file__))
TOP = os.path.dirname(HERE)
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(TOP, "oracle"))

import build as B  # noqa: E402
from c2o.check import Oracle  # noqa: E402  (verdicts only; never used for expectations)

INPUT_PINS = {
    "witnesses.json": ("/tmp/rusty-tiles-c2-domain-preparation-20261011/witnesses.json",
                       "cee818541b58c1cda6fe1e5ba0d7f86e3688d1515ffff7bc22e727f89e438cb2"),
    "slice1-witness-expectations.json": (
        "/tmp/rusty-tiles-c2-contract-20261011/verify/slice1-witness-expectations.json",
        "5086105c7bef651ccd62aeeafcc523aa36b742988610933a1d35befa983b1bcb"),
    "w13-normalized.json": ("/tmp/rusty-tiles-c2-contract-20261011/verify/w13-normalized.json",
                            "356c36fa2040186f13f71c66c6758b670a0aafd217635332721b17c76f536a17"),
}


def load_inputs():
    os.makedirs(os.path.join(HERE, "inputs"), exist_ok=True)
    out = {}
    for name, (source, digest) in INPUT_PINS.items():
        local = os.path.join(HERE, "inputs", name)
        if not os.path.exists(local):
            shutil.copyfile(source, local)
        with open(local, "rb") as fh:
            data = fh.read()
        if hashlib.sha256(data).hexdigest() != digest:
            raise SystemExit("input %s does not match its pin" % name)
        out[name] = json.loads(data)
    return out


def safe_eval(expr, names=None):
    """Exact value of a +,-,*,/ expression over integer literals and names."""
    names = names or {}

    def ev(node):
        if isinstance(node, ast.Expression):
            return ev(node.body)
        if isinstance(node, ast.Constant) and isinstance(node.value, int):
            return Fraction(node.value)
        if isinstance(node, ast.Name):
            return names[node.id]
        if isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.USub):
            return -ev(node.operand)
        if isinstance(node, ast.BinOp):
            a, b = ev(node.left), ev(node.right)
            return {ast.Add: a + b, ast.Sub: a - b, ast.Mult: a * b}.get(type(node.op)) \
                if not isinstance(node.op, ast.Div) else a / b
        raise ValueError("unsupported expression %r" % expr)

    return ev(ast.parse(expr, mode="eval"))


def F(v):
    return [Fraction(x) for x in v]


# ---------------------------------------------------------------- payloads


def payload_from_spec(spec, patch=None, b3dm_table=None, b3dm_bin=b""):
    """GLB (or b3dm when b3dm_table is given) from a witnesses.json case."""
    g = B.Gltf()
    ctype = spec.get("componentType", 5126)
    declared = (spec.get("declared_min"), spec.get("declared_max"))
    pos = g.positions(spec["positions"], ctype, spec.get("normalized", False),
                      spec.get("viewless", False), declared)
    prim = {"attributes": {"POSITION": pos}, "mode": spec.get("mode", 0)}
    if "indices" in spec:
        prim["indices"] = g.indices(spec["indices"], spec.get("indexType", 5123))
    g.doc["meshes"] = [{"primitives": [prim]}]
    g.doc["nodes"] = copy.deepcopy(spec["nodes"])
    g.doc["scenes"] = copy.deepcopy(spec["scenes"])
    if "scene" in spec:
        g.doc["scene"] = spec["scene"]
    if ctype != 5126:
        g.doc["extensionsUsed"] = ["KHR_mesh_quantization"]
        g.doc["extensionsRequired"] = ["KHR_mesh_quantization"]
    if patch:
        patch(g)
    glb = g.glb()
    if b3dm_table is not None:
        return B.b3dm(glb, b3dm_table, b3dm_bin)
    return glb


def add_used(g, name, required=False):
    g.doc.setdefault("extensionsUsed", []).append(name)
    if required:
        g.doc.setdefault("extensionsRequired", []).append(name)


# ---------------------------------------------------------------- cases


class Case:
    """One fixture: tileset documents, payload members, expectations and the
    tested volumes (for one-ulp mutants)."""

    def __init__(self, cid, group, expect, source=None):
        self.id = cid
        self.group = group
        self.expect = expect  # {"outcome":..., "reasons"|"facts"}
        self.source = source
        self.tilesets = {}  # member -> dict
        self.payloads = []  # (member, bytes)
        self.limits = {}
        self.tracks = []  # [(node, [point at level 0, 1, ...])] in streaming order
        self.tested = []  # {"member", "path", "level", "role"}
        self.literal = []  # {"what", "track", "level"|"node_yp", "expected"}
        self.production = None  # production expectation when it differs (C1 precedes)
        self.c1_refused = None  # observed production C1 refusal kind (coordinator parity run 1)
        self.notes = []
        self.mutants = True
        self.large = False
        self.draw_visits = None
        self.leaf_depth = 0  # tree depth of the tile whose content holds the tracks
        self.raw_replace = []  # (member, old bytes, new bytes) applied after JSON serialization

    def members(self):
        out = []
        for name, doc in self.tilesets.items():
            data = B.json_bytes(doc)
            for member, old, new in self.raw_replace:
                if member == name:
                    assert data.count(old) == 1
                    data = data.replace(old, new)
            out.append((name, data))
        return out + list(self.payloads)

    def get(self, member, path):
        obj = self.tilesets[member]
        for key in path:
            obj = obj[key]
        return obj

    def set(self, member, path, value):
        obj = self.tilesets[member]
        for key in path[:-1]:
            obj = obj[key]
        obj[path[-1]] = value


def chain_case(case, payload_name, payload, transforms, level0_tracks, content_volume=True,
               ext_index=None, root_extra=None):
    """Nested tiles root-first with `transforms` (None = no transform); the last
    tile holds the payload. With ext_index e, tile e is the root of external
    tileset ext.json, referenced by tile e-1. Volumes are tight boxes around the
    points; each ancestor is the image of its child box."""
    n = len(transforms)
    case.payloads.append((payload_name, payload))
    tracks = []
    for node, p0 in level0_tracks:
        levels = [list(p0)]
        for level in range(1, n):
            child = transforms[n - level]
            p = levels[-1]
            levels.append(B.apply(B.col_major(child), p) if child is not None else list(p))
        tracks.append((node, levels))
    case.tracks = tracks
    case.leaf_depth = n - 1
    boxes = [B.tight_box([t[1][0] for t in tracks])]
    for level in range(1, n):
        child = transforms[n - level]
        boxes.append(B.image_box(boxes[-1], child) if child is not None else copy.deepcopy(boxes[-1]))
    # Build tiles from the leaf upwards.
    tiles = [None] * n
    for i in reversed(range(n)):
        level = n - 1 - i
        t = B.tile(copy.deepcopy(boxes[level]), transform=transforms[i],
                   geometric_error=0.0 if i == n - 1 else 10.0 * (n - i))
        tiles[i] = t
    leaf = tiles[-1]
    leaf["content"] = {"uri": payload_name}
    if content_volume:
        leaf["content"]["boundingVolume"] = copy.deepcopy(boxes[0])
    for i in range(n - 1):
        if ext_index is not None and i == ext_index - 1:
            tiles[i]["content"] = {"uri": "ext.json",
                                   "boundingVolume": copy.deepcopy(tiles[i]["boundingVolume"])}
        else:
            tiles[i]["children"] = [tiles[i + 1]]
    if root_extra:
        tiles[0].update(root_extra)
    if ext_index is None:
        case.tilesets["tileset.json"] = B.tileset(tiles[0])
        roots = [("tileset.json", 0)]
    else:
        case.tilesets["tileset.json"] = B.tileset(tiles[0])
        case.tilesets["ext.json"] = B.tileset(tiles[ext_index])
        roots = [("tileset.json", 0), ("ext.json", ext_index)]
    # Tested volume paths.
    for i in range(n):
        level = n - 1 - i
        member, start = roots[0] if ext_index is None or i < ext_index else roots[1]
        path = ("root",) + ("children", 0) * (i - start)
        role = "own" if level == 0 else "ancestor"
        case.tested.append({"member": member, "path": path + ("boundingVolume",),
                            "level": level, "role": role})
        if level == 0 and content_volume:
            case.tested.append({"member": member, "path": path + ("content", "boundingVolume"),
                                "level": 0, "role": "content"})
        if ext_index is not None and i == ext_index - 1:
            case.tested.append({"member": member, "path": path + ("content", "boundingVolume"),
                                "level": level, "role": "externalContent"})
    return case


def separating_box(p, avoid):
    """Single-point box tight on p whose faces exclude `avoid` where p != avoid."""
    c, h = [], []
    for j in range(3):
        if avoid[j] > p[j]:
            cj = B.round_down(p[j])
            hj = B.round_up(p[j] - Fraction(cj))
        elif avoid[j] < p[j]:
            cj = B.round_up(p[j])
            hj = B.round_up(Fraction(cj) - p[j])
        else:
            cj = float(p[j])
            hj = B.round_up(abs(p[j] - Fraction(cj)))
        c.append(cj)
        h.append(hj)
    box = {"box": [c[0], c[1], c[2], h[0], 0.0, 0.0, 0.0, h[1], 0.0, 0.0, 0.0, h[2]]}
    assert B.box_contains(box, p) and (p == avoid or not B.box_contains(box, avoid))
    return box


def literal_points(w, key="own_tile_local"):
    return [F(p) for p in w["expected"][key]]


def build_cases(inputs):
    W = {w["id"][:3]: w for w in inputs["witnesses.json"]["cases"]}
    ctx = {c["id"]: c for c in inputs["witnesses.json"]["context_controls"]}
    cases = []

    def contained():
        return {"outcome": "Contained"}

    def ni(*reasons):
        return {"outcome": "NotInspected", "reasons": sorted(reasons)}

    def nodes_of(w):
        return [n for n in range(len(w["nodes"])) if "mesh" in w["nodes"][n]]

    def single(cid, group, w, expect, points=None, patch=None, source=None, nodes=None, **kw):
        case = Case(cid, group, expect, source or w["id"])
        pts = points if points is not None else literal_points(w)
        sel = nodes if nodes is not None else w["expected"].get("selected_nodes", nodes_of(w))
        tracks = list(zip(sel, pts))
        chain_case(case, "w.glb", payload_from_spec(w, patch), [None], tracks, **kw)
        return case

    # ---- 13 literal witnesses (W01 also as the no-RTC Contained variant)
    w = W["W01"]
    rtc_points = literal_points(w)
    case = Case("W01-ordered-node-U-RTC-tile-chain", "literal", ni("rtcCenter"), w["id"])
    chain_case(case, "w.b3dm", payload_from_spec(w, b3dm_table={"BATCH_LENGTH": 0, "RTC_CENTER": w["rtc"]}),
               w["tile_chain_root_first"], [(0, rtc_points[0])])
    case.literal.append({"what": "node_yp", "track": 0, "node_yp": True, "expected": F(w["expected"]["node_yp"][0])})
    case.literal.append({"what": "own_tile_local minus rtc (slice 1 does not apply RTC)", "track": 0,
                         "level": 0, "expected": [a - b for a, b in zip(rtc_points[0], F(w["rtc"]))]})
    case.literal.append({"what": "parent_tile_local (literal, includes RTC) == T1(own_tile_local)",
                         "generator_only": True, "track": 0, "level": 1,
                         "expected": F(w["expected"]["parent_tile_local"][0])})
    case.notes.append("Volumes enclose the RTC-applied literal points (slice-2 frame truth).")
    cases.append(case)
    no_rtc = [a - b for a, b in zip(rtc_points[0], F(w["rtc"]))]
    case = Case("W01-noRTC-variant", "literal", contained(), w["id"])
    chain_case(case, "w.b3dm", payload_from_spec(w, b3dm_table={"BATCH_LENGTH": 0}),
               w["tile_chain_root_first"], [(0, no_rtc)])
    case.literal.append({"what": "own_tile_local [4,-42,22]", "track": 0, "level": 0, "expected": F([4, -42, 22])})
    case.literal.append({"what": "node_yp", "track": 0, "node_yp": True, "expected": F(w["expected"]["node_yp"][0])})
    cases.append(case)

    for key in ("W02", "W03", "W04", "W05", "W07", "W08", "W10", "W12"):
        w = W[key]
        case = single(w["id"], "literal", w, contained())
        for i, p in enumerate(literal_points(w)):
            case.literal.append({"what": "own_tile_local", "track": i, "level": 0, "expected": p})
        if "node_yp" in w["expected"]:
            case.literal.append({"what": "node_yp", "track": 0, "node_yp": True,
                                 "expected": F(w["expected"]["node_yp"][0])})
        case.draw_visits = w["expected"]["draw_visits"]
        cases.append(case)

    w = W["W06"]
    pts = [[safe_eval(s) for s in w["expected"]["own_tile_local_rational"][0]]]
    case = single(w["id"], "literal", w, contained(), points=pts)
    case.literal.append({"what": "own_tile_local_rational", "track": 0, "level": 0, "expected": pts[0]})
    cases.append(case)

    w = W["W09"]
    case = single(w["id"], "literal", w, contained())
    case.literal.append({"what": "node_yp", "track": 0, "node_yp": True, "expected": F(w["expected"]["node_yp"][0])})
    case.literal.append({"what": "own_tile_local", "track": 0, "level": 0, "expected": F([1, 2, 3])})
    cases.append(case)

    w = W["W11"]
    case = Case(w["id"], "literal", contained(), w["id"])
    chain_case(case, "w.glb", payload_from_spec(w), w["tile_chain_root_first"],
               [(0, F(w["expected"]["own_tile_local"][0]))], ext_index=w["external_root_chain_index"])
    case.literal.append({"what": "own_tile_local", "track": 0, "level": 0, "expected": F(w["expected"]["own_tile_local"][0])})
    case.literal.append({"what": "external_root_local", "track": 0, "level": 1,
                         "expected": F(w["expected"]["external_root_local"][0])})
    case.literal.append({"what": "parent_tile_local", "track": 0, "level": 2,
                         "expected": F(w["expected"]["parent_tile_local"][0])})
    cases.append(case)

    w = W["W13"]
    norm = inputs["w13-normalized.json"]
    pts = [[safe_eval(s.replace("/", "/")) for s in norm["own_tile_local_exact"]]]
    case = single(w["id"], "literal", w, contained(), points=pts, nodes=[1])
    case.literal.append({"what": "own_tile_local (normalized meaning, verify/w13-normalized.json)",
                         "track": 0, "level": 0, "expected": pts[0]})
    consts = {k: safe_eval(v) for k, v in w["expected"]["constants"].items()}
    raw = [safe_eval(e, consts) for e in w["expected"]["own_tile_local_exact_expression"][0]]
    case.raw_reading = raw
    # Place each face between the two readings: tight on the normalized value,
    # with the raw-polynomial value outside (the only way to observe the 1e-15 gap).
    box = separating_box(pts[0], raw)
    for tv in case.tested:
        case.set(tv["member"], tv["path"], copy.deepcopy(box))
    case.notes.append("The one-ulp row-0/row-2 boxes are tight on the normalized reading; the raw "
                      "polynomial reading %s lies outside them (fault 'use raw polynomial')."
                      % [float(v) for v in raw])
    cases.append(case)

    # ---- 17 context recipes
    w03, w12, w02, w01 = W["W03"], W["W12"], W["W02"], W["W01"]
    w12pts = literal_points(w12)

    # 1. same URI, other reference outside
    def same_uri(cid, exclude):
        case = Case(cid, "context", {"outcome": "Outside", "facts": {"reference": 1, "level": 0, "depth": 1,
                                                                    "role": "own", "node": 1}}
                    if exclude else contained(), "same-URI-other-reference-outside")
        pts = literal_points(w03)
        box_a = B.tight_box(pts)
        box_b = B.tight_box(pts[:1]) if exclude else copy.deepcopy(box_a)
        shift = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 100, 0, 0, 1]
        root_box = {"box": [56.0, -3.0, 2.0, 60.0, 0, 0, 0, 8.0, 0, 0, 0, 8.0]}
        case.tilesets["tileset.json"] = B.tileset(B.tile(root_box, geometric_error=10.0, children=[
            B.tile(box_a, content="w.glb"), B.tile(box_b, content="w.glb", transform=shift)]))
        case.payloads.append(("w.glb", payload_from_spec(w03)))
        case.tracks = [(0, [pts[0]]), (1, [pts[1]])]
        case.leaf_depth = 1
        case.tested = [{"member": "tileset.json", "path": ("root", "children", 0, "boundingVolume"),
                        "level": 0, "role": "own"}]
        case.mutants = not exclude
        return case
    cases.append(same_uri("same-URI-other-reference-outside", True))
    cases.append(same_uri("same-URI-both-references-inside (control base)", False))

    # 2. child volume escapes, content inside
    case = Case("child-volume-escapes-but-content-inside", "context", contained(),
                "child-volume-escapes-but-content-inside")
    case.production = "C1:InvalidInput(hierarchyBounds)"
    case.c1_refused = "invalid_input"
    spec = {"positions": [[0, 0, 0]], "nodes": [{"mesh": 0}], "scenes": [{"nodes": [0]}], "scene": 0}
    case.payloads.append(("w.glb", payload_from_spec(spec)))
    case.tilesets["tileset.json"] = B.tileset(B.tile({"sphere": [0, 0, 0, 2.0]}, geometric_error=10.0,
                                                     children=[B.tile({"sphere": [0, 0, 0, 3.0]}, content="w.glb")]))
    case.mutants = False
    case.notes.append("Oracle C2 verdict is Contained; production stops at C1 hierarchyBounds (D4).")
    cases.append(case)

    # 3. sole scene, no default
    spec = copy.deepcopy(w12)
    del spec["scene"]
    cases.append(single("sole-scene-no-default", "context", spec, contained(), points=w12pts,
                        source="sole-scene-no-default"))
    # 4. ambiguous
    spec = copy.deepcopy(w02)
    del spec["scene"]
    cases.append(single("ambiguous-scene-no-default", "context", spec, ni("sceneSelection"),
                        points=literal_points(w02)))
    # 5. node cycle
    spec = copy.deepcopy(w12)
    spec["nodes"][0]["children"] = [0]
    cases.append(single("node-cycle", "context", spec, {"outcome": "InvalidNode"}, points=w12pts))
    # 6. shared child
    spec = copy.deepcopy(w03)
    spec["nodes"].append({})
    spec["nodes"][0]["children"] = [2]
    spec["nodes"][1]["children"] = [2]
    cases.append(single("shared-child-not-shared-mesh", "context", spec, {"outcome": "InvalidNode"},
                        points=literal_points(w03)))
    # 7. matrix and TRS on the RTC b3dm
    spec = copy.deepcopy(w01)
    spec["nodes"][0]["translation"] = [0, 0, 0]
    case = Case("matrix-and-TRS", "context", {"outcome": "InvalidNode"}, "matrix-and-TRS")
    chain_case(case, "w.b3dm", payload_from_spec(spec, b3dm_table={"BATCH_LENGTH": 0, "RTC_CENTER": w01["rtc"]}),
               w01["tile_chain_root_first"], [(0, rtc_points[0])])
    cases.append(case)
    # 8. zero quaternion
    spec = copy.deepcopy(w12)
    spec["nodes"][0]["rotation"] = [0, 0, 0, 0]
    cases.append(single("zero-quaternion", "context", spec, {"outcome": "InvalidNode"}, points=w12pts))

    # 9. unknown node position extension (declared optional)
    def acme(g):
        add_used(g, "ACME_position")
        g.doc["nodes"][0]["extensions"] = {"ACME_position": {"offset": [100, 0, 0]}}
    cases.append(single("unknown-node-position-extension", "context", w12, ni("gltfExtension"),
                        points=w12pts, patch=acme, source="unknown-node-position-extension"))

    # 10. opaque extras
    def extras(g):
        g.doc["nodes"][0]["extras"] = {"extensions": {"ACME_position": {"offset": [100, 0, 0]}}}
    cases.append(single("opaque-extras-position-like-data", "context", w12, contained(), points=w12pts,
                        patch=extras, source="opaque-extras-position-like-data"))

    # 11. morph targets with zero weight
    def morph(g):
        acc = g.positions([[1, 0, 0]])
        g.doc["meshes"][0]["primitives"][0]["targets"] = [{"POSITION": acc}]
        g.doc["nodes"][0]["weights"] = [0]
    cases.append(single("selected-morph-even-zero-weight", "context", w12, ni("morphTargets"),
                        points=w12pts, patch=morph, source="selected-morph-even-zero-weight"))

    # 12. skin, with and without joint attributes
    def skin(with_attributes):
        def patch(g):
            g.doc["nodes"][0]["skin"] = 0
            g.doc["nodes"][0]["children"] = [1]
            g.doc["nodes"].append({})
            g.doc["skins"] = [{"joints": [1]}]
            if with_attributes:
                view = g.view(bytes([0, 0, 0, 0]), target=34962)
                j = g.accessor(view, 5121, 1, "VEC4")
                view = g.view(struct.pack("<4f", 1, 0, 0, 0), target=34962)
                wt = g.accessor(view, 5126, 1, "VEC4")
                g.doc["meshes"][0]["primitives"][0]["attributes"].update({"JOINTS_0": j, "WEIGHTS_0": wt})
        return patch
    case = single("selected-skin", "context", w12, ni("skin"), points=w12pts, patch=skin(True),
                  source="selected-skin")
    case.production = "C1:Unsupported"
    case.c1_refused = "unsupported"
    case.expect_c1_caveat = "JOINTS_0"
    cases.append(case)
    cases.append(single("selected-skin-without-joint-attributes", "context", w12, ni("skin"),
                        points=w12pts, patch=skin(False), source="selected-skin (contract note)"))

    # 13. animation
    def anim(g):
        view = g.view(struct.pack("<f", 0.0))
        inp = g.accessor(view, 5126, 1, "SCALAR", min=[0.0], max=[0.0])
        view = g.view(struct.pack("<3f", 0, 0, 0))
        out = g.accessor(view, 5126, 1, "VEC3")
        g.doc["animations"] = [{"channels": [{"sampler": 0, "target": {"node": 0, "path": "translation"}}],
                                "samplers": [{"input": inp, "output": out}]}]
    cases.append(single("animation-no-chosen-playback", "context", w12, ni("animation"), points=w12pts,
                        patch=anim, source="animation-no-chosen-playback"))

    # 14. GPU instancing, optional and required
    def gpu(required):
        def patch(g):
            add_used(g, "EXT_mesh_gpu_instancing", required)
            view = g.view(struct.pack("<3f", 0, 0, 0))
            acc = g.accessor(view, 5126, 1, "VEC3")
            g.doc["nodes"][0]["extensions"] = {"EXT_mesh_gpu_instancing": {"attributes": {"TRANSLATION": acc}}}
        return patch
    cases.append(single("GPU-instancing-not-core-instances", "context", w12, ni("gltfExtension"),
                        points=w12pts, patch=gpu(False), source="GPU-instancing-not-core-instances"))
    case = single("GPU-instancing-required", "context", w12, ni("gltfExtension"), points=w12pts,
                  patch=gpu(True), source="GPU-instancing-not-core-instances (required form)")
    case.production = "C1:Unsupported"
    case.c1_refused = "unsupported"
    case.expect_c1_caveat = "EXT_mesh_gpu_instancing"
    cases.append(case)

    # 15. region ancestor
    case = Case("region-bound", "context", ni("regionVolume"), "region-bound")
    ecef = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 6378137.0, 0, 0, 1]
    case.payloads.append(("w.glb", payload_from_spec(w12)))
    case.tilesets["tileset.json"] = B.tileset(B.tile(
        {"region": [-0.001, -0.001, 0.001, 0.001, -1000.0, 1000.0]}, geometric_error=10.0,
        children=[B.tile(B.tight_box(w12pts), content="w.glb", transform=ecef)]))
    case.production = "C1:Unsupported (mixed region/Cartesian containment)"
    case.c1_refused = "unsupported"
    cases.append(case)

    # C1-passing regionVolume variant: every tile and content volume is a region, nested.
    case = Case("region-only-hierarchy", "context", ni("regionVolume"), "region-bound (C1-passing variant)")
    spec = {"positions": [[6378137.0, 0, 0]], "nodes": [{"mesh": 0}], "scenes": [{"nodes": [0]}], "scene": 0}
    case.payloads.append(("w.glb", payload_from_spec(spec)))
    leaf = B.tile({"region": [-0.001, -0.001, 0.001, 0.001, -10.0, 10.0]}, content="w.glb",
                  content_volume={"region": [-0.0005, -0.0005, 0.0005, 0.0005, -5.0, 5.0]})
    case.tilesets["tileset.json"] = B.tileset(B.tile(
        {"region": [-0.01, -0.01, 0.01, 0.01, -100.0, 100.0]}, geometric_error=10.0, children=[leaf]))
    case.notes.append("glTF (6378137, 0, 0) is ECEF (6378137, 0, 0) after U: lon 0, lat 0, height 0, "
                      "inside every region; no tile transform (regions ignore transforms)")
    cases.append(case)

    # 16/17. RTC binary form and range overrun
    rtc_bin = bytes(4) + struct.pack("<3f", *w01["rtc"])
    for cid, offset in (("RTC-binary-form", 4), ("RTC-range-overrun", 8)):
        case = Case(cid, "context", ni("rtcCenter"), cid)
        chain_case(case, "w.b3dm", payload_from_spec(
            w01, b3dm_table={"BATCH_LENGTH": 0, "RTC_CENTER": {"byteOffset": offset}}, b3dm_bin=rtc_bin),
            w01["tile_chain_root_first"], [(0, rtc_points[0])])
        cases.append(case)

    # ---- new slice-1 controls
    controls = (build_new_controls(W, contained, ni, single) + v33_cases(W, contained, ni, single)
                + role_order_case(contained) + review_b1_cases())
    return cases + controls


def build_new_controls(W, contained, ni, single):
    cases = []
    w12 = W["W12"]
    w12pts = literal_points(w12)

    # CESIUM_RTC optional with a root extension object
    def cesium(g):
        add_used(g, "CESIUM_RTC")
        g.doc["extensions"] = {"CESIUM_RTC": {"center": [1.0, 2.0, 3.0]}}
    cases.append(single("CESIUM_RTC-optional", "control", w12, ni("gltfExtension"), points=w12pts,
                        patch=cesium, source="newSlice1Controls[1]"))

    # quaternion window
    def quat_case(cid, x, expect):
        spec = {"positions": [[1, 2, 3]], "nodes": [{"rotation": [x, 0.0, 0.0, 1.0], "mesh": 0}],
                "scenes": [{"nodes": [0]}], "scene": 0}
        p = B.tile_points(spec["nodes"], [0], F([1, 2, 3]))
        return single(cid, "control", spec, expect, points=[p], nodes=[0], source="newSlice1Controls[2]")
    cases.append(quat_case("quaternion-window-at-2^-18", 2.0 ** -9, contained()))
    cases.append(quat_case("quaternion-window-next-beyond", B.next_up(2.0 ** -9), ni("quaternionWindow")))

    # rotated box
    case = Case("rotated-box", "control", ni("rotatedBox"), "newSlice1Controls[3]")
    case.payloads.append(("w.glb", payload_from_spec(w12)))
    c = [float(v) for v in w12pts[0]]
    case.tilesets["tileset.json"] = B.tileset(B.tile({"box": c + [3.0, 4.0, 0.0, -4.0, 3.0, 0.0, 0.0, 0.0, 5.0]},
                                                     content="w.glb"))
    cases.append(case)

    # external content volume (I2): base contained, shrunk excludes
    for cid, shrink in (("external-content-volume-contains", False), ("external-content-volume-excludes", True)):
        expect = ({"outcome": "Outside", "facts": {"level": 1, "depth": 1, "role": "externalContent"}}
                  if shrink else contained())
        case = Case(cid, "control", expect, "newSlice1Controls[4]")
        chain_case(case, "w.glb", payload_from_spec(w12), [None, None], [(0, w12pts[0])],
                   content_volume=False, ext_index=1)
        if shrink:
            # Referencing tile box (8,-42,14) +- 8 holds both the external root box (the
            # point) and the content box x in [0, 7], which excludes the point x = 8.
            case.set("tileset.json", ("root", "boundingVolume"),
                     {"box": [8.0, -42.0, 14.0, 8.0, 0.0, 0.0, 0.0, 8.0, 0.0, 0.0, 0.0, 8.0]})
            case.set("tileset.json", ("root", "content", "boundingVolume"),
                     {"box": [3.5, -42.0, 14.0, 3.5, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]})
            case.mutants = False
        cases.append(case)

    # tileset S2 declaration, tile metadata
    case = single("tileset-extension-S2-declared", "control", w12, ni("tilesetExtension"), points=w12pts,
                  source="newSlice1Controls[5]")
    case.tilesets["tileset.json"]["extensionsUsed"] = ["3DTILES_bounding_volume_S2"]
    case.mutants = False
    cases.append(case)
    case = single("tile-metadata", "control", w12, ni("tileMetadata"), points=w12pts, source="newSlice1Controls[6]")
    ts = case.tilesets["tileset.json"]
    ts["schema"] = {"id": "c2", "classes": {"t": {"properties": {"name": {"type": "STRING"}}}}}
    ts["root"]["metadata"] = {"class": "t", "properties": {"name": "x"}}
    case.mutants = False
    cases.append(case)

    # tight non-integer centre box
    x = B.f32(0.3)
    h = Fraction(x) - Fraction(0.1)
    assert Fraction(float(h)) == h, "x - 0.1 must be a double"
    for cid, vx, hx, expect in (
            ("tight-non-integer-centre-on-boundary", x, float(h), contained()),
            ("tight-non-integer-centre-vertex-one-ulp-beyond", B.f32_next_up(x), float(h),
             {"outcome": "Outside", "facts": {"level": 0, "depth": 0, "role": "own"}}),
            ("tight-non-integer-centre-h-one-ulp-smaller", x, B.next_down(float(h)),
             {"outcome": "Outside", "facts": {"level": 0, "depth": 0, "role": "own"}})):
        spec = {"positions": [[vx, 0, 0]], "nodes": [{"mesh": 0}], "scenes": [{"nodes": [0]}], "scene": 0}
        case = Case(cid, "control", expect, "newSlice1Controls[7]")
        case.payloads.append(("w.glb", payload_from_spec(spec)))
        case.tilesets["tileset.json"] = B.tileset(B.tile({"box": [0.1, 0.0, 0.0, hx, 0, 0, 0, 1.0, 0, 0, 0, 1.0]},
                                                         content="w.glb"))
        case.mutants = False
        case.notes.append("centre 0.1 (binary64), vertex f32(0.3) on the upper face of row 0")
        cases.append(case)

    cases += worked_example_cases(contained, ni)
    cases += work_limit_cases(contained, ni)
    cases += rotated_chain_cases(contained, ni)
    cases += polygon_cases(contained)
    cases += identity_cases(contained)
    cases += hull_cases(contained, ni)
    cases += repeated_index_cases(contained, ni)
    return cases


def worked_example_cases(contained, ni):
    out = []
    nodes = [{"translation": [10.5, 0, -3], "scale": [2, 4, 0.25], "mesh": 0}]
    raw = [[0, 0, 0], [65535, 0, 0], [0, 65535, 0], [65535, 65535, 0]]
    spec = {"positions": raw, "componentType": 5123, "normalized": True, "nodes": nodes,
            "scenes": [{"nodes": [0]}], "scene": 0}
    payload = payload_from_spec(spec)
    child_t = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 1000.25, -20, 0, 1]
    pts = [B.tile_points(nodes, [0], [Fraction(c, 65535) for c in v]) for v in raw]
    for variant, c1 in (("literal", 0.0), ("adjusted", 3.0)):
        own = {"box": [1.5, c1, 2.0, 100.0, 0, 0, 0, 0.0, 0, 0, 0, 50.0]}
        inside = all(B.box_contains(own, p) for p in pts)
        for bits in (29, 28):
            cid = "worked-example-%s-containmentSpanBits-%d" % (variant, bits)
            if bits == 28:
                expect = ni("numericSpan")
            elif inside:
                expect = contained()
            else:
                expect = {"outcome": "Outside", "facts": {"level": 0, "depth": 1, "role": "own"}}
            case = Case(cid, "control", expect, "newSlice1Controls[8] / contract Worked example")
            case.payloads.append(("w.glb", payload))
            case.tilesets["tileset.json"] = B.tileset(B.tile(
                {"box": [0.0, 0.0, 0.0, 2048.0, 0, 0, 0, 2048.0, 0, 0, 0, 2048.0]}, geometric_error=10.0,
                children=[B.tile(own, content="w.glb", transform=child_t)]))
            case.limits = {"containmentSpanBits": bits}
            case.expect_spans = {"sInst": 5, "levelSpans": [23, 29]}
            case.mutants = False
            if variant == "literal":
                case.notes.append("v3.3 (1): the literal worked example is in domain at 29 with verdict "
                                  "Outside (own box row 1 has c=0, h=0; every vertex has tile row 1 = "
                                  "3 - 0.25*z/65535 in [2.75, 3]).")
            else:
                case.notes.append("v3.3 (1): the Contained-at-cap / numericSpan-at-28 control. Own box "
                                  "centre row 1 = 3 (z = 0 for every vertex); spans stay sInst=5, [23, 29].")
            out.append(case)
    # One-ulp mutant of the adjusted variant, at the default cap (any one-ulp change of
    # a worked-example volume value changes its spans past 29).
    case = Case("worked-example-adjusted-default-cap", "control", contained(), "worked example")
    case.payloads.append(("w.glb", payload))
    case.tilesets["tileset.json"] = B.tileset(B.tile(
        {"box": [0.0, 0.0, 0.0, 2048.0, 0, 0, 0, 2048.0, 0, 0, 0, 2048.0]}, geometric_error=10.0,
        children=[B.tile({"box": [1.5, 3.0, 2.0, 100.0, 0, 0, 0, 0.0, 0, 0, 0, 50.0]}, content="w.glb",
                         transform=child_t)]))
    case.tracks = [(0, [p, B.apply(B.col_major(child_t), p)]) for p in pts]
    case.leaf_depth = 1
    case.tested = [{"member": "tileset.json", "path": ("root", "children", 0, "boundingVolume"),
                    "level": 0, "role": "own"}]
    out.append(case)
    return out


def meshopt_glb(positions, indices, nodes, scene_nodes):
    """GLB whose POSITION (ATTRIBUTES) and indices (TRIANGLES) views are meshopt
    compressed into the BIN chunk, with a URI-less fallback buffer 1."""
    raw = b"".join(struct.pack("<3f", *p) for p in positions)
    comp_v = B.meshopt_attributes(raw, len(positions), 12)
    comp_i = B.meshopt_triangles(indices)
    binary = bytearray(comp_v + bytes(-len(comp_v) % 4))
    off_i = len(binary)
    binary += comp_i + bytes(-len(comp_i) % 4)
    off_fallback_i = len(raw) + (-len(raw) % 4)
    fl = [[B.f32(c) for c in p] for p in positions]
    doc = {
        "asset": {"version": "2.0", "generator": "rusty-tiles C2 oracle witness generator"},
        "buffers": [{"byteLength": len(binary)},
                    {"byteLength": off_fallback_i + 2 * len(indices),
                     "extensions": {"EXT_meshopt_compression": {"fallback": True}}}],
        "bufferViews": [
            {"buffer": 1, "byteOffset": 0, "byteLength": len(raw), "byteStride": 12, "target": 34962,
             "extensions": {"EXT_meshopt_compression": {
                 "buffer": 0, "byteOffset": 0, "byteLength": len(comp_v), "byteStride": 12,
                 "count": len(positions), "mode": "ATTRIBUTES", "filter": "NONE"}}},
            {"buffer": 1, "byteOffset": off_fallback_i, "byteLength": 2 * len(indices), "target": 34963,
             "extensions": {"EXT_meshopt_compression": {
                 "buffer": 0, "byteOffset": off_i, "byteLength": len(comp_i), "byteStride": 2,
                 "count": len(indices), "mode": "TRIANGLES"}}}],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": len(positions), "type": "VEC3",
             "min": [min(p[j] for p in fl) for j in range(3)], "max": [max(p[j] for p in fl) for j in range(3)]},
            {"bufferView": 1, "componentType": 5123, "count": len(indices), "type": "SCALAR"}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1, "mode": 4}]}],
        "nodes": nodes, "scenes": [{"nodes": scene_nodes}], "scene": 0,
        "extensionsUsed": ["EXT_meshopt_compression"],
        "extensionsRequired": ["EXT_meshopt_compression"],
    }
    return B.glb_bytes(doc, bytes(binary))


def work_limit_cases(contained, ni):
    nodes = [{"children": [1]}, {"translation": [1.0, 2.0, 3.0], "children": [2]},
             {"scale": [2.0, 2.0, 2.0], "mesh": 0}]
    positions = [[0, 0, 0], [1, 0, 0], [0, 1, 0]]
    payload = meshopt_glb(positions, [0, 1, 2], nodes, [0])
    pts = [B.tile_points(nodes, [0, 1, 2], F(p)) for p in positions]
    case = Case("work-base-meshopt", "control", contained(), "newSlice1Controls[9]")
    chain_case(case, "w.glb", payload, [None, [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 10.0, 0, 0, 1]],
               [(2, p) for p in pts])
    case.work_base = True
    return [case]


def rotated_chain_cases(contained, ni):
    rng = random.Random(20261011)
    quats, trans = [], []
    for _ in range(40):
        q = [rng.uniform(-1, 1) for _ in range(4)]
        n = math.sqrt(sum(v * v for v in q))
        quats.append([v / n for v in q])
        trans.append([rng.uniform(-10, 10) for _ in range(3)])

    def spec_for(depth):
        nodes = []
        for i in range(depth):
            node = {"rotation": quats[i], "translation": trans[i]}
            if i + 1 < depth:
                node["children"] = [i + 1]
            else:
                node["mesh"] = 0
            nodes.append(node)
        return {"positions": [[1, 2, 3]], "nodes": nodes, "scenes": [{"nodes": [0]}], "scene": 0}

    def make(depth, sphere):
        spec = spec_for(depth)
        p = B.tile_points(spec["nodes"], list(range(depth)), F([1, 2, 3]))
        cid = "rotated-TRS-chain-depth-%d-%s" % (depth, "with-sphere" if sphere else "box-only")
        case = Case(cid, "control", ni("numericSpan") if sphere else contained(), "newSlice1Controls[11]")
        chain_case(case, "w.glb", payload_from_spec(spec), [None], [(depth - 1, p)], content_volume=False)
        if sphere:
            c = case.get("tileset.json", ("root", "boundingVolume"))["box"][:3]
            case.tilesets["tileset.json"]["root"]["content"]["boundingVolume"] = {"sphere": c + [100.0]}
            case.mutants = False
        return case

    # Depth: the smallest depth where the oracle's normative spans put the sphere
    # variant over 4096 while the box-only variant stays within (parameter choice only).
    import tempfile
    chosen = None
    with tempfile.TemporaryDirectory() as tmp:
        for depth in range(10, 33):
            spans = []
            for sphere in (False, True):
                case = make(depth, sphere)
                path = os.path.join(tmp, "probe.3tz")
                B.write_3tz(path, case.members())
                report = Oracle(path, {"containmentSpanBits": 1 << 30}).run()
                spans.append(max([report["precheck"]["references"][0]["sInst"]]
                                 + report["precheck"]["references"][0]["levelSpans"]))
            if spans[0] <= 4096 < spans[1]:
                chosen = depth
                break
    out = [make(chosen, False), make(chosen, True)]
    out[1].production = "C1:InvalidInput (content bounds escape tile)"
    out[1].c1_refused = "invalid_input"
    # C1-passing variant: the tile box encloses the content sphere (radius 100 in a +-128 box).
    enclosed = make(chosen, True)
    enclosed.id += "-enclosed"
    box = enclosed.get("tileset.json", ("root", "boundingVolume"))["box"]
    enclosed.set("tileset.json", ("root", "boundingVolume"),
                 {"box": box[:3] + [128.0, 0.0, 0.0, 0.0, 128.0, 0.0, 0.0, 0.0, 128.0]})
    out.append(enclosed)
    for case in out:
        case.notes.append("depth %d chosen as the smallest depth (from 10) where box-only spans <= 4096 "
                          "< spans with the sphere (generic normalized quaternions, seed 20261011)" % chosen)
    return out


def polygon_cases(contained):
    g = B.Gltf()
    positions = [[0, 0, 0], [1, 0, 0], [0, 1, 0], [500, 500, 500]]
    pos = g.positions(positions)
    idx = g.indices([0, 1, 2], 5125)
    ext = {"count": 1, "indicesOffsets": g.scalars([0], 5125), "loopIndices": g.scalars([0, 1, 3], 5125),
           "loopIndicesOffsets": g.scalars([0], 5125)}
    g.doc["meshes"] = [{"primitives": [{"attributes": {"POSITION": pos}, "indices": idx, "mode": 4,
                                        "extensions": {"EXT_mesh_polygon": ext}}]}]
    g.doc["nodes"] = [{"mesh": 0}]
    g.doc["scenes"] = [{"nodes": [0]}]
    g.doc["scene"] = 0
    g.doc["extensionsUsed"] = ["EXT_mesh_polygon"]
    case = Case("EXT_mesh_polygon-undrawn-loop-vertex-outside", "control", contained(), "newSlice1Controls[12]")
    pts = [B.up(F(p)) for p in positions[:3]]
    chain_case(case, "w.glb", g.glb(), [None], [(0, p) for p in pts])
    case.expect_observation = {"outsideAnyVolume": 1, "firstOutside": [3]}
    return [case]


def identity_cases(contained):
    out = []
    parent = {"translation": [3.25, -7.0, 1000000.0], "rotation": [0.6, 0.0, 0.0, 0.8], "children": [1]}
    for name, child in (("mesh-only", {"mesh": 0}),
                        ("identity-TRS", {"mesh": 0, "translation": [0, 0, 0], "rotation": [0, 0, 0, 1],
                                          "scale": [1, 1, 1]}),
                        ("identity-matrix", {"mesh": 0, "matrix": [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]})):
        spec = {"positions": [[1, 2, 3]], "nodes": [parent, child], "scenes": [{"nodes": [0]}], "scene": 0}
        p = B.tile_points(spec["nodes"], [0, 1], F([1, 2, 3]))
        case = Case("identity-node-equivalence-" + name, "control", contained(), "newSlice1Controls[13]")
        chain_case(case, "w.glb", payload_from_spec(spec), [None, [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 5, 0, 0, 1]],
                   [(1, p)])
        case.identity_group = True
        out.append(case)
    return out


def hull_cases(contained, ni):
    out = []
    big, small = 2.0 ** 1000, 2.0 ** -1000
    nodes = [{"scale": [big, big, big], "mesh": 0}, {"scale": [small, small, small], "mesh": 0}]
    for cid, roots, expect in (("hull-one-instance", [0], contained()),
                               ("hull-two-instances", [0, 1], ni("numericSpan"))):
        spec = {"positions": [[1, 2, 3]], "nodes": nodes, "scenes": [{"nodes": roots}], "scene": 0}
        pts = [B.tile_points(nodes, [n], F([1, 2, 3])) for n in roots]
        d2 = max(sum(c * c for c in p) for p in pts)
        r = float(math.sqrt(14.0)) * big
        while Fraction(r) ** 2 < d2:
            r = B.next_up(r)
        while Fraction(B.next_down(r)) ** 2 >= d2:
            r = B.next_down(r)
        case = Case(cid, "control", expect, "newSlice1Controls[14] (v3.1 H1)")
        case.payloads.append(("w.glb", payload_from_spec(spec)))
        case.tilesets["tileset.json"] = B.tileset(B.tile({"sphere": [0.0, 0.0, 0.0, r]}, content="w.glb"))
        case.tracks = [(n, [p]) for n, p in zip(roots, pts)]
        case.sphere_mutant = ("tileset.json", ("root", "boundingVolume"), B.next_down(r))
        out.append(case)
    return out


# Hand derivation of the repeated-index archive's work units (C2-WB 3.1, 3.2;
# independent of the oracle). Per payload: FLOAT POSITION word pair (-149, 127),
# one skipped {"mesh":0} node so G = I and W = (0, 0). Level 0 (child tile) and
# level 1 (root, no transform) carry the same box c = (0.5, 0, 0.5),
# h = (0.5, 0, 0.5): D_0 = sum[(-149,127), (-1,-1)] = (-149, 128), span 278, so
# s_0 = s_1 = 278 and q = ceil(278/64)^2 = 25 per level.
#   T = 3 distinct vertices (deduplicated, not 3,999,990 occurrences)
#   V = max(20, 25) + max(25, 25) [level 0: base + own box]
#     + max(25, 25)               [level 1: root box, no step]       = 75
#   vertex units = 3 * 75 = 225; Nodes = 1 (skipped node)
#   Idx = 3,999,990 indices; E = 9 + 3,999,990 = 3,999,999
#   Dec = 0 (no external, data-URI or meshopt buffer); Res = 0; Prim = 1
#   B = the GLB member length; J = value nodes of its JSON chunk (counted below)
#   + 1152 + 8 * 2 levels
# three identical payloads, one reference each.
REPEATED_INDEX_UNITS = None  # set by repeated_index_cases() from the payload bytes


def json_value_nodes(value):
    """Root plus every scalar/container value; object keys are not value nodes."""
    if isinstance(value, dict):
        return 1 + sum(json_value_nodes(v) for v in value.values())
    if isinstance(value, list):
        return 1 + sum(json_value_nodes(v) for v in value)
    return 1


def repeated_index_hand_units(glb):
    json_len = struct.unpack_from("<I", glb, 12)[0]
    J = json_value_nodes(json.loads(glb[20:20 + json_len]))
    per_payload = (3 * (max(20, 25) + max(25, 25) + max(25, 25)) + 1 + 3999990 + (9 + 3999990)
                   + 0 + len(glb) + 16 * J + 160 * 0 + 4 * 1 + 1152 + 8 * 2)
    return 3 * per_payload


def repeated_index_cases(contained, ni):
    out = []
    positions = [[0, 0, 0], [1, 0, 0], [0, 1, 0]]
    pattern = [0, 1, 2] * (3999990 // 3)
    case = Case("repeated-index-three-payloads", "control", contained(), "newSlice1Controls[10]")
    children = []
    for i in range(3):
        g = B.Gltf()
        pos = g.positions(positions)
        idx = g.indices(pattern, 5123)
        g.doc["meshes"] = [{"primitives": [{"attributes": {"POSITION": pos}, "indices": idx, "mode": 4}]}]
        g.doc["nodes"] = [{"mesh": 0}]
        g.doc["scenes"] = [{"nodes": [0]}]
        g.doc["scene"] = 0
        case.payloads.append(("p%d.glb" % i, g.glb()))
    global REPEATED_INDEX_UNITS
    assert len({p for _, p in case.payloads}) == 1  # three identical payloads
    REPEATED_INDEX_UNITS = repeated_index_hand_units(case.payloads[0][1])
    for i in range(3):
        children.append(B.tile(B.tight_box([B.up(F(p)) for p in positions]), content="p%d.glb" % i))
    case.tilesets["tileset.json"] = B.tileset(B.tile(B.tight_box([B.up(F(p)) for p in positions]),
                                                     geometric_error=10.0, children=children))
    case.large = True
    case.mutants = False
    case.notes.append("24 MB stored archive: generated with --large into controls/large/ (not committed).")
    out.append(case)
    return out


def v33_cases(W, contained, ni, single):
    """Fixtures for the coordinator's v3.3 decisions (3), (8) and (10)."""
    out = []
    w12, w02 = W["W12"], W["W02"]
    w12pts = literal_points(w12)
    vector = "3DTILES_content_gltf_vector"

    # (3) clip: true is a non-allowlisted use on either side.
    for cid, ext, expect in (("vector-content-without-clip", {"vector": True}, contained()),
                             ("vector-content-clip-true", {"vector": True, "clip": True},
                              ni("tilesetExtension"))):
        case = single(cid, "control", w12, expect, points=w12pts, source="v3.3 (3)")
        ts = case.tilesets["tileset.json"]
        ts["extensionsUsed"] = [vector]
        ts["root"]["content"]["extensions"] = {vector: ext}
        out.append(case)

    def gltf_clip(g):
        add_used(g, vector)
        g.doc["extensions"] = {vector: {"vector": True, "clip": True}}
    out.append(single("vector-clip-on-glTF-side", "control", w12, ni("gltfExtension"), points=w12pts,
                      patch=gltf_clip, source="v3.3 (3)"))

    # (8) non-affine tile transform and an overflowing tileset literal are InvalidInput.
    case = Case("tile-transform-non-affine", "control", {"outcome": "InvalidInput"}, "v3.3 (8)")
    chain_case(case, "w.glb", payload_from_spec(w12),
               [None, [1, 0, 0, 0.5, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]], [(0, w12pts[0])])
    case.mutants = False
    out.append(case)
    case = single("tileset-number-overflow", "control", w12, {"outcome": "InvalidInput"}, points=w12pts,
                  source="v3.3 (8) / contract Numbers")
    case.tilesets["tileset.json"]["geometricError"] = 12345.678
    case.raw_replace.append(("tileset.json", b"12345.678", b"1e999"))
    case.mutants = False
    case.production = "C1:Unsupported (JSON exceeds finite owned-document representation)"
    case.c1_refused = "unsupported"
    case.notes.append("Contract 'Numbers' says an overflowing literal is InvalidInput; production C1 "
                      "refuses it as Unsupported in its JSON layer. The oracle keeps the contract text.")
    out.append(case)

    # (8) duplicates in children or scene roots are InvalidInput(node).
    spec = copy.deepcopy(w12)
    spec["nodes"][0]["children"] = [1, 1]
    spec["nodes"].append({})
    out.append(single("duplicate-child", "control", spec, {"outcome": "InvalidNode"}, points=w12pts,
                      nodes=[0], source="v3.3 (8)"))
    spec = copy.deepcopy(w12)
    spec["scenes"] = [{"nodes": [0, 0]}]
    out.append(single("duplicate-scene-root", "control", spec, {"outcome": "InvalidNode"}, points=w12pts,
                      nodes=[0], source="v3.3 (8)"))

    # (10) each contents entry is a separate reference, in array order.
    a, b = w12pts[0], literal_points(w02)[0]
    for cid, exclude in (("contents-array-both-inside", False), ("contents-array-second-outside", True)):
        expect = ({"outcome": "Outside", "facts": {"reference": 1, "level": 0, "depth": 0, "role": "content",
                                                   "node": 1}} if exclude else contained())
        case = Case(cid, "control", expect, "v3.3 (10)")
        case.payloads += [("a.glb", payload_from_spec(w12)), ("b.glb", payload_from_spec(w02))]
        tile = B.tile(B.tight_box([a, b]))
        tile["contents"] = [{"uri": "a.glb", "boundingVolume": B.tight_box([a])},
                            {"uri": "b.glb", "boundingVolume": B.tight_box([a] if exclude else [b])}]
        case.tilesets["tileset.json"] = B.tileset(tile)
        case.tracks = [(0, [a]), (1, [b])]
        case.tested = [{"member": "tileset.json", "path": ("root", "boundingVolume"), "level": 0, "role": "own"}]
        case.mutants = not exclude
        out.append(case)
    return out


def role_order_case(contained):
    """Production mutation testing, parity run 2: a vertex outside BOTH the
    externalContent and the ancestor volume at the same level, so that only the
    Role order (ExternalContent before Ancestor) gives role externalContent.

    Exact declared nesting cannot hold here: the vertex lies in the external root
    box, so if the external root were exactly inside the referencing tile's box the
    vertex would be inside it too. The referencing tile box therefore excludes the
    vertex by a quarter ulp, and the external root escapes it by that much. This
    relies on C1 hierarchyBounds slack (all 152 one-ulp mutants passed C1 in
    parity run 1)."""
    p = F([8, -42, 14])
    case = Case("external-content-and-ancestor-both-exclude", "control",
                {"outcome": "Outside", "facts": {"reference": 0, "level": 1, "depth": 1,
                                                 "role": "externalContent", "node": 0}},
                "production mutation testing (role order within a level)")
    case.payloads.append(("w.glb", payload_from_spec({"positions": [[1, 2, 3]],
                                                      "nodes": [{"translation": [7, 12, 39], "mesh": 0}],
                                                      "scenes": [{"nodes": [0]}], "scene": 0})))
    assert B.tile_points([{"translation": [7, 12, 39]}], [0], F([1, 2, 3])) == p
    ext_root = {"box": [4.0, -42.0, 14.0, 4.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]}  # x in [0, 8]
    ref_box = {"box": [4.0, -42.0, 14.0, B.next_down(4.0), 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]}
    content = {"box": [3.75, -42.0, 14.0, 3.25, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]}  # x in [0.5, 7]
    assert B.box_contains(ext_root, p) and not B.box_contains(ref_box, p) and not B.box_contains(content, p)
    case.tilesets["tileset.json"] = B.tileset({"boundingVolume": ref_box, "geometricError": 10.0,
                                               "content": {"uri": "ext.json", "boundingVolume": content}})
    case.tilesets["ext.json"] = B.tileset({"boundingVolume": ext_root, "geometricError": 0.0,
                                           "content": {"uri": "w.glb"}})
    case.tracks = [(0, [p, p])]
    case.leaf_depth = 1
    case.mutants = False
    case.notes.append("Under slot order [Ancestor, ..., ExternalContent] the reported role would be "
                      "ancestor; Role order gives externalContent (v3.2 (3)).")
    case.notes.append("Not buildable with exact nesting; C1-passing only through hierarchyBounds slack "
                      "for a quarter-ulp external-root escape. The external root is not compared with "
                      "the referencing content volume by C1 (external-content-volume-excludes passed "
                      "C1 in parity run 1).")
    return [case]


def review_b1_cases():
    """PR #157 nonauthor review B1 (probe/empty_mesh_span.py, sha256 db8b8886...),
    regenerated here: three nested nodes rotated by [2^-1000, 0, 0, 1] end in an
    empty mesh (no primitives). Alone, the URI has no drawn instance and skips the
    span check (v3.2 (4)): Contained. With a real triangle instance added at the
    scene root, sInst(u) covers the empty mesh's instance path too: numericSpan."""
    x = 2.0 ** -1000
    rot = [x, 0.0, 0.0, 1.0]
    out = []
    for cid, real, expect in (("review-B1-empty-mesh-only", False, {"outcome": "Contained"}),
                              ("review-B1-empty-mesh-plus-real",
                               True, {"outcome": "NotInspected", "reasons": ["numericSpan"]})):
        nodes = [{"rotation": rot, "children": [1]}, {"rotation": rot, "children": [2]},
                 {"rotation": rot, "mesh": 0}]
        doc = {"asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}],
               "meshes": [{"primitives": []}], "nodes": nodes}
        binary = b""
        if real:
            binary = struct.pack("<9f", 0, 0, 0, 1, 0, 0, 0, 1, 0)
            doc["buffers"] = [{"byteLength": 36}]
            doc["bufferViews"] = [{"buffer": 0, "byteLength": 36}]
            doc["accessors"] = [{"bufferView": 0, "componentType": 5126, "type": "VEC3", "count": 3,
                                 "min": [0, 0, 0], "max": [1, 1, 0]}]
            doc["meshes"].append({"primitives": [{"attributes": {"POSITION": 0}}]})
            doc["nodes"].append({"mesh": 1})
            doc["scenes"][0]["nodes"].append(3)
        case = Case(cid, "review", expect, "PR #157 nonauthor review B1")
        case.payloads.append(("a.glb", B.glb_bytes(doc, binary)))
        case.tilesets["tileset.json"] = B.tileset(B.tile({"box": [0, 0, 0, 10, 0, 0, 0, 10, 0, 0, 0, 10]},
                                                         content="a.glb"))
        case.mutants = False
        if real:
            case.expect_sinst = 6008
        else:
            case.notes.append("No drawn vertex, so no one-ulp mutant exists.")
        out.append(case)
    return out


# ---------------------------------------------------------------- C1 precondition (v3.3 (9))


def _intervals(bv):
    """Exact per-row [lo, hi] of a signed-permutation box, or None."""
    if "box" not in bv:
        return None
    b = [Fraction(v) for v in bv["box"]]
    cols = [b[3:6], b[6:9], b[9:12]]
    out = []
    for row in range(3):
        cover = [col[row] for col in cols if col[row] != 0]
        if len(cover) > 1 or any(sum(1 for r in range(3) if col[r] != 0) > 1 for col in cols):
            return None
        h = abs(cover[0]) if cover else Fraction(0)
        out.append((b[row] - h, b[row] + h))
    return out


def _image(iv, transform):
    """Exact axis-aligned image of row intervals under a signed-permutation scale
    + translation transform, or None for other transforms."""
    if transform is None:
        return iv
    T = B.col_major(transform)
    out = []
    for r in range(3):
        cols = [m for m in range(3) if T[r][m] != 0]
        if len(cols) != 1 or any(T[3][c] != (1 if c == 3 else 0) for c in range(4)):
            return None
        m = cols[0]
        ends = sorted([T[r][m] * iv[m][0] + T[r][3], T[r][m] * iv[m][1] + T[r][3]])
        out.append(tuple(ends))
    return out


def _sphere_intervals(bv):
    c = [Fraction(v) for v in bv["sphere"]]
    return [(c[j] - c[3], c[j] + c[3]) for j in range(3)]


def _kind(bv):
    return "region" if "region" in bv else ("sphere" if "sphere" in bv else ("box" if "box" in bv else None))


def _escape(child, parent, transform=None):
    """Exact amount by which `child` (mapped by transform) escapes `parent`.

    Returns a Fraction, "mixed" for region versus Cartesian nesting, or None when
    the generator cannot judge the shapes (rotated boxes, non-permutation transforms,
    transformed sphere in sphere)."""
    ck, pk = _kind(child), _kind(parent)
    if ck is None or pk is None:
        return None
    if (ck == "region") != (pk == "region"):
        return "mixed"
    if ck == "region":
        c, p = [Fraction(v) for v in child["region"]], [Fraction(v) for v in parent["region"]]
        return max([Fraction(0), p[0] - c[0], p[1] - c[1], c[2] - p[2], c[3] - p[3], p[4] - c[4], c[5] - p[5]])
    if ck == "sphere" and pk == "sphere":
        if transform is not None:
            return None
        c, p = [Fraction(v) for v in child["sphere"]], [Fraction(v) for v in parent["sphere"]]
        d2 = sum((c[j] - p[j]) ** 2 for j in range(3))
        if p[3] >= c[3] and d2 <= (p[3] - c[3]) ** 2:
            return Fraction(0)
        return Fraction(math.sqrt(float(d2))) + c[3] - p[3]
    # The child's exact axis-aligned extent: a box's intervals, or a sphere's
    # bounding box (exact containment test against an axis box parent).
    ci = _intervals(child) if ck == "box" else _sphere_intervals(child)
    if ci is None:
        return None
    ci = _image(ci, transform)
    if ci is None:
        return None
    if pk == "box":
        pi = _intervals(parent)
        if pi is None:
            return None
        return max([Fraction(0)] + [pi[j][0] - ci[j][0] for j in range(3)] + [ci[j][1] - pi[j][1] for j in range(3)])
    if ck != "box":
        return None
    # Box in sphere: the farthest corner must be within the radius.
    p = [Fraction(v) for v in parent["sphere"]]
    far2 = sum(max((ci[j][0] - p[j]) ** 2, (ci[j][1] - p[j]) ** 2) for j in range(3))
    if far2 <= p[3] ** 2:
        return Fraction(0)
    return Fraction(math.sqrt(float(far2))) - p[3]


def c1_precondition(case, report=None):
    """Best static judgement of whether production's C1 refuses the fixture before
    C2. Declared nesting is checked exactly (tile in parent, content in tile,
    external root in its referencing tile and, conditionally, in that content
    volume) for box, sphere and region shapes; region nested with box/sphere is
    refused as Unsupported (parity run 1). The oracle's c1Caveats for
    non-allowlisted required extensions and JOINTS_n/WEIGHTS_n attributes are
    refusals too (parity run 1)."""
    escapes = []
    mixed = []

    def note(pair, child, parent, transform=None, conditional=False):
        e = _escape(child, parent, transform)
        if e == "mixed":
            mixed.append(pair)
            return
        if e is None or e == 0:
            return
        values = [abs(float(v)) for v in (parent.get("box") or parent.get("sphere") or parent.get("region"))]
        face = (max(values[:3]) + max(values[3:])) or 1.0
        escapes.append({"pair": pair, "escape": float(e), "ulps": float(e / Fraction(math.ulp(face))),
                        "conditional": conditional})

    def walk(member, tile, where, parent_bv):
        bv = tile["boundingVolume"]
        if parent_bv is not None:
            note("%s in parent" % where, bv, parent_bv, tile.get("transform"))
        contents = ([tile["content"]] if "content" in tile else []) + tile.get("contents", [])
        for ci, content in enumerate(contents):
            cbv = content.get("boundingVolume")
            if cbv is not None:
                note("%s content[%d] in tile" % (where, ci), cbv, bv)
            if content["uri"].endswith(".json"):
                ext = case.tilesets[content["uri"]]["root"]
                note("%s external root in referencing tile" % content["uri"], ext["boundingVolume"], bv,
                     ext.get("transform"))
                if cbv is not None:
                    note("%s external root in referencing content volume" % content["uri"],
                         ext["boundingVolume"], cbv, ext.get("transform"), conditional=True)
                walk(content["uri"], ext, content["uri"] + ":root", None)
        for i, child in enumerate(tile.get("children", [])):
            walk(member, child, "%s.children[%d]" % (where, i), bv)

    walk("tileset.json", case.tilesets["tileset.json"]["root"], "tileset.json:root", None)
    refusals = [c for c in ((report or {}).get("c1Caveats") or [])
                if "extensionsRequired" in c or "attribute" in c]

    def non_finite(value):
        if isinstance(value, float):
            return not math.isfinite(value)
        if isinstance(value, dict):
            return any(non_finite(v) for v in value.values())
        if isinstance(value, list):
            return any(non_finite(v) for v in value)
        return False
    for name, data in case.members():
        if name.endswith(".json") and non_finite(json.loads(data)):
            refusals.append("%s: JSON number literal overflows binary64 (C1 JSON layer: Unsupported)" % name)
    hard = [e for e in escapes if not e["conditional"]]
    if mixed or refusals:
        return {"judgement": "likely", "expectedProductionIfRefused": "C1:Unsupported",
                "mixedRegionCartesian": mixed, "c1Caveats": refusals, "escapes": escapes}
    if not escapes:
        return None
    if any(e["ulps"] > 4 for e in hard):
        judgement = "likely"
    elif hard:
        judgement = "possible: declared-nesting escape of at most 4 ulps; refused only if C1 slack is smaller"
    else:
        judgement = ("possible: only if C1 compares an external tileset root with the referencing "
                     "content.boundingVolume")
    return {"judgement": judgement, "expectedProductionIfRefused": "C1:InvalidInput(hierarchyBounds)",
            "escapes": escapes}


# ---------------------------------------------------------------- expectations for mutants


def stream_first_failure(case, level, role, volume):
    """First track (streaming order) excluded by a mutated volume at (level, role)."""
    for node, levels in case.tracks:
        if not B.box_contains(volume, levels[level]):
            return node
    return None


def mutants_of(case):
    out = []
    for tv in case.tested:
        base = case.get(tv["member"], tv["path"])
        if "box" not in base:
            continue
        points = [levels[tv["level"]] for _, levels in case.tracks]
        for row, mutated, how in B.ulp_mutants(base, points):
            m = copy.deepcopy(case)
            m.id = "%s~ulp-L%d-%s-r%d" % (case.id, tv["level"], tv["role"], row)
            m.group = case.group + "-mutant"
            m.set(tv["member"], tv["path"], mutated)
            node = stream_first_failure(case, tv["level"], tv["role"], mutated)
            m.expect = {"outcome": "Outside", "facts": {"level": tv["level"], "depth": case.leaf_depth,
                                                        "role": tv["role"], "node": node}}
            m.notes = ["one-ulp mutant of %s: %s at %s" % (case.id, how, "/".join(map(str, tv["path"])))]
            if tv["level"] > 0 or tv["role"] == "content" or base["box"][3 + 4 * row] == 0:
                m.notes.append("declared nesting may be broken by one ulp; production C1 "
                               "hierarchyBounds slack decides whether C2 is reached")
            m.mutants = False
            m.literal = []
            m.expect_observation = None
            if hasattr(m, "raw_reading"):
                del m.raw_reading
            out.append(m)
    if getattr(case, "sphere_mutant", None):
        member, path, r = case.sphere_mutant
        m = copy.deepcopy(case)
        m.id = case.id + "~ulp-sphere-r"
        m.group = case.group + "-mutant"
        m.get(member, path)["sphere"][3] = r
        m.expect = {"outcome": "Outside", "facts": {"level": 0, "depth": case.leaf_depth, "role": "own",
                                                    "node": case.tracks[0][0]}}
        m.notes = ["sphere radius one ulp smaller than the smallest enclosing binary64 radius"]
        m.mutants = False
        out.append(m)
    return out


# ---------------------------------------------------------------- evaluation


def compare(expect, report, case):
    outcome = report.get("outcome")
    if outcome != expect["outcome"]:
        return False
    if outcome == "NotInspected":
        return sorted(report.get("reasons", [])) == expect["reasons"]
    if outcome == "Outside":
        facts = report["facts"]
        for key, value in expect.get("facts", {}).items():
            if value is not None and facts.get(key) != value:
                return False
    if getattr(case, "expect_c1_caveat", None):
        if not any(case.expect_c1_caveat in c for c in report.get("c1Caveats", [])):
            return False
    return True


def literal_results(case, oracle):
    out = []
    trace = oracle.trace()
    for chk in case.literal:
        if chk.get("generator_only"):
            node, levels = case.tracks[chk["track"]]
            got = levels[chk["level"]]
            out.append({"what": chk["what"] + " [generator chain arithmetic]",
                        "expected": [str(v) for v in chk["expected"]], "got": [str(v) for v in got],
                        "ok": got == chk["expected"]})
            continue
        entry = trace[chk["track"]] if chk["track"] < len(trace) else None
        if entry is None:
            out.append({"what": chk["what"], "ok": False, "got": None})
            continue
        got = entry["node_yp"] if chk.get("node_yp") else entry["levels"][chk["level"]]
        out.append({"what": chk["what"], "expected": [str(v) for v in chk["expected"]],
                    "got": [str(v) for v in got], "node": entry["node"], "ok": list(got) == list(chk["expected"])})
    if case.draw_visits is not None:
        out.append({"what": "draw_visits", "expected": case.draw_visits, "got": len(trace),
                    "ok": len(trace) == case.draw_visits})
    if hasattr(case, "raw_reading"):
        own = case.get("tileset.json", ("root", "boundingVolume"))
        raw_inside = B.box_contains(own, case.raw_reading)
        out.append({"what": "W13 raw-polynomial reading is outside the normalized-tight own box",
                    "got": not raw_inside, "ok": not raw_inside})
    return out


def run_case(case, directory, report_rows, large_ok):
    if case.large and not large_ok:
        report_rows.append({"id": case.id, "group": case.group, "skipped": "needs --large"})
        return None
    path = os.path.join(directory, case.id.replace("/", "_").replace(" ", "_") + ".3tz")
    digest = B.write_3tz(path, case.members())
    oracle = Oracle(path, case.limits)
    report = oracle.run()
    row = {"id": case.id, "group": case.group, "source": case.source,
           "file": os.path.relpath(path, TOP), "sha256": digest, "limits": case.limits,
           "expected": case.expect, "oracle": {k: report.get(k) for k in ("outcome", "reasons", "facts", "why")},
           "c1Caveats": report.get("c1Caveats", []), "notes": case.notes}
    if case.production:
        row["expectedProduction"] = case.production
    if case.c1_refused:
        # Production stops in C1; the oracle (no C1 model) is judged on its C2-level expectation.
        row["expected"] = {"outcome": "C1Refused", "kind": case.c1_refused}
        row["oracleExpected"] = case.expect
    pre = c1_precondition(case, report)
    if pre:
        row["c1Precondition"] = pre
    row["agree"] = compare(case.expect, report, case)
    if report.get("outcome") in ("Contained", "Outside") or case.literal:
        try:
            row["literal"] = literal_results(case, oracle)
            row["agree"] = row["agree"] and all(r["ok"] for r in row["literal"])
        except Exception as err:  # never hide a trace failure
            row["literal"] = [{"what": "trace", "ok": False, "got": repr(err)}]
            row["agree"] = False
    if getattr(case, "expect_spans", None):
        refs = (report.get("precheck") or {}).get("references") or []
        got = {"sInst": refs[0].get("sInst"), "levelSpans": refs[0].get("levelSpans")} if refs else None
        row["spans"] = {"expected": case.expect_spans, "got": got}
        row["agree"] = row["agree"] and got == case.expect_spans
    if getattr(case, "expect_sinst", None) is not None:
        refs = (report.get("precheck") or {}).get("references") or []
        got = refs[0].get("sInst") if refs else None
        row["sInst"] = {"expected": case.expect_sinst, "got": got}
        row["agree"] = row["agree"] and got == case.expect_sinst
    if getattr(case, "expect_observation", None):
        obs = report.get("observations") or []
        got = {k: obs[0][k] for k in case.expect_observation} if obs else None
        row["observation"] = {"expected": case.expect_observation, "got": got}
        row["agree"] = row["agree"] and got == case.expect_observation
    if report.get("precheck"):
        row["precheck"] = report["precheck"]
    report_rows.append(row)
    return report


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--large", action="store_true", help="also build and run the 24 MB repeated-index control")
    args = parser.parse_args()
    inputs = load_inputs()
    cases = build_cases(inputs)
    wdir = os.path.join(HERE, "fixtures")
    cdir = os.path.join(TOP, "controls", "fixtures")
    ldir = os.path.join(TOP, "controls", "large")
    for d in (wdir, cdir, ldir):
        os.makedirs(d, exist_ok=True)
    for d in (wdir, cdir):
        for name in os.listdir(d):
            os.remove(os.path.join(d, name))
    rows = []
    reports = {}
    all_cases = []
    for case in cases:
        all_cases.append(case)
        if case.mutants and case.expect["outcome"] == "Contained":
            all_cases += mutants_of(case)
    for case in all_cases:
        directory = ldir if case.large else (cdir if case.group.startswith("control") else wdir)
        reports[case.id] = run_case(case, directory, rows, args.large)
    controls = build_controls(cases, reports, rows, cdir, ldir, args.large)
    identity = [r for r in rows if r["id"].startswith("identity-node-equivalence-") and "~" not in r["id"]]
    spans = [json.dumps(r.get("precheck", {}).get("references", [{}])[0].get("levelSpans")) for r in identity]
    summary = {
        "cases": len([r for r in rows if "skipped" not in r]),
        "agree": len([r for r in rows if r.get("agree")]),
        "disagree": [r["id"] for r in rows if "skipped" not in r and not r.get("agree")],
        "skipped": [r["id"] for r in rows if "skipped" in r],
        "c1PreconditionLikely": [r["id"] for r in rows
                                 if (r.get("c1Precondition") or {}).get("judgement") == "likely"],
        "c1PreconditionPossible": len([r for r in rows
                                       if (r.get("c1Precondition") or {}).get("judgement", "").startswith("possible")]),
        "identityLevelSpansIdentical": len(set(spans)) == 1,
        "controlPairs": len(controls["pairs"]),
        "controlPairsAgree": len([p for p in controls["pairs"] if p["agree"]]),
    }
    with open(os.path.join(HERE, "report.json"), "w") as fh:
        json.dump({"summary": summary, "inputs": {k: v[1] for k, v in INPUT_PINS.items()},
                   "rows": rows}, fh, indent=1, sort_keys=True)
    print(json.dumps(summary, indent=1))
    return 0 if not summary["disagree"] and summary["controlPairsAgree"] == summary["controlPairs"] else 1


def build_controls(cases, reports, rows, cdir, ldir, large_ok):
    """Data-control pairs: base fixture + mutated fixture, both hashed, with expectations."""
    by_id = {r["id"]: r for r in rows}
    pairs = []

    def pair(name, base_id, mutated_id, what):
        base, mut = by_id.get(base_id), by_id.get(mutated_id)
        if base is None or mut is None:
            raise SystemExit("control %s refers to unknown case" % name)
        if "skipped" in base or "skipped" in mut:
            pairs.append({"control": name, "what": what, "skipped": "needs --large", "agree": True})
            return
        pairs.append({"control": name, "what": what,
                      "base": {k: base[k] for k in ("id", "file", "sha256", "limits", "expected", "oracle")},
                      "mutated": {k: mut[k] for k in ("id", "file", "sha256", "limits", "expected", "oracle")},
                      "agree": bool(base["agree"] and mut["agree"])})

    pair("one-ulp-outside", "W12-TRS-half-turn-order", "W12-TRS-half-turn-order~ulp-L0-own-r0",
         "own box row 0 half-length one ulp smaller")
    pair("RTC-added", "W01-noRTC-variant", "W01-ordered-node-U-RTC-tile-chain", "b3dm feature table gains RTC_CENTER")
    pair("quaternion-past-window", "quaternion-window-at-2^-18", "quaternion-window-next-beyond",
         "x component moves to the next binary64")
    pair("span-at-cap-and-one-over", "worked-example-adjusted-containmentSpanBits-29",
         "worked-example-adjusted-containmentSpanBits-28", "same fixture, containmentSpanBits 29 -> 28")
    pair("external-content-volume-shrunk", "external-content-volume-contains", "external-content-volume-excludes",
         "referencing tile content.boundingVolume shrunk to exclude the payload position")
    pair("same-URI-different-reference", "same-URI-both-references-inside (control base)",
         "same-URI-other-reference-outside", "second reference's own volume excludes the second instance")
    pair("hull-two-instances", "hull-one-instance", "hull-two-instances", "second instance added to the URI")
    pair("tight-non-integer-centre", "tight-non-integer-centre-on-boundary",
         "tight-non-integer-centre-vertex-one-ulp-beyond", "vertex moves one f32 ulp beyond the face")
    pair("rotated-chain-sphere", [c.id for c in cases if c.id.startswith("rotated-TRS") and "box-only" in c.id][0],
         [c.id for c in cases if c.id.startswith("rotated-TRS") and c.id.endswith("with-sphere-enclosed")][0],
         "sphere content volume added at the same depth")
    # Undrawn outlier made drawn (W04 indices [0] -> [0, 1]).
    w04 = [c for c in cases if c.id.startswith("W04")][0]
    drawn = copy.deepcopy(w04)
    drawn.id = "W04-outlier-made-drawn"
    drawn.group = "control"
    spec = json.loads(json.dumps(load_inputs()["witnesses.json"]["cases"][3]))
    spec["indices"] = [0, 1]
    drawn.payloads = [("w.glb", payload_from_spec(spec))]
    drawn.expect = {"outcome": "Outside", "facts": {"level": 0, "depth": 0, "role": "own", "vertex": 1,
                                                    "primitive": 0}}
    drawn.literal = []
    drawn.draw_visits = None
    drawn.notes = ["W04 with indices [0, 1]: the outlier (1000, -1000, 1000) is drawn"]
    run_case(drawn, cdir, rows, large_ok)
    by_id[drawn.id] = rows[-1]
    pair("undrawn-outlier-made-drawn", w04.id, drawn.id, "index 1 added to the draw")
    # Work caps at the base archive's computed totals and one below.
    base_row = by_id["work-base-meshopt"]
    totals = base_row["precheck"]["totals"]
    work_case = [c for c in cases if c.id == "work-base-meshopt"][0]
    for limit, key in (("containmentWorkUnits", "units"), ("containmentBytesRead", "bytes"),
                       ("containmentDecodedBytes", "decoded"), ("containmentNodeDepth", "maxNodeDepth")):
        for delta in (0, -1):
            c = copy.deepcopy(work_case)
            c.id = "work-%s-%s" % (limit, "at-sum" if delta == 0 else "sum-minus-1")
            c.limits = {limit: totals[key] + delta}
            c.expect = {"outcome": "Contained"} if delta == 0 else {"outcome": "NotInspected", "reasons": ["workLimit"]}
            c.mutants = False
            c.literal = []
            run_case(c, cdir, rows, large_ok)
            by_id[c.id] = rows[-1]
        pair("%s-at-cap-and-one-over" % limit, "work-%s-at-sum" % limit, "work-%s-sum-minus-1" % limit,
             "%s = archive total (%d), then total - 1" % (limit, totals[key]))
    # Repeated-index pairs (large). The units cap comes from the hand derivation
    # REPEATED_INDEX_UNITS (independent of the oracle); the oracle's own total must equal it.
    rep = [c for c in cases if c.id == "repeated-index-three-payloads"][0]
    rep_row = by_id.get(rep.id)
    if rep_row and "skipped" not in rep_row:
        oracle_units = rep_row["precheck"]["totals"]["units"]
        rep_row["handDerivedUnits"] = REPEATED_INDEX_UNITS
        if oracle_units != REPEATED_INDEX_UNITS:
            rep_row["agree"] = False
        for suffix, units, expect in (
                ("units-minus-1", REPEATED_INDEX_UNITS - 1, {"outcome": "NotInspected", "reasons": ["workLimit"]}),
                ("units-at-sum", REPEATED_INDEX_UNITS, {"outcome": "Contained"})):
            c = copy.deepcopy(rep)
            c.id = "repeated-index-three-payloads-" + suffix
            c.limits = {"containmentWorkUnits": units}
            c.expect = expect
            c.notes = ["containmentWorkUnits = %d (hand-derived deduplicated sum%s)"
                       % (units, " - 1" if units != REPEATED_INDEX_UNITS else "")]
            run_case(c, ldir, rows, large_ok)
            by_id[c.id] = rows[-1]
        pair("repeated-index-work-units", rep.id, rep.id + "-units-minus-1",
             "containmentWorkUnits one below the archive sum")
        pair("repeated-index-work-units-at-cap", rep.id + "-units-at-sum", rep.id + "-units-minus-1",
             "containmentWorkUnits exactly the deduplicated sum (%d), then one below; charging T per "
             "index occurrence instead of per distinct vertex exceeds the cap" % REPEATED_INDEX_UNITS)
    else:
        pairs.append({"control": "repeated-index-work-units", "skipped": "needs --large", "agree": True})
        pairs.append({"control": "repeated-index-work-units-at-cap", "skipped": "needs --large", "agree": True})
    manifest = {"record": "C2 slice-1 data controls: base and mutated fixtures with expected verdicts. "
                          "Production and the oracle must both report both verdicts.",
                "pairs": pairs}
    with open(os.path.join(TOP, "controls", "manifest.json"), "w") as fh:
        json.dump(manifest, fh, indent=1, sort_keys=True)
    return manifest


if __name__ == "__main__":
    sys.exit(main())
