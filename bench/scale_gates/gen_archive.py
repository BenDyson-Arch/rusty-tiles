#!/usr/bin/env python3
"""Synthetic 3TZ archives for the (d) hierarchy and delivery validator gates. No tiler needed.

  implicit  QUADTREE implicit tileset with full constant availability: availableLevels A,
            subtreeLevels L (A % L == 0). Tiles = (4^A - 1) / 3; the default A=12, L=6 gives
            5,592,405 tiles (>= 2^22) in 1 + 4^6 = 4,097 subtree members, a few KiB each.
            Content (one small in-box triangle GLB per tile) on levels < --content-levels.
  large     explicit tileset whose child contents are GLBs of --member-mib each (default 120 MiB,
            under the validator's 128 MiB per-member limit), totalling over --target-gib (default
            4.5 GiB). Each GLB is a valid triangle whose BIN chunk is padded with unreferenced
            bytes. With the default --fill zeros the padding is written as file holes, so the
            archive costs almost no disk; --fill pattern writes deterministic dense bytes.
  combined  the implicit hierarchy and padded sparse GLBs in ONE archive: defaults give
            5,592,405 available tiles and 85 x 56 MiB contents (>4 GiB), 4,097 subtrees.

Archives are stored ZIP with ZIP64 records wherever a field overflows, every member referenced,
and the @3dtilesIndex1@ index (MD5 of the name + u64 local-header offset, sorted by the two
little-endian u64 halves of the MD5) as the last central-directory entry. The manifest records
the expected validator counts.

  python3 -I bench/scale_gates/gen_archive.py implicit --out archives/implicit-2p22
  python3 -I bench/scale_gates/gen_archive.py large --out archives/large-4p5g
"""
import argparse, hashlib, json, math, pathlib, shutil, struct, sys, time, zlib

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import numpy as np  # noqa: E402
import scalecommon as sc  # noqa: E402

INDEX = '@3dtilesIndex1@'
U32 = 0xFFFFFFFF
ZERO_BLOCK = bytes(16 * sc.MiB)


class ZipStream:
    """Stored-only ZIP writer that streams member bytes and keeps only directory rows in memory.
    A member is header bytes plus an optional tail of `pad` filler bytes (holes when sparse)."""

    def __init__(self, path):
        self.path = pathlib.Path(path)
        self.f = open(self.path, 'xb')
        self.rows = []

    def add(self, name, data, pad=0, fill='zeros', fill_seed=0):
        """Write one member; the CRC is patched into the local header after streaming the tail."""
        nb = name.encode('ascii')
        at = self.f.tell()
        size = len(data) + pad
        if size >= U32:
            raise ValueError('members of 4 GiB or more are outside this writer')
        self.f.write(struct.pack('<4s5H3I2H', b'PK\x03\x04', 20, 0, 0, 0, 0x21, 0, size, size, len(nb), 0))
        self.f.write(nb)
        self.f.write(data)
        crc = zlib.crc32(data)
        left = pad
        if fill == 'zeros':
            while left:
                n = min(left, len(ZERO_BLOCK))
                crc = zlib.crc32(memoryview(ZERO_BLOCK)[:n], crc); left -= n
            if pad:
                self.f.seek(pad, 1)  # a hole; later writes (or finish's truncate) extend the file
        else:
            stream = sc.Stream(fill_seed, len(self.rows))
            while left:
                n = min(left, 8 * sc.MiB)
                block = stream.raw(-(-n // 8)).astype('<u8').tobytes()[:n]  # byte order fixed
                crc = zlib.crc32(block, crc); self.f.write(block); left -= n
        crc &= U32
        end = self.f.tell()
        self.f.seek(at + 14); self.f.write(struct.pack('<I', crc)); self.f.seek(end)
        self.rows.append((nb, at, size, crc))
        return at

    def finish(self):
        index = sorted(((hashlib.md5(nb).digest(), at) for nb, at, _, _ in self.rows),
                       key=lambda r: struct.unpack('<QQ', r[0]))
        payload = b''.join(h + struct.pack('<Q', at) for h, at in index)
        self.add(INDEX, payload)
        cd_at = self.f.tell()
        for nb, at, size, crc in self.rows:
            extra = b''
            offset = at
            if at >= U32:
                extra = struct.pack('<HHQ', 1, 8, at); offset = U32
            version = 45 if extra else 20
            self.f.write(struct.pack('<4s6H3I5H2I', b'PK\x01\x02', version, version, 0, 0, 0, 0x21, crc, size, size,
                                     len(nb), len(extra), 0, 0, 0, 0, offset))
            self.f.write(nb); self.f.write(extra)
        cd_size = self.f.tell() - cd_at
        n = len(self.rows)
        zip64 = n >= 0xFFFF or cd_size >= U32 or cd_at >= U32
        if zip64:
            z_at = self.f.tell()
            self.f.write(struct.pack('<4sQ2H2I4Q', b'PK\x06\x06', 44, 45, 45, 0, 0, n, n, cd_size, cd_at))
            self.f.write(struct.pack('<4sIQI', b'PK\x06\x07', 0, z_at, 1))
        self.f.write(struct.pack('<4s4H2IH', b'PK\x05\x06', 0, 0, min(n, 0xFFFF), min(n, 0xFFFF),
                                 min(cd_size, U32), min(cd_at, U32), 0))
        self.f.close()
        return dict(members=n, central_directory_bytes=cd_size, zip64=zip64)


def glb(positions, pad_to=None):
    """Triangle GLB (glTF Y-up). With pad_to the BIN chunk is padded to make the GLB exactly that
    long; returns (head bytes, pad bytes) so the pad can be streamed or left as a hole."""
    pos = np.asarray(positions, dtype='<f4')
    data = pos.tobytes()
    doc = dict(asset=dict(version='2.0', generator='rusty-tiles scale_gates gen_archive'), scene=0,
               scenes=[dict(nodes=[0])], nodes=[dict(mesh=0)], meshes=[dict(primitives=[dict(attributes=dict(POSITION=0),
                                                                                         mode=4)])],
               accessors=[dict(bufferView=0, componentType=5126, count=3, type='VEC3',
                               min=[float(v) for v in pos.min(axis=0)], max=[float(v) for v in pos.max(axis=0)])],
               bufferViews=[dict(buffer=0, byteOffset=0, byteLength=len(data))], buffers=[dict(byteLength=0)])
    def build(bin_len):
        doc['buffers'][0]['byteLength'] = bin_len
        text = json.dumps(doc, separators=(',', ':')).encode()
        return text + b' ' * (-len(text) % 4)
    bin_len = len(data)
    text = build(bin_len)
    pad = 0
    if pad_to:
        for _ in range(3):  # byteLength digits may change the JSON length
            bin_len = pad_to - 12 - 8 - len(text) - 8
            text = build(bin_len)
        assert 12 + 8 + len(text) + 8 + bin_len == pad_to and bin_len % 4 == 0, (pad_to, bin_len)
        pad = bin_len - len(data)
    total = 12 + 8 + len(text) + 8 + bin_len
    head = (struct.pack('<4sII', b'glTF', 2, total) + struct.pack('<I4s', len(text), b'JSON') + text
            + struct.pack('<I4s', bin_len, b'BIN\0') + data)
    return head, pad


def enu_transform(lon, lat, height):
    """Column-major ENU->ECEF at a WGS84 point. It uses libm, so values are rounded (1e-12 rotation,
    0.1 mm translation) to keep tileset.json bytes stable across platforms."""
    a, f = 6378137.0, 1 / 298.257223563
    e2 = f * (2 - f)
    lo, la = math.radians(lon), math.radians(lat)
    n = a / math.sqrt(1 - e2 * math.sin(la) ** 2)
    x = (n + height) * math.cos(la) * math.cos(lo)
    y = (n + height) * math.cos(la) * math.sin(lo)
    z = (n * (1 - e2) + height) * math.sin(la)
    east = [-math.sin(lo), math.cos(lo), 0.0]
    north = [-math.sin(la) * math.cos(lo), -math.sin(la) * math.sin(lo), math.cos(la)]
    up = [math.cos(la) * math.cos(lo), math.cos(la) * math.sin(lo), math.sin(la)]
    rot = [round(v, 12) + 0.0 for v in east + [0.0] + north + [0.0] + up + [0.0]]
    return rot + [round(v, 4) for v in (x, y, z)] + [1.0]


def tile_triangle(half, hz, level, x, y):
    """In-box triangle (glTF Y-up) for the tile at (level, x, y) of a box with half extent `half`."""
    size = 2.0 * half / (1 << level)
    cx = -half + (x + 0.5) * size
    cy = -half + (y + 0.5) * size
    r = 0.3 * size
    # tiles z-up (X, Y, Z) <- glTF (x, -z, y)
    return [[cx - r, 0.0, -(cy - r)], [cx + r, 0.0, -(cy - r)], [cx, min(hz * 0.5, r), -(cy + r)]]


def morton2(x, y):
    out = 0
    for b in range(32):
        out |= ((x >> b) & 1) << (2 * b) | ((y >> b) & 1) << (2 * b + 1)
    return out


def subtree_bytes(doc, binary=b''):
    text = json.dumps(doc, separators=(',', ':')).encode()
    text += b' ' * (-len(text) % 8)
    binary += b'\0' * (-len(binary) % 8)
    return struct.pack('<4sIQQ', b'subt', 1, len(text), len(binary)) + text + binary


def prepare(out, force):
    out = sc.prepare_output(out, force)
    out.mkdir(parents=True, exist_ok=True)
    return out


def implicit(args):
    A, L, D = args.levels, args.subtree_levels, args.content_levels
    combined = args.kind == 'combined'
    if not (1 <= L <= A and 0 <= D <= L) or (combined and D == 0):
        raise SystemExit('require positive levels/subtree-levels and 0 <= content-levels <= subtree-levels; combined needs content')
    member = args.member_mib * sc.MiB if combined else None
    if combined and not (1 <= args.member_mib <= 120):
        raise SystemExit('--member-mib must be in [1,120]')
    if A % L:
        raise SystemExit('--levels must be a multiple of --subtree-levels (constant availability per subtree)')
    if D > L:
        raise SystemExit('--content-levels must not exceed --subtree-levels (content lives in the root subtree)')
    tiles = ((1 << (2 * A)) - 1) // 3
    sub_tiles = ((1 << (2 * L)) - 1) // 3
    contents = ((1 << (2 * D)) - 1) // 3
    subtrees = sum(1 << (2 * k * L) for k in range(A // L))
    out = prepare(args.out, args.force)
    disk = sc.disk_preflight(out, subtrees * 512 + contents * 2048 + (64 * sc.MiB if combined else sc.MiB), args.reserve_gib * sc.GiB)
    started = time.monotonic()
    half, hz = args.half_size, 50.0
    root = dict(boundingVolume=dict(box=[0, 0, 0, half, 0, 0, 0, half, 0, 0, 0, hz]), geometricError=args.root_error,
                refine='ADD', implicitTiling=dict(subdivisionScheme='QUADTREE', subtreeLevels=L, availableLevels=A,
                                                  subtrees=dict(uri='subtrees/{level}/{x}/{y}.subtree')))
    if D:
        root['content'] = dict(uri='content/{level}/{x}/{y}.glb')
    if args.anchor:
        root['transform'] = enu_transform(*args.anchor)
    tileset = dict(asset=dict(version='1.1'), geometricError=2 * args.root_error, root=root)
    z = ZipStream(out / 'archive.3tz')
    z.add('tileset.json', json.dumps(tileset, separators=(',', ':')).encode())
    # root subtree: all tiles, children everywhere below unless the root subtree is the last layer
    bits = bytearray(-(-sub_tiles // 8))
    for i in range(contents):
        bits[i // 8] |= 1 << (i % 8)
    last_layer = A // L == 1
    root_doc = dict(tileAvailability=dict(constant=1), childSubtreeAvailability=dict(constant=0 if last_layer else 1))
    binary = b''
    if D:
        binary = bytes(bits)
        root_doc.update(buffers=[dict(byteLength=len(binary))],
                        bufferViews=[dict(buffer=0, byteOffset=0, byteLength=len(binary))],
                        contentAvailability=[dict(bitstream=0, availableCount=contents)])
    z.add('subtrees/0/0/0.subtree', subtree_bytes(root_doc, binary))
    inner = dict(tileAvailability=dict(constant=1), childSubtreeAvailability=dict(constant=1))
    leaf = dict(tileAvailability=dict(constant=1), childSubtreeAvailability=dict(constant=0))
    if D:
        inner['contentAvailability'] = [dict(constant=0)]
        leaf['contentAvailability'] = [dict(constant=0)]
    inner_b, leaf_b = subtree_bytes(inner), subtree_bytes(leaf)
    for k in range(1, A // L):
        level = k * L
        body = leaf_b if k == A // L - 1 else inner_b
        side = 1 << level
        for y in range(side):
            for x in range(side):
                z.add(f'subtrees/{level}/{x}/{y}.subtree', body)
    for level in range(D):
        side = 1 << level
        for y in range(side):
            for x in range(side):
                head, pad = glb(tile_triangle(half, hz, level, x, y), pad_to=member)
                z.add(f'content/{level}/{x}/{y}.glb', head, pad=pad)
    info = z.finish()
    path = out / 'archive.3tz'
    params = dict(levels=A, subtree_levels=L, content_levels=D, half_size_m=half, root_error=args.root_error,
                  anchor=args.anchor, subdivision='QUADTREE')
    if combined:
        params.update(member_mib=args.member_mib, fill='zeros')
    truth = dict(input_file='archive.3tz', tiles=tiles, tiles_at_least_2p22=tiles >= 1 << 22,
                 subtrees=subtrees, subtree_tiles=sub_tiles, contents=contents, archive_bytes=path.stat().st_size,
                 members=info['members'], zip64=info['zip64'],
                 expected_validator=dict(tiles=tiles, contentReferences=contents, entries=info['members'],
                                         min_hierarchy_visits=tiles + 3))
    if combined:
        truth.update(allocated_bytes=path.stat().st_blocks * 512,
                     over_4gib=path.stat().st_size > 4 * sc.GiB,
                     combined_scale_met=tiles >= 1 << 22 and path.stat().st_size > 4 * sc.GiB,
                     member_bytes=member)
        truth['expected_validator']['min_source_archive_bytes'] = path.stat().st_size
    manifest = sc.write_manifest(out, 'archive-combined' if combined else 'archive-implicit', params, ['archive.3tz'], truth,
                                 dict(disk_preflight=disk, generation_seconds=round(time.monotonic() - started, 3)))
    print(json.dumps(dict(out=str(path), tiles=tiles, subtrees=subtrees, members=info['members'],
                          bytes=manifest['total_bytes'])))
    return manifest


def large(args):
    member = args.member_mib * sc.MiB
    count = max(1, -(-int(args.target_gib * sc.GiB) // member))
    total_estimate = count * member
    out = prepare(args.out, args.force)
    dense = args.fill != 'zeros'
    disk = sc.disk_preflight(out, total_estimate if dense else 64 * sc.MiB, args.reserve_gib * sc.GiB)
    started = time.monotonic()
    side = math.isqrt(count - 1) + 1
    half, hz = args.half_size, 50.0
    child_half = half / side
    children = []
    for i in range(count):
        cx, cy = -half + (i % side + 0.5) * 2 * child_half, -half + (i // side + 0.5) * 2 * child_half
        children.append(dict(boundingVolume=dict(box=[cx, cy, 0, child_half, 0, 0, 0, child_half, 0, 0, 0, hz]),
                             geometricError=0.0, content=dict(uri=f'content/{i}.glb')))
    root = dict(boundingVolume=dict(box=[0, 0, 0, half, 0, 0, 0, half, 0, 0, 0, hz]), geometricError=half,
                refine='ADD', children=children)
    if args.anchor:
        root['transform'] = enu_transform(*args.anchor)
    tileset = dict(asset=dict(version='1.1'), geometricError=2 * half, root=root)
    z = ZipStream(out / 'archive.3tz')
    z.add('tileset.json', json.dumps(tileset, separators=(',', ':')).encode())
    for i in range(count):
        cx, cy = -half + (i % side + 0.5) * 2 * child_half, -half + (i // side + 0.5) * 2 * child_half
        r = 0.3 * child_half
        head, pad = glb([[cx - r, 0, -(cy - r)], [cx + r, 0, -(cy - r)], [cx, r * 0.1, -(cy + r)]], pad_to=member)
        z.add(f'content/{i}.glb', head, pad=pad, fill=args.fill, fill_seed=args.seed)
    info = z.finish()
    path = out / 'archive.3tz'
    st = path.stat()
    params = dict(target_gib=args.target_gib, member_mib=args.member_mib, fill=args.fill, seed=args.seed,
                  half_size_m=half, anchor=args.anchor)
    truth = dict(input_file='archive.3tz', archive_bytes=st.st_size, allocated_bytes=st.st_blocks * 512,
                 over_4gib=st.st_size > 4 * sc.GiB, contents=count, member_bytes=member, members=info['members'],
                 zip64=info['zip64'], tiles=count + 1,
                 expected_validator=dict(tiles=count + 1, contentReferences=count, entries=info['members'],
                                         min_source_archive_bytes=st.st_size))
    # sha256 of a sparse 4.5 GiB file reads zeros from holes: cheap, but skip with --no-hash
    files = ['archive.3tz'] if not args.no_hash else []
    manifest = sc.write_manifest(out, 'archive-large', params, files, truth,
                                 dict(disk_preflight=disk, generation_seconds=round(time.monotonic() - started, 3)))
    print(json.dumps(dict(out=str(path), bytes=st.st_size, allocated=st.st_blocks * 512, contents=count,
                          members=info['members'], zip64=info['zip64'])))
    return manifest


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest='kind', required=True)
    for name in ('implicit', 'large', 'combined'):
        s = sub.add_parser(name)
        s.add_argument('--out', required=True, help='directory (relative: under ~/.cache/rusty-tiles-scale)')
        s.add_argument('--force', action='store_true')
        s.add_argument('--reserve-gib', type=float, default=20.0)
        s.add_argument('--half-size', type=float, default=4096.0, help='root box half extent, metres')
        s.add_argument('--anchor', type=float, nargs=3, default=[115.86, -31.95, 20.0],
                       metavar=('LON', 'LAT', 'H'), help='ENU root transform; pass --no-anchor for none')
        s.add_argument('--no-anchor', dest='anchor', action='store_const', const=None)
        if name != 'large':
            s.add_argument('--levels', type=int, default=12, help='availableLevels (12 -> 5,592,405 tiles)')
            s.add_argument('--subtree-levels', type=int, default=6)
            s.add_argument('--content-levels', type=int, default=4 if name == 'combined' else 1, help='levels < D carry a triangle GLB')
            s.add_argument('--root-error', type=float, default=4096.0)
            if name == 'combined':
                s.add_argument('--member-mib', type=int, default=56)
        else:
            s.add_argument('--target-gib', type=float, default=4.5)
            s.add_argument('--member-mib', type=int, default=120)
            s.add_argument('--fill', choices=['zeros', 'pattern'], default='zeros',
                           help='zeros: sparse holes (cheap); pattern: dense deterministic bytes')
            s.add_argument('--seed', type=int, default=20261013)
            s.add_argument('--no-hash', action='store_true', help='skip the manifest sha256 of the archive')
    a = p.parse_args(argv)
    return large(a) if a.kind == 'large' else implicit(a)


if __name__ == '__main__':
    main()
