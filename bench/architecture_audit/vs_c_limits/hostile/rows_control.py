"""CI control for the validator's scratch-row classes (2026-10-30), without Node or meshopt.

Every leaf point, segment and face, every ancestor requirement, border image, coincident-face
twin and multi-edge face is one row of the native delivery check's scratch store, keyed by a
decoded position. These archives make those keys pseudo-random, so a row store whose cost grew
with its size would show it (`docs/architecture/validator-limits-amendment.md`, time statement).

Classes (positions: a pseudo-random 128^3 lattice in [1, 2)^3, every corner a new vertex):

- `leafrand`: N leaves of T triangles under a content-less root.
- `twin`: two coarse cells, each over N/2 leaves; the second cell's leaves repeat the first's
  positions with new vertex ids, so every key is shared across the cells (ancestor
  requirements, multi edges, coincident corner sets). Each cell's coarse payload keeps every leaf
  face, so its border is exact: valid, through the border images and their pairwise rules.
- `chain`: a single-child chain of K coarse tiles with one payload over N leaves whose
  triangles are each doubled; the coarse payload doubles COARSE of them, so every coarse edge
  is used twice in one direction (descendant incidence, edge-face and twin lookups per tile).
  Refused by the coarse-drop recount after every coarse tile is checked.
- `revfan` (review 1 of PR #309): a single-child chain of coarse tiles whose every edge is used
  twice forward and also carries M reversed leaf faces. Valid.

`check` runs each class at CI size and asserts its verdict and a structured budget refusal: a
smoke test of the delivery paths, not of row cost (the bulk loads are covered by unit tests in
`src/validate/hlod_delivery/bulk.rs`). On revfan it also measures the units each added chain tile
charges and asserts the 320-unit seeks of the lookups that tile repeats, a timing-free signal
that the former 128-unit seeks and one-unit rows fail.

Buffers are plain (no meshopt), so S is larger per row than in the timing archives
(`leafrand.py`, `rowclasses.py`, upstream meshoptimizer streams); the keys and the work per row
are the same.

usage:
  rows_control.py generate KIND N T SEED OUT.3tz [K] [COARSE]
  rows_control.py revfan OUT.3tz CHAIN BASE M LEAVES [SEG]
  rows_control.py check --binary BIN --work DIR   (CI size)
"""
import argparse
import hashlib
import json
import pathlib
import random
import struct
import subprocess
import sys
import zipfile

SOURCE_PRIMITIVE = [('expanded_ordinal', 'UINT64', 8), ('material_index', 'UINT32', 4), ('mesh_index', 'UINT32', 4),
                    ('node_index', 'UINT32', 4), ('primitive_index', 'UINT32', 4)]
SOURCE_TRIANGLE = [('attribute_flags', 'UINT8', 1), ('degenerate', 'UINT8', 1)] + SOURCE_PRIMITIVE + [
    ('triangle_index', 'UINT64', 8), ('vertex_0', 'UINT64', 8), ('vertex_1', 'UINT64', 8), ('vertex_2', 'UINT64', 8)]


def schema(columns):
    return {'properties': {k: {'componentType': t, 'required': True, 'type': 'SCALAR'} for k, t, _ in columns}}


class Glb:
    def __init__(self):
        self.bin, self.views, self.accessors = bytearray(), [], []

    def view(self, data, target=None):
        self.bin += b'\0' * (-len(self.bin) % 8)
        v = {'buffer': 0, 'byteOffset': len(self.bin), 'byteLength': len(data)}
        if target:
            v['target'] = target
        self.bin += data
        self.views.append(v)
        return len(self.views) - 1

    def accessor(self, data, component, kind, count, target, **extra):
        self.accessors.append(dict(bufferView=self.view(data, target), componentType=component, type=kind,
                                   count=count, **extra))
        return len(self.accessors) - 1

    def finish(self, primitive, tables, classes, serial):
        doc = {'asset': {'version': '2.0', 'generator': f'rows_control {serial}'}, 'scene': 0,
               'scenes': [{'nodes': [0]}], 'nodes': [{'mesh': 0}], 'meshes': [{'primitives': [primitive]}],
               'buffers': [{'byteLength': 0}], 'bufferViews': self.views, 'accessors': self.accessors,
               'extensions': {'EXT_structural_metadata': {
                   'schema': {'id': 'rusty_tiles_rows_control_v1', 'classes': classes}, 'propertyTables': tables}},
               'extensionsUsed': ['EXT_mesh_features', 'EXT_structural_metadata']}
        binary = bytes(self.bin) + b'\0' * (-len(self.bin) % 4)
        doc['buffers'][0]['byteLength'] = len(binary)
        j = json.dumps(doc, separators=(',', ':')).encode()
        j += b' ' * (-len(j) % 4)
        return (struct.pack('<III', 0x46546C67, 2, 28 + len(j) + len(binary)) + struct.pack('<II', len(j), 0x4E4F534A)
                + j + struct.pack('<II', len(binary), 0x004E4942) + binary)


def table(g, cls, count, columns, values):
    props = {}
    for name, _, width in columns:
        fmt = {1: 'B', 4: 'I', 8: 'Q'}[width]
        props[name] = {'values': g.view(struct.pack(f'<{count}{fmt}', *values.get(name, [0] * count)))}
    return {'class': cls, 'count': count, 'properties': props}


def payload(xyz, serial, first_vertex=None):
    """A leaf (with `first_vertex`: a source_triangle table naming new original vertices) or a
    coarse payload (source_primitive only), len(xyz) / 9 triangles of distinct vertices."""
    g = Glb()
    t = len(xyz) // 9
    v = 3 * t
    position = g.accessor(struct.pack(f'<{3 * v}f', *xyz), 5126, 'VEC3', v, 34962,
                          min=[min(xyz[k::3]) for k in range(3)], max=[max(xyz[k::3]) for k in range(3)])
    attributes = {'POSITION': position,
                  '_FEATURE_ID_0': g.accessor(bytes(4 * v), 5126, 'SCALAR', v, 34962)}
    features = [{'featureCount': 1, 'attribute': 0, 'propertyTable': 0, 'label': 'source_primitive'}]
    tables = [table(g, 'source_primitive', 1, SOURCE_PRIMITIVE, {})]
    classes = {'source_primitive': schema(SOURCE_PRIMITIVE)}
    if first_vertex is not None:
        attributes['_FEATURE_ID_1'] = g.accessor(struct.pack(f'<{v}f', *[float(c // 3) for c in range(v)]), 5126,
                                                 'SCALAR', v, 34962)
        features.append({'featureCount': t, 'attribute': 1, 'propertyTable': 1, 'label': 'source_triangle'})
        values = {'triangle_index': list(range(t))}
        for j in range(3):
            values[f'vertex_{j}'] = list(range(first_vertex + j, first_vertex + v, 3))
        tables.append(table(g, 'source_triangle', t, SOURCE_TRIANGLE, values))
        classes['source_triangle'] = schema(SOURCE_TRIANGLE)
    primitive = {'attributes': attributes, 'mode': 4,
                 'indices': g.accessor(struct.pack(f'<{v}I', *range(v)), 5125, 'SCALAR', v, 34963),
                 'extensions': {'EXT_mesh_features': {'featureIds': features}}}
    return g.finish(primitive, tables, classes, serial)


def lattice(rnd, triangles):
    return [1 + rnd.randrange(128) / 128 for _ in range(9 * triangles)]


BOX = {'box': [0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0, 2]}


def archive(kind, n, tri, seed, out, chain=1, coarse=1024):
    members = []

    def tile(name, blob, error=0, children=None):
        members.append((name, blob))
        t = {'boundingVolume': BOX, 'geometricError': error, 'refine': 'REPLACE', 'content': {'uri': name}}
        if children is not None:
            t['children'] = children
        return t

    leaf = lambda i: random.Random(seed * 1_000_003 + i)
    if kind == 'leafrand':
        children = [tile(f'g{i}.glb', payload(lattice(leaf(i), tri), i, i * 3 * tri)) for i in range(n)]
    elif kind == 'twin':
        half = n // 2
        children = []
        for c in range(2):
            leaves = [tile(f'c{c}g{i}.glb', payload(lattice(leaf(i), tri), c * half + i, (c * half + i) * 3 * tri))
                      for i in range(half)]
            # The cell's coarse payload keeps every leaf face, so its border is exact and the
            # validation runs through the border images and their pairwise rules.
            faces = [x for i in range(half) for x in lattice(leaf(i), tri)]
            children.append(tile(f'coarse{c}.glb', payload(faces, 1 << 30 | c), 1, leaves))
    elif kind == 'chain':
        leaves, picked = [], []
        for i in range(n):
            once = lattice(leaf(i), tri // 2)
            if len(picked) < 9 * coarse:
                picked += once[:9 * (coarse // n + 1)]
            doubled = [x for t in range(tri // 2) for x in once[9 * t:9 * t + 9] * 2]
            leaves.append(tile(f'g{i}.glb', payload(doubled, i, i * 3 * tri)))
        picked = picked[:9 * coarse]
        blob = payload([x for t in range(len(picked) // 9) for x in picked[9 * t:9 * t + 9] * 2], 1 << 30)
        children = [tile('coarse.glb', blob, 1, leaves)]
        for _ in range(chain - 1):
            children = [{'boundingVolume': BOX, 'geometricError': 1, 'refine': 'REPLACE',
                         'content': {'uri': 'coarse.glb'}, 'children': children}]
    else:
        raise SystemExit(f'unknown class {kind}')
    root = {'boundingVolume': BOX, 'geometricError': 200, 'refine': 'REPLACE', 'children': children}
    return write(out, root, members)


def revfan(out, chain, base, m, leaves, seg=1 << 30):
    """Review 1's revfan (PR #309): LEAVES leaves each hold BASE/LEAVES base triangles doubled and,
    for each base edge (a, b), M copies of the reversed face (b, a, d). One coarse payload doubles
    every base triangle and is shared by a single-child chain of CHAIN coarse tiles, split into an
    external tileset every SEG levels. Every coarse edge is used twice forward and supported twice
    forward, so each chain tile looks up that edge's forward edge faces; the M reversed rows lie
    on the same key, in the other direction."""
    rnd = random.Random(309)
    used = set()

    def pt():
        while True:
            p = tuple(1 + rnd.randrange(1 << 12) / (1 << 12) for _ in range(3))
            if p not in used:
                used.add(p)
                return p
    tris = [[pt(), pt(), pt()] for _ in range(base)]
    flat = lambda ts: [x for t in ts for p in t for x in p]
    members, vertex, children = [], 0, []
    per = base // leaves
    for i in range(leaves):
        ts = []
        for a, b, c in tris[i * per:(i + 1) * per]:
            ts += [[a, b, c], [a, b, c]]
            for x, y in [(a, b), (b, c), (c, a)]:
                d = pt()
                ts += [[y, x, d]] * m
        name = f'g{i}.glb'
        members.append((name, payload(flat(ts), i, vertex)))
        vertex += 3 * len(ts)
        children.append({'boundingVolume': BOX, 'geometricError': 0, 'refine': 'REPLACE', 'content': {'uri': name}})
    coarse = [t for t in tris[:per * leaves] for _ in range(2)]
    members.append(('coarse.glb', payload(flat(coarse), 1 << 30)))
    node = {'boundingVolume': BOX, 'geometricError': 1, 'refine': 'REPLACE', 'content': {'uri': 'coarse.glb'},
            'children': children}
    documents = []
    for level in range(1, chain):
        if level % seg == 0:
            name = f'sub{len(documents)}.json'
            documents.append((name, json.dumps({'asset': {'version': '1.1'}, 'geometricError': 1, 'root': node}).encode()))
            node = {'boundingVolume': BOX, 'geometricError': 1, 'refine': 'REPLACE', 'content': {'uri': name}}
        node = {'boundingVolume': BOX, 'geometricError': 1, 'refine': 'REPLACE', 'content': {'uri': 'coarse.glb'},
                'children': [node]}
    root = {'boundingVolume': BOX, 'geometricError': 200, 'refine': 'REPLACE', 'children': [node]}
    return write(out, root, documents + members)


def write(out, root, members):
    tileset = json.dumps({'asset': {'version': '1.1'}, 'geometricError': 200, 'root': root}).encode()
    index = []
    with zipfile.ZipFile(out, 'w') as z:
        for name, blob in [('tileset.json', tileset)] + members:
            info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
            info.external_attr = 0o100644 << 16
            z.writestr(info, blob)
            index.append((hashlib.md5(name.encode()).digest(), z.getinfo(name).header_offset))
        index.sort(key=lambda p: struct.unpack('<QQ', p[0]))
        info = zipfile.ZipInfo('@3dtilesIndex1@', (1980, 1, 1, 0, 0, 0))
        info.external_attr = 0o100644 << 16
        z.writestr(info, b''.join(h + struct.pack('<Q', o) for h, o in index))
    return sum(i.compress_size for i in zipfile.ZipFile(out).infolist())


def validate(binary, path, work, budget=None):
    cmd = [binary, '--json', 'validate', str(path), '--scratchDir', str(work)]
    if budget is not None:
        cmd += ['--maxVolume', f'totalPayloadElements={budget}']
    r = subprocess.run(cmd, capture_output=True, text=True)
    report = json.loads(r.stdout)
    error = report.get('error') or {}
    return report.get('ok'), error.get('code'), error.get('message') or '', (error.get('limit') or {}).get('observed')


def work(binary, path, scratch):
    """The units a validation charges: the smallest admitting `totalPayloadElements` budget."""
    admitted = lambda b: validate(binary, path, scratch, b)[1] != 'resource_limit'
    hi = 1 << 16
    while not admitted(hi):
        hi *= 2
    lo = hi // 2
    while hi - lo > 1:
        mid = (lo + hi) // 2
        lo, hi = (lo, mid) if admitted(mid) else (mid, hi)
    return hi


def check(binary, work_dir):
    work_dir = pathlib.Path(work_dir)
    work_dir.mkdir(parents=True, exist_ok=True)
    unbounded = 1 << 40
    cases = [('leafrand', 4, 512, 1, 1, True, None),
             ('twin', 4, 512, 3, 1, True, None),
             ('chain', 4, 512, 5, 3, False, 'coarseDrops: tile 1 drops')]
    for kind, n, tri, seed, chain, ok, message in cases:
        path = work_dir / f'{kind}.3tz'
        archive(kind, n, tri, seed, path, chain, 64)
        got = validate(binary, path, work_dir, unbounded)
        assert got[0] is ok and (message is None or message in got[2]), (kind, got)
        # Work is charged before it runs: a budget below the class's work is a structured refusal.
        low = validate(binary, path, work_dir, 1_000_000)
        assert low[1] == 'resource_limit' and 'totalPayloadElements=1000000' in low[2], (kind, low)
        print(json.dumps({'class': kind, 'verdict': got[:3], 'refused_at': low[3]}))
    # revfan: every chain tile repeats its coarse lookups. Each of the 3·BASE coarse edges takes
    # two seeks (descendant incidence, forward edge faces) and each of the 3·BASE coincident
    # corners at least two (twins, then their faces): at least 12·BASE seeks of 320 units per
    # chain tile. The former 128-unit seeks and one-unit rows (develop f02e8774) charge 12,094
    # units per added tile here, under that floor; 320-unit seeks charge 39,694 (2026-10-31).
    base, works = 8, []
    for k in (2, 6):
        path = work_dir / f'revfan{k}.3tz'
        revfan(path, k, base, 40, 2)
        got = validate(binary, path, work_dir, unbounded)
        assert got[0] is True, ('revfan', got)
        works.append(work(binary, path, work_dir))
    per_tile = (works[1] - works[0]) / 4
    floor = 12 * base * 320
    assert per_tile >= floor, ('revfan units per chain tile', per_tile, floor)
    print(json.dumps({'class': 'revfan', 'units': works, 'per_chain_tile': per_tile, 'floor': floor}))


def main():
    p = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    sub = p.add_subparsers(dest='command', required=True)
    g = sub.add_parser('generate')
    g.add_argument('kind')
    for name in ['n', 'tri', 'seed']:
        g.add_argument(name, type=int)
    g.add_argument('out', type=pathlib.Path)
    g.add_argument('chain', type=int, nargs='?', default=1)
    g.add_argument('coarse', type=int, nargs='?', default=1024)
    r = sub.add_parser('revfan')
    r.add_argument('out', type=pathlib.Path)
    for name in ['chain', 'base', 'm', 'leaves']:
        r.add_argument(name, type=int)
    r.add_argument('seg', type=int, nargs='?', default=1 << 30)
    c = sub.add_parser('check')
    c.add_argument('--binary', required=True)
    c.add_argument('--work', required=True)
    a = p.parse_args()
    if a.command == 'generate':
        stored = archive(a.kind, a.n, a.tri, a.seed, a.out, a.chain, a.coarse)
        print(json.dumps({'class': a.kind, 'leaves': a.n, 'triangles': a.tri, 'stored': stored,
                          'budget': max(16_000_000, 77 * stored)}))
    elif a.command == 'revfan':
        print(json.dumps({'class': 'revfan', 'stored': revfan(a.out, a.chain, a.base, a.m, a.leaves, a.seg)}))
    else:
        check(a.binary, a.work)


if __name__ == '__main__':
    sys.exit(main())
