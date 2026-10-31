"""Fixture building blocks for the witness and control generator.

Independent of the oracle: no `c2o` import. Contains a minimal GLB/b3dm
writer, a stored 3TZ writer, double-rounding helpers and the small exact
frame helpers used only to place volumes around independently known points.
"""

import hashlib
import json
import struct
import zipfile
from fractions import Fraction

# ---------------------------------------------------------------- doubles


def _bits(x):
    return struct.unpack("<q", struct.pack("<d", x))[0]


def _from_bits(b):
    return struct.unpack("<d", struct.pack("<q", b))[0]


def next_up(x):
    if x == 0:
        return 5e-324
    b = _bits(x)
    return _from_bits(b + 1) if x > 0 else _from_bits(b - 1)


def next_down(x):
    return -next_up(-x)


def round_up(fr):
    """Smallest binary64 >= fr."""
    d = float(fr)
    while Fraction(d) < fr:
        d = next_up(d)
    while Fraction(next_down(d)) >= fr:
        d = next_down(d)
    return d


def round_down(fr):
    return -round_up(-fr)


def f32(x):
    return struct.unpack("<f", struct.pack("<f", x))[0]


def f32_next_up(x):
    b = struct.unpack("<I", struct.pack("<f", x))[0]
    b = b + 1 if x > 0 else (b - 1 if x < 0 else 1)
    return struct.unpack("<f", struct.pack("<I", b))[0]


# ---------------------------------------------------------------- exact frames


def mat_identity():
    return [[Fraction(int(i == j)) for j in range(4)] for i in range(4)]


def mat_mul(a, b):
    return [[sum(a[i][k] * b[k][j] for k in range(4)) for j in range(4)] for i in range(4)]


def col_major(m):
    return [[Fraction(m[4 * c + r]) for c in range(4)] for r in range(4)]


def node_local(node):
    """T * R(q) * S with the normalized meaning R = H(q)/|q|^2, or the matrix."""
    if "matrix" in node:
        return col_major(node["matrix"])
    t = [Fraction(v) for v in node.get("translation", [0, 0, 0])]
    s = [Fraction(v) for v in node.get("scale", [1, 1, 1])]
    x, y, z, w = [Fraction(v) for v in node.get("rotation", [0, 0, 0, 1])]
    n = x * x + y * y + z * z + w * w
    R = [[1 - 2 * (y * y + z * z) / n, 2 * (x * y - z * w) / n, 2 * (x * z + y * w) / n],
         [2 * (x * y + z * w) / n, 1 - 2 * (x * x + z * z) / n, 2 * (y * z - x * w) / n],
         [2 * (x * z - y * w) / n, 2 * (y * z + x * w) / n, 1 - 2 * (x * x + y * y) / n]]
    M = mat_identity()
    for r in range(3):
        for c in range(3):
            M[r][c] = R[r][c] * s[c]
        M[r][3] = t[r]
    return M


def apply(M, p):
    return [sum(M[r][c] * p[c] for c in range(3)) + M[r][3] for r in range(3)]


def up(p):
    return [p[0], -p[2], p[1]]


def tile_points(nodes, path, x):
    """Own-tile-local point for node-input x along a node path (root first)."""
    M = mat_identity()
    for n in path:
        M = mat_mul(M, node_local(nodes[n]))
    return up(apply(M, x))


# ---------------------------------------------------------------- volumes


def tight_box(points):
    """Box with each row as tight as binary64 allows around exact points.

    c_j = nearest double of the row midpoint, h_j = smallest double with every
    point inside. Shrinking h_j by one ulp (or, when h_j == 0, moving c_j by one
    ulp) excludes the extreme point."""
    c, h = [], []
    for j in range(3):
        lo = min(p[j] for p in points)
        hi = max(p[j] for p in points)
        cj = float((lo + hi) / 2)
        hj = round_up(max(hi - Fraction(cj), Fraction(cj) - lo))
        c.append(cj)
        h.append(hj)
    return {"box": [c[0], c[1], c[2], h[0], 0.0, 0.0, 0.0, h[1], 0.0, 0.0, 0.0, h[2]]}


def box_cr(bv):
    b = bv["box"]
    return [b[0], b[1], b[2]], [abs(b[3]) + abs(b[4]) + abs(b[5]),
                                 abs(b[6]) + abs(b[7]) + abs(b[8]),
                                 abs(b[9]) + abs(b[10]) + abs(b[11])]


def box_contains(bv, p):
    if "box" in bv:
        b = bv["box"]
        cols = [b[3:6], b[6:9], b[9:12]]
        for row in range(3):
            covering = [col[row] for col in cols if col[row] != 0]
            h = abs(Fraction(covering[0])) if covering else Fraction(0)
            if abs(p[row] - Fraction(b[row])) > h:
                return False
        return True
    s = bv["sphere"]
    return sum((p[j] - Fraction(s[j])) ** 2 for j in range(3)) <= Fraction(s[3]) ** 2


def image_box(bv, transform):
    """Axis box containing the image of an axis box under a signed-permutation
    scale + translation transform (exact when representable)."""
    c, h = box_cr(bv)
    T = col_major(transform)
    cc, hh = [], []
    for r in range(3):
        col = [m for m in range(3) if T[r][m] != 0]
        if len(col) != 1:
            raise ValueError("transform is not a signed permutation scale")
        m = col[0]
        centre = T[r][m] * Fraction(c[m]) + T[r][3]
        half = abs(T[r][m]) * Fraction(h[m])
        cr = float(centre)
        cc.append(cr)
        hh.append(round_up(half + abs(Fraction(cr) - centre)))
    return {"box": [cc[0], cc[1], cc[2], hh[0], 0.0, 0.0, 0.0, hh[1], 0.0, 0.0, 0.0, hh[2]]}


def ulp_mutants(bv, points):
    """One-ulp mutants of an axis box: per row, shrink h by one ulp, or when
    h == 0 move c by one ulp. Only mutants that exclude a point are returned:
    list of (row, mutated volume, description)."""
    out = []
    for row in range(3):
        b = list(bv["box"])
        hidx = 3 + 4 * row  # diagonal entry of column `row`
        if b[hidx] != 0:
            b[hidx] = next_down(abs(b[hidx])) * (1 if b[hidx] > 0 else -1)
            how = "h[%d] -> one ulp smaller" % row
        else:
            b[row] = next_up(b[row])
            how = "c[%d] -> one ulp larger (h[%d] == 0)" % (row, row)
        mutated = {"box": b}
        if any(not box_contains(mutated, p) for p in points):
            out.append((row, mutated, how))
    return out


# ---------------------------------------------------------------- glTF


COMPONENT = {5120: ("b", 1), 5121: ("B", 1), 5122: ("h", 2), 5123: ("H", 2), 5125: ("I", 4),
             5126: ("f", 4)}


class Gltf:
    def __init__(self):
        self.doc = {"asset": {"version": "2.0", "generator": "rusty-tiles C2 oracle witness generator"}}
        self.bin = bytearray()
        self.views = []
        self.accessors = []

    def view(self, data, target=None, stride=None):
        while len(self.bin) % 4:
            self.bin.append(0)
        v = {"buffer": 0, "byteOffset": len(self.bin), "byteLength": len(data)}
        if stride is not None:
            v["byteStride"] = stride
        if target is not None:
            v["target"] = target
        self.bin += data
        self.views.append(v)
        return len(self.views) - 1

    def accessor(self, view, ctype, count, atype, **extra):
        a = {"componentType": ctype, "count": count, "type": atype}
        if view is not None:
            a["bufferView"] = view
        a.update(extra)
        self.accessors.append(a)
        return len(self.accessors) - 1

    def positions(self, values, ctype=5126, normalized=False, viewless=False, declared=None):
        """VEC3 POSITION accessor; 1- and 2-byte types use a 4-aligned stride."""
        fmt, size = COMPONENT[ctype]
        if viewless:
            lo, hi = declared
            return self.accessor(None, ctype, len(values), "VEC3", min=lo, max=hi)
        stride = None if ctype == 5126 else (4 if size == 1 else 8)
        data = bytearray()
        for v in values:
            data += struct.pack("<3" + fmt, *v)
            if stride:
                data += bytes(stride - 3 * size)
        if ctype == 5126:
            vals = [[f32(c) for c in v] for v in values]
        else:
            vals = [list(v) for v in values]
        lo = [min(v[j] for v in vals) for j in range(3)]
        hi = [max(v[j] for v in vals) for j in range(3)]
        extra = {"min": lo, "max": hi}
        if normalized:
            extra["normalized"] = True
        view = self.view(bytes(data), target=34962, stride=stride)
        return self.accessor(view, ctype, len(values), "VEC3", **extra)

    def scalars(self, values, ctype, target=None):
        fmt, _ = COMPONENT[ctype]
        view = self.view(struct.pack("<%d%s" % (len(values), fmt), *values), target=target)
        return self.accessor(view, ctype, len(values), "SCALAR")

    def indices(self, values, ctype=5123):
        return self.scalars(values, ctype, target=34963)

    def glb(self):
        doc = dict(self.doc)
        if self.bin:
            while len(self.bin) % 4:
                self.bin.append(0)
            doc["buffers"] = [{"byteLength": len(self.bin)}]
            doc["bufferViews"] = self.views
        if self.accessors:
            doc["accessors"] = self.accessors
        return glb_bytes(doc, bytes(self.bin))


def glb_bytes(doc, binary):
    js = json.dumps(doc, separators=(",", ":"), sort_keys=True).encode()
    js += b" " * (-len(js) % 4)
    out = bytearray(struct.pack("<4sII", b"glTF", 2, 0))
    out += struct.pack("<II", len(js), 0x4E4F534A) + js
    if binary:
        binary = binary + bytes(-len(binary) % 4)
        out += struct.pack("<II", len(binary), 0x004E4942) + binary
    struct.pack_into("<I", out, 8, len(out))
    return bytes(out)


def b3dm(glb, feature_json, feature_bin=b""):
    js = json.dumps(feature_json, separators=(",", ":")).encode()
    js += b" " * (-(28 + len(js)) % 8)
    fb = feature_bin + bytes(-len(feature_bin) % 8)
    body = js + fb + glb
    header = struct.pack("<4s6I", b"b3dm", 1, 28 + len(body), len(js), len(fb), 0, 0)
    return header + body


# ---------------------------------------------------------------- tilesets and 3TZ


def tile(volume, content=None, content_volume=None, transform=None, children=None,
         geometric_error=0.0, **extra):
    t = {"boundingVolume": volume, "geometricError": geometric_error}
    if transform is not None:
        t["transform"] = transform
    if content is not None:
        t["content"] = {"uri": content}
        if content_volume is not None:
            t["content"]["boundingVolume"] = content_volume
    if children:
        t["children"] = children
    t.update(extra)
    return t


def tileset(root, version="1.1", **extra):
    root.setdefault("refine", "ADD")
    doc = {"asset": {"version": version}, "geometricError": 1000.0, "root": root}
    doc.update(extra)
    return doc


def json_bytes(doc):
    return json.dumps(doc, separators=(",", ":"), sort_keys=True).encode()


def write_3tz(path, members):
    """Stored (uncompressed) deterministic 3TZ with a trailing @3dtilesIndex1@."""
    entries = []
    with zipfile.ZipFile(path, "w", zipfile.ZIP_STORED) as z:
        for name, data in members:
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_STORED
            info.create_system = 3
            info.external_attr = 0o100644 << 16
            z.writestr(info, data)
            entries.append((hashlib.md5(name.encode()).digest(), info.header_offset))
        entries.sort(key=lambda e: struct.unpack("<QQ", e[0]))
        index = b"".join(h + struct.pack("<Q", off) for h, off in entries)
        info = zipfile.ZipInfo("@3dtilesIndex1@", date_time=(1980, 1, 1, 0, 0, 0))
        info.compress_type = zipfile.ZIP_STORED
        info.create_system = 3
        info.external_attr = 0o100644 << 16
        z.writestr(info, index)
    with open(path, "rb") as fh:
        return hashlib.sha256(fh.read()).hexdigest()


# ---------------------------------------------------------------- minimal meshopt encoders


def meshopt_attributes(data, count, stride):
    """Valid ATTRIBUTES (0xa0) stream: one block per 256 elements at most, every
    group stored raw (mode 3), baseline element all zeros in the tail."""
    assert stride % 4 == 0 and stride <= 256
    out = bytearray([0xA0])
    max_block = min((8192 // stride) & ~15, 256)
    prev = [0] * stride
    done = 0
    while done < count:
        n = min(count - done, max_block)
        groups = (n + 15) // 16
        for k in range(stride):
            out += bytes([0xFF] * ((groups + 3) // 4))
            p = prev[k]
            for e in range(groups * 16):
                v = data[(done + e) * stride + k] if e < n else p
                d = (v - p) & 0xFF
                out.append(((d << 1) ^ (0xFF if d & 0x80 else 0)) & 0xFF)
                p = v
            prev[k] = data[(done + n - 1) * stride + k]
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


def meshopt_triangles(indices):
    """Valid TRIANGLES (0xe1) stream using only explicit 0xff codes."""
    assert len(indices) % 3 == 0
    codes = bytearray()
    data = bytearray()
    last = 0
    for t in range(0, len(indices), 3):
        codes.append(0xFF)
        data.append(0xFF)  # Z = W = 0xf: b and c explicit
        for v in indices[t:t + 3]:
            delta = (v - last) & 0xFFFFFFFF
            signed = delta - (1 << 32) if delta >> 31 else delta
            zz = (signed << 1) ^ (signed >> 31) if signed >= 0 else ((-signed - 1) << 1) | 1
            data += _varint(zz & 0xFFFFFFFF)
            last = v
    return bytes([0xE1]) + bytes(codes) + bytes(data) + bytes(16)
