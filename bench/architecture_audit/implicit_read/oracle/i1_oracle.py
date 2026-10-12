#!/usr/bin/env python3
"""O-implicit: independent oracle for rusty-tiles I1 (implicit effective-volume read owner).

Contract: I1 contract draft v3.1, sha256 9674e60b959eea0a4a985b6beec7a278ec79c08579f6c19e58f5a070aee03496, with the
v3.2 coordinator rulings in change-log.md (sha256 52b01d2b71436bcae5ecfb005693122559a794020c9d4522af40803a0c6898a3).
Pure Python 3 standard library; no repository imports; never derived from `src/`.

Run as `python3 -I i1_oracle.py [--limit name=value ...] [--exact] ARCHIVE...`.  Each archive yields one canonical
JSON record (see README.md, "Record format").  Verdicts: Accepted, InvalidInput, Unsupported, ResourceLimit (with the
limit name), or Undecided (the oracle's own bound or an explicitly unmodelled margin; never a verdict).

Scope (contract "Independent acceptance plan" A): payload members are opaque (C1 is excluded); the explicit skeleton
around implicit roots is modelled only as far as the visit/depth/reference accounting and the two hierarchy checks.
"""
import hashlib
import json
import math
import re
import struct
import sys
import zipfile
from fractions import Fraction

ORACLE_ID = "O-implicit/1 (v3.2 rulings)"
RULINGS_SHA256 = "52b01d2b71436bcae5ecfb005693122559a794020c9d4522af40803a0c6898a3"
CONTRACT_SHA256 = "9674e60b959eea0a4a985b6beec7a278ec79c08579f6c19e58f5a070aee03496"
INDEX_MEMBER = "@3dtilesIndex1@"

DEFAULT_LIMITS = {
    "hierarchy_visits": 65536,
    "hierarchy_depth": 128,
    "member_bytes": 64 << 20,
    "total_bytes_read": 2 << 30,
    "document_decoded_bytes": 64 << 20,
    "references": 262144,
    "json_bytes": 8 << 20,
    "json_depth": 64,
    "document_items": 65536,
}
# The oracle's own work bound.  Exceeding it is "Undecided", never a verdict.
ORACLE_BUDGET = {"tiles": 4_000_000, "bit_bytes": 8 << 30, "emissions": 4_000_000}

TEMPLATE_CAP = 1024
U64 = (1 << 64) - 1

# Contract "Supported domain / Unsupported": the Unsupported semantic set.
UNSUPPORTED_ANY = frozenset({
    "TILE_BOUNDING_SPHERE", "TILE_BOUNDING_REGION", "TILE_BOUNDING_S2_CELL",
    "TILE_MINIMUM_HEIGHT", "TILE_MAXIMUM_HEIGHT", "TILE_TRANSFORM", "TILE_REFINE",
    "CONTENT_URI", "CONTENT_GROUP_ID",
    "CONTENT_BOUNDING_BOX", "CONTENT_BOUNDING_REGION", "CONTENT_BOUNDING_SPHERE", "CONTENT_BOUNDING_S2_CELL",
    "CONTENT_MINIMUM_HEIGHT", "CONTENT_MAXIMUM_HEIGHT",
})
INTERPRETED = ("TILE_BOUNDING_BOX", "TILE_GEOMETRIC_ERROR")
UNSUPPORTED_CONTENT_EXTRA = frozenset(INTERPRETED)
MODIFIERS = ("offset", "scale", "noData", "default")


class Outcome(Exception):
    """A terminal outcome: InvalidInput, Unsupported, ResourceLimit, Undecided or InvalidState (oracle bug)."""

    def __init__(self, kind, reason, stage="", limit=None):
        super().__init__(f"{kind}: {reason} [{stage}]")
        self.kind, self.reason, self.stage, self.limit = kind, reason, stage, limit


def invalid(reason, stage=""):
    raise Outcome("InvalidInput", reason, stage)


def unsupported(reason, stage=""):
    raise Outcome("Unsupported", reason, stage)


def resource(limit, reason, stage=""):
    raise Outcome("ResourceLimit", reason, stage, limit)


def undecided(reason, stage=""):
    raise Outcome("Undecided", reason, stage)


# --------------------------------------------------------------------------------------------------------------
# Morton, levels and ranks (AVAILABILITY.adoc; contract §3)

def level_offset(n, level):
    """(N^level - 1)/(N - 1): the number of nodes above `level` in a subtree."""
    return (n ** level - 1) // (n - 1)


def interleave(coords):
    """Morton index with x as the least significant bit (AVAILABILITY.adoc examples)."""
    dims = len(coords)
    out, bit = 0, 0
    width = max(c.bit_length() for c in coords) if coords else 0
    for b in range(width):
        for d in range(dims):
            out |= ((coords[d] >> b) & 1) << bit
            bit += 1
    return out


def deinterleave(m, dims, level):
    coords = [0] * dims
    for b in range(level):
        for d in range(dims):
            coords[d] |= ((m >> (b * dims + d)) & 1) << b
    return coords


def child_coords(x, y, z, c):
    """Contract §3: child index c sets x = 2x + (c&1), y = 2y + ((c>>1)&1), z = 2z + ((c>>2)&1)."""
    return 2 * x + (c & 1), 2 * y + ((c >> 1) & 1), 2 * z + ((c >> 2) & 1)


def f64hex(v):
    return struct.pack(">d", v).hex()


# --------------------------------------------------------------------------------------------------------------
# JSON (C1-like strict parse with JsonLimits; see README "Modelling choices")

class _Dup(Exception):
    pass


def _pairs(pairs):
    out = {}
    for k, v in pairs:
        if k in out:
            raise _Dup(k)
        out[k] = v
    return out


def _bad_constant(name):
    raise ValueError(f"non-finite constant {name}")


def parse_json(data, limits, stage):
    if len(data) > limits["json_bytes"]:
        resource("json_bytes", "JSON document exceeds json_bytes", stage)
    try:
        text = bytes(data).decode("utf-8")
    except UnicodeDecodeError:
        invalid("JSON is not UTF-8", stage)
    if text.startswith("﻿"):
        invalid("JSON byte-order mark", stage)
    try:
        value = json.loads(text, object_pairs_hook=_pairs, parse_constant=_bad_constant)
    except _Dup as exc:
        invalid(f"duplicate JSON key {exc}", stage)
    except (ValueError, RecursionError):
        invalid("malformed JSON", stage)
    items, depth = 0, 0
    stack = [(value, 1)]
    while stack:
        v, d = stack.pop()
        items += 1
        if isinstance(v, dict):
            depth = max(depth, d)
            stack.extend((x, d + 1) for x in v.values())
        elif isinstance(v, list):
            depth = max(depth, d)
            stack.extend((x, d + 1) for x in v)
        elif isinstance(v, float) and not math.isfinite(v):
            invalid("non-finite JSON number", stage)
    if items > limits["document_items"]:
        resource("document_items", "JSON exceeds document_items", stage)
    if depth > limits["json_depth"]:
        resource("json_depth", "JSON exceeds json_depth", stage)
    return value


def is_uint(v):
    return isinstance(v, int) and not isinstance(v, bool) and 0 <= v <= U64


def is_num(v):
    return isinstance(v, (int, float)) and not isinstance(v, bool) and math.isfinite(v)


# --------------------------------------------------------------------------------------------------------------
# URI resolution (contract §3 step 3; mirrors the uri() semantics the contract states) and template normalization

_HEX = frozenset(b"0123456789abcdefABCDEF")
_CONTROL = re.compile("[\x00-\x1f\x7f-\x9f]")


def decode_static(text, stage):
    """Percent-decode a static run.  Malformed escape, decoded '/' or '\\', or a control character: InvalidInput."""
    if "%" in text:
        raw = text.encode("utf-8")
        out = bytearray()
        i = 0
        while i < len(raw):
            if raw[i] == 0x25:
                h = raw[i + 1:i + 3]
                if len(h) < 2 or h[0] not in _HEX or h[1] not in _HEX:
                    invalid("malformed percent escape", stage)
                out.append(int(h, 16))
                i += 3
            else:
                out.append(raw[i])
                i += 1
        try:
            text = out.decode("utf-8")
        except UnicodeDecodeError:
            invalid("percent escape decodes to invalid UTF-8", stage)
    if "/" in text or "\\" in text or _CONTROL.search(text):
        invalid("decoded segment contains a separator or control character", stage)
    return text


def base_dir(member):
    return member.split("/")[:-1]


def resolve_uri(base, value, stage):
    """uri(base, value): confined, normalized member name relative to directory segments `base`."""
    if not isinstance(value, str) or value == "":
        invalid("empty or non-string uri", stage)
    if "\\" in value:
        invalid("backslash in uri", stage)
    if any(c in value for c in ":?#") or value.startswith("/"):
        unsupported("uri scheme, query, fragment or absolute path", stage)
    parts = list(base)
    for raw in value.split("/"):
        seg = decode_static(raw, stage)
        if seg in ("", "."):
            continue
        if seg == "..":
            if not parts:
                invalid("uri escapes the archive root", stage)
            parts.pop()
            continue
        parts.append(seg)
    return "/".join(parts)


_BRACES = re.compile(r"[{}]")


class Template:
    """A template normalized once at root admission (contract §3 steps 1-6).  Segments are tuples of parts;
    a part is ("lit", decoded text) or ("var", name).  Substitution only inserts decimal digits."""

    def __init__(self, segments, raw_len):
        self.segments = segments
        self.raw_len = raw_len
        self.text = "/".join("".join(v if k == "lit" else "{" + v + "}" for k, v in seg) for seg in segments)
        self.length = len(self.text.encode("utf-8"))

    def substitute(self, level, x, y, z):
        vals = {"level": str(level), "x": str(x), "y": str(y), "z": str(z)}
        return "/".join("".join(v if k == "lit" else vals[v] for k, v in seg) for seg in self.segments)


def _ascii_endswith_json(text):
    tail = text[-5:]
    if len(tail) < 5 or tail[0] != ".":
        return False
    return "".join(chr(ord(c) + 32) if "A" <= c <= "Z" else c for c in tail[1:]) == "json"


LEGACY_FORMATS = ("b3dm", "i3dm", "pnts", "cmpt")


def _ascii_legacy_suffix(text):
    """H7 (PROPOSED): the legacy 3D Tiles 1.0 format named by a final `.b3dm`/`.i3dm`/`.pnts`/`.cmpt`
    (ASCII case-insensitive), else None."""
    tail = text[-5:]
    if len(tail) < 5 or tail[0] != ".":
        return None
    suffix = "".join(chr(ord(c) + 32) if "A" <= c <= "Z" else c for c in tail[1:])
    return suffix if suffix in LEGACY_FORMATS else None


def normalize_template(raw, base, octree, is_content, stage):
    # Step 1: raw syntax.
    if not isinstance(raw, str):
        invalid("template uri is not a string", stage)
    if "\\" in raw:
        invalid("backslash in template", stage)
    seq = []
    pos = 0
    for m in _BRACES.finditer(raw):
        i = m.start()
        if i < pos:
            continue
        if raw[i] == "}":
            invalid("stray '}' in template", stage)
        j = raw.find("}", i + 1)
        k = raw.find("{", i + 1)
        if j < 0 or (0 <= k < j):
            invalid("unbalanced '{' in template", stage)
        name = raw[i + 1:j]
        if name not in ("level", "x", "y", "z"):
            invalid("unknown template variable", stage)
        if name == "z" and not octree:
            invalid("{z} in a QUADTREE template", stage)
        if i > pos:
            seq.append(("lit", raw[pos:i]))
        seq.append(("var", name))
        pos = j + 1
    if pos < len(raw):
        seq.append(("lit", raw[pos:]))
    required = {"level", "x", "y"} | ({"z"} if octree else set())
    if required - {v for k, v in seq if k == "var"}:
        invalid("template misses a required variable", stage)
    if any(c in raw for c in ":?#") or raw.startswith("/"):
        unsupported("template uri scheme, query, fragment or absolute path", stage)
    # Step 2: escape guard, every segment.
    for m in re.finditer(r"\{", raw):
        j = m.start()
        if "%" in raw[max(0, j - 2):j].rsplit("/", 1)[-1]:
            unsupported("escape adjacent to template variable", stage)
    # Step 3: normalize from the directory of the tileset JSON holding R.
    if _CONTROL.search(raw):
        invalid("control character in template", stage)
    out = [(("lit", s),) for s in base]

    def flush(cur):
        if not cur:
            return
        if len(cur) == 1 and cur[0][0] == "lit":
            text = cur[0][1]
            if text == ".":
                return
            text = decode_static(text, stage)
            if text in ("", "."):
                return
            if text == "..":
                if not out:
                    invalid("template escapes the archive root", stage)
                out.pop()
                return
            out.append((("lit", text),))
            return
        parts = []
        for kind, val in cur:
            if kind == "lit":
                val = decode_static(val, stage)
                if parts and parts[-1][0] == "lit":
                    parts[-1] = ("lit", parts[-1][1] + val)
                    continue
            parts.append((kind, val))
        out.append(tuple(parts))

    cur = []
    for kind, val in seq:
        if kind == "var":
            cur.append(("var", val))
            continue
        pieces = val.split("/")
        if pieces[0]:
            cur.append(("lit", pieces[0]))
        for p in pieces[1:]:
            flush(cur)
            cur = [("lit", p)] if p else []
    flush(cur)
    # Step 4: JSON detection on the last segment of the normalized template (content templates only).
    if is_content and out and out[-1][-1][0] == "lit" and _ascii_endswith_json(out[-1][-1][1]):
        unsupported("external tileset content template", stage)
    # Step 4b (H7, PROPOSED): legacy detection on the same last segment (content templates only).
    if is_content and out and out[-1][-1][0] == "lit":
        legacy = _ascii_legacy_suffix(out[-1][-1][1])
        if legacy is not None:
            unsupported(f"legacy {legacy} content template", stage)
    # Step 5: variable survival.
    if required - {v for seg in out for k, v in seg if k == "var"}:
        unsupported("template variables eliminated by dot-segments", stage)
    t = Template(out, len(raw))
    # Step 6: length cap.
    if t.length > TEMPLATE_CAP:
        unsupported("normalized template exceeds 1 KiB", stage)
    return t


# --------------------------------------------------------------------------------------------------------------
# Boxes, exact derived cells, containment and error order (contract §4)

def box_values(arr, stage, what):
    if not isinstance(arr, list) or len(arr) != 12 or not all(is_num(v) for v in arr):
        invalid(f"{what}: box is not 12 finite numbers", stage)
    return tuple(float(v) for v in arr)


def derived_cell(root, level, coords, octree):
    """Exact rational cell: centre c_j + sum_k a_kj((2 i_k + 1)/2^l - 1); half-axis a_k/2^l (QUADTREE keeps axis 2)."""
    c = [Fraction(v) for v in root[0:3]]
    axes = [[Fraction(v) for v in root[3 + 3 * k:6 + 3 * k]] for k in range(3)]
    div = 1 << level
    sub = 3 if octree else 2
    centre = list(c)
    half = []
    for k in range(3):
        if k < sub:
            f = Fraction(2 * coords[k] + 1, div) - 1
            for j in range(3):
                centre[j] += axes[k][j] * f
            half.append([a / div for a in axes[k]])
        else:
            half.append(list(axes[k]))
    return centre, half


def exact_box(values):
    return [Fraction(v) for v in values[0:3]], [[Fraction(v) for v in values[3 + 3 * k:6 + 3 * k]] for k in range(3)]


def tolerance(outer_axes):
    """1e-6*(1 + ||all half-axis components||), evaluated in binary64 on the container's components."""
    return 1e-6 * (1.0 + math.sqrt(sum(float(v) * float(v) for row in outer_axes for v in row)))


def contains(outer_exact, inner_exact, stage):
    """validate.rs:297 as stated by the contract: tolerance 1e-6*(1 + ||all half-axis components||) of the container.
    Axis-aligned containers are decided exactly; others in binary64 with a boundary guard (Undecided)."""
    oc, oa = outer_exact
    ic, ia = inner_exact
    tol = tolerance(oa)
    owner = {}
    aligned = True
    for k in range(3):
        nz = [j for j in range(3) if oa[k][j] != 0]
        if len(nz) > 1 or (nz and nz[0] in owner):
            aligned = False
            break
        if nz:
            owner[nz[0]] = k
    if aligned:
        ftol = Fraction(tol)
        for j in range(3):
            h = abs(oa[owner[j]][j]) if j in owner else Fraction(0)
            reach = abs(ic[j] - oc[j]) + sum(abs(ia[k][j]) for k in range(3))
            if reach > h + ftol:
                return False
        return True
    # General orthogonal container in binary64.
    a = [[float(v) for v in row] for row in oa]
    lens = [math.sqrt(sum(v * v for v in row)) for row in a]
    nzk = [k for k in range(3) if lens[k] > 0]
    for p in range(len(nzk)):
        for q in range(p + 1, len(nzk)):
            i, j = nzk[p], nzk[q]
            if abs(sum(a[i][t] * a[j][t] for t in range(3))) > 1e-9 * lens[i] * lens[j]:
                undecided("non-orthogonal container box", stage)
    units = [None, None, None]
    for k in nzk:
        units[k] = [v / lens[k] for v in a[k]]
    for k in range(3):
        if units[k] is None:
            basis = [u for u in units if u is not None]
            for e in ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]):
                v = list(e)
                for u in basis:
                    d = sum(v[t] * u[t] for t in range(3))
                    v = [v[t] - d * u[t] for t in range(3)]
                n = math.sqrt(sum(t * t for t in v))
                if n > 1e-6:
                    units[k] = [t / n for t in v]
                    break
    fc = [float(v) for v in ic]
    fa = [[float(v) for v in row] for row in ia]
    occ = [float(v) for v in oc]
    for k in range(3):
        u = units[k]
        reach = abs(sum((fc[t] - occ[t]) * u[t] for t in range(3))) + sum(
            abs(sum(fa[m][t] * u[t] for t in range(3))) for m in range(3))
        bound = lens[k] + tol
        if abs(reach - bound) <= 1e-9 * (1.0 + abs(bound)):
            undecided("containment within floating-point guard of the tolerance", stage)
        if reach > bound:
            return False
    return True


def error_ok(child, parent):
    """validate.rs:1312 as stated by the contract: child > parent + 1e-8*(1 + parent) is a violation."""
    return not (child > parent + 1e-8 * (1.0 + parent))


def parse_transform(arr, stage):
    if arr is None:
        return None
    if not isinstance(arr, list) or len(arr) != 16 or not all(is_num(v) for v in arr):
        invalid("transform is not 16 finite numbers", stage)
    return [float(v) for v in arr]


def transform_box(m, values):
    """Box in the parent frame: centre by the affine map, half-axes by its linear part (column-major)."""
    if m is None:
        return values

    def lin(v):
        return [m[0] * v[0] + m[4] * v[1] + m[8] * v[2],
                m[1] * v[0] + m[5] * v[1] + m[9] * v[2],
                m[2] * v[0] + m[6] * v[1] + m[10] * v[2]]
    c = lin(values[0:3])
    c = [c[0] + m[12], c[1] + m[13], c[2] + m[14]]
    out = c
    for k in range(3):
        out = out + lin(values[3 + 3 * k:6 + 3 * k])
    return tuple(out)


def max_scale(m):
    if m is None:
        return 1.0
    cols = [m[0:3], m[4:7], m[8:11]]
    return max(math.sqrt(sum(v * v for v in c)) for c in cols)


def is_identity(m):
    return m is None or m == [1.0, 0, 0, 0, 0, 1.0, 0, 0, 0, 0, 1.0, 0, 0, 0, 0, 1.0]


# --------------------------------------------------------------------------------------------------------------
# Availability sources (contract §2a)

_SPREAD = {}


def _spread_table(n):
    if n not in _SPREAD:
        width = n  # each source bit becomes n bits
        table = []
        for b in range(256):
            v = 0
            for i in range(8):
                if (b >> i) & 1:
                    v |= ((1 << width) - 1) << (i * width)
            table.append(v.to_bytes(width, "little"))
        _SPREAD[n] = table
    return _SPREAD[n]


def spread(bits_int, count, n):
    """Replicate each of `count` bits n times (n = 4 or 8)."""
    data = bits_int.to_bytes((count + 7) // 8, "little")
    table = _spread_table(n)
    return int.from_bytes(b"".join(map(table.__getitem__, data)), "little")


_NONZERO = re.compile(rb"[^\x00]")


class Source:
    """A deduplicated availability source: C0, C1 or V(view, bits)."""

    def __init__(self, kind, bits, loader=None, view=None):
        self.kind, self.bits, self._loader, self.view = kind, bits, loader, view
        self._data = None
        self.popcount = None
        self._int = None
        self._setbits = None
        self._rank = None

    @property
    def data(self):
        if self._data is None:
            self._data = self._loader()
        return self._data

    def as_int(self):
        if self._int is None:
            self._int = int.from_bytes(self.data, "little")
        return self._int

    def bit(self, i):
        if self.kind == "C1":
            return True
        if self.kind == "C0":
            return False
        return (self.data[i >> 3] >> (i & 7)) & 1 == 1

    def setbits(self):
        if self._setbits is None:
            out = []
            data = self.data
            for m in _NONZERO.finditer(data):
                i = m.start()
                b = data[i]
                while b:
                    low = b & -b
                    out.append(i * 8 + low.bit_length() - 1)
                    b ^= low
            self._setbits = out
        return self._setbits

    def range_int(self, start, count):
        if self.kind == "C1":
            return (1 << count) - 1
        if self.kind == "C0":
            return 0
        lo = start >> 3
        hi = (start + count + 7) >> 3
        return (int.from_bytes(self.data[lo:hi + 1], "little") >> (start & 7)) & ((1 << count) - 1)

    def rank(self, i):
        """popcount(bits[0..i)) (strict rank)."""
        if self.kind == "C1":
            return i
        if self._rank is None:
            blk = 512
            cum = [0]
            data = self.data
            for s in range(0, len(data), blk):
                cum.append(cum[-1] + int.from_bytes(data[s:s + blk], "little").bit_count())
            self._rank = cum
        blk = 512
        byte = i >> 3
        b0 = (byte // blk) * blk
        r = self._rank[byte // blk] + int.from_bytes(self.data[b0:byte], "little").bit_count()
        if i & 7:
            r += (self.data[byte] & ((1 << (i & 7)) - 1)).bit_count()
        return r


# --------------------------------------------------------------------------------------------------------------
# Archive and resource accounting

class Archive:
    def __init__(self, path):
        self.path = path
        with open(path, "rb") as f:
            raw = f.read()
        self.sha256 = hashlib.sha256(raw).hexdigest()
        self.zip = zipfile.ZipFile(path)
        self.sizes = {}
        for info in self.zip.infolist():
            if info.filename == INDEX_MEMBER:
                continue
            self.sizes[info.filename] = info.file_size
        self.total_member_bytes = sum(self.sizes.values())

    def read(self, name):
        return self.zip.read(name)


class State:
    def __init__(self, archive, limits):
        self.a = archive
        self.lim = limits
        self.visits = 0
        self.refs = 0
        self.bytes_read = 0
        self.scan_bytes = 0
        self.tbr_margin_open = False
        self.live = 0
        self.max_live = 0
        self.cumulative_subtree_bytes = 0
        self.used = set()
        self.resolved_subtrees = set()
        self.class_records = {}
        self.class_scans = 0
        self.max_depth = -1
        self.tiles = []
        self.explicit_nodes = 0
        self.explicit_contents = 0
        self.external_links = 0
        self.tilesets = 0
        self.roots = []
        self.subtrees = 0
        self.implicit_tiles = 0
        self.emitted = 0
        self.bit_bytes = 0
        self.unmodelled = 2 * archive.total_member_bytes

    # Visits (contract §7 formula)
    def visit(self, n, stage):
        if n > self.lim["hierarchy_visits"] - self.visits:
            resource("hierarchy_visits", "hierarchy visits exceed limit", stage)
        self.visits += n

    def depth(self, d, stage):
        if d > self.lim["hierarchy_depth"]:
            resource("hierarchy_depth", "hierarchy depth exceeds limit", stage)
        self.max_depth = max(self.max_depth, d)

    def reference(self, name, stage):
        """ArchiveResources::reference: one charge, records use, checks existence."""
        if self.refs + 1 > self.lim["references"]:
            resource("references", "references exceed limit", stage)
        self.refs += 1
        if name not in self.a.sizes:
            invalid(f"missing member {name}", stage)
        self.used.add(name)

    def reference_reserved(self, name, stage):
        if name not in self.a.sizes:
            invalid(f"missing member {name}", stage)
        self.used.add(name)

    def charge_bytes(self, n, stage, what):
        if self.bytes_read + n > self.lim["total_bytes_read"]:
            msg = ("availability scan exceeds total_bytes_read work limit" if what == "scan"
                   else "member reads exceed total_bytes_read work limit")
            resource("total_bytes_read", msg, stage)
        self.bytes_read += n
        if self.bytes_read + self.unmodelled > self.lim["total_bytes_read"]:
            self.tbr_margin_open = True

    def read_member(self, name, stage, live=False):
        size = self.a.sizes[name]
        if size > self.lim["member_bytes"]:
            resource("member_bytes", "member exceeds member_bytes", stage)
        if live and size > self.lim["document_decoded_bytes"] - self.live:
            resource("document_decoded_bytes", "live-path subtree bytes exceed document_decoded_bytes", stage)
        self.charge_bytes(size, stage, "member")
        if live:
            self.live += size
            self.max_live = max(self.max_live, self.live)
        return self.a.read(name)

    def bits_work(self, n, stage):
        self.bit_bytes += n
        if self.bit_bytes > ORACLE_BUDGET["bit_bytes"]:
            undecided("oracle bit-work budget exhausted", stage)


# --------------------------------------------------------------------------------------------------------------
# Metadata classes (contract §5; classification once per (schema document, class), DC-4)

class ClassRecord:
    def __init__(self, cls_id, cls):
        self.id = cls_id
        props = cls.get("properties", {}) if isinstance(cls, dict) else {}
        if not isinstance(props, dict):
            props = {}
        self.props = props
        seen = {}
        self.duplicate = None
        self.unsupported_any = None
        self.unsupported_content = None
        self.interp = {}
        for pid, p in props.items():
            if not isinstance(p, dict):
                continue
            sem = p.get("semantic")
            if not isinstance(sem, str):
                continue
            if sem in seen and self.duplicate is None:
                self.duplicate = sem
            seen[sem] = pid
            if sem in UNSUPPORTED_ANY and self.unsupported_any is None:
                self.unsupported_any = sem
            if sem in UNSUPPORTED_CONTENT_EXTRA and self.unsupported_content is None:
                self.unsupported_content = sem
            if sem in INTERPRETED and sem not in self.interp:
                self.interp[sem] = (pid, p)

    def check_content_use(self, stage):
        if self.duplicate is not None:
            invalid(f"duplicated semantic {self.duplicate} in class {self.id}", stage)
        bad = self.unsupported_any or self.unsupported_content
        if bad is not None:
            unsupported(f"semantic {bad} in content class {self.id}", stage)

    def check_tile_use(self, stage):
        if self.duplicate is not None:
            invalid(f"duplicated semantic {self.duplicate} in class {self.id}", stage)
        if self.unsupported_any is not None:
            unsupported(f"semantic {self.unsupported_any} in tile class {self.id}", stage)
        desc = {}
        for sem, (pid, p) in self.interp.items():
            if any(m in p for m in MODIFIERS):
                unsupported(f"offset, scale, noData or default on {sem}", stage)
            comp = p.get("componentType")
            ok = p.get("type") == "SCALAR" and comp in ("FLOAT32", "FLOAT64") and not p.get("normalized", False)
            if sem == "TILE_BOUNDING_BOX":
                ok = ok and p.get("array") is True and p.get("count") == 12
            else:
                ok = ok and not p.get("array", False) and "count" not in p
            if not ok:
                invalid(f"{sem} property has an unsupported shape", stage)
            desc[sem] = (pid, 4 if comp == "FLOAT32" else 8, p.get("required") is True)
        return desc


def class_record(st, doc_key, schema, cls_id, stage):
    key = (doc_key, cls_id)
    if key not in st.class_records:
        classes = schema.get("classes") if isinstance(schema, dict) else None
        if not isinstance(cls_id, str) or not isinstance(classes, dict) or cls_id not in classes:
            invalid(f"unknown metadata class {cls_id!r}", stage)
        st.class_scans += 1
        st.class_records[key] = ClassRecord(cls_id, classes[cls_id])
    return st.class_records[key]


# --------------------------------------------------------------------------------------------------------------
# Implicit root admission (contract precedence: fields, scheme, levels; volume, shape; templates; header classes)

class Root:
    pass


def admit_root(st, tile, doc, depth, refine, stage):
    it = tile["implicitTiling"]
    if not isinstance(it, dict):
        invalid("implicitTiling is not an object", stage)
    for f in ("subdivisionScheme", "subtreeLevels", "availableLevels", "subtrees"):
        if f not in it:
            invalid(f"implicitTiling misses {f}", stage)
    scheme = it["subdivisionScheme"]
    if not isinstance(scheme, str):
        invalid("subdivisionScheme is not a string", stage)
    if scheme not in ("QUADTREE", "OCTREE"):
        unsupported(f"subdivision scheme {scheme}", stage)
    levels = {}
    for f in ("subtreeLevels", "availableLevels"):
        v = it[f]
        if not isinstance(v, int) or isinstance(v, bool) or v < 1 or v > U64:
            invalid(f"{f} is not an integer >= 1", stage)
        if v > 32:
            unsupported(f"{f} > 32", stage)
        levels[f] = v
    subtrees = it["subtrees"]
    if not isinstance(subtrees, dict) or "uri" not in subtrees:
        invalid("missing subtree template", stage)
    # Volume kind and root shape.
    bv = tile.get("boundingVolume")
    if not isinstance(bv, dict):
        invalid("implicit root without boundingVolume", stage)
    ext = bv.get("extensions")
    if isinstance(ext, dict) and "3DTILES_bounding_volume_S2" in ext:
        unsupported("S2 implicit root volume", stage)
    if "box" in bv:
        box = box_values(bv["box"], stage, "implicit root")
    elif "region" in bv:
        unsupported("region implicit root volume", stage)
    elif "sphere" in bv:
        invalid("sphere implicit root volume", stage)
    else:
        invalid("implicit root volume kind", stage)
    if "children" in tile:
        invalid("implicit root has children", stage)
    if "metadata" in tile:
        invalid("implicit root has metadata", stage)
    if "content" in tile and "contents" in tile:
        invalid("implicit root has both content and contents", stage)
    if "content" in tile:
        slots = [tile["content"]]
    elif "contents" in tile:
        slots = tile["contents"]
        if not isinstance(slots, list) or not slots:
            invalid("contents is not a non-empty array", stage)
    else:
        slots = []
    for c in slots:
        if not isinstance(c, dict):
            invalid("content is not an object", stage)
        if "boundingVolume" in c:
            invalid("implicit content template has boundingVolume", stage)
    if len(slots) > 65535:
        undecided("more content slots than the u16 slot index", stage)
    octree = scheme == "OCTREE"
    base = base_dir(doc["name"])
    templates = []
    for k, c in enumerate(slots):
        if "uri" not in c:
            invalid("content template misses uri", stage)
        templates.append(normalize_template(c["uri"], base, octree, True, f"{stage}/template[{k}]"))
    subtree_t = normalize_template(subtrees["uri"], base, octree, False, f"{stage}/subtrees")
    for k, c in enumerate(slots):
        md = c.get("metadata")
        if md is not None:
            if not isinstance(md, dict):
                invalid("content header metadata is not an object", stage)
            rec = class_record(st, doc["key"], doc["schema"], md.get("class"), f"{stage}/header[{k}]")
            rec.check_content_use(f"{stage}/header[{k}]")
    r = Root()
    r.doc = doc
    r.octree = octree
    r.n = 8 if octree else 4
    r.L = levels["subtreeLevels"]
    r.A = levels["availableLevels"]
    r.box = box
    r.error = float(tile["geometricError"])
    r.refine = refine
    r.depth = depth
    r.templates = templates
    r.subtree_t = subtree_t
    r.emitted = 0
    return r


# --------------------------------------------------------------------------------------------------------------
# Subtree admission (contract §1, §2, §2a steps 1-12, §5)

class Subtree:
    pass


def _avail(obj, what, bits, views, stage):
    if not isinstance(obj, dict):
        invalid(f"{what} is not an object", stage)
    has_b, has_c = "bitstream" in obj, "constant" in obj
    if has_b == has_c:
        invalid(f"{what}: exactly one of bitstream and constant", stage)
    count = obj.get("availableCount")
    if count is not None and not is_uint(count):
        invalid(f"{what}: availableCount is not a u64", stage)
    if has_c:
        c = obj["constant"]
        if not is_uint(c) or c > 1:
            invalid(f"{what}: constant is not 0 or 1", stage)
        if what == "tileAvailability" and c == 0:
            invalid("tileAvailability.constant 0", stage)
        return ("C1" if c == 1 else "C0", None, bits), count
    v = obj["bitstream"]
    if not is_uint(v) or v >= len(views):
        invalid(f"{what}: bitstream view index", stage)
    if views[v][2] != (bits + 7) // 8:
        invalid(f"{what}: bitstream byteLength != ceil(bits/8)", stage)
    return ("V", v, bits), count


def _check_extensions(v, stage):
    stack = [v]
    while stack:
        x = stack.pop()
        if isinstance(x, dict):
            if "extensions" in x:
                unsupported("extensions in subtree JSON", stage)
            stack.extend(v for k, v in x.items() if k != "extras")
        elif isinstance(x, list):
            stack.extend(x)


def admit_subtree(st, root, level, gx, gy, gz):
    n, L, A = root.n, root.L, root.A
    name = root.subtree_t.substitute(level, gx, gy, gz)
    stage = f"subtree {name}"
    if name in st.resolved_subtrees:
        unsupported("repeated subtree member", stage)
    st.resolved_subtrees.add(name)
    st.reference(name, stage)
    data = st.read_member(name, stage, live=True)
    held = len(data)
    sub = Subtree()
    sub.name, sub.level, sub.coords, sub.held = name, level, (gx, gy, gz), held
    # §1 step 1: header.
    if data[:4] != b"subt":
        head = data[3:] if data[:3] == b"\xef\xbb\xbf" else data
        if head.lstrip(b" \t\n\r")[:1] == b"{":
            unsupported("JSON-format subtree", stage)
        invalid("subtree magic is not subt", stage)
    if len(data) < 24:
        invalid("subtree header shorter than 24 bytes", stage)
    _, version, jlen, blen = struct.unpack_from("<4sIQQ", data, 0)
    if version != 1:
        invalid("subtree version is not 1", stage)
    if jlen == 0 or jlen % 8 or blen % 8:
        invalid("subtree chunk lengths", stage)
    if 24 + jlen + blen != len(data):
        invalid("subtree header lengths do not match member length", stage)
    # §1 step 2: JSON chunk.
    js = parse_json(data[24:24 + jlen], st.lim, stage)
    if not isinstance(js, dict):
        invalid("subtree JSON is not an object", stage)
    _check_extensions(js, stage)
    binchunk = memoryview(data)[24 + jlen:]
    # §1 step 3: minItems arrays.
    for f in ("buffers", "bufferViews", "propertyTables", "contentAvailability", "contentMetadata"):
        if f in js and (not isinstance(js[f], list) or not js[f]):
            invalid(f"{f} is empty or not an array", stage)
    # §1 step 4: buffers.
    buffers = []
    for i, b in enumerate(js.get("buffers", [])):
        if not isinstance(b, dict) or not is_uint(b.get("byteLength")) or b["byteLength"] < 1:
            invalid("buffer byteLength", stage)
        bl = b["byteLength"]
        if "uri" not in b:
            if i != 0:
                invalid("internal buffer is not buffer 0", stage)
            if blen == 0:
                invalid("internal buffer without BIN chunk", stage)
            if bl > blen or blen - bl >= 8:
                invalid("internal buffer length does not match BIN", stage)
            if any(binchunk[bl:]):
                invalid("non-zero BIN padding", stage)
            buffers.append(binchunk[:bl])
        else:
            u = b["uri"]
            if not isinstance(u, str):
                invalid("buffer uri is not a string", stage)
            if u[:5].lower() == "data:":
                invalid("data URI buffer", stage)
            member = resolve_uri(base_dir(name), u, stage)
            st.reference(member, stage)
            ext = st.read_member(member, stage, live=True)
            sub.held += len(ext)
            if len(ext) < bl:
                invalid("external buffer shorter than byteLength", stage)
            buffers.append(memoryview(ext)[:bl])
    if blen > 0 and not (buffers and "uri" not in js["buffers"][0]):
        invalid("non-empty BIN without an internal buffer", stage)
    st.cumulative_subtree_bytes += sub.held
    # §1 step 5: buffer views.
    views = []
    for bv in js.get("bufferViews", []):
        if not isinstance(bv, dict):
            invalid("bufferView is not an object", stage)
        bi, off, ln = bv.get("buffer"), bv.get("byteOffset"), bv.get("byteLength")
        if not is_uint(bi) or bi >= len(buffers):
            invalid("bufferView buffer index", stage)
        if off is None:
            invalid("bufferView byteOffset is required", stage)
        if not is_uint(off):
            invalid("bufferView byteOffset is not a JSON u64 integer", stage)
        if off % 8:
            invalid("bufferView byteOffset alignment", stage)
        if not is_uint(ln) or ln < 1:
            invalid("bufferView byteLength", stage)
        if off + ln > len(buffers[bi]):
            invalid("bufferView exceeds its buffer", stage)
        views.append((bi, off, ln))
    # §2a step 1: availability objects, O(1) each.
    N_T = level_offset(n, L)
    C = n ** L
    if "tileAvailability" not in js or "childSubtreeAvailability" not in js:
        invalid("tileAvailability and childSubtreeAvailability are required", stage)
    tile_key, tile_count = _avail(js["tileAvailability"], "tileAvailability", N_T, views, stage)
    K = len(root.templates)
    if K:
        ca = js.get("contentAvailability")
        if not isinstance(ca, list) or len(ca) != K:
            invalid("contentAvailability count differs from the template count", stage)
        content = [_avail(o, "contentAvailability", N_T, views, stage) for o in ca]
    else:
        if "contentAvailability" in js:
            invalid("contentAvailability without content templates", stage)
        content = []
    child_key, child_count = _avail(js["childSubtreeAvailability"], "childSubtreeAvailability", C, views, stage)
    # Deduplicate sources by (kind, view, bits).
    sources = {}

    def src(key):
        if key not in sources:
            kind, v, bits = key
            if kind == "V":
                bi, off, ln = views[v]
                sources[key] = Source("V", bits, lambda bi=bi, off=off, ln=ln: bytes(buffers[bi][off:off + ln]), v)
            else:
                s = Source(kind, bits)
                s.popcount = bits if kind == "C1" else 0
                sources[key] = s
        return sources[key]
    tile_src = src(tile_key)
    content_src = [src(k) for k, _ in content]
    child_src = src(child_key)
    # §2a step 2: Scan charge over distinct View sources, before any bit pass.
    scan = sum((s.bits + 7) // 8 for k, s in sources.items() if k[0] == "V")
    st.charge_bytes(scan, f"{stage} §2a.2", "scan")
    st.scan_bytes += scan
    sub.scan = scan
    # §2a step 3: one pass per distinct View source (popcount, trailing zeros).
    for key, s in sources.items():
        if key[0] != "V":
            continue
        st.bits_work(len(s.data), stage)
        s.popcount = s.as_int().bit_count()
        if s.bits % 8 and s.data[-1] >> (s.bits % 8):
            invalid("availability trailing bits are not zero", f"{stage} §2a.3")
    # §2a step 4: availableCount per object.
    for key, cnt in [(tile_key, tile_count)] + content + [(child_key, child_count)]:
        if cnt is not None and cnt != sources[key].popcount:
            invalid("availableCount differs from the popcount", f"{stage} §2a.4")
    # §2a step 5: root bit and parent rule.
    if tile_src.kind == "V":
        if not tile_src.bit(0):
            invalid("subtree root tile bit is 0", f"{stage} §2a.5")
        _parent_rule(st, tile_src, n, L, stage)
    # §2a step 6: levels.
    g = level
    if tile_src.kind == "C1":
        if g + L - 1 >= A:
            invalid("available tile at or beyond availableLevels", f"{stage} §2a.6")
    elif A - g < L:
        start = level_offset(n, A - g)
        if tile_src.range_int(start, N_T - start):
            invalid("available tile at or beyond availableLevels", f"{stage} §2a.6")
    if child_src.popcount > 0 and g + L >= A:
        invalid("child subtree bit at or beyond availableLevels", f"{stage} §2a.6")
    # §2a step 7: content => tile, once per distinct content source.
    done = set()
    for s in content_src:
        if id(s) in done:
            continue
        done.add(id(s))
        if s.kind == "C0" or s is tile_src or tile_src.kind == "C1":
            continue
        if s.kind == "C1":
            if tile_src.popcount != N_T:
                invalid("content available where the tile is not", f"{stage} §2a.7")
            continue
        st.bits_work(2 * len(s.data), stage)
        if s.popcount and s.as_int() & ~tile_src.as_int():
            invalid("content available where the tile is not", f"{stage} §2a.7")
    # §2a step 8: child => tile.
    if child_src.kind != "C0" and tile_src.kind != "C1":
        last = level_offset(n, L - 1)
        cnt = n ** (L - 1)
        parents = tile_src.range_int(last, cnt)
        if child_src.kind == "C1":
            if parents != (1 << cnt) - 1:
                invalid("child subtree available under an unavailable tile", f"{stage} §2a.8")
        elif child_src.popcount:
            st.bits_work(len(child_src.data) + (cnt + 7) // 8 * (n + 1), stage)
            if child_src.popcount <= 200000:
                for m in child_src.setbits():
                    if not (parents >> (m // n)) & 1:
                        invalid("child subtree available under an unavailable tile", f"{stage} §2a.8")
            elif child_src.as_int() & ~spread(parents, cnt, n):
                invalid("child subtree available under an unavailable tile", f"{stage} §2a.8")
    # §2a step 9: visit reservation.
    P_t = tile_src.popcount
    st.visit(P_t, f"{stage} §2a.9")
    # §2a step 10: content-index reservation.
    E = sum(s.popcount for s in content_src)
    if E > st.lim["references"] - st.refs:
        resource("references", "content index reservation exceeds references", f"{stage} §2a.10")
    st.refs += E
    # §2a step 11: rank table (nothing to model).
    # §2a step 12 / §5: property tables.
    tables = js.get("propertyTables", [])
    doc = root.doc
    for t in tables:
        if not isinstance(t, dict):
            invalid("propertyTable is not an object", stage)
        cls = t.get("class")
        classes = doc["schema"].get("classes") if isinstance(doc["schema"], dict) else None
        if not isinstance(cls, str) or not isinstance(classes, dict) or cls not in classes:
            invalid(f"property table class {cls!r} does not exist", f"{stage} §5")
        if not is_uint(t.get("count")) or t["count"] < 1:
            invalid("property table count", f"{stage} §5")
        props = t.get("properties", {})
        if not isinstance(props, dict) or ("properties" in t and not props):
            invalid("property table properties", f"{stage} §5")
        cprops = classes[cls].get("properties", {}) if isinstance(classes[cls], dict) else {}
        for pid, col in props.items():
            if pid not in cprops:
                invalid(f"table property {pid} not in class", f"{stage} §5")
            if not isinstance(col, dict) or not is_uint(col.get("values")) or col["values"] >= len(views):
                invalid("table property values view", f"{stage} §5")
            for f in ("arrayOffsets", "stringOffsets"):
                if f in col and (not is_uint(col[f]) or col[f] >= len(views)):
                    invalid(f"table property {f} view", f"{stage} §5")
    sub.box_col = sub.err_col = None
    if "tileMetadata" in js:
        ti = js["tileMetadata"]
        if not is_uint(ti) or ti >= len(tables):
            invalid("tileMetadata index", f"{stage} §5")
        t = tables[ti]
        rec = class_record(st, doc["key"], doc["schema"], t["class"], f"{stage} §5")
        desc = rec.check_tile_use(f"{stage} §5")
        if t["count"] != P_t:
            invalid("tileMetadata count differs from the tile popcount", f"{stage} §5")
        for sem, (pid, size, required) in desc.items():
            col = t.get("properties", {}).get(pid)
            if col is None:
                if required:
                    invalid(f"required {sem} property omitted from the table", f"{stage} §5")
                continue
            if any(m in col for m in ("offset", "scale")):
                unsupported(f"offset or scale on {sem}", f"{stage} §5")
            per = 12 if sem == "TILE_BOUNDING_BOX" else 1
            bi, off, ln = views[col["values"]]
            if ln != P_t * per * size:
                invalid(f"{sem} values byteLength", f"{stage} §5")
            raw = bytes(buffers[bi][off:off + ln])
            fmt = "<%d%s" % (P_t * per, "f" if size == 4 else "d")
            st.bits_work(ln, stage)
            vals = struct.unpack(fmt, raw)
            if not all(math.isfinite(v) for v in vals):
                invalid(f"non-finite {sem} row", f"{stage} §5")
            if sem == "TILE_GEOMETRIC_ERROR":
                if any(v < 0 for v in vals):
                    invalid("negative TILE_GEOMETRIC_ERROR row", f"{stage} §5")
                sub.err_col = vals
            else:
                sub.box_col = vals
    if "contentMetadata" in js:
        cm = js["contentMetadata"]
        if not K or len(cm) != K:
            invalid("contentMetadata count differs from the template count", f"{stage} §5")
        for k, ti in enumerate(cm):
            if not is_uint(ti) or ti >= len(tables):
                invalid("contentMetadata index", f"{stage} §5")
            t = tables[ti]
            rec = class_record(st, doc["key"], doc["schema"], t["class"], f"{stage} §5")
            rec.check_content_use(f"{stage} §5")
            if t["count"] != content_src[k].popcount:
                invalid("contentMetadata count differs from the slot popcount", f"{stage} §5")
    sub.tile, sub.content, sub.child = tile_src, content_src, child_src
    sub.P_t, sub.E = P_t, E
    sub.emitted = 0
    sub.visited = 0
    st.subtrees += 1
    return sub


def _parent_rule(st, s, n, L, stage):
    st.bits_work(len(s.data) * (2 + n), stage)
    if s.popcount <= 200000:
        for i in s.setbits():
            if i == 0:
                continue
            l = 1
            while level_offset(n, l + 1) <= i:
                l += 1
            m = i - level_offset(n, l)
            if not s.bit(level_offset(n, l - 1) + m // n):
                invalid("available tile with an unavailable parent", f"{stage} §2a.5")
        return
    for l in range(1, L):
        cnt = n ** l
        kids = s.range_int(level_offset(n, l), cnt)
        if not kids:
            continue
        parents = s.range_int(level_offset(n, l - 1), cnt // n)
        if kids & ~spread(parents, cnt // n, n):
            invalid("available tile with an unavailable parent", f"{stage} §2a.5")


# --------------------------------------------------------------------------------------------------------------
# Walk (contract §2a "Walk", §3 traversal, §4)

def walk_root(st, root, exact):
    """Depth-first pre-order: tile, its emitted contents in slot order, then available children ascending."""
    parent = {"level": -1, "values": root.box, "cell": None, "error": root.error}
    sub = admit_subtree(st, root, 0, 0, 0, 0)
    _walk_subtree(st, root, sub, parent, exact)
    if root.templates and root.emitted == 0:
        invalid("content templates present but no tile has content", "post-walk (:299)")


def volume_exact(root, t):
    if t["cell"] is not None:
        level, coords = t["cell"]
        return derived_cell(root.box, level, coords, root.octree)
    return exact_box(t["values"])


def _walk_subtree(st, root, sub, parent, exact):
    n, L = root.n, root.L
    g = sub.level
    gx, gy, gz = sub.coords
    groups = {}
    for k, s in enumerate(sub.content):
        if s.kind == "C0" or s.popcount == 0:
            continue
        groups.setdefault(id(s), (s, []))[1].append(k)
    groups = list(groups.values())

    def visit(ll, lx, ly, lz, morton, par):
        idx = level_offset(n, ll) + morton
        level = g + ll
        x, y, z = (gx << ll) | lx, (gy << ll) | ly, (gz << ll) | lz
        stage = f"tile {level}/{x}/{y}/{z}"
        st.depth(root.depth + level, stage)
        sub.visited += 1
        st.implicit_tiles += 1
        if st.implicit_tiles > ORACLE_BUDGET["tiles"]:
            undecided("oracle tile budget exhausted", stage)
        if sub.visited > sub.P_t:
            raise Outcome("InvalidState", "visit beyond reservation", stage)
        rank = None
        if sub.box_col is not None or sub.err_col is not None:
            rank = sub.tile.rank(idx)
            if rank >= sub.P_t:
                raise Outcome("InvalidState", "rank beyond count", stage)
        coords = (x, y, z)
        if sub.box_col is not None:
            vals = tuple(sub.box_col[rank * 12:rank * 12 + 12])
            vol = "row:" + ",".join(f64hex(v) for v in vals)
            cell = None
        else:
            vals = None
            vol = "derived-root" if level == 0 else (
                "derived:" + ",".join(f64hex(v) for v in root.box) + f";{level};{x},{y},{z}")
            cell = (level, coords)
        if sub.err_col is not None:
            err = sub.err_col[rank]
        else:
            exact_err = Fraction(root.error) / (1 << level)
            if level >= 1 and exact_err != 0 and exact_err < Fraction(sys.float_info.min):
                unsupported("derived geometric error would be a nonzero subnormal", stage)
            err = float(exact_err) if level >= 1 else root.error
        me = {"level": level, "values": vals, "cell": cell, "error": err}
        # hierarchyBounds and geometricErrorOrder against the path parent.
        if not (cell is not None and par["cell"] is not None):
            # A derived cell inside a derived parent cell of the same root holds exactly (self-test identity).
            if not contains(volume_exact(root, par), volume_exact(root, me), stage):
                invalid("hierarchyBounds: tile volume outside its parent", stage)
        if not error_ok(err, par["error"]):
            invalid("geometricErrorOrder: tile error exceeds its parent", stage)
        contents = []
        for s, slots in groups:
            if s.bit(idx):
                for k in slots:
                    contents.append(k)
        contents.sort()
        out = []
        for k in contents:
            member = root.templates[k].substitute(level, x, y, z)
            st.reference_reserved(member, stage)
            out.append([k, member])
        sub.emitted += len(out)
        root.emitted += len(out)
        st.emitted += len(out)
        if st.emitted > ORACLE_BUDGET["emissions"]:
            undecided("oracle emission budget exhausted", stage)
        rec = {"doc": root.doc["name"], "level": level, "x": x, "y": y, "z": z, "depth": root.depth + level,
               "parent": par["level"], "volume": vol, "error": f64hex(err), "refine": root.refine,
               "contents": out}
        if exact and cell is not None:
            c, h = volume_exact(root, me)
            rec["exact"] = {"centre": [str(v) for v in c], "halfAxes": [[str(v) for v in r] for r in h]}
        st.tiles.append(rec)
        if ll < L - 1:
            for c in range(n):
                cx, cy, cz = child_coords(lx, ly, lz, c)
                cm = morton * n + c
                if sub.tile.bit(level_offset(n, ll + 1) + cm):
                    visit(ll + 1, cx, cy, cz, cm, me)
        else:
            for c in range(n):
                cm = morton * n + c
                if sub.child.bit(cm):
                    cx, cy, cz = child_coords(lx, ly, lz, c)
                    child = admit_subtree(st, root, g + L, (gx << L) | cx, (gy << L) | cy, (gz << L) | cz)
                    _walk_subtree(st, root, child, me, exact)

    visit(0, 0, 0, 0, 0, parent)
    if sub.emitted != sub.E or sub.visited != sub.P_t:
        raise Outcome("InvalidState", "emitted or visited differs from the reservation", f"subtree {sub.name}")
    st.live -= sub.held


# --------------------------------------------------------------------------------------------------------------
# Explicit skeleton around implicit roots

def load_schema(st, ts, name, stage):
    if "schema" in ts:
        return ts["schema"]
    if "schemaUri" in ts:
        member = resolve_uri(base_dir(name), ts["schemaUri"], stage)
        st.reference(member, stage)
        return parse_json(st.read_member(member, stage), st.lim, stage)
    return None


def run_tileset(st, name, parent, depth, refine_in, active, exact):
    stage = f"tileset {name}"
    st.visit(1, stage)
    st.tilesets += 1
    ts = parse_json(st.read_member(name, stage), st.lim, stage)
    if not isinstance(ts, dict) or not isinstance(ts.get("root"), dict) or not isinstance(ts.get("asset"), dict):
        invalid("tileset JSON shape", stage)
    if not is_num(ts.get("geometricError")) or ts["geometricError"] < 0:
        invalid("tileset geometricError", stage)
    doc = {"name": name, "key": name, "schema": load_schema(st, ts, name, stage)}
    active.add(name)
    doc["error"] = float(ts["geometricError"])
    doc["root_tile"] = ts["root"]
    node(st, ts["root"], doc, parent, depth, refine_in, active, exact)
    active.discard(name)
    st.visit(1, f"exit {stage}")


def node(st, tile, doc, parent, depth, refine_in, active, exact):
    stage = f"node in {doc['name']} depth {depth}"
    if not isinstance(tile, dict):
        invalid("tile is not an object", stage)
    st.visit(1, stage)
    st.depth(depth, stage)
    st.explicit_nodes += 1
    if not is_num(tile.get("geometricError")) or tile["geometricError"] < 0:
        invalid("tile geometricError", stage)
    err = float(tile["geometricError"])
    refine = tile.get("refine", refine_in)
    if refine is None:
        undecided("root refine absent", stage)
    if refine not in ("ADD", "REPLACE"):
        invalid("refine is not ADD or REPLACE", stage)
    m = parse_transform(tile.get("transform"), stage)
    bv = tile.get("boundingVolume")
    if not isinstance(bv, dict):
        invalid("tile boundingVolume", stage)
    values = box_values(bv["box"], stage, "tile") if "box" in bv else None
    if parent is not None:
        if values is None or parent["values"] is None:
            undecided("hierarchyBounds involving a non-box explicit volume", stage)
        if not is_identity(m) and abs(max_scale(m) - 1.0) > 1e-12:
            undecided("scaling transform between checked tiles", stage)
        if not contains(exact_box(parent["values"]), exact_box(transform_box(m, values)), stage):
            invalid("hierarchyBounds: tile volume outside its parent", stage)
        if not error_ok(err, parent["error"]):
            invalid("geometricErrorOrder: tile error exceeds its parent", stage)
    if tile is doc.get("root_tile") and not error_ok(err, doc["error"]):
        # Pre-I1 explicit rule (post-freeze amendment 4): a document root's error <= the document's geometricError.
        invalid("geometricErrorOrder: root error exceeds its document geometricError", stage)
    ext = tile.get("extensions")
    if isinstance(ext, dict) and "3DTILES_implicit_tiling" in ext:
        unsupported("3DTILES_implicit_tiling (1.0)", stage)
    if "implicitTiling" in tile:
        root = admit_root(st, tile, doc, depth, refine, f"root admission in {doc['name']}")
        st.roots.append({"doc": doc["name"], "scheme": "OCTREE" if root.octree else "QUADTREE",
                         "subtreeLevels": root.L, "availableLevels": root.A, "refine": refine,
                         "templates": [t.text for t in root.templates], "subtrees": root.subtree_t.text})
        walk_root(st, root, exact)
        return
    me = {"values": values, "error": err}
    if "content" in tile and "contents" in tile:
        invalid("tile has both content and contents", stage)
    contents = [tile["content"]] if "content" in tile else tile.get("contents", [])
    for c in contents:
        if not isinstance(c, dict):
            invalid("content is not an object", stage)
        member = resolve_uri(base_dir(doc["name"]), c.get("uri"), stage)
        st.reference(member, stage)
        if member.endswith(".json"):
            if member in active:
                invalid("external tileset cycle", stage)
            sub_parent = {"values": values, "error": err}
            st.external_links += 1
            run_tileset(st, member, sub_parent, depth + 1, refine, active, exact)
        else:
            st.explicit_contents += 1
    children = tile.get("children", [])
    if not isinstance(children, list):
        invalid("children is not an array", stage)
    for ch in children:
        node(st, ch, doc, me, depth + 1, refine, active, exact)


# --------------------------------------------------------------------------------------------------------------
# Entry point

def tiles_digest(tiles):
    blob = json.dumps(tiles, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode("ascii")
    return hashlib.sha256(blob).hexdigest()


def evaluate(path, limits=None, exact=False):
    lim = dict(DEFAULT_LIMITS)
    if limits:
        unknown = set(limits) - set(lim)
        if unknown:
            raise SystemExit(f"unknown limits {sorted(unknown)}")
        lim.update(limits)
    archive = Archive(path)
    st = State(archive, lim)
    verdict = {"kind": "Accepted", "reason": None, "stage": None, "limit": None}
    try:
        if "tileset.json" not in archive.sizes:
            invalid("archive has no tileset.json", "archive")
        run_tileset(st, "tileset.json", None, 0, None, set(), exact)
        if not st.roots:
            undecided("archive has no implicit root (explicit-only archives are outside I1)", "archive")
    except Outcome as o:
        verdict = {"kind": o.kind, "reason": o.reason, "stage": o.stage, "limit": o.limit}
    if st.tbr_margin_open and verdict["kind"] != "Undecided" and verdict.get("limit") != "total_bytes_read":
        verdict = {"kind": "Undecided", "reason": "total_bytes_read within the unmodelled-read margin "
                   "(hash/CRC and payload reads are not modelled)", "stage": verdict["stage"], "limit": None,
                   "shadowed": verdict}
    accepted = verdict["kind"] == "Accepted"
    roots = len(st.roots)
    record = {
        "oracle": ORACLE_ID,
        "contract": CONTRACT_SHA256,
        "rulings": RULINGS_SHA256,
        "archive": path.rsplit("/", 1)[-1],
        "archiveSha256": archive.sha256,
        "limits": lim,
        "verdict": verdict,
        "counts": {
            "visits": st.visits,
            "referencesCharged": st.refs,
            "bytesReadModelled": st.bytes_read,
            "scanBytes": st.scan_bytes,
            "subtrees": st.subtrees,
            "implicitTiles": st.implicit_tiles,
            "emittedContents": st.emitted,
            "explicitNodes": st.explicit_nodes,
            "explicitPayloadContents": st.explicit_contents,
            "tilesets": st.tilesets,
            "roots": roots,
            "tiles": st.explicit_nodes + st.implicit_tiles - roots,
            "externalTilesetLinks": st.external_links,
            # Every content entry, external-tileset links included (production's counter; amendment 3).
            "contentReferences": st.emitted + st.explicit_contents + st.external_links,
            "maxLivePathBytes": st.max_live,
            "cumulativeSubtreeBytes": st.cumulative_subtree_bytes,
            "maxDepth": st.max_depth,
            "classScans": st.class_scans,
        },
        "roots": st.roots,
    }
    if accepted:
        record["tiles"] = st.tiles
        record["tilesSha256"] = tiles_digest([{k: v for k, v in t.items() if k != "exact"} for t in st.tiles])
        record["unreferencedMembers"] = sorted(set(archive.sizes) - st.used - {"tileset.json"})
    return record


def main(argv):
    limits, exact, paths, digest_only = {}, False, [], False
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "--limit":
            k, v = argv[i + 1].split("=", 1)
            limits[k] = int(v)
            i += 2
            continue
        if a == "--exact":
            exact = True
        elif a == "--summary":
            digest_only = True
        else:
            paths.append(a)
        i += 1
    if not paths:
        print(__doc__, file=sys.stderr)
        return 2
    for p in paths:
        rec = evaluate(p, limits, exact)
        if digest_only:
            rec.pop("tiles", None)
            rec.pop("limits", None)
        print(json.dumps(rec, sort_keys=True, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
