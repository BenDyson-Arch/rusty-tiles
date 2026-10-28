"""D11 coarse drops (owner decision 2026-10-28), recounted from published bytes.

A coarse payload may lose whole components of its immediate children's payloads and
declares how many (asset.extras.rustyTiles.droppedComponents). A child component
(faces joined by shared f32 keys) is dropped when the parent holds none of its keys.
Counts must equal the declarations, and every dropped key must lie within the
parent's geometricError of its surface (exact distances). Border-touching drops lose
shared keys and are refused by the census, never here.
"""
import hashlib
import json
import struct
from fractions import Fraction

# A parent's geometricError covers a dropped key up to this relative rounding.
TOLERANCE = 1e-6


def key(p):
    return tuple(0.0 if v == 0 else v for v in struct.unpack('<3f', struct.pack('<3f', *p)))


def declared(data, require):
    if data[:4] != b'glTF':
        return 0
    length = int.from_bytes(data[12:16], 'little')
    doc = json.loads(data[20:20 + length])
    value = doc.get('asset', {}).get('extras', {}).get('rustyTiles', {}).get('droppedComponents', 0)
    require(type(value) is int and value >= 0, 'coarseDrops', 'droppedComponents is not an unsigned integer')
    return value


def inspect(store, tiles, archive, require, distance2):
    digests = {}
    for t in tiles:
        digests[t['id']] = tuple(sorted(hashlib.sha256(archive.read(u)).digest() for u in t['contents']))
    result = dict(droppedComponents=0, droppedByDepth=[])

    def children(t):
        out = []
        for c in t['children']:
            if not c['contents'] or digests[c['id']] == digests[t['id']]:
                # Placeholders and copies down a single-child chain stand for the cell.
                out.extend(children(c))
            else:
                out.append(c)
        return out

    for t in tiles:
        if not t['contents'] or not t['children']:
            continue
        parent = list(store.faces(int(t['id'])))
        kept = {key(v) for f in parent for v in f['xyz']}
        root = {}

        def find(k):
            while root[k] != k:
                root[k] = root[root[k]]
                k = root[k]
            return k
        for c in children(t):
            for f in store.faces(int(c['id'])):
                ks = [key(v) for v in f['xyz']]
                if len(set(ks)) < 3:
                    continue
                for k in ks:
                    root.setdefault(k, k)
                for k in ks[1:]:
                    a, b = find(ks[0]), find(k)
                    root[max(a, b)] = min(a, b)
        present = {find(k) for k in root if k in kept}
        dropped = {}
        for k in root:
            r = find(k)
            if r not in present:
                dropped.setdefault(r, []).append(k)
        want = sum(declared(archive.read(u), require) for u in t['contents'])
        require(len(dropped) == want, 'coarseDrops',
                f'tile {t["id"]} drops {len(dropped)} child components but declares {want}')
        if not dropped:
            continue
        reach = t['g'] * (1 + TOLERANCE) + TOLERANCE
        bound = Fraction(reach) * Fraction(reach)
        for points in dropped.values():
            for p in points:
                near = any(
                    all(min(v[a] for v in f['xyz']) - reach <= p[a] <= max(v[a] for v in f['xyz']) + reach
                        for a in range(3)) and distance2(p, f['xyz']) <= bound
                    for f in parent)
                require(near, 'coarseDrops',
                        f'tile {t["id"]} drops a component point {p} farther than its geometricError {t["g"]}')
        depth = t['depth']
        result['droppedByDepth'] += [0] * (depth + 1 - len(result['droppedByDepth']))
        result['droppedByDepth'][depth] += len(dropped)
        result['droppedComponents'] += len(dropped)
    return result
