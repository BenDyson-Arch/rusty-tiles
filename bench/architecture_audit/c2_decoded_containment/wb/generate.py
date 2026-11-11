#!/usr/bin/env python3
"""C2-WB class fixtures: the calibration matrix (contract C2-WB v2.1 section 5 and 5.1)
plus per-term identity fixtures and phase-1 chain-bit controls.

    python3 -I wb/generate.py [--large] [--only ID_SUBSTRING]

For every fixture:
  * the expected C2-WB unit total is computed twice: by `wb/hand.py` from
    construction counts that this generator derives from the written bytes
    (stdlib `json`, no `c2o`), and by the oracle; the two must be identical;
  * the per-URI facts T, Idx, E, Dec, B, J, Res, Prim and the node units must
    agree term by term, and every hand-stated span class q = ceil(s/64)^2 must
    equal the oracle's;
  * CI-size fixtures carry an at-cap pair: containmentWorkUnits = total gives
    the expected verdict, total - 1 gives NotInspected(workLimit).

CI-size fixtures are written to wb/fixtures/ (committed). Matrix-size fixtures
are written to wb/large/ (gitignored) only with --large; the oracle runs them
in precheck-only mode (units, no streaming) and their verdict is Contained by
construction, as checked on their CI-size twins. Outputs: wb/manifest.json.
"""

import argparse
import base64
import hashlib
import json
import math
import os
import random
import struct
import sys
import time
from fractions import Fraction

HERE = os.path.dirname(os.path.abspath(__file__))
TOP = os.path.dirname(HERE)
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(TOP, "witnesses"))
sys.path.insert(0, os.path.join(TOP, "oracle"))

import build as B  # noqa: E402  (writers only; independent of c2o)
import hand as H  # noqa: E402
from c2o.check import Oracle  # noqa: E402  (compared against; never used for expectations)

BOX2 = {"box": [0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0]}
BOX4 = {"box": [0.0, 0.0, 0.0, 4.0, 0.0, 0.0, 0.0, 4.0, 0.0, 0.0, 0.0, 4.0]}
SPH1 = {"sphere": [0.0, 0.0, 0.0, 1.0]}
IDENT = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]
ROT45 = {"box": [0.0, 0.0, 0.0, 0.5, 0.5, 0.0, -0.5, 0.5, 0.0, 0.0, 0.0, 0.5]}  # not a signed permutation
UNBOUNDED = 2 ** 62

# Hand-derived span classes (B1 normative spans; derivations in README "Span classes").
Q_F32_BOX = 25      # f32 word pair (-149,127) [+ a one-term translation or centre]: s in 277..320
Q_F32_SPHERE = 81   # d = P (span 277), d*d (-298,255), 3-term sum (-298,257): s = 556
Q_SMALL = 1         # u16 words (0,15) with small boxes/spheres/transforms: s <= 64
Q_BOX_8LIMBS = 64   # c0 = 2^320: (-149,127) + (320,320) -> (-149,321), s = 471
Q_SPHERE_16LIMBS = 256  # c0 = 2^340: d0 (-149,341), d0^2 (-298,683), sum3 (-298,685), s = 984
NODE_Q_SMALL = {1: 1}  # one non-skipped node with s_step <= 64 (dyadic translation/scale, unit matrix)


# ---------------------------------------------------------------- payload writers

def f32_points(n):
    """n f32-exact points with every coordinate in [-5/16, 5/16]."""
    return [(((i % 11) - 5) / 16, (((i // 11) % 11) - 5) / 16, (((i // 121) % 11) - 5) / 16) for i in range(n)]


def pack_points(points):
    return struct.pack("<%df" % (3 * len(points)), *[c for p in points for c in p])


def bounds(points):
    return ([min(p[j] for p in points) for j in range(3)], [max(p[j] for p in points) for j in range(3)])


def glb_from_json(js, binary):
    js = js + b" " * (-len(js) % 4)
    out = bytearray(struct.pack("<4sII", b"glTF", 2, 0))
    out += struct.pack("<II", len(js), 0x4E4F534A) + js
    if binary:
        binary = binary + bytes(-len(binary) % 4)
        out += struct.pack("<II", len(binary), 0x004E4942) + binary
    struct.pack_into("<I", out, 8, len(out))
    return bytes(out)


def jdump(doc):
    return json.dumps(doc, separators=(",", ":"), sort_keys=True).encode()


def point_doc(points, mode=0, extras=None, positions_extra=None):
    """One POSITION accessor (f32) in buffer 0, one primitive, node {"mesh":0}."""
    lo, hi = bounds(points)
    acc = {"bufferView": 0, "componentType": 5126, "count": len(points), "type": "VEC3", "min": lo, "max": hi}
    acc.update(positions_extra or {})
    doc = {"asset": {"version": "2.0"}, "buffers": [{"byteLength": 12 * len(points)}],
           "bufferViews": [{"buffer": 0, "byteLength": 12 * len(points), "byteStride": 12}],
           "accessors": [acc], "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "mode": mode}]}],
           "nodes": [{"mesh": 0}], "scenes": [{"nodes": [0]}], "scene": 0}
    if extras is not None:
        doc["extras"] = extras
    return doc


def point_glb(points, **kw):
    return glb_from_json(jdump(point_doc(points, **kw)), pack_points(points))


def nest(value, depth):
    for _ in range(depth):
        value = [value]
    return value


# ---------------------------------------------------------------- meshopt writers (NONE filter)

def meshopt_attributes_bits(data, count, stride):
    """ATTRIBUTES (0xa0) stream choosing, per group of 16 byte deltas, the smallest
    of mode 0 (zeros), 1 (2-bit + sentinels), 2 (4-bit + sentinels), 3 (raw).
    Baseline element all zeros (both tail readings agree)."""
    out = bytearray([0xA0])
    max_block = min((8192 // stride) & ~15, 256)
    prev = [0] * stride
    done = 0
    while done < count:
        n = min(count - done, max_block)
        groups = (n + 15) // 16
        for k in range(stride):
            p = prev[k]
            zz = []
            for e in range(groups * 16):
                v = data[(done + e) * stride + k] if e < n else p
                d = (v - p) & 0xFF
                zz.append(((d << 1) ^ (0xFF if d & 0x80 else 0)) & 0xFF)
                p = v
            prev[k] = data[(done + n - 1) * stride + k]
            header = bytearray((groups + 3) // 4)
            payload = bytearray()
            for g in range(groups):
                vals = zz[16 * g:16 * g + 16]
                sizes = {0: 0 if not any(vals) else None, 1: 4 + sum(v >= 3 for v in vals),
                         2: 8 + sum(v >= 15 for v in vals), 3: 16}
                mode = min((s, m) for m, s in sizes.items() if s is not None)[1]
                header[g // 4] |= mode << ((g % 4) * 2)
                if mode == 3:
                    payload += bytes(vals)
                elif mode in (1, 2):
                    bits = 2 if mode == 1 else 4
                    sentinel = (1 << bits) - 1
                    per = 8 // bits
                    packed = bytearray(16 * bits // 8)
                    extra = bytearray()
                    for i, v in enumerate(vals):
                        f = v if v < sentinel else sentinel
                        if v >= sentinel:
                            extra.append(v)
                        packed[i // per] |= f << (8 - bits * (i % per + 1))
                    payload += packed + extra
            out += header + payload
        done += n
    out += bytes(max(32, stride))
    return bytes(out)


def _varint(v):
    out = bytearray()
    while True:
        b = v & 0x7F
        v >>= 7
        out.append(b | (0x80 if v else 0))
        if not v:
            return bytes(out)


def meshopt_indices(indices):
    """INDICES (0xd1) stream: per index a varint of (delta << 2 | sign << 1 | baseline),
    alternating baselines 0 and 1; 4-byte tail."""
    last = [0, 0]
    out = bytearray([0xD1])
    for i, v in enumerate(indices):
        b = i & 1
        delta = v - last[b]
        out += _varint((delta << 2) | b if delta >= 0 else ((~delta) << 2) | 2 | b)
        last[b] = v
    return bytes(out) + bytes(4)


def meshopt_glb(points, indices=None, index_mode=None, attr_encoder="raw", prim_mode=0):
    """POSITION via ATTRIBUTES; optional u32 indices via TRIANGLES or INDICES; URI-less fallback buffer 1."""
    raw = pack_points(points)
    enc = B.meshopt_attributes if attr_encoder == "raw" else meshopt_attributes_bits
    comp_v = enc(raw, len(points), 12)
    binary = bytearray(comp_v + bytes(-len(comp_v) % 4))
    fb_len = len(raw)
    views = [{"buffer": 1, "byteOffset": 0, "byteLength": len(raw), "byteStride": 12, "target": 34962,
              "extensions": {"EXT_meshopt_compression": {
                  "buffer": 0, "byteOffset": 0, "byteLength": len(comp_v), "byteStride": 12,
                  "count": len(points), "mode": "ATTRIBUTES", "filter": "NONE"}}}]
    lo, hi = bounds(points)
    accessors = [{"bufferView": 0, "componentType": 5126, "count": len(points), "type": "VEC3", "min": lo, "max": hi}]
    prim = {"attributes": {"POSITION": 0}, "mode": prim_mode}
    if indices is not None:
        comp_i = B.meshopt_triangles(indices) if index_mode == "TRIANGLES" else meshopt_indices(indices)
        off_i = len(binary)
        binary += comp_i + bytes(-len(comp_i) % 4)
        views.append({"buffer": 1, "byteOffset": fb_len, "byteLength": 4 * len(indices), "target": 34963,
                      "extensions": {"EXT_meshopt_compression": {
                          "buffer": 0, "byteOffset": off_i, "byteLength": len(comp_i), "byteStride": 4,
                          "count": len(indices), "mode": index_mode}}})
        accessors.append({"bufferView": 1, "componentType": 5125, "count": len(indices), "type": "SCALAR"})
        prim["indices"] = 1
        fb_len += 4 * len(indices)
    doc = {"asset": {"version": "2.0"},
           "buffers": [{"byteLength": len(binary)},
                       {"byteLength": fb_len, "extensions": {"EXT_meshopt_compression": {"fallback": True}}}],
           "bufferViews": views, "accessors": accessors, "meshes": [{"primitives": [prim]}],
           "nodes": [{"mesh": 0}], "scenes": [{"nodes": [0]}], "scene": 0,
           "extensionsUsed": ["EXT_meshopt_compression"], "extensionsRequired": ["EXT_meshopt_compression"]}
    return glb_from_json(jdump(doc), bytes(binary))


# ---------------------------------------------------------------- long-significand decimals

def long_literal(target, digits):
    """A decimal literal of about `digits` significant characters that rounds (to nearest
    binary64) to `target`: the exact midpoint between next_down(target) and target, then
    zeros, then a final 1 that decides the rounding upward. Correct rounding needs every digit.
    A negative target is the negation of the positive literal (it rounds toward the target)."""
    assert target != 0
    if target < 0:
        return "-" + long_literal(-target, digits - 1)
    mid =(Fraction(B.next_down(target)) + Fraction(target)) / 2
    k = mid.denominator.bit_length() - 1
    text = str(mid.numerator * 5 ** k).rjust(k + 1, "0")
    lit = text[:-k] + "." + text[-k:]
    lit = lit + "0" * max(0, digits - len(lit) - 1) + "1"
    assert float(lit) == target and Fraction(lit) != Fraction(target)
    return lit


def long_decimal_glb(points, matrix_nodes, trs_nodes, bound_accessors, digits):
    """Selected nodes whose matrix/TRS values and accessor min/max are long-significand
    literals that round to exact simple values (spans stay those of the simple values)."""
    lits = {}

    def L(target):
        key = "@@L%d@@" % len(lits)
        lits[key] = long_literal(target, digits)
        return key
    lo, hi = bounds(points)
    doc = point_doc(points, positions_extra={"min": [L(v) for v in lo], "max": [L(v) for v in hi]})
    nodes = []
    for i in range(matrix_nodes):
        m = [L(1.0), 0.0, 0.0, 0.0, 0.0, L(1.0), 0.0, 0.0, 0.0, 0.0, L(1.0), 0.0,
             L(0.25), L(-0.25), L(0.125), 1.0]
        nodes.append({"matrix": m, "mesh": 0})
    for i in range(trs_nodes):
        nodes.append({"translation": [L(0.25), L(0.5), L(-0.125)], "rotation": [0.0, 0.0, 0.0, L(1.0)],
                      "scale": [L(1.0), L(1.0), L(1.0)], "mesh": 0})
    doc["nodes"] = nodes
    doc["scenes"] = [{"nodes": list(range(len(nodes)))}]
    for _ in range(bound_accessors):
        doc["accessors"].append({"componentType": 5126, "count": 1, "type": "SCALAR",
                                 "min": [L(0.5)], "max": [L(0.5)]})
    js = jdump(doc)
    for key, lit in lits.items():
        js = js.replace(b'"' + key.encode() + b'"', lit.encode(), 1)
    return glb_from_json(js, pack_points(points))


# ---------------------------------------------------------------- hand facts from bytes

def hand_payload(member, data, archive):
    """Per-URI facts derived from the written bytes with stdlib json only."""
    docs = []
    # H7: every admitted payload is GLB or glTF JSON; legacy content is Unsupported in C1.
    glb = data if data[:4] == b"glTF" else None
    if glb is not None:
        jlen = struct.unpack_from("<I", glb, 12)[0]
        doc = H.json_of(glb[20:20 + jlen])
    else:
        doc = H.json_of(data)
    docs.append(doc)
    facts = H.gltf_counts(doc)
    Bv, Dec, Res = len(data), 0, 0
    for buf in doc.get("buffers", []):
        uri = buf.get("uri")
        if uri is None:
            continue
        if uri.startswith("data:"):
            Dec += len(base64.b64decode(uri.split(",", 1)[1]))
            continue
        Res += 1
        Dec += len(archive[uri])
        Bv += len(archive[uri])
    for img in doc.get("images", []):
        uri = img.get("uri")
        if isinstance(uri, str) and not uri.startswith("data:"):
            Res += 1
            Bv += len(archive[uri])
    for view in doc.get("bufferViews", []):
        if "EXT_meshopt_compression" in view.get("extensions", {}):
            Dec += view["byteLength"]
    facts.update({"B": Bv, "Dec": Dec, "Res": Res, "J": sum(H.value_nodes(d) for d in docs)})
    return facts


# ---------------------------------------------------------------- fixtures and layouts

class Fx:
    def __init__(self, fid, matrix, size, title):
        self.id = fid
        self.matrix = matrix  # C2-WB section 5 / 5.1 row, or "terms" / "chain bits"
        self.size = size  # "ci" | "large"
        self.title = title
        self.docs = {}
        self.extra = {}  # non-payload members (external buffers)
        self.payloads = {}
        self.refs = []  # (count, member, ops, qs or None = oracle spans)
        self.node_q = {}  # member -> {q: visits} for non-skipped visits, or None = oracle spans
        self.expect = {"outcome": "Contained"}
        self.limits = {}
        self.cap_pair = True
        self.notes = []

    def members(self):
        names = ["tileset.json"] + sorted(n for n in self.docs if n != "tileset.json")
        out = [(n, B.json_bytes(self.docs[n])) for n in names]
        return out + sorted(self.payloads.items()) + sorted(self.extra.items())


def tile(volume, **kw):
    kw.setdefault("geometric_error", 1.0)
    return B.tile(volume, **kw)


def path_docs(fx, frames, volume, bottom, transform=None, prefix="p", per_doc=25):
    """`frames` nested tiles from the root down to the bottom tile X; X gets `bottom`
    (dict of extra keys: content/children). At most `per_doc` nested tiles per JSON
    document (C1 JSON depth 64); a document's last tile references the next one."""
    segments = [min(per_doc, frames - s) for s in range(0, frames, per_doc)]
    nxt = None
    for di in reversed(range(len(segments))):
        name = "tileset.json" if di == 0 else "%s%d.json" % (prefix, di)
        inner = None
        for k in reversed(range(segments[di])):
            t = tile(volume, transform=transform)
            if inner is None:
                if nxt is None:
                    t.update(bottom)
                else:
                    t["content"] = {"uri": nxt}
            else:
                t["children"] = [inner]
            inner = t
        fx.docs[name] = B.tileset(inner)
        nxt = name


def kid_layout(fx, member, K, q, frames=1, per_doc=5000):
    """K references, each a leaf tile (BOX2) in one of ceil(K/per_doc) external tilesets
    e<d>.json, linked from tiles under a `frames`-deep path. Levels per reference:
    kid, e<d> root, link tile, then the path: 3 + frames, all BOX2, no transforms."""
    D = -(-K // per_doc)
    links = []
    for d in range(D):
        n = min(per_doc, K - d * per_doc)
        # Distinct geometric errors (i * 2^-24, below the parent's): C1's tileset
        # schema requires unique `children` items. Units do not depend on them.
        kids = [tile(BOX2, content=member, geometric_error=i * 2.0 ** -24) for i in range(n)]
        fx.docs["e%d.json" % d] = B.tileset(tile(BOX2, children=kids))
        links.append(tile(BOX2, content="e%d.json" % d))
    path_docs(fx, frames, BOX2, {"children": links})
    levels = 3 + frames
    fx.refs.append((K, member, [(1, 0, 1, 0)] + [(0, 0, 1, 0)] * (levels - 1), [q] * levels))


# ---------------------------------------------------------------- the class matrix

def classes(large):
    """Yield (fixture builder, size) pairs; builders return an Fx."""
    out = []

    def add(fn, *args):
        out.append((fn, args))

    S = "large" if large else "ci"
    pick = (lambda ci, lg: lg) if large else (lambda ci, lg: ci)

    # --- section 5, geometry classes
    for levels in (16, 32):
        add(ext_two_spheres_step, levels, pick(64, 20000), S)
    add(chain, "sphere-chain-L16", "sphere chain, 16 levels", SPH1, None, 16, pick(64, 20000), Q_F32_SPHERE, S)
    add(chain, "box-step-L16", "box + step, 16 levels", BOX2, IDENT, 16, pick(64, 20000), Q_F32_BOX, S)
    add(chain, "box-chain-L16", "box chain, 16 levels", BOX2, None, 16, pick(64, 20000), Q_F32_BOX, S)
    add(single_tile, "own-content-sphere-1M", "own + content sphere, 1M vertices", SPH1, SPH1,
        pick(1000, 1000000), Q_F32_SPHERE, S)
    add(single_tile, "one-box-1M", "one box, 1M vertices", BOX2, None, pick(1000, 1000000), Q_F32_BOX, S)
    add(single_tile, "large-span-box-8-limbs", "large spans (box 8 limbs)",
        {"box": [2.0 ** 320, 0.0, 0.0, 2.0 ** 321, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0]}, None,
        pick(64, 100000), Q_BOX_8LIMBS, S)
    add(single_tile, "large-span-sphere-16-limbs", "large spans (sphere 16 limbs)",
        {"sphere": [2.0 ** 340, 0.0, 0.0, 2.0 ** 341]}, None, pick(64, 100000), Q_SPHERE_16LIMBS, S)
    add(node_class, "nodes-translation", pick(50, 15000), pick(2, 20), False, S)
    add(node_class, "nodes-rotated", pick(50, 15000), pick(2, 20), True, S)
    add(tiny_refs, 1, pick(10, 100000), S)
    add(tiny_refs, 100, pick(10, 100000), S)
    add(mid_payload_refs, pick(20, 20000), S)
    # --- section 5, C1-structure classes
    add(unselected_nodes, pick(600, 60000), pick(2, 20), S)
    add(skipped_nodes, pick(300, 30000), pick(2, 20), S)
    for n in (2000, 20000):
        add(extbuf, n if large else n // 100, pick(2, 20), S)
    for m in (40, 200):
        add(shared_accessor, pick(160, 16000), m if large else m // 10, S)
    add(json_flat, "numbers-glb", pick(1200, 120000), pick(5, 50), S)
    add(json_flat, "string-glb", pick(10240, 1 << 20), pick(5, 50), S)
    add(json_gltf_refs, pick(10240, 1 << 20), pick(10, 1000), S)
    add(json_deep, "string", "glb", 60, pick(10240, 1 << 20), pick(5, 50), S)
    for d in (1, 30, 62):
        add(json_deep, "numbers", "glb", d, pick(1200, 120000), pick(5, 50), S)
    # H7 retired the six b3dm JSON classes (legacy content is Unsupported in C1);
    # their GLB twins above keep the JSON-node cost covered.
    add(quad, pick(60, 6000), pick(2, 20), S)
    add(quad_b, "quadB", pick(50, 5000), pick(160, 16000), pick(4, 40), S)
    add(quad_b, "e2e", pick(50, 5000), pick(155, 15500), pick(5, 500), S)
    # --- section 5.1
    add(viewless, "SCALAR", pick(4000, 3999992), pick(2, 20), S)
    add(viewless, "MAT4", pick(250, 249992), pick(2, 20), S)
    add(long_decimals, pick(2, 150), pick(2, 150), pick(10, 2500), pick(2, 20), S)
    add(meshopt_class, "attributes-raw", pick(300, 1000000), 0, None, pick(2, 4), S)
    add(meshopt_class, "attributes-bits", pick(300, 1000000), 0, None, pick(2, 4), S)
    add(meshopt_class, "triangles", pick(300, 100000), pick(900, 3000000), "TRIANGLES", pick(2, 4), S)
    add(meshopt_class, "indices", pick(300, 100000), pick(900, 3000000), "INDICES", pick(2, 4), S)
    if not large:
        for fn in TERMS + CHAIN_BITS + READINGS:
            add(fn)
    return out


def ext_two_spheres_step(levels, n, size):
    """`levels` external tilesets in a chain; every root is SPH1 with an identity
    transform and references the next through a content SPH1. Level 0: own + content
    sphere; every level >= 1: externalContent + ancestor sphere + step."""
    fx = Fx("extTS-L%d" % levels, "two spheres + step per level, %d levels (extTS-L%d)" % (levels, levels),
            size, "external chain, two spheres and a step per level")
    fx.payloads["w.glb"] = point_glb(f32_points(n))
    for i in range(levels):
        name = "tileset.json" if i == 0 else "x%d.json" % i
        uri = "w.glb" if i == levels - 1 else "x%d.json" % (i + 1)
        fx.docs[name] = B.tileset(tile(SPH1, transform=IDENT, content=uri, content_volume=SPH1))
    fx.refs.append((1, "w.glb", [(1, 0, 0, 2)] + [(0, 1, 0, 2)] * (levels - 1), [Q_F32_SPHERE] * levels))
    fx.node_q["w.glb"] = {}
    return fx


def chain(fid, matrix, volume, transform, levels, n, q, size):
    fx = Fx(fid, matrix, size, "nested tiles, payload at the leaf")
    fx.payloads["w.glb"] = point_glb(f32_points(n))
    path_docs(fx, levels, volume, {"content": {"uri": "w.glb"}}, transform=transform)
    kind = (0, 0, 1, 0) if "box" in volume else (0, 0, 0, 1)
    step = 1 if transform is not None else 0
    ops = [(1,) + kind[1:]] + [(0, step) + kind[2:]] * (levels - 1)
    fx.refs.append((1, "w.glb", ops, [q] * levels))
    fx.node_q["w.glb"] = {}
    return fx


def single_tile(fid, matrix, volume, content_volume, n, q, size):
    fx = Fx(fid, matrix, size, "one tile")
    fx.payloads["w.glb"] = point_glb(f32_points(n))
    fx.docs["tileset.json"] = B.tileset(tile(volume, content="w.glb", content_volume=content_volume))
    vols = [volume] + ([content_volume] if content_volume else [])
    fx.refs.append((1, "w.glb", [(1, 0, sum("box" in v for v in vols), sum("sphere" in v for v in vols))], [q]))
    fx.node_q["w.glb"] = {}
    return fx


def node_class(fid, N, K, rotated, size):
    """N selected scene-root nodes, each translated (dyadic, |t| <= 1/2) or rotated
    (generic unit quaternion), each an instance of a one-vertex POINTS mesh."""
    fx = Fx(fid, "translation-only / rotated nodes", size,
            "%d %s nodes x %d references" % (N, "rotated" if rotated else "translation-only", K))
    rng = random.Random(20261012)
    doc = point_doc([(0.25, 0.25, 0.25)])
    nodes = []
    for i in range(N):
        if rotated:
            q = [rng.gauss(0, 1) for _ in range(4)]
            norm = math.sqrt(sum(c * c for c in q))
            nodes.append({"rotation": [c / norm for c in q], "mesh": 0})
        else:
            nodes.append({"translation": [((i % 8) - 4) / 8, (((i // 8) % 8) - 4) / 8, (((i // 64) % 8) - 4) / 8],
                          "mesh": 0})
    doc["nodes"] = nodes
    doc["scenes"] = [{"nodes": list(range(N))}]
    fx.payloads["n.glb"] = glb_from_json(jdump(doc), pack_points([(0.25, 0.25, 0.25)]))
    kid_layout(fx, "n.glb", K, None if rotated else Q_F32_BOX)
    if rotated:
        fx.refs[-1] = fx.refs[-1][:3] + (None,)
        fx.node_q["n.glb"] = None
        fx.notes.append("generic quaternions: node and level spans are taken from the oracle (B1); "
                        "every weight, count and term is still applied by hand")
    else:
        fx.node_q["n.glb"] = {1: N}
    return fx


def tiny_refs(depth, K, size):
    frames = 1 if depth == 1 else 100
    fid = "tiny-refs-L%d" % (3 + frames)
    fx = Fx(fid, "%s tiny references, depth %d" % ("100,000" if size == "large" else K, depth), size,
            "%d references to a one-vertex payload, %d levels each" % (K, 3 + frames))
    fx.payloads["t.glb"] = point_glb([(0.25, 0.25, 0.25)])
    kid_layout(fx, "t.glb", K, Q_F32_BOX, frames=frames)
    fx.node_q["t.glb"] = {}
    return fx


def mid_payload_refs(K, size):
    """Many references to one mid-size indexed payload (the W3-reference class, own payload)."""
    fx = Fx("refs-to-mid-payload", "20,000 references to a W3 payload (own mid-size payload)", size,
            "%d references to a 1,000-vertex, 3,000-index payload" % K)
    pts = f32_points(1000)
    idx = [(t + k) % 1000 for t in range(1000) for k in range(3)]
    raw = pack_points(pts)
    ib = struct.pack("<%dH" % len(idx), *idx)
    lo, hi = bounds(pts)
    doc = {"asset": {"version": "2.0"}, "buffers": [{"byteLength": len(raw) + len(ib)}],
           "bufferViews": [{"buffer": 0, "byteLength": len(raw), "byteStride": 12},
                           {"buffer": 0, "byteOffset": len(raw), "byteLength": len(ib)}],
           "accessors": [{"bufferView": 0, "componentType": 5126, "count": 1000, "type": "VEC3", "min": lo, "max": hi},
                         {"bufferView": 1, "componentType": 5123, "count": len(idx), "type": "SCALAR"}],
           "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1, "mode": 4}]}],
           "nodes": [{"translation": [0.25, 0.5, -0.25], "mesh": 0}], "scenes": [{"nodes": [0]}], "scene": 0}
    fx.payloads["m.glb"] = glb_from_json(jdump(doc), raw + ib)
    kid_layout(fx, "m.glb", K, Q_F32_BOX)
    fx.node_q["m.glb"] = NODE_Q_SMALL
    return fx


def unselected_nodes(N, K, size):
    fx = Fx("unselected-nodes", "unselected {} nodes, 60,000 x 20 references", size,
            "%d unselected {} nodes x %d references" % (N, K))
    doc = point_doc([(0.25, 0.25, 0.25)])
    doc["nodes"] += [{} for _ in range(N)]
    fx.payloads["u.glb"] = glb_from_json(jdump(doc), pack_points([(0.25, 0.25, 0.25)]))
    kid_layout(fx, "u.glb", K, Q_F32_BOX)
    fx.node_q["u.glb"] = {}
    return fx


def skipped_nodes(N, K, size):
    fx = Fx("skipped-nodes", "selected skipped nodes, 30,000 x 20", size,
            "%d selected skipped {} children x %d references" % (N, K))
    doc = point_doc([(0.25, 0.25, 0.25)])
    doc["nodes"] = [{"mesh": 0, "children": list(range(1, N + 1))}] + [{} for _ in range(N)]
    fx.payloads["s.glb"] = glb_from_json(jdump(doc), pack_points([(0.25, 0.25, 0.25)]))
    kid_layout(fx, "s.glb", K, Q_F32_BOX)
    fx.node_q["s.glb"] = {}
    return fx


def extbuf(N, K, size):
    fx = Fx("extbuf-%d" % N, "external buffers, 2,000 / 20,000 x 20", size,
            ".gltf with %d external buffers (one 12-byte, the rest 4-byte) x %d references" % (N, K))
    doc = point_doc([(0.25, 0.25, 0.25)])
    doc["buffers"] = [{"uri": "b0.bin", "byteLength": 12}] + [{"uri": "b%d.bin" % i, "byteLength": 4}
                                                              for i in range(1, N)]
    fx.payloads["g.gltf"] = jdump(doc)
    fx.extra["b0.bin"] = pack_points([(0.25, 0.25, 0.25)])
    for i in range(1, N):
        fx.extra["b%d.bin" % i] = struct.pack("<I", i)
    kid_layout(fx, "g.gltf", K, Q_F32_BOX)
    fx.node_q["g.gltf"] = {}
    return fx


def shared_accessor(P, M, size):
    fx = Fx("shared-accessor-%dx%d" % (P, M), "primitives sharing one accessor, 16,000 x 40 / x 200 instances",
            size, "%d POINTS primitives on one accessor x %d instances, one reference" % (P, M))
    doc = point_doc([(0.25, 0.25, 0.25)])
    doc["meshes"] = [{"primitives": [{"attributes": {"POSITION": 0}, "mode": 0} for _ in range(P)]}]
    doc["nodes"] = [{"mesh": 0} for _ in range(M)]
    doc["scenes"] = [{"nodes": list(range(M))}]
    fx.payloads["p.glb"] = glb_from_json(jdump(doc), pack_points([(0.25, 0.25, 0.25)]))
    kid_layout(fx, "p.glb", 1, Q_F32_BOX)
    fx.node_q["p.glb"] = {}
    return fx


def json_flat(kind, N, K, size):
    fx = Fx("json-flat-" + kind, "JSON: flat dense numbers GLB / b3dm, 1 MiB flat string", size,
            "%s, %d %s x %d references" % (kind, N, "characters" if "string" in kind else "numbers", K))
    pt = [(0.25, 0.25, 0.25)]
    if kind == "numbers-glb":
        member, data = "j.glb", point_glb(pt, extras=[0] * N)
    else:
        member, data = "j.glb", point_glb(pt, extras="x" * N)
    fx.payloads[member] = data
    kid_layout(fx, member, K, Q_F32_BOX)
    fx.node_q[member] = {}
    return fx


def json_gltf_refs(N, K, size):
    fx = Fx("json-gltf-1MiB-refs", "1 MiB JSON payload x 1,000 references", size,
            ".gltf with a %d-character extras string and a data-URI buffer x %d references" % (N, K))
    doc = point_doc([(0.25, 0.25, 0.25)], extras="x" * N)
    doc["buffers"] = [{"byteLength": 12, "uri": "data:application/octet-stream;base64,"
                       + base64.b64encode(pack_points([(0.25, 0.25, 0.25)])).decode()}]
    fx.payloads["j.gltf"] = jdump(doc)
    kid_layout(fx, "j.gltf", K, Q_F32_BOX)
    fx.node_q["j.gltf"] = {}
    return fx


def json_deep(what, where, depth, N, K, size):
    value = [0] * N if what == "numbers" else "x" * N
    fx = Fx("json-deep-%s-%s-%d" % (what, where, depth),
            "deep %s (GLB extras), depth %d" % (what, depth),
            size, "%s nested %d deep in %s x %d references" % (what, depth, where, K))
    pt = [(0.25, 0.25, 0.25)]
    member, data = "d.glb", point_glb(pt, extras=nest(value, depth))
    fx.payloads[member] = data
    kid_layout(fx, member, K, Q_F32_BOX)
    fx.node_q[member] = {}
    fx.notes.append("deepest value at JSON depth %d (root 0)" % (depth + 2 if what == "numbers" else depth + 1))
    return fx


def _accessor_mesh(A, P, M):
    acc = [{"bufferView": 0, "componentType": 5126, "count": 1, "type": "VEC3",
            "min": [0.25, 0.25, 0.25], "max": [0.25, 0.25, 0.25]} for _ in range(A)]
    doc = {"asset": {"version": "2.0"}, "accessors": acc, "buffers": [{"byteLength": 12}],
           "bufferViews": [{"buffer": 0, "byteLength": 12, "byteStride": 12}],
           "meshes": [{"primitives": [{"attributes": {"POSITION": i % A}, "mode": 0} for i in range(P)]}],
           "nodes": [{"mesh": 0} for _ in range(M)], "scenes": [{"nodes": list(range(M))}], "scene": 0}
    return glb_from_json(jdump(doc), pack_points([(0.25, 0.25, 0.25)]))


def quad(P, M, size):
    fx = Fx("quad", "one accessor per primitive, 6,000 x 20 (quad)", size,
            "%d primitives each with its own accessor x %d instances, one reference" % (P, M))
    fx.payloads["q.glb"] = _accessor_mesh(P, P, M)
    kid_layout(fx, "q.glb", 1, Q_F32_BOX)
    fx.node_q["q.glb"] = {}
    return fx


def quad_b(fid, A, P, M, size):
    row = ("5,000 accessors x 16,000 primitives x 40 (quadB)" if fid == "quadB"
           else "end-to-end, 5,000 x 15,500 x 500 (one reference; the many-accessor case)")
    fx = Fx(fid, row, size, "%d accessors x %d primitives (primitive i uses accessor i mod A) x %d instances, "
            "one reference" % (A, P, M))
    fx.payloads["q.glb"] = _accessor_mesh(A, P, M)
    kid_layout(fx, "q.glb", 1, Q_F32_BOX)
    fx.node_q["q.glb"] = {}
    return fx


def viewless(shape, N, K, size):
    comps = H.TYPE_SIZE[shape]
    fx = Fx("viewless-" + shape, "viewless accessors, unused 4M-component accessor x 20, SCALAR / MAT4 (5.1)",
            size, "unused viewless %s accessor, %d components x %d references" % (shape, N * comps, K))
    doc = point_doc([(0.25, 0.25, 0.25)])
    doc["accessors"].append({"componentType": 5126, "count": N, "type": shape})
    fx.payloads["v.glb"] = glb_from_json(jdump(doc), pack_points([(0.25, 0.25, 0.25)]))
    kid_layout(fx, "v.glb", K, Q_F32_BOX)
    fx.node_q["v.glb"] = {}
    return fx


def long_decimals(mat, trs, accs, K, size):
    digits = 800
    fx = Fx("long-decimals", "long-significand decimals in node matrix/TRS and accessor min/max (5.1)", size,
            "%d matrix + %d TRS selected nodes and %d bounded accessors with %d-character literals x %d "
            "references" % (mat, trs, accs, digits, K))
    fx.payloads["l.glb"] = long_decimal_glb(f32_points(8), mat, trs, accs, digits)
    kid_layout(fx, "l.glb", K, Q_F32_BOX)
    fx.node_q["l.glb"] = {1: mat + trs}
    fx.notes.append("every literal rounds to a simple value (1, 0.25, 0.5, ...) only after its last digit: "
                    "the exact midpoint below the target, zeros, then 1")
    return fx


def meshopt_class(kind, nv, ni, mode, K, size):
    fx = Fx("meshopt-" + kind, "meshopt NONE filter, worst mode per decoded byte (5.1): " + kind, size,
            "%d vertices via ATTRIBUTES (%s groups)%s x %d references"
            % (nv, "bits" if kind == "attributes-bits" else "raw",
               "" if mode is None else ", %d u32 indices via %s" % (ni, mode), K))
    pts = f32_points(nv)
    if mode is None:
        data = meshopt_glb(pts, attr_encoder="bits" if kind == "attributes-bits" else "raw")
    else:
        idx = [(t // 3 + t % 3) % nv for t in range(ni)]
        data = meshopt_glb(pts, idx, mode, prim_mode=4 if mode == "TRIANGLES" else 1)
    fx.payloads["o.glb"] = data
    kid_layout(fx, "o.glb", K, Q_F32_BOX)
    fx.node_q["o.glb"] = {}
    return fx


# ---------------------------------------------------------------- per-term identities (CI only)

def _u16_payload(nodes=None, prims=1, extras=None, gltf_external=False):
    """Four u16 (unnormalized) vertices: every span class is q = 1, so every floor applies."""
    raw = [[0, 0, 0], [1, 0, 0], [0, 1, 0], [0, 0, 1]]
    data = b"".join(struct.pack("<3H", *v) + bytes(2) for v in raw)
    doc = {"asset": {"version": "2.0"}, "extensionsUsed": ["KHR_mesh_quantization"],
           "extensionsRequired": ["KHR_mesh_quantization"],
           "buffers": [{"byteLength": len(data)}],
           "bufferViews": [{"buffer": 0, "byteLength": len(data), "byteStride": 8, "target": 34962}],
           "accessors": [{"bufferView": 0, "componentType": 5123, "count": 4, "type": "VEC3",
                          "min": [0, 0, 0], "max": [1, 1, 1]}],
           "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "mode": 0}] * prims}],
           "nodes": nodes or [{"mesh": 0}], "scenes": [{"nodes": [0]}], "scene": 0}
    if extras is not None:
        doc["extras"] = extras
    if gltf_external:
        doc["buffers"][0]["uri"] = "p.bin"
        return jdump(doc), data
    return glb_from_json(jdump(doc), data), None


def _terms(fid, title, tileset_root, refs, node_q=None, **payload):
    def build():
        fx = Fx("terms-" + fid, "per-term identity (C2-WB 3.1/3.2)", "ci", title)
        data, ext = _u16_payload(**payload)
        member = "w.gltf" if ext is not None else "w.glb"
        fx.payloads[member] = data
        if ext is not None:
            fx.extra["p.bin"] = ext
        fx.docs["tileset.json"] = B.tileset(tileset_root(member))
        fx.refs = [(1, member, ops, [Q_SMALL] * len(ops)) for ops in refs]
        fx.node_q[member] = node_q or {}
        return fx
    build.__name__ = "terms_" + fid
    return build


SPH4, SPH8 = {"sphere": [0.0, 0.0, 0.0, 4.0]}, {"sphere": [0.0, 0.0, 0.0, 8.0]}
TERMS = [
    _terms("base", "u16 points, one tile box: V = 20 + 25", lambda m: tile(BOX4, content=m), [[(1, 0, 1, 0)]]),
    _terms("content-box", "+ content box: +25 per vertex", lambda m: tile(BOX4, content=m, content_volume=BOX4),
           [[(1, 0, 2, 0)]]),
    _terms("content-sphere", "+ content sphere: +32 per vertex (sphere floor)",
           lambda m: tile(BOX4, content=m, content_volume=SPH4), [[(1, 0, 1, 1)]]),
    _terms("parent", "+ parent box, no transform: +25 per vertex, +8",
           lambda m: tile(BOX4, children=[tile(BOX4, content=m)]), [[(1, 0, 1, 0), (0, 0, 1, 0)]]),
    _terms("parent-step", "+ parent box, child transform: +(30 + 25) per vertex, +8",
           lambda m: tile(BOX4, children=[tile(BOX4, content=m, transform=IDENT)]), [[(1, 0, 1, 0), (0, 1, 1, 0)]]),
    _terms("parent-sphere-step", "+ parent sphere, child transform: +(30 + 32) per vertex, +8",
           lambda m: tile(SPH8, children=[tile(BOX4, content=m, transform=IDENT)]), [[(1, 0, 1, 0), (0, 1, 0, 1)]]),
    _terms("node", "non-skipped node (translation): Nodes 1 -> 256",
           lambda m: tile(BOX4, content=m), [[(1, 0, 1, 0)]], node_q={1: 1},
           nodes=[{"translation": [1.0, 0.0, 0.0], "mesh": 0}]),
    _terms("json", "+ extras {k: [0, 0, 0, 0]}: J + 6", lambda m: tile(BOX4, content=m), [[(1, 0, 1, 0)]],
           extras={"k": [0, 0, 0, 0]}),
    _terms("prim", "two primitives on one accessor: Prim 2, Idx 8, T unchanged",
           lambda m: tile(BOX4, content=m), [[(1, 0, 1, 0)]], prims=2),
    _terms("res", ".gltf with one external buffer: Res 1, Dec and B count it", lambda m: tile(BOX4, content=m),
           [[(1, 0, 1, 0)]], gltf_external=True),
    _terms("two-refs", "two references (root and child) to one URI: the per-URI facts twice",
           lambda m: tile(BOX4, content=m, children=[tile(BOX4, content=m)]),
           [[(1, 0, 1, 0)], [(1, 0, 1, 0), (0, 0, 1, 0)]]),
]


# ---------------------------------------------------------------- phase-1 chain bits (CI only)

def chain_worklimit_then_rotated():
    fx = Fx("chain-worklimit-then-rotated-content", "chain bits (C2-WB 3.3)", "ci",
            "reference 0 exceeds the cap; reference 1's content volume is rotated: rotatedBox, not workLimit")
    fx.payloads["a.glb"] = point_glb(f32_points(64))
    fx.payloads["b.glb"] = point_glb([(0.25, 0.25, 0.25)])
    fx.docs["tileset.json"] = B.tileset(tile(BOX2, children=[
        tile(BOX2, content="a.glb", geometric_error=0.0),
        tile(BOX2, content="b.glb", content_volume=ROT45, geometric_error=0.0)]))
    fx.expect = {"outcome": "NotInspected", "reasons": ["rotatedBox"]}
    fx.limits = {"containmentWorkUnits": 100}
    fx.cap_pair = False
    return fx


def chain_worklimit_base():
    fx = chain_worklimit_then_rotated()
    fx.id = "chain-worklimit-base"
    fx.title = "the same archive with an axis-aligned content box: workLimit at the same cap"
    fx.docs["tileset.json"]["root"]["children"][1]["content"]["boundingVolume"] = BOX2
    fx.expect = {"outcome": "NotInspected", "reasons": ["workLimit"]}
    return fx


def chain_region_ancestor():
    fx = Fx("chain-region-ancestor", "chain bits (C2-WB 3.3)", "ci",
            "region-only hierarchy, region ancestors two levels up, cap 1: regionVolume, not workLimit")
    fx.payloads["w.glb"] = point_glb([(6378137.0, 0.0, 0.0)])
    leaf = tile({"region": [-0.001, -0.001, 0.001, 0.001, -10.0, 10.0]}, content="w.glb", geometric_error=0.0,
                content_volume={"region": [-0.0005, -0.0005, 0.0005, 0.0005, -5.0, 5.0]})
    mid = tile({"region": [-0.005, -0.005, 0.005, 0.005, -50.0, 50.0]}, children=[leaf])
    fx.docs["tileset.json"] = B.tileset(tile({"region": [-0.01, -0.01, 0.01, 0.01, -100.0, 100.0]},
                                             children=[mid]))
    fx.expect = {"outcome": "NotInspected", "reasons": ["regionVolume"]}
    fx.limits = {"containmentWorkUnits": 1}
    fx.cap_pair = False
    return fx


def chain_external_rotated():
    fx = Fx("chain-external-rotated-content", "chain bits (C2-WB 3.3)", "ci",
            "external tileset referenced through a rotated content volume, cap 1: rotatedBox")
    fx.payloads["w.glb"] = point_glb(f32_points(8))
    box = {"box": [0.0, 0.0, 0.0, 0.375, 0.0, 0.0, 0.0, 0.375, 0.0, 0.0, 0.0, 0.375]}
    fx.docs["tileset.json"] = B.tileset(tile(BOX2, content="ext.json", content_volume=ROT45))
    fx.docs["ext.json"] = B.tileset(tile(box, children=[tile(box, content="w.glb", geometric_error=0.0)]))
    fx.expect = {"outcome": "NotInspected", "reasons": ["rotatedBox"]}
    fx.limits = {"containmentWorkUnits": 1}
    fx.cap_pair = False
    return fx


CHAIN_BITS = [chain_worklimit_then_rotated, chain_worklimit_base, chain_region_ancestor, chain_external_rotated]


def reading_res_duplicate_uri():
    """Contract reading probe: two buffer entries name the same external member.
    O-WB reading: Res counts buffer/image entries with a non-data uri (2 here); Dec counts
    each owned buffer (slice-1 model). B counts the member once per entry too (12 bytes
    more than once per member), per the C1 payload contract (c1-payload-integrity-contract.md
    :330-331): "Repeated resource declarations charge repeated actual bytes; no digest/URI
    deduplication changes that meaning."
    A per-distinct-member Res would give 160 units less."""
    fx = Fx("reading-res-duplicate-uri", "reading probe (Res per entry)", "ci",
            ".gltf whose buffers 0 and 1 both name b0.bin x 2 references")
    doc = point_doc([(0.25, 0.25, 0.25)])
    doc["buffers"] = [{"uri": "b0.bin", "byteLength": 12}, {"uri": "b0.bin", "byteLength": 12}]
    fx.payloads["g.gltf"] = jdump(doc)
    fx.extra["b0.bin"] = pack_points([(0.25, 0.25, 0.25)])
    kid_layout(fx, "g.gltf", 2, Q_F32_BOX)
    fx.node_q["g.gltf"] = {}
    fx.notes.append("reading: Res = 2 (per buffer entry); per distinct member it would be 1 (-160 per reference)")
    return fx


READINGS = [reading_res_duplicate_uri]


# ---------------------------------------------------------------- evaluation

def hand_total(fx, archive, oracle_payloads=None, oracle_refs=None):
    """C2-WB total from hand facts. Span classes come from the fixture, or (None) from
    the oracle's B1 spans; every weight, count and term is applied here."""
    uris = {}
    for member in fx.payloads:
        u = hand_payload(member, fx.payloads[member], archive)
        node_q = fx.node_q.get(member, {})
        if node_q is None:
            steps = {int(k): v for k, v in oracle_payloads[member]["nodeStepSpans"].items()}
            node_q = {}
            for s, n in steps.items():
                node_q[H.q_of_span(s)] = node_q.get(H.q_of_span(s), 0) + n
        if sum(node_q.values()) != u["steps"]:
            raise SystemExit("%s: hand node classes cover %d visits, the document has %d non-skipped"
                             % (fx.id, sum(node_q.values()), u["steps"]))
        u["Nodes"] = u["skipped"] + sum(n * max(H.NODE_FLOOR, H.NODE_WEIGHT * q) for q, n in node_q.items())
        u["nodeQ"] = node_q
        uris[member] = u
    total = 0
    detail = []
    oracle_refs = list(oracle_refs or [])
    pos = 0
    for count, member, ops, qs in fx.refs:
        u = uris[member]
        if qs is None:
            spans = oracle_refs[pos]["levelSpans"]
            if any(r["levelSpans"] != spans for r in oracle_refs[pos:pos + count]):
                raise SystemExit("%s: oracle-sourced spans differ between identical references" % fx.id)
            qs = [H.q_of_span(s) for s in spans]
        per = H.units_of_reference(u, ops, qs)
        total += count * per
        detail.append({"count": count, "member": member, "levels": len(ops), "q": qs,
                       "V": H.V(ops, qs), "unitsPerReference": per})
        pos += count
    return total, uris, detail


FACT_KEYS = ("T", "Idx", "E", "Dec", "B", "J", "Res", "Prim", "Nodes")


def run(fx, path, large, verify_caps=True):
    rel = os.path.relpath(path, TOP)
    digest = B.write_3tz(path, fx.members())
    archive = dict(fx.members())
    detail = not large or sum(c for c, *_ in fx.refs) <= 1000
    limits = dict(fx.limits)
    if large:
        limits.setdefault("containmentWorkUnits", UNBOUNDED)
    t0 = time.process_time()
    report = Oracle(path, limits, precheck_only=large, detail=detail).run()
    seconds = time.process_time() - t0
    row = {"id": fx.id, "matrix": fx.matrix, "size": fx.size, "title": fx.title, "file": rel, "sha256": digest,
           "bytes": os.path.getsize(path), "references": sum(c for c, *_ in fx.refs), "notes": fx.notes}
    problems = []
    outcome = {k: report.get(k) for k in ("outcome", "reasons", "facts", "why") if report.get(k) is not None}
    row["oracle"] = outcome
    want = {"outcome": "PrecheckPassed"} if large and fx.expect["outcome"] == "Contained" else fx.expect
    if outcome.get("outcome") != want["outcome"] or ("reasons" in want and outcome.get("reasons") != want["reasons"]):
        problems.append("verdict %s, expected %s" % (outcome, want))
    row["expected"] = fx.expect
    if fx.limits:
        row["limits"] = fx.limits
    if fx.expect["outcome"] != "Contained":
        row["agree"] = not problems
        row["problems"] = problems
        return row
    payloads = {p["member"]: p for p in report.get("payloads", [])}
    pre = report.get("precheck") or {}
    total, uris, refs = hand_total(fx, archive, payloads, pre.get("references"))
    oracle_total = (pre.get("totals") or {}).get("units")
    row["units"] = total
    row["oracleUnits"] = oracle_total
    row["terms"] = pre.get("terms")
    row["referencesHand"] = refs
    row["payloadFacts"] = {m: {k: u[k] for k in FACT_KEYS + ("skipped", "steps", "nodeQ")} for m, u in uris.items()}
    if total != oracle_total:
        problems.append("hand total %s != oracle total %s" % (total, oracle_total))
    for member, u in uris.items():
        got = payloads.get(member, {})
        for k in FACT_KEYS:
            if got.get(k) != u[k]:
                problems.append("%s %s: hand %s, oracle %s" % (member, k, u[k], got.get(k)))
    if pre.get("references"):
        pos = 0
        for count, member, ops, qs in fx.refs:
            for r in pre["references"][pos:pos + count]:
                got_q = [H.q_of_span(s) for s in r.get("levelSpans", [])]
                if qs is not None and got_q != qs:
                    problems.append("reference %d q %s, hand %s" % (r["reference"], got_q, qs))
                    break
                if [tuple(x[1:5]) for x in r.get("levelOps", [])] != [tuple(o) for o in ops]:
                    problems.append("reference %d level operations differ from the construction" % r["reference"])
                    break
            pos += count
    for name, cap in (("2^28", 2 ** 28), ("2^29", 2 ** 29)):
        row["at" + name] = "Contained" if total <= cap else "workLimit"
    if fx.cap_pair and not large and verify_caps:
        pair = {}
        for label, cap, expect in (("atCap", total, "Contained"), ("oneUnder", total - 1, "NotInspected")):
            r = Oracle(path, dict(fx.limits, containmentWorkUnits=cap)).run()
            ok = r.get("outcome") == expect and (expect == "Contained" or r.get("reasons") == ["workLimit"])
            pair[label] = {"containmentWorkUnits": cap, "outcome": r.get("outcome"), "reasons": r.get("reasons"),
                           "ok": ok}
            if not ok:
                problems.append("at-cap pair %s: %s %s" % (label, r.get("outcome"), r.get("reasons")))
        row["capPair"] = pair
    row["agree"] = not problems
    row["problems"] = problems
    row["_seconds"] = seconds
    return row


def worked_example_identity():
    """C2-WB 3.6 on the committed slice-1 fixture, hand facts from its bytes."""
    path = os.path.join(TOP, "controls", "fixtures", "worked-example-adjusted-containmentSpanBits-29.3tz")
    import zipfile
    with zipfile.ZipFile(path) as z:
        archive = {n: z.read(n) for n in z.namelist()}
    u = hand_payload("w.glb", archive["w.glb"], archive)
    hand = 400 + 256 + 4 + u["E"] + u["Dec"] + u["B"] + 16 * u["J"] + 160 * u["Res"] + 4 * u["Prim"] + 1152 + 16
    r = Oracle(path, {"containmentSpanBits": 29}).run()
    got = r["precheck"]["totals"]["units"]
    ref = r["precheck"]["references"][0]
    ok = (got == hand and ref["V"] == 100 and ref["vertexUnits"] == 400 and r["precheck"]["terms"]["Nodes"] == 256
          and u["Res"] == 0 and u["Prim"] == 1 and u["T"] == 4 and u["Idx"] == 4)
    return {"id": "worked-example-identity", "file": os.path.relpath(path, TOP),
            "formula": "400 + 256 + 4 + E + Dec + B + 16 J + 160 Res + 4 Prim + 1152 + 16",
            "facts": {k: u[k] for k in ("T", "Idx", "E", "Dec", "B", "J", "Res", "Prim")},
            "V": ref["V"], "vertexUnits": ref["vertexUnits"], "nodeUnits": r["precheck"]["terms"]["Nodes"],
            "units": hand, "oracleUnits": got, "agree": ok}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--large", action="store_true", help="also build and run the matrix-size fixtures")
    parser.add_argument("--only", help="run only fixtures whose id contains this text (no manifest write)")
    args = parser.parse_args()
    fdir, ldir = os.path.join(HERE, "fixtures"), os.path.join(HERE, "large")
    os.makedirs(fdir, exist_ok=True)
    manifest_path = os.path.join(HERE, "manifest.json")
    previous = []
    if os.path.exists(manifest_path):
        with open(manifest_path) as fh:
            current = {fn(*a).id for fn, a in classes(True)} if not args.large else set()
            previous = [r for r in json.load(fh)["fixtures"]
                        if r.get("size") == "large" and r["id"] in current]
    if not args.only:
        for name in os.listdir(fdir):
            os.remove(os.path.join(fdir, name))
    rows = []
    plan = [(fn, a, False) for fn, a in classes(False)]
    if args.large:
        os.makedirs(ldir, exist_ok=True)
        plan += [(fn, a, True) for fn, a in classes(True)]
    for fn, a, large in plan:
        fx = fn(*a)
        if args.only and args.only not in fx.id:
            continue
        path = os.path.join(ldir if large else fdir, fx.id + ".3tz")
        row = run(fx, path, large)
        seconds = row.pop("_seconds", 0.0)
        rows.append(row)
        print("%-6s %-44s %-5s units %15s  oracle %6.1fs" % (fx.size, fx.id, "ok" if row["agree"] else "FAIL",
                                                             row.get("units", "-"), seconds), flush=True)
        for p in row["problems"]:
            print("       ", p)
    if args.only:
        return 0 if all(r["agree"] for r in rows) else 1
    if not args.large:
        rows += previous  # matrix-size rows are kept as recorded by the last --large run
    identity = worked_example_identity()
    print("worked-example identity:", "ok" if identity["agree"] else "FAIL", identity["units"])
    summary = {"fixtures": len(rows), "agree": len([r for r in rows if r["agree"]]),
               "disagree": [r["id"] + "@" + r["size"] for r in rows if not r["agree"]],
               "capPairs": len([r for r in rows if "capPair" in r]),
               "workedExampleIdentity": identity["agree"]}
    with open(manifest_path, "w") as fh:
        json.dump({"record": "C2-WB class fixtures (O-WB). `units` is the expected C2-WB total; the oracle's "
                             "total and the per-URI facts must equal the hand derivation. CI-size rows carry "
                             "the at-cap pair (containmentWorkUnits = units -> Contained; units - 1 -> "
                             "NotInspected(workLimit)).",
                   "summary": summary, "workedExample": identity, "fixtures": rows}, fh, indent=1, sort_keys=True)
        fh.write("\n")
    print(json.dumps(summary, indent=1))
    return 0 if not summary["disagree"] and identity["agree"] else 1


if __name__ == "__main__":
    sys.exit(main())
