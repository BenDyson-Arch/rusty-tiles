"""D11 coarse drops (owner decision 2026-10-28), recounted from published bytes.

A coarse payload may lose whole components of its immediate children's payloads and
declares how many (asset.extras.rustyTiles.droppedComponents). A child component
(faces joined by shared f32 keys) is dropped when the parent holds none of its keys.
Counts must equal the declarations. A drop at or above the children's largest G is a
coarse drop: the children's faces must exceed the 8,192 cap, and no border-free child
component smaller than it may stay (smallest first). Every dropped key, and S2's
fixed sample points on every dropped face (the face's child G added off its
vertices), must lie within the parent's geometricError of its surface (exact
distances). Border-touching drops lose shared keys and are refused by the census.
"""
import hashlib
import json
import math
import struct
from fractions import Fraction

# A parent's geometricError covers a dropped key up to this relative rounding.
TOLERANCE = 1e-6
CAP = 8192


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


def samples(t):
    """S2's fixed controls: the quarter grid (corners and edge points included), centroid."""
    for a in range(5):
        for b in range(5 - a):
            w = (a / 4, b / 4, (4 - a - b) / 4)
            yield [w[0] * t[0][k] + w[1] * t[1][k] + w[2] * t[2][k] for k in range(3)]
    yield [(t[0][k] + t[1][k] + t[2][k]) / 3 for k in range(3)]


def diagonal(points):
    return math.sqrt(sum((max(p[k] for p in points) - min(p[k] for p in points)) ** 2 for k in range(3)))


def inspect(store, tiles, archive, require, distance2):
    digests = {t['id']: tuple(sorted(hashlib.sha256(archive.read(u)).digest() for u in t['contents']))
               for t in tiles}
    result = dict(droppedComponents=0, droppedByDepth=[])
    owners = None  # key -> leaf intervals holding it, built on first need

    def holders():
        nonlocal owners
        if owners is None:
            owners = {}
            for t in tiles:
                if t['contents'] and not t['children']:
                    for f in store.faces(int(t['id'])):
                        for v in f['xyz']:
                            owners.setdefault(key(v), set()).add(t['lo'])
        return owners

    def children(t, digest):
        out = []
        for c in t['children']:
            if not c['contents'] or digests[c['id']] == digest:
                # Placeholders and copies down a single-child chain stand for the cell.
                out.extend(children(c, digest))
            else:
                out.append(c)
        return out

    for t in tiles:
        if not t['contents']:
            continue
        want = sum(declared(archive.read(u), require) for u in t['contents'])
        if not t['children']:
            require(want == 0, 'coarseDrops', f'leaf tile {t["id"]} declares dropped components')
            continue
        parent = list(store.faces(int(t['id'])))
        kept = {key(v) for f in parent for v in f['xyz']}
        root = {}

        def find(k):
            while root[k] != k:
                root[k] = root[root[k]]
                k = root[k]
            return k
        child_faces, count_faces, child_g = [], 0, 0.0
        for c in children(t, digests[t['id']]):
            child_g = max(child_g, c['g'])
            for f in store.faces(int(c['id'])):
                ks = [key(v) for v in f['xyz']]
                if len(set(ks)) < 3:
                    continue
                count_faces += 1
                for k in ks:
                    root.setdefault(k, k)
                for k in ks[1:]:
                    a, b = find(ks[0]), find(k)
                    root[max(a, b)] = min(a, b)
                child_faces.append((ks, c['g']))
        components = {}
        for k in root:
            components.setdefault(find(k), []).append(k)
        present = {find(k) for k in root if k in kept}
        dropped = {r: m for r, m in components.items() if r not in present}
        require(len(dropped) == want, 'coarseDrops',
                f'tile {t["id"]} drops {len(dropped)} child components but declares {want}')
        if not dropped:
            continue
        largest = max(diagonal(m) for m in dropped.values())
        if largest >= child_g:
            require(count_faces > CAP, 'coarseDrops',
                    f'tile {t["id"]} drops a component of diagonal {largest} at or above its children\'s '
                    f'G {child_g} though its children\'s {count_faces} faces fit the {CAP} cap')
            for r, members in components.items():
                if r in present and diagonal(members) < largest:
                    touches = any(not t['lo'] <= lo < t['hi'] for k in members for lo in holders().get(k, ()))
                    require(touches, 'coarseDrops',
                            f'tile {t["id"]} keeps a border-free child component of diagonal {diagonal(members)} '
                            f'but drops one of {largest}: drops go smallest first')
        base = t['g'] * (1 + TOLERANCE) + TOLERANCE

        def near(p, slack):
            reach = base + slack
            bound = Fraction(reach) * Fraction(reach)
            return any(
                all(min(v[a] for v in f['xyz']) - reach <= p[a] <= max(v[a] for v in f['xyz']) + reach
                    for a in range(3)) and distance2(p, f['xyz']) <= bound
                for f in parent)
        for members in dropped.values():
            for p in members:
                require(near(p, 0.0), 'coarseDrops',
                        f'tile {t["id"]} drops surface at {p} farther than its geometricError {t["g"]}')
        for ks, g in child_faces:
            if find(ks[0]) in present:
                continue
            for p in samples(ks):
                require(near(p, g), 'coarseDrops',
                        f'tile {t["id"]} drops surface at {p} farther than its geometricError {t["g"]}')
        depth = t['depth']
        result['droppedByDepth'] += [0] * (depth + 1 - len(result['droppedByDepth']))
        result['droppedByDepth'][depth] += len(dropped)
        result['droppedComponents'] += len(dropped)
    return result
