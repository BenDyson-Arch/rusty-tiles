"""Write a valid, sparse, >1 GiB stored 3TZ for the VS-C opt-in cap controls.

Authored from the VS-C amendment and the ZIP/3TZ format only (not from src/, tests/
or bindings/).  Python 3 stdlib only; run as ``python3 -I sparse_archive.py OUT.3tz``.

Each GLB member is a real glTF 2.0 binary (one POINTS primitive with a 3-vertex
POSITION accessor in the first 36 bytes of the BIN chunk); the rest of the BIN chunk
is zeros that are never written (a filesystem hole).
"""
import argparse
import hashlib
import json
import os
import struct
import sys
import zlib

SENT = 0xFFFFFFFF
INDEX_NAME = "@3dtilesIndex1@"
DOS_TIME, DOS_DATE = 0, 0x21
BLOCK = b"\0" * (1 << 20)


def crc_zeros(crc, n):
    while n > 0:
        k = min(n, len(BLOCK))
        crc = zlib.crc32(BLOCK[:k] if k != len(BLOCK) else BLOCK, crc)
        n -= k
    return crc


def glb_parts(total):
    """Return (prefix_bytes, hole_len): the member is prefix + hole_len zero bytes."""
    if total % 4:
        raise SystemExit("--member-bytes must be a multiple of 4")
    if total > 0xFFFFFFFC:
        raise SystemExit("--member-bytes exceeds the glTF 4 GiB length field")
    binlen = total
    for _ in range(10):
        doc = {
            "asset": {"version": "2.0"},
            "scene": 0,
            "scenes": [{"nodes": [0]}],
            "nodes": [{"mesh": 0}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "mode": 0}]}],
            "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                           "min": [0, 0, 0], "max": [0, 0, 0]}],
            "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 36, "target": 34962}],
            "buffers": [{"byteLength": binlen}],
        }
        js = json.dumps(doc, separators=(",", ":")).encode()
        js += b" " * (-len(js) % 4)
        new = total - (12 + 8 + len(js) + 8)
        if new == binlen:
            break
        binlen = new
    else:
        raise SystemExit("glb length did not converge")
    if binlen < 36 or binlen % 4:
        raise SystemExit("--member-bytes too small")
    head = struct.pack("<4sII", b"glTF", 2, total)
    head += struct.pack("<I4s", len(js), b"JSON") + js
    head += struct.pack("<I4s", binlen, b"BIN\0")
    prefix = head + b"\0" * 36
    assert len(prefix) + (binlen - 36) == total
    return prefix, binlen - 36


def tileset_bytes(uris):
    box = [0, 0, 0, 1e6, 0, 0, 0, 1e6, 0, 0, 0, 1e6]
    doc = {
        "asset": {"version": "1.1"},
        "geometricError": 1000.0,
        "root": {
            "boundingVolume": {"box": box},
            "geometricError": 100.0,
            "refine": "ADD",
            "children": [{"boundingVolume": {"box": box}, "geometricError": 1.0,
                          "content": {"uri": u}} for u in uris],
        },
    }
    return json.dumps(doc, separators=(",", ":")).encode()


class Writer:
    def __init__(self, f):
        self.f = f
        self.records = []  # (name, offset, size, crc)

    def local(self, name, size, crc):
        nb = name.encode("ascii")
        z64 = size >= SENT
        extra = struct.pack("<HHQQ", 1, 16, size, size) if z64 else b""
        s = SENT if z64 else size
        hdr = struct.pack("<IHHHHHIIIHH", 0x04034B50, 45 if z64 else 20, 0, 0, DOS_TIME,
                          DOS_DATE, crc, s, s, len(nb), len(extra))
        off = self.f.tell()
        self.f.write(hdr + nb + extra)
        self.records.append((name, off, size, crc))

    def small(self, name, data):
        self.local(name, len(data), zlib.crc32(data))
        self.f.write(data)

    def hole_member(self, name, prefix, hole, crc_cache):
        key = (prefix, hole)
        if key not in crc_cache:
            crc_cache[key] = crc_zeros(zlib.crc32(prefix), hole)
        self.local(name, len(prefix) + hole, crc_cache[key])
        self.f.write(prefix)
        self.f.seek(hole, 1)  # leave a hole; length is fixed by later writes


def build(path, members, member_bytes, force_z64_eocd):
    names = ["tiles/t%d.glb" % i for i in range(members)]
    prefix, hole = glb_parts(member_bytes)
    cache = {}
    with open(path, "wb") as f:
        w = Writer(f)
        w.small("tileset.json", tileset_bytes(names))
        for nm in names:
            w.hole_member(nm, prefix, hole, cache)
        rows = sorted(((hashlib.md5(n.encode("ascii")).digest(), off)
                       for (n, off, _s, _c) in w.records), key=lambda r: r[0])
        index = b"".join(d + struct.pack("<Q", off) for d, off in rows)
        w.small(INDEX_NAME, index)
        cd_off = f.tell()
        cd = bytearray()
        for name, off, size, crc in w.records:
            nb = name.encode("ascii")
            fields = []
            if size >= SENT:
                fields += [size, size]
            if off >= SENT:
                fields.append(off)
            extra = struct.pack("<HH%dQ" % len(fields), 1, 8 * len(fields), *fields) if fields else b""
            z64 = bool(fields)
            cd += struct.pack("<IHHHHHHIIIHHHHHII", 0x02014B50, 45 if z64 else 20,
                              45 if z64 else 20, 0, 0, DOS_TIME, DOS_DATE, crc,
                              SENT if size >= SENT else size, SENT if size >= SENT else size,
                              len(nb), len(extra), 0, 0, 0, 0, SENT if off >= SENT else off)
            cd += nb + extra
        f.write(cd)
        n = len(w.records)
        if force_z64_eocd or n >= 0xFFFF or cd_off >= SENT or len(cd) >= SENT:
            z_off = f.tell()
            f.write(struct.pack("<IQHHIIQQQQ", 0x06064B50, 44, 45, 45, 0, 0, n, n, len(cd), cd_off))
            f.write(struct.pack("<IIQI", 0x07064B50, 0, z_off, 1))
            f.write(struct.pack("<IHHHHIIH", 0x06054B50, 0, 0, 0xFFFF, 0xFFFF, SENT, SENT, 0))
        else:
            f.write(struct.pack("<IHHHHIIH", 0x06054B50, 0, 0, n, n, len(cd), cd_off, 0))
        f.truncate(f.tell())
        length = f.tell()
    return length, n, sum(r[2] for r in w.records)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("out")
    ap.add_argument("--members", type=int, default=9)
    ap.add_argument("--member-bytes", type=int, default=120 * 1024 * 1024)
    ap.add_argument("--force-zip64-eocd", action="store_true",
                    help="write a ZIP64 end record even when not required")
    a = ap.parse_args(argv)
    if a.members < 1:
        raise SystemExit("--members must be >= 1")
    length, n, s = build(a.out, a.members, a.member_bytes, a.force_zip64_eocd)
    print(json.dumps({"path": os.path.abspath(a.out), "length": length, "N": n, "S": s}))


if __name__ == "__main__":
    sys.exit(main())
