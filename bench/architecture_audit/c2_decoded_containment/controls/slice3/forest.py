"""Slice-3 controls derived from real forest producer output (contract 9.B, 9.C).

Inputs are R's producer archives, pinned by sha256 (`INPUTS`). Every edit is a
same-length in-place rewrite of one stored member (its CRC in the local and
central headers follows), except JSON text edits, which rebuild the stored ZIP
in the original member order with a fresh 3TZ index.

The exact vertex extremes e(t) that the whole-subtree-tight construction needs
come from O-explicit's decoder (`Oracle.trace`, pinned), in R's frame (implicit
levels have no tile step, so level 0 of an implicit reference is R's frame).
The verdicts are stated by construction and then checked by the oracle.
"""

import hashlib
import json
import math
import re
import struct
import sys
import zlib
from fractions import Fraction

INDEX = "@3dtilesIndex1@"

# b51f83d forest corpus (contract 9.B; probes/validate-corpus-b51f83d.json lists the same hashes).
INPUTS = {
    "W1": "572e664409802600da9827fb508e31fbb99d3517f37ae6032616135f693e343d",
    "W2": "29ac8851a677eb948dae6f75cb68d06219299f7d15c002f96116e34d23af8ae9",
    "W4-polygons": "d36e732390c0e4b30eda5cd0c84d7a6ef88de38e2d4f57c46b201b6880fa7244",
    "W5-vec-perf-quantize": "a3b4a32fcb6a55e0238a03c3fbf206d7d7a1e85a4457a56163822bfdd2cea81a",
    "W6": "751e6fbbe86cb18b1f7c71135edc72b307db55bb054aae93945de478f34187dd",
    "W7": "1f8ec4df9356557b8663dfa5a085c77f290c4dbb876cfa0181f57a20ecd99ffe",
}


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def pred(x):
    return math.nextafter(x, -math.inf)


def succ(x):
    return math.nextafter(x, math.inf)


def round_up(fr):
    """Smallest binary64 >= the exact rational fr."""
    x = float(fr)
    while Fraction(x) < fr:
        x = succ(x)
    while Fraction(pred(x)) >= fr:
        x = pred(x)
    return x


# ---------------------------------------------------------------------------------------------------------------
# Stored ZIP / 3TZ

class Zip:
    """A stored ZIP read as raw records (every forest archive is stored; asserted)."""

    def __init__(self, data):
        self.data = bytes(data)
        eocd = self.data.rfind(b"PK\x05\x06")
        _, _, _, _, count, cd_size, cd_off, _ = struct.unpack("<IHHHHIIH", self.data[eocd:eocd + 22])
        self.entries = []
        pos = cd_off
        for _ in range(count):
            fields = struct.unpack("<IHHHHHHIIIHHHHHII", self.data[pos:pos + 46])
            method, crc, csize, usize, nlen, xlen, clen, off = (fields[4], fields[7], fields[8], fields[9],
                                                                fields[10], fields[11], fields[12], fields[16])
            name = self.data[pos + 46:pos + 46 + nlen].decode("utf-8")
            assert method == 0 and csize == usize, name
            lnlen, lxlen = struct.unpack("<HH", self.data[off + 26:off + 30])
            start = off + 30 + lnlen + lxlen
            self.entries.append({"name": name, "cd": pos, "off": off, "start": start, "size": usize})
            pos += 46 + nlen + xlen + clen
        self.by_name = {e["name"]: e for e in self.entries}

    def read(self, name):
        e = self.by_name[name]
        return self.data[e["start"]:e["start"] + e["size"]]

    def names(self):
        return [e["name"] for e in self.entries]

    def replace(self, changes):
        """New archive bytes with members replaced ({name: bytes})."""
        if all(len(v) == self.by_name[k]["size"] for k, v in changes.items()):
            out = bytearray(self.data)
            for name, new in changes.items():
                e = self.by_name[name]
                crc = zlib.crc32(new) & 0xFFFFFFFF
                out[e["start"]:e["start"] + e["size"]] = new
                struct.pack_into("<I", out, e["off"] + 14, crc)
                struct.pack_into("<I", out, e["cd"] + 16, crc)
            return bytes(out)
        members = [(n, changes.get(n, self.read(n))) for n in self.names() if n != INDEX]
        return write_stored(members)


def write_stored(members):
    """Stored ZIP in the given member order, 3TZ index last (md5 of the name, little-endian sort)."""
    out = bytearray()
    central = []
    index = []

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

    for name, data in members:
        index.append((hashlib.md5(name.encode("utf-8")).digest(), entry(name, data)))
    index.sort(key=lambda e: (int.from_bytes(e[0][:8], "little"), int.from_bytes(e[0][8:], "little")))
    entry(INDEX, b"".join(h + struct.pack("<Q", o) for h, o in index))
    cd = len(out)
    for c in central:
        out.extend(c)
    out.extend(struct.pack("<IHHHHIIH", 0x06054B50, 0, 0, len(central), len(central), len(out) - cd, cd, 0))
    return bytes(out)


# ---------------------------------------------------------------------------------------------------------------
# Rows, JSON numbers, GLB JSON

def number_text(v):
    text = repr(float(v))
    assert float(text) == v
    return text


def replace_json_number(text, path_regex, index, new_value):
    """Replace the index-th number of the first JSON array matched by `path_regex` (its group 1 is the array
    body) in a JSON text, leaving the rest of the text byte-identical."""
    m = re.search(path_regex, text)
    if m is None:
        raise RuntimeError("pattern not found: %s" % path_regex)
    body = m.group(1)
    parts = body.split(",")
    parts[index] = number_text(new_value)
    return text[:m.start(1)] + ",".join(parts) + text[m.end(1):]


def glb_json(data):
    jl = struct.unpack("<I", data[12:16])[0]
    assert data[16:20] == b"JSON"
    return data[20:20 + jl].decode("utf-8"), jl


def glb_with_json(data, text):
    """Same GLB with a new JSON chunk text, padded with spaces to 4 bytes; BIN chunk unchanged."""
    _, jl = glb_json(data)
    rest = data[20 + jl:]
    js = text.encode("utf-8")
    js += b" " * (-len(js) % 4)
    total = 12 + 8 + len(js) + len(rest)
    return data[:8] + struct.pack("<I", total) + struct.pack("<I", len(js)) + b"JSON" + js + rest


# ---------------------------------------------------------------------------------------------------------------
# Analysis through the pinned oracles

def analyse(path, ImplicitOracle):
    """Per implicit tile: its I1 record, its own effective-volume box, and the exact extremes of the drawn
    vertices of its own contents in R's frame."""
    oracle = ImplicitOracle(path, detail=False)
    report = oracle.run()
    refs = {r.index: r for r in oracle.traversal.references}
    tiles = {}
    for ref in refs.values():
        imp = getattr(ref, "implicit", None)
        if imp is None:
            continue
        key = (imp["root"], imp["level"], imp["x"], imp["y"], imp["z"])
        t = tiles.setdefault(key, {"key": key, "refs": [], "own": None, "volume": ref.levels[0].volumes[0].volume,
                                   "depth": ref.depth, "member": ref.member})
        t["refs"].append(ref.index)
    for row in oracle.trace():
        ref = refs[row["reference"]]
        imp = getattr(ref, "implicit", None)
        if imp is None:
            continue
        key = (imp["root"], imp["level"], imp["x"], imp["y"], imp["z"])
        p = row["levels"][0]
        t = tiles[key]
        if t["own"] is None:
            t["own"] = [[p[j], p[j]] for j in range(3)]
        for j in range(3):
            t["own"][j][0] = min(t["own"][j][0], p[j])
            t["own"][j][1] = max(t["own"][j][1], p[j])
    return oracle, report, tiles


def children_of(tiles):
    kids = {k: [] for k in tiles}
    for k in tiles:
        root, level, x, y, z = k
        if level == 0:
            continue
        parent = (root, level - 1, x >> 1, y >> 1, z >> 1)
        if parent in kids:
            kids[parent].append(k)
    return kids


def subtree_extreme(tiles, kids, key, axis, sign):
    """e(t): the exact extreme on (axis, sign) over every content at or below `key` (None if none)."""
    best = None
    stack = [key]
    while stack:
        k = stack.pop()
        own = tiles[k]["own"]
        if own is not None:
            v = own[axis][1] if sign > 0 else own[axis][0]
            if best is None or (v > best if sign > 0 else v < best):
                best = v
        stack.extend(kids[k])
    return best


def row_values(volume):
    raw = volume.raw["box"]
    return [float(v) for v in raw]


def tighten(row, axis, sign, e):
    """Contract 9.C step 1: c' = RN((old far face + e) / 2), h' the smallest binary64 with c' -+ h' beyond the
    old opposite face and c' +- h' reaching e. Returns the new row, c', h'."""
    c = Fraction(row[axis])
    h = Fraction(abs(row[3 + 4 * axis]))
    if sign > 0:
        lower = c - h
        c2 = float((lower + e) / 2)
        h2 = round_up(max(Fraction(c2) - lower, e - Fraction(c2)))
    else:
        upper = c + h
        c2 = float((upper + e) / 2)
        h2 = round_up(max(upper - Fraction(c2), Fraction(c2) - e))
    out = list(row)
    out[axis] = c2
    out[3 + 4 * axis] = h2 if row[3 + 4 * axis] >= 0 else -h2
    return out, c2, h2


def is_axis_aligned(row):
    return all(row[3 + 3 * k + j] == 0 for k in range(3) for j in range(3) if j != k)


def tight_subtree(tiles, kids, x_key, axis, sign):
    """Rows rewritten for the whole-subtree-tight base at X (contract 9.C steps 1-2): {key: (old, new)}."""
    out = {}
    stack = [x_key]
    while stack:
        k = stack.pop()
        stack.extend(kids[k])
        e = subtree_extreme(tiles, kids, k, axis, sign)
        if e is None:
            continue
        old = row_values(tiles[k]["volume"])
        new, _, _ = tighten(old, axis, sign, e)
        out[k] = (old, new)
    return out


def apply_rows(zipf, changes, prefix="subtrees/"):
    """Apply row rewrites [(old, new)] in order; each old row must occur exactly once across the subtree
    members at the time it is rewritten. Returns the new archive bytes."""
    members = {}
    for old, new in changes:
        if old == new:
            continue
        oldb, newb = struct.pack("<12d", *old), struct.pack("<12d", *new)
        hits = []
        for name in zipf.names():
            if name.startswith(prefix):
                data = members.get(name, zipf.read(name))
                count = data.count(oldb)
                if count:
                    hits.append((name, count, data))
        if len(hits) != 1 or hits[0][1] != 1:
            raise RuntimeError("row is not unique: %r" % [(h[0], h[1]) for h in hits])
        name, _, data = hits[0]
        members[name] = data.replace(oldb, newb)
    return zipf.replace(members) if members else zipf.data


def describe(key):
    return "%d/%d/%d/%d" % key[1:]


def dumps(obj):
    return json.dumps(obj, sort_keys=True)


if __name__ == "__main__":  # pragma: no cover
    sys.exit("imported by generate.py")
