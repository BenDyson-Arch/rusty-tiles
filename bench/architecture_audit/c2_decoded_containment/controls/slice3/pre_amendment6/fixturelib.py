"""Byte-reproducible fixture primitives for the I1 oracle controls (stdlib only; independent of the oracle code).

Nothing here imports the oracle: member names, availability bits and rows are computed from first principles, so an
oracle defect is not mirrored into the fixtures.
"""
import hashlib
import json
import struct
import zlib
from fractions import Fraction

INDEX = "@3dtilesIndex1@"


# ---------------------------------------------------------------------------------------------------------------
# Serialization helpers

def cjson(obj):
    return json.dumps(obj, separators=(",", ":"), ensure_ascii=False, allow_nan=False).encode("utf-8")


def pad(b, n, fill=b"\x00"):
    return bytes(b) + fill * (-len(b) % n)


def f32(v):
    return struct.unpack("<f", struct.pack("<f", v))[0]


def glb_point(p):
    """A minimal glTF 2.0 GLB: one POINTS primitive with one float32 POSITION (min = max = the point)."""
    q = [f32(v) for v in p]
    assert q == list(p), p
    gltf = {"accessors": [{"bufferView": 0, "componentType": 5126, "count": 1, "max": q, "min": q, "type": "VEC3"}],
            "asset": {"version": "2.0"}, "bufferViews": [{"buffer": 0, "byteLength": 12}],
            "buffers": [{"byteLength": 12}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "mode": 0}]}],
            "nodes": [{"mesh": 0}], "scene": 0, "scenes": [{"nodes": [0]}]}
    js = pad(cjson(gltf), 4, b" ")
    bn = struct.pack("<3f", *q)
    total = 12 + 8 + len(js) + 8 + len(bn)
    return (b"glTF" + struct.pack("<II", 2, total) + struct.pack("<I", len(js)) + b"JSON" + js
            + struct.pack("<I", len(bn)) + b"BIN\x00" + bn)


def glb_tile_point(p):
    """A GLB whose point lands at tile-frame `p` after glTF's y-up to z-up rotation U(x, y, z) = (x, -z, y)
    (core README :441).  Used for explicit payloads, which C2 checks; implicit payloads keep glb_point."""
    return glb_point([p[0], p[2], -p[1]])


def subtree_binary(js_bytes, bin_bytes, magic=b"subt", version=1):
    js = pad(js_bytes, 8, b" ")
    bn = pad(bin_bytes, 8)
    return struct.pack("<4sIQQ", magic, version, len(js), len(bn)) + js + bn


def write_3tz(members):
    """Stored ZIP with tileset.json first, other members in byte order, and the 3TZ index last.
    Header fields mirror the coordinator's convert outputs (version 20, UTF-8 flag, 1980-01-01, mode 0644)."""
    names = sorted(members, key=lambda n: n.encode("utf-8"))
    if "tileset.json" in members:
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
        off = entry(n, bytes(members[n]))
        index.append((hashlib.md5(n.encode("utf-8")).digest(), off))
    index.sort(key=lambda e: (int.from_bytes(e[0][:8], "little"), int.from_bytes(e[0][8:], "little")))
    entry(INDEX, b"".join(h + struct.pack("<Q", o) for h, o in index))
    cd = len(out)
    for c in central:
        out.extend(c)
    out.extend(struct.pack("<IHHHHIIH", 0x06054B50, 0, 0, len(central), len(central), len(out) - cd, cd, 0))
    return bytes(out)


# ---------------------------------------------------------------------------------------------------------------
# Implicit geometry (independent restatement of the subdivision rules)

def morton(coords):
    out = 0
    dims = len(coords)
    for b in range(32):
        for d in range(dims):
            out |= ((coords[d] >> b) & 1) << (b * dims + d)
    return out


def offset(n, level):
    return (n ** level - 1) // (n - 1)


def cell(root, octree, level, x, y, z):
    """Exact cell as floats (asserting exact representability)."""
    c = [Fraction(v) for v in root[0:3]]
    axes = [[Fraction(v) for v in root[3 + 3 * k:6 + 3 * k]] for k in range(3)]
    idx = (x, y, z)
    sub = 3 if octree else 2
    centre = list(c)
    half = []
    for k in range(3):
        if k < sub:
            f = Fraction(2 * idx[k] + 1, 1 << level) - 1
            centre = [centre[j] + axes[k][j] * f for j in range(3)]
            half.append([a / (1 << level) for a in axes[k]])
        else:
            half.append(axes[k])
    vals = centre + [v for r in half for v in r]
    out = [float(v) for v in vals]
    assert all(Fraction(o) == v for o, v in zip(out, vals))
    return out


def parent_of(t):
    l, x, y, z = t
    return (l - 1, x >> 1, y >> 1, z >> 1)


def subst(template, l, x, y, z):
    return template.replace("{level}", str(l)).replace("{x}", str(x)).replace("{y}", str(y)).replace("{z}", str(z))


def bits(indices, count):
    b = bytearray((count + 7) // 8)
    for i in indices:
        b[i >> 3] |= 1 << (i & 7)
    return b


# ---------------------------------------------------------------------------------------------------------------
# Subtree intermediate representation

class SubIR:
    """A subtree before layout.  Availability entries: ("const", v, count_or_None) or ("bits", bytearray, count)."""

    def __init__(self, name, level, coords):
        self.name, self.level, self.coords = name, level, coords
        self.tile = None
        self.content = []
        self.child = None
        self.tables = []          # (class, count, {prop: {"values": bytes, ...extra}})
        self.tile_metadata = None
        self.content_metadata = None
        self.extra_views = []     # unused views (bytes)
        self.external = None      # member name for an external buffer holding every view
        self.alias_content = False
        self.tiles = []           # global tiles in this subtree, rank order
        self.extra_json = {}
        self.post_js = []         # callables(js) after layout
        self.post_bin = []        # callables(bin bytearray, js)
        self.json_text = None     # callable(js) -> bytes overriding compact JSON
        self.header = None        # callable(bytes) -> bytes over the final member
        self.drop = False


def layout(ir):
    """Lay views out at 8-byte offsets in one buffer; returns (json dict, buffer bytes)."""
    views, buf = [], bytearray()
    index_of = {}

    def add(data, key=None):
        if key is not None and key in index_of:
            return index_of[key]
        off = len(buf) + (-len(buf) % 8)
        buf.extend(b"\x00" * (off - len(buf)))
        buf.extend(data)
        views.append({"buffer": 0, "byteLength": len(data), "byteOffset": off})
        if key is not None:
            index_of[key] = len(views) - 1
        return len(views) - 1

    def avail(entry, kind, alias=False):
        if entry[0] == "const":
            o = {"constant": entry[1]}
            if entry[2] is not None:
                o["availableCount"] = entry[2]
            return o
        data, count = entry[1], entry[2]
        v = add(bytes(data), ("avail", bytes(data)) if alias else None)
        o = {"bitstream": v}
        if count is not None:
            o["availableCount"] = count
        return o

    tile = avail(ir.tile, "tile")
    content = [avail(c, "content", ir.alias_content) for c in ir.content]
    child = avail(ir.child, "child")
    tables = []
    for cls, count, props in ir.tables:
        tp = {}
        for pid, col in props.items():
            entry = {"values": add(col["values"])}
            for f in ("arrayOffsets", "stringOffsets"):
                if f in col:
                    entry[f] = add(col[f])
            for f, v in col.items():
                if f not in ("values", "arrayOffsets", "stringOffsets"):
                    entry[f] = v
            tp[pid] = entry
        t = {"class": cls, "count": count}
        if tp:
            t["properties"] = tp
        tables.append(t)
    for data in ir.extra_views:
        add(data)
    js = {}
    if views:
        b = {"byteLength": len(buf)}
        if ir.external:
            b = {"uri": ir.external[1], "byteLength": len(buf)}
        js["buffers"] = [b]
        js["bufferViews"] = views
    if tables:
        js["propertyTables"] = tables
    js["tileAvailability"] = tile
    if content:
        js["contentAvailability"] = content
    js["childSubtreeAvailability"] = child
    if ir.tile_metadata is not None:
        js["tileMetadata"] = ir.tile_metadata
    if ir.content_metadata is not None:
        js["contentMetadata"] = ir.content_metadata
    js.update(ir.extra_json)
    return js, buf


def serialize(ir):
    """Returns {member: bytes} for the subtree (and its external buffer, if any)."""
    js, buf = layout(ir)
    for f in ir.post_js:
        f(js)
    out = {}
    if ir.external:
        for f in ir.post_bin:
            f(buf, js)
        out[ir.external[0]] = bytes(buf)
        binchunk = b""
    else:
        binchunk = bytearray(pad(buf, 8)) if buf else bytearray()
        for f in ir.post_bin:
            f(binchunk, js)
    text = ir.json_text(js) if ir.json_text else cjson(js)
    member = subtree_binary(text, binchunk)
    if ir.header:
        member = ir.header(member)
    out[ir.name] = member
    return out


# ---------------------------------------------------------------------------------------------------------------
# Implicit tileset builder

class Slot:
    def __init__(self, template, avail, member_template=None, header=None, payload=None, metadata_class=None):
        self.template = template
        self.avail = avail                    # set of tiles with content
        self.member_template = member_template or template
        self.header = header or {}
        self.payload = payload                # callable(tile, centre) -> bytes, default glb_point(centre)
        self.metadata_class = metadata_class  # opaque content-metadata class name (UINT8 "weight")


class Implicit:
    def __init__(self, scheme, L, A, box, error, tiles, slots=(), subtree_template="subtrees/{level}/{x}/{y}.subtree",
                 refine="REPLACE", transform=None):
        self.scheme, self.L, self.A = scheme, L, A
        self.octree = scheme == "OCTREE"
        self.n = 8 if self.octree else 4
        self.box, self.error = list(box), error
        self.tiles = set(tiles)
        self.slots = list(slots)
        self.subtree_template = subtree_template
        self.subtree_member_template = None
        self.refine, self.transform = refine, transform
        # rows
        self.rows_box = True
        self.rows_error = True
        self.comp = "FLOAT64"
        self.tile_class = "tile"
        self.box_prop, self.err_prop = "bounds", "error"
        self.row_box = None                   # callable(tile) -> 12 floats
        self.row_error = None                 # callable(tile) -> float
        self.flag_column = False              # opaque BOOLEAN column placed last (BIN padding exists)
        self.table_box = True                 # include the box column in tables
        self.table_error = True
        # encodings
        self.force_bitstream = True
        self.tile_count = True                # availableCount on tileAvailability
        self.content_count = False
        self.child_count = False
        self.alias_content = False
        self.subtree_hooks = []               # callables(ir)
        self.extra_subtrees = {}              # name -> SubIR to add
        self.content_metadata_omit_empty = True

    # -- geometry
    def coords3(self, t):
        return t[1], t[2], t[3] if self.octree else 0

    def centre(self, t):
        c = cell(self.box, self.octree, t[0], *self.coords3(t))
        return c[0:3]

    def default_row(self, t):
        return cell(self.box, self.octree, t[0], *self.coords3(t))

    # -- members
    def subtree_name(self, t):
        return subst(self.subtree_member_template or self.subtree_template, t[0], t[1], t[2], t[3])

    def schema_classes(self):
        classes = {}
        props = {}
        comp = self.comp
        if self.rows_box or self.rows_error or self.flag_column:
            if self.rows_box:
                props[self.box_prop] = {"array": True, "componentType": comp, "count": 12,
                                        "semantic": "TILE_BOUNDING_BOX", "type": "SCALAR"}
            if self.rows_error:
                props[self.err_prop] = {"componentType": comp, "semantic": "TILE_GEOMETRIC_ERROR", "type": "SCALAR"}
            if self.flag_column:
                props["flag"] = {"type": "BOOLEAN"}
            classes[self.tile_class] = {"properties": props}
        for s in self.slots:
            if s.metadata_class:
                classes.setdefault(s.metadata_class, {"properties": {"weight": {"componentType": "UINT8",
                                                                              "type": "SCALAR"}}})
        return classes

    def root_tile(self):
        t = {"boundingVolume": {"box": self.box}}
        if len(self.slots) == 1:
            c = {"uri": self.slots[0].template}
            c.update(self.slots[0].header)
            t["content"] = c
        elif self.slots:
            t["contents"] = []
            for s in self.slots:
                c = {"uri": s.template}
                c.update(s.header)
                t["contents"].append(c)
        t["geometricError"] = self.error
        t["implicitTiling"] = {"availableLevels": self.A, "subdivisionScheme": self.scheme,
                               "subtreeLevels": self.L, "subtrees": {"uri": self.subtree_template}}
        if self.refine is not None:
            t["refine"] = self.refine
        if self.transform is not None:
            t["transform"] = self.transform
        return t

    def subtree_roots(self):
        """Subtree roots reachable from the root through available tiles at multiples of L."""
        roots = [(0, 0, 0, 0)]
        out = []
        while roots:
            r = roots.pop(0)
            out.append(r)
            for t in sorted(self.tiles):
                if t[0] == r[0] + self.L and self.ancestor(t, r[0]) == r:
                    roots.append(t)
        return out

    def ancestor(self, t, level):
        d = t[0] - level
        return (level, t[1] >> d, t[2] >> d, t[3] >> d)

    def build_irs(self):
        n, L = self.n, self.L
        T = offset(n, L)
        C = n ** L
        irs = {}
        for r in self.subtree_roots():
            g = r[0]
            ir = SubIR(self.subtree_name(r), g, r[1:])
            mine = [t for t in self.tiles if g <= t[0] < g + L and self.ancestor(t, g) == r]
            idx = {}
            for t in mine:
                ll = t[0] - g
                local = [t[1] - (r[1] << ll), t[2] - (r[2] << ll)] + ([t[3] - (r[3] << ll)] if self.octree else [])
                idx[t] = offset(n, ll) + morton(local)
            ranked = sorted(mine, key=lambda t: idx[t])
            ir.tiles = ranked
            P = len(ranked)
            tile_bits = bits([idx[t] for t in ranked], T)
            if P == T and not self.force_bitstream:
                ir.tile = ("const", 1, P if self.tile_count else None)
            else:
                ir.tile = ("bits", tile_bits, P if self.tile_count else None)
            for s in self.slots:
                have = [t for t in ranked if t in s.avail]
                cnt = len(have) if self.content_count else None
                if not have and not self.force_bitstream:
                    ir.content.append(("const", 0, cnt))
                else:
                    ir.content.append(("bits", bits([idx[t] for t in have], T), cnt))
            kids = [t for t in self.tiles if t[0] == g + L and self.ancestor(t, g) == r]
            kid_idx = []
            for t in kids:
                local = [t[1] - (r[1] << L), t[2] - (r[2] << L)] + ([t[3] - (r[3] << L)] if self.octree else [])
                kid_idx.append(morton(local))
            cc = len(kid_idx) if self.child_count else None
            if not kid_idx and (not self.force_bitstream or C > 4096):
                ir.child = ("const", 0, cc)
            else:
                ir.child = ("bits", bits(kid_idx, C), cc)
            ir.alias_content = self.alias_content
            props = {}
            if self.comp == "FLOAT32":
                for t in ranked:
                    vals = list((self.row_box or self.default_row)(t)) + [
                        (self.row_error or (lambda t: self.error / (1 << t[0])))(t)]
                    assert all(struct.unpack("<f", struct.pack("<f", v))[0] == v for v in vals), t
            if self.rows_box and self.table_box:
                fmt = "<12f" if self.comp == "FLOAT32" else "<12d"
                props[self.box_prop] = {"values": b"".join(
                    struct.pack(fmt, *(self.row_box or self.default_row)(t)) for t in ranked)}
            if self.rows_error and self.table_error:
                fmt = "<f" if self.comp == "FLOAT32" else "<d"
                props[self.err_prop] = {"values": b"".join(
                    struct.pack(fmt, (self.row_error or (lambda t: self.error / (1 << t[0])))(t)) for t in ranked)}
            if self.flag_column:
                props["flag"] = {"values": bytes(bits([i for i, t in enumerate(ranked) if t[0] % 2 == 0], P))}
            if self.rows_box or self.rows_error or self.flag_column:
                ir.tables.append((self.tile_class, P, props))
                ir.tile_metadata = 0
            if self.slots and all(s.metadata_class for s in self.slots):
                counts = [len([t for t in ranked if t in s.avail]) for s in self.slots]
                if all(counts) or not self.content_metadata_omit_empty:
                    ir.content_metadata = []
                    for s, cnt in zip(self.slots, counts):
                        ir.content_metadata.append(len(ir.tables))
                        ir.tables.append((s.metadata_class, cnt, {"weight": {"values": bytes(range(1, cnt + 1))}}))
            irs[ir.name] = ir
        for name, ir in self.extra_subtrees.items():
            irs[name] = ir
        for hook in self.subtree_hooks:
            hook(irs)
        return irs

    def payloads(self):
        out = {}
        for s in self.slots:
            for t in sorted(s.avail):
                if t not in self.tiles:
                    continue
                name = s.member_template(t) if callable(s.member_template) else subst(s.member_template, *t)
                c = self.centre(t)
                out[name] = s.payload(t, c) if s.payload else glb_point(c)
        return out

    def members(self):
        out = {}
        for ir in self.build_irs().values():
            if not ir.drop:
                out.update(serialize(ir))
        out.update(self.payloads())
        return out


def tileset(root, classes=None, error=None, extra=None, schema_id="i1o"):
    doc = {"asset": {"version": "1.1"}, "geometricError": root["geometricError"] if error is None else error,
           "root": root}
    if classes:
        doc["schema"] = {"classes": classes, "id": schema_id}
    if extra:
        doc.update(extra)
    return doc
