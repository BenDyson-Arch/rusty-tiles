"""Synthesize explicit-tileset stress 3TZ archives at exact entry/visit counts (stdlib only).

Usage: python3 -I stress.py OUT.3tz --entries N --visits M [--per-doc 3000] [--cd-bytes B]
                             [--max-contents 2]

Independent of the product: literal stored ZIP/ZIP64 records and a sorted
raw-MD5 @3dtilesIndex1@ index, as in implicit_rewrite/archive_read_acceptance.

Shape: tileset.json holds a root tile (one GLB) and K link tiles, each naming an
external tileset e<k>.json at the archive root (URIs resolve per document). Each external tileset holds a root tile and a flat
list of child tiles, each with its own GLB. Every tile shares one box and every
GLB is the same in-box triangle, so C1 admits and C2 reports Contained.

Counting (product traversal, src/validate.rs Check::tileset):
  visits  = 3 + 4K + C + E     (Tileset+Exit per document, one per tile node,
                                E extra tiles without content)
  entries = 3 + 2K + C - E + X (index, tileset.json, K externals, 1+K+C-E GLBs,
                                X extra contents on `contents` tiles, at most
                                max-contents - 1 per tile)
  references = entries - 1 + K (every member except the index once; each
                                external tileset twice: as tile content and
                                again when its document is entered)
C is the number of external child tiles. Solving for exact N and M gives
X - E = N - M + 2K. --cd-bytes pads GLB names so the central directory is
exactly B bytes (46 fixed bytes plus the name per entry, no extra fields).
"""
import argparse
import binascii
import hashlib
import json
import struct

INDEX = b'@3dtilesIndex1@'
BOX = [0, 0, 0, 10, 0, 0, 0, 10, 0, 0, 0, 10]


def glb():
    positions = struct.pack('<9f', -1, -1, -1, 1, -1, -1, 0, 1, 1)
    doc = {'asset': {'version': '2.0'}, 'scene': 0, 'scenes': [{'nodes': [0]}], 'nodes': [{'mesh': 0}],
           'meshes': [{'primitives': [{'attributes': {'POSITION': 0}}]}],
           'accessors': [{'bufferView': 0, 'componentType': 5126, 'count': 3, 'type': 'VEC3',
                          'min': [-1, -1, -1], 'max': [1, 1, 1]}],
           'bufferViews': [{'buffer': 0, 'byteOffset': 0, 'byteLength': len(positions)}],
           'buffers': [{'byteLength': len(positions)}]}
    encoded = json.dumps(doc, separators=(',', ':')).encode()
    encoded += b' ' * (-len(encoded) % 4)
    return (struct.pack('<III', 0x46546C67, 2, 28 + len(encoded) + len(positions))
            + struct.pack('<II', len(encoded), 0x4E4F534A) + encoded
            + struct.pack('<II', len(positions), 0x004E4942) + positions)


def tile(contents=(), children=None):
    node = {'boundingVolume': {'box': BOX}, 'geometricError': 0}
    if len(contents) == 1:
        node['content'] = {'uri': contents[0]}
    elif contents:
        node['contents'] = [{'uri': uri} for uri in contents]
    if children:
        node['children'] = children
    return node


def document(root):
    return json.dumps({'asset': {'version': '1.1'}, 'geometricError': 0, 'root': root},
                      separators=(',', ':')).encode()


def plan(entries, visits, per_doc, contents):
    """Smallest K whose children fit per_doc per external document."""
    k = 1
    while True:
        children = visits - 3 - 4 * k  # before balancing extras
        extra = entries - visits + 2 * k  # X - E
        x, e = max(extra, 0), max(-extra, 0)
        c = children
        if c < 0:
            raise SystemExit('counts too small')
        if -(-c // k) <= per_doc and e + -(-x // (contents - 1)) <= c:
            return k, c, x, e
        k += 1


def write(path, members):
    rows = []
    with open(path, 'xb') as f:
        def member(name, data):
            at = f.tell()
            crc = binascii.crc32(data) & 0xffffffff
            f.write(struct.pack('<4s5H3I2H', b'PK\x03\x04', 20, 0, 0, 0, 0, crc, len(data), len(data), len(name), 0))
            f.write(name)
            f.write(data)
            rows.append((name, at, len(data), crc))
        for name, data in members:
            member(name.encode(), data)
        index = sorted(((hashlib.md5(n).digest(), at) for n, at, _, _ in rows),
                       key=lambda x: struct.unpack('<QQ', x[0]))
        member(INDEX, b''.join(h + struct.pack('<Q', at) for h, at in index))
        cd_at = f.tell()
        for name, at, size, crc in rows:
            f.write(struct.pack('<4s6H3I5H2I', b'PK\x01\x02', 20, 20, 0, 0, 0, 0, crc, size, size, len(name),
                                0, 0, 0, 0, 0, at))
            f.write(name)
        cd_size = f.tell() - cd_at
        n = len(rows)
        z_at = f.tell()
        f.write(struct.pack('<4sQ2H2I4Q', b'PK\x06\x06', 44, 45, 45, 0, 0, n, n, cd_size, cd_at))
        f.write(struct.pack('<4sIQI', b'PK\x06\x07', 0, z_at, 1))
        f.write(struct.pack('<4s4H2IH', b'PK\x05\x06', 0, 0, min(n, 0xffff), min(n, 0xffff),
                            0xffffffff, 0xffffffff, 0))
    return n, cd_size


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('out')
    p.add_argument('--entries', type=int, required=True)
    p.add_argument('--visits', type=int, required=True)
    p.add_argument('--per-doc', type=int, default=3000)
    p.add_argument('--cd-bytes', type=int)
    p.add_argument('--max-contents', type=int, default=2, help='contents per child tile (entries > visits)')
    a = p.parse_args()
    k, c, x, e = plan(a.entries, a.visits, a.per_doc, a.max_contents)
    payload = glb()
    glbs = iter(range(1 << 30))
    pads = {}
    name = lambda: (lambda i: f'g/{i}{"x" * pads.get(i, 0)}.glb')(next(glbs))
    if a.cd_bytes:
        # Dry run for the unpadded name lengths, then spread the deficit over GLB names.
        total_glbs = 1 + k + c - e + x
        base = sum(46 + len(f'g/{i}.glb') for i in range(total_glbs))
        base += sum(46 + len(f'e{i}.json') for i in range(k)) + 46 + len('tileset.json') + 46 + len(INDEX)
        deficit = a.cd_bytes - base
        assert deficit >= 0, (a.cd_bytes, base)
        pads = {i: deficit // total_glbs + (i < deficit % total_glbs) for i in range(total_glbs)}
    members, links = [], []
    root_glb = name()
    split = [c // k + (i < c % k) for i in range(k)]
    extras, empties = x, e
    for i, count in enumerate(split):
        children = []
        for _ in range(count):
            if empties:
                children.append(tile())
                empties -= 1
            elif extras:
                more = min(extras, a.max_contents - 1)
                children.append(tile(tuple(name() for _ in range(1 + more))))
                extras -= more
            else:
                children.append(tile((name(),)))
        ext = f'e{i}.json'
        members.append((ext, document(tile((name(),), children))))
        links.append(tile((ext,)))
    assert not extras and not empties
    members.insert(0, ('tileset.json', document(tile((root_glb,), links))))
    glb_count = next(glbs)
    members += [(f'g/{i}{"x" * pads.get(i, 0)}.glb', payload) for i in range(glb_count)]
    n, cd = write(a.out, members)
    tiles = 1 + 2 * k + c
    expect = {'entries': n, 'visits': 3 + 4 * k + c, 'references': n - 1 + k, 'tiles': tiles,
              'contentReferences': k + glb_count, 'externalTilesets': k, 'extraContents': x,
              'emptyTiles': e, 'centralDirectoryBytes': cd, 'maxChildrenPerDocument': max(split)}
    assert n == a.entries and expect['visits'] == a.visits, expect
    assert not a.cd_bytes or cd == a.cd_bytes, (cd, a.cd_bytes)
    print(json.dumps(expect))


if __name__ == '__main__':
    main()
