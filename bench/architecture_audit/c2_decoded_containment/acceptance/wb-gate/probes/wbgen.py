#!/usr/bin/env python3
"""C2-WB measurement shapes: explicit 3TZ archives whose C2 streaming cost isolates one operation class.

Own transcription (no repository or reviewer code imported). Every archive is designed to be
Contained under C2 and admitted by C1 (`hierarchyBounds` included); the harness checks both.

Shape families (`chain(...)`):
  vol   box | sph          volume kind at every level
  step  bool               a tile transform on every non-root tile (one tile step per ancestor level)
  ext   bool               every ancestor level reached through an external tileset whose referencing
                           content carries a boundingVolume (two volume tests per ancestor level)
  mag   ("small",)         1-limb spans (u8 raw words, values below 2^14)
        ("big", E)         D genuinely about E bits: centres at 2^E (and steps of 2^E)
        ("scale", E)       steps x -> 2^-E x + 2^E: frame k holds about (k+1)E genuine bits
  word  u8 | f32

`refs(...)` builds many references to tiny payloads, optionally with extra JSON or BIN bytes, at a
chosen explicit depth, optionally ending with one over-cap payload (precheck-only arm).
"""
import hashlib
import json
import struct
import zlib

INDEX = "@3dtilesIndex1@"


def write_3tz(members, path):
    names = sorted(members, key=lambda n: n.encode("utf-8"))
    names.remove("tileset.json")
    names.insert(0, "tileset.json")
    out = bytearray()
    central = []

    def entry(name, data):
        nb = name.encode("utf-8")
        crc = zlib.crc32(data) & 0xFFFFFFFF
        off = len(out)
        out.extend(struct.pack("<IHHHHHIIIHH", 0x04034B50, 20, 0x0800, 0, 0, 0x21, crc, len(data), len(data),
                               len(nb), 0))
        out.extend(nb)
        out.extend(data)
        central.append(struct.pack("<IHHHHHHIIIHHHHHII", 0x02014B50, 0x0314, 20, 0x0800, 0, 0, 0x21, crc,
                                   len(data), len(data), len(nb), 0, 0, 0, 0, 0o100644 << 16, off) + nb)
        return off

    index = []
    for n in names:
        index.append((hashlib.md5(n.encode("utf-8")).digest(), entry(n, bytes(members[n]))))
    index.sort(key=lambda e: (int.from_bytes(e[0][:8], "little"), int.from_bytes(e[0][8:], "little")))
    entry(INDEX, b"".join(h + struct.pack("<Q", o) for h, o in index))
    cd = len(out)
    for c in central:
        out.extend(c)
    out.extend(struct.pack("<IHHHHIIH", 0x06054B50, 0, 0, len(central), len(central), len(out) - cd, cd, 0))
    with open(path, "wb") as fh:
        fh.write(out)


def twin(members):
    """Same archive with asset.gltfUpAxis = "Z": a C2 exclusion, so no precheck and no streaming."""
    m = dict(members)
    doc = json.loads(m["tileset.json"])
    doc["asset"]["gltfUpAxis"] = "Z"
    m["tileset.json"] = cj(doc)
    return m


def cj(obj):
    return json.dumps(obj, separators=(",", ":")).encode()


def pad(b, n, c=b" "):
    return b + c * ((-len(b)) % n)


def glb(n, word="u8", extra_json=0, extra_bin=0, point=None):
    if point is not None:
        pts = [point] * n
    else:
        pts = [((i * 37) % 256, (i * 101) % 256, (i * 211) % 256) for i in range(n)]
    if word == "u8":
        bn = b"".join(struct.pack("<BBBx", *p) for p in pts)
        acc = {"bufferView": 0, "componentType": 5121, "count": n, "type": "VEC3",
               "min": [min(p[k] for p in pts) for k in range(3)], "max": [max(p[k] for p in pts) for k in range(3)]}
        view = {"buffer": 0, "byteLength": len(bn), "byteStride": 4}
        ext = {"extensionsUsed": ["KHR_mesh_quantization"], "extensionsRequired": ["KHR_mesh_quantization"]}
    else:
        bn = b"".join(struct.pack("<3f", *map(float, p)) for p in pts)
        acc = {"bufferView": 0, "componentType": 5126, "count": n, "type": "VEC3",
               "min": [float(min(p[k] for p in pts)) for k in range(3)],
               "max": [float(max(p[k] for p in pts)) for k in range(3)]}
        view = {"buffer": 0, "byteLength": len(bn)}
        ext = {}
    used = len(bn)
    bn = bn + b"\0" * extra_bin
    g = dict(asset={"version": "2.0"}, accessors=[acc], bufferViews=[view], buffers=[{"byteLength": len(bn)}],
             meshes=[{"primitives": [{"attributes": {"POSITION": 0}, "mode": 0}]}],
             nodes=[{"mesh": 0}], scene=0, scenes=[{"nodes": [0]}], **ext)
    if extra_json:
        g["extras"] = {"pad": "x" * extra_json}
    js = pad(cj(g), 4)
    bn = pad(bn, 4, b"\0")
    assert used <= len(bn)
    total = 12 + 8 + len(js) + 8 + len(bn)
    return (b"glTF" + struct.pack("<II", 2, total) + struct.pack("<I", len(js)) + b"JSON" + js
            + struct.pack("<I", len(bn)) + b"BIN\x00" + bn)


def _volume(kind, c, h):
    if kind == "box":
        return {"box": [c, c, c, h, 0, 0, 0, h, 0, 0, 0, h]}
    return {"sphere": [c, c, c, h * 2.0]}  # radius 2h >= sqrt(3) h covers the box of half h


def _frames(L, step, mag):
    """Per chain level l (0 = leaf): (centre, half, child-to-parent transform of level l-1 or None)."""
    out = []
    kind = mag[0]
    for l in range(L):
        if kind == "small":
            t = 1.0 if step else 0.0
            c, h = 128.0 + l * t, 1000.0 + 2 * l
            tr = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, t, t, t, 1] if step and l > 0 else None
        elif kind == "big":
            E = mag[1]
            t = 2.0 ** E if step else 0.0
            c, h = 2.0 ** E + l * t, 2.0 ** (E + 1)
            tr = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, t, t, t, 1] if step and l > 0 else None
        else:  # scale
            E = mag[1]
            s, t = 2.0 ** -E, 2.0 ** E
            if l == 0:
                c, h = 128.0, 1000.0
            else:
                c, h = t, (1.0 if l == 1 else 2.0)
            tr = [s, 0, 0, 0, 0, s, 0, 0, 0, 0, s, 0, t, t, t, 1] if l > 0 else None
        out.append((c, h, tr))
    return out


def chain(L, vol="box", step=False, ext=False, mag=("small",), n=100000, word="u8", content_volume=False):
    """L chain levels; the leaf (level 0) holds c.glb. Level l's transform is carried by the tile at level
    l-1 (the child), as in 3D Tiles. With ext, every level boundary is an external tileset document."""
    frames = _frames(L, step, mag)
    m = {"c.glb": glb(n, word)}

    def tile(l):
        c, h, _ = frames[l]
        t = {"boundingVolume": _volume(vol, c, h), "geometricError": float(l)}
        if l + 1 < L and frames[l + 1][2] is not None:
            t["transform"] = frames[l + 1][2]
        return t

    if not ext:
        node = tile(0)
        node["content"] = {"uri": "c.glb"}
        if content_volume:
            node["content"]["boundingVolume"] = _volume(vol, *frames[0][:2])
        for l in range(1, L):
            parent = tile(l)
            parent["children"] = [node]
            node = parent
        m["tileset.json"] = cj({"asset": {"version": "1.1"}, "geometricError": float(L), "root": node})
        return m
    # External chain: document k (k = L-1 is tileset.json) has root = level k tile.
    for l in range(L):
        name = "tileset.json" if l == L - 1 else "d%d.json" % l
        root = tile(l)
        if l == 0:
            root["content"] = {"uri": "c.glb"}
            if content_volume:
                root["content"]["boundingVolume"] = _volume(vol, *frames[0][:2])
        else:
            root["content"] = {"uri": "d%d.json" % (l - 1), "boundingVolume": _volume(vol, *frames[l][:2])}
        m[name] = cj({"asset": {"version": "1.1"}, "geometricError": float(l), "root": root})
    return m


def refs(docs, tiles, contents, depth=1, extra_json=0, extra_bin=0, overflow=0, payload=None, half=10.0):
    """docs x tiles x contents references to one 1-point GLB. `depth` - 1 single-tile documents chain
    above the hub tile (each adds one chain level; JSON nesting stays shallow). overflow > 0 replaces the
    very last reference by an f32 payload of `overflow` points (precheck-only arm)."""
    def box(x=0.0, h=10.0):
        h = h * half / 10.0
        return {"box": [x, 0, 0, h, 0, 0, 0, h, 0, 0, 0, h]}
    m = {"c.glb": payload if payload is not None else glb(1, "f32", extra_json, extra_bin, point=(1.0, 1.0, -1.0))}
    for d in range(docs):
        kids = []
        for k in range(tiles):
            cs = [{"uri": "c.glb"}] * contents
            if overflow and d == docs - 1 and k == tiles - 1:
                cs = cs[:-1] + [{"uri": "big.glb"}]
            kids.append({"boundingVolume": box(k / 65536), "geometricError": 0, "contents": cs})
        m["e%d.json" % d] = cj({"asset": {"version": "1.1"}, "geometricError": 1,
                                "root": {"boundingVolume": box(0.0, 20.0), "geometricError": 1, "children": kids}})
    links = [{"boundingVolume": box(d / 65536, 30.0), "geometricError": 1, "content": {"uri": "e%d.json" % d}}
             for d in range(docs)]
    hub = {"boundingVolume": box(0.0, 40.0), "geometricError": 1, "children": links}
    if depth <= 1:
        m["tileset.json"] = cj({"asset": {"version": "1.1"}, "geometricError": 1, "root": hub})
    else:
        m["h.json"] = cj({"asset": {"version": "1.1"}, "geometricError": 1, "root": hub})
        for k in range(depth - 1):
            name = "tileset.json" if k == 0 else "p%d.json" % k
            nxt = "h.json" if k == depth - 2 else "p%d.json" % (k + 1)
            m[name] = cj({"asset": {"version": "1.1"}, "geometricError": 1,
                          "root": {"boundingVolume": box(0.0, 40.0), "geometricError": 1, "content": {"uri": nxt}}})
    if overflow:
        m["big.glb"] = glb(overflow, "f32", point=(1.0, 1.0, -1.0))
    return m


def glb_nodes(count, depth=1, rotate=True):
    """One POINTS mesh (one u8 vertex at the origin) instanced by `count` TRS nodes, as `count/depth`
    chains of `depth` nodes (the mesh on each chain's last node). Exact-unit quaternion (0.6, 0, 0, 0.8)."""
    bn = struct.pack("<BBBx", 1, 1, 1)
    nodes = []
    roots = []
    for c in range(count // depth):
        first = len(nodes)
        for k in range(depth):
            n = {"translation": [0.5, 0.25, 0.125]}
            if rotate:
                n["rotation"] = [0.6, 0.0, 0.0, 0.8]
            if k + 1 < depth:
                n["children"] = [len(nodes) + 1]
            else:
                n["mesh"] = 0
            nodes.append(n)
        roots.append(first)
    g = dict(asset={"version": "2.0"},
             accessors=[{"bufferView": 0, "componentType": 5121, "count": 1, "type": "VEC3", "min": [1, 1, 1],
                         "max": [1, 1, 1]}],
             bufferViews=[{"buffer": 0, "byteLength": 4, "byteStride": 4}], buffers=[{"byteLength": 4}],
             meshes=[{"primitives": [{"attributes": {"POSITION": 0}, "mode": 0}]}],
             nodes=nodes, scene=0, scenes=[{"nodes": roots}],
             extensionsUsed=["KHR_mesh_quantization"], extensionsRequired=["KHR_mesh_quantization"])
    js = pad(cj(g), 4)
    total = 12 + 8 + len(js) + 8 + len(bn)
    return (b"glTF" + struct.pack("<II", 2, total) + struct.pack("<I", len(js)) + b"JSON" + js
            + struct.pack("<I", len(bn)) + b"BIN\x00" + bn)


# ---- C2-WB v2: C1-structure shapes (constructions after the delta review's shapes.py, re-implemented) ----

P1 = struct.pack("<3f", 1.0, -1.0, 1.0)  # U maps it to (1, -1, -1)... inside every +-10 box


def glb_raw(g, bn):
    js = pad(cj(g), 4)
    bn = pad(bn, 4, b"\0") if bn else b""
    out = b"glTF" + struct.pack("<II", 2, 12 + 8 + len(js) + (8 + len(bn) if bn else 0))
    out += struct.pack("<I", len(js)) + b"JSON" + js
    if bn:
        out += struct.pack("<I", len(bn)) + b"BIN\x00" + bn
    return out


def point_gltf(extra=None):
    g = dict(asset={"version": "2.0"},
             accessors=[{"bufferView": 0, "componentType": 5126, "count": 1, "type": "VEC3",
                         "min": [1.0, -1.0, 1.0], "max": [1.0, -1.0, 1.0]}],
             bufferViews=[{"buffer": 0, "byteLength": 12}], buffers=[{"byteLength": 12}],
             meshes=[{"primitives": [{"attributes": {"POSITION": 0}, "mode": 0}]}],
             nodes=[{"mesh": 0}], scene=0, scenes=[{"nodes": [0]}])
    if extra is not None:
        g["extras"] = extra
    return g


def nest(value, depth):
    for _ in range(depth):
        value = [value]
    return value


def b3dm_raw(glb_bytes, bt_json):
    ft = b'{"BATCH_LENGTH":0}'
    ft = ft + b" " * ((4 - len(ft)) % 8)
    bt = bt_json + b" " * ((-len(bt_json)) % 8)
    body = ft + bt
    total = 28 + len(body) + len(glb_bytes)
    padn = (-total) % 8
    return (b"b3dm" + struct.pack("<6I", 1, total + padn, len(ft), 0, len(bt), 0) + body + glb_bytes
            + b"\0" * padn)


def c1_shape(name):
    """Returns (payload member name, payload bytes, extra members). Names as in the delta review."""
    parts = name.split("-")
    if name == "json-flat":
        return "c.glb", glb_raw(point_gltf("x" * (1 << 20)), P1), {}
    if parts[0] == "json" and parts[1] == "deep":
        return "c.glb", glb_raw(point_gltf(nest("x" * (1 << 20), int(parts[2]))), P1), {}
    if parts[0] == "json" and parts[1] == "nums":
        return "c.glb", glb_raw(point_gltf(nest([0] * 120000, int(parts[2]))), P1), {}
    if parts[0] == "b3dm":
        d = int(parts[2])
        inner = [0] * 120000 if parts[1] == "nums" else "x" * (1 << 20)
        return "c.b3dm", b3dm_raw(glb_raw(point_gltf(), P1), cj({"p": nest(inner, d)})), {}
    if parts[0] in ("quad", "quadB", "shared"):
        if parts[0] == "quad":
            A = P = int(parts[1])
            M = int(parts[2])
        elif parts[0] == "quadB":
            A, P, M = map(int, parts[1:4])
        else:  # shared: one POSITION accessor used by P primitives (linear in today's mark)
            A, P, M = 1, int(parts[1]), int(parts[2])
        acc = [{"bufferView": 0, "componentType": 5126, "count": 1, "type": "VEC3", "min": [1.0, -1.0, 1.0],
                "max": [1.0, -1.0, 1.0]} for _ in range(A)]
        prims = [{"attributes": {"POSITION": i % A}, "mode": 0} for i in range(P)]
        g = dict(asset={"version": "2.0"}, accessors=acc,
                 bufferViews=[{"buffer": 0, "byteLength": 12, "byteStride": 12}], buffers=[{"byteLength": 12}],
                 meshes=[{"primitives": prims}], nodes=[{"mesh": 0} for _ in range(M)], scene=0,
                 scenes=[{"nodes": list(range(M))}])
        return "c.glb", glb_raw(g, P1), {}
    if parts[0] == "extbuf":
        N = int(parts[1])
        g = point_gltf()
        g["buffers"] = [{"uri": "b0.bin", "byteLength": 12}] + [{"uri": "b%d.bin" % i, "byteLength": 4}
                                                               for i in range(1, N)]
        extra = {"b0.bin": P1}
        for i in range(1, N):
            extra["b%d.bin" % i] = b"\0\0\0\0"
        return "c.gltf", cj(g), extra
    if parts[0] == "unselected":
        g = point_gltf()
        g["nodes"] = [{"mesh": 0}] + [{} for _ in range(int(parts[1]))]
        return "c.glb", glb_raw(g, P1), {}
    if parts[0] == "skipped":
        N = int(parts[1])
        g = point_gltf()
        g["nodes"] = [{"mesh": 0, "children": list(range(1, N + 1))}] + [{} for _ in range(N)]
        return "c.glb", glb_raw(g, P1), {}
    raise ValueError(name)


def refs_named(docs, tiles, contents, member, payload, extra=None, half=10.0):
    """refs() with an arbitrary payload member name and extra members (external resources)."""
    m = refs(docs, tiles, contents, payload=payload, half=half)
    if member != "c.glb":
        m[member] = m.pop("c.glb")
        for name in list(m):
            if name.endswith(".json"):
                m[name] = m[name].replace(b'"uri":"c.glb"', ('"uri":"%s"' % member).encode())
    m.update(extra or {})
    return m
