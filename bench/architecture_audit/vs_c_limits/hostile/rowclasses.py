"""Hostile HLOD scratch-row classes beside leafrand (c6), for the time statement.
usage: rowclasses.py KIND N_LEAVES TRIANGLES SEED OUT.3tz [K] [COARSE]
Leaves are leafrand's: T nondegenerate triangles, every corner a distinct vertex on a
pseudo-random 128^3 lattice in [1,2)^3, with source_triangle tables.
KIND:
  twin      two coarse cells A and B under a content-less root, each over N/2 leaves; B's leaves
            repeat A's positions (new vertex ids), so every key is shared across the cells:
            ancestor requirements, multi edges (edge faces), coincident corner sets (twins)
  chain     a single-child chain of K coarse tiles with one payload over N leaves whose
            triangles are each doubled (same winding); the coarse payload doubles COARSE of those
            triangles, so every coarse edge is used twice: keyed lookups per chain tile
Streams use the upstream meshoptimizer encoder (bombs_lib.enc / raw)."""
import sys, json, zipfile, hashlib, struct, pathlib, random
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
sys.argv, args = sys.argv[:1], sys.argv[1:]
import bombs_lib as b
kind, n, tri, seed, out = args[0], int(args[1]), int(args[2]), int(args[3]), pathlib.Path(args[4])
K = int(args[5]) if len(args) > 5 else 1
COARSE = int(args[6]) if len(args) > 6 else 1024
b.TMP.mkdir(parents=True, exist_ok=True)


def lattice(rnd, count):
    return [1 + rnd.randrange(128) / 128 for _ in range(3 * count)]


def leaf_glb(serial, xyz, first_vertex):
    """Identity leaf: len(xyz)/9 triangles, distinct vertices, source_triangle rows."""
    d = b.Doc()
    t = len(xyz) // 9
    v = 3 * t
    pos = d.accessor(d.meshopt(b.raw(struct.pack(f'<{3 * v}f', *xyz), v, 12), v, 12, 'ATTRIBUTES', 34962), 5126, 'VEC3', v,
                     min=[min(xyz[k::3]) for k in range(3)], max=[max(xyz[k::3]) for k in range(3)])
    f0 = d.accessor(d.meshopt(b.enc('zeros', v, 4), v, 4, 'ATTRIBUTES', 34962), 5126, 'SCALAR', v)
    f1 = d.accessor(d.meshopt(b.raw(struct.pack(f'<{v}f', *[float(c // 3) for c in range(v)]), v, 4), v, 4, 'ATTRIBUTES', 34962), 5126, 'SCALAR', v)
    idx = d.accessor(d.meshopt(b.enc('tri-seq', t, 4), 3 * t, 4, 'TRIANGLES', 34963), 5125, 'SCALAR', 3 * t)
    sp = {k: {'values': d.plain(b'\0' * w)} for k, w in [('expanded_ordinal', 8), ('material_index', 4), ('mesh_index', 4), ('node_index', 4), ('primitive_index', 4)]}
    st = {k: {'values': d.plain(b'\0' * t)} for k in ['attribute_flags', 'degenerate']}
    for k in ['material_index', 'mesh_index', 'node_index', 'primitive_index']:
        st[k] = {'values': d.meshopt(b.enc('zeros', t, 4), t, 4, 'ATTRIBUTES', None)}
    st['expanded_ordinal'] = {'values': d.meshopt(b.enc('zeros', t, 8), t, 8, 'ATTRIBUTES', None)}
    st['triangle_index'] = {'values': d.meshopt(b.raw(struct.pack(f'<{t}Q', *range(t)), t, 8), t, 8, 'ATTRIBUTES', None)}
    for j in range(3):
        st[f'vertex_{j}'] = {'values': d.meshopt(b.raw(struct.pack(f'<{t}Q', *range(first_vertex + j, first_vertex + v, 3)), t, 8), t, 8, 'ATTRIBUTES', None)}
    for view in d.views:
        if view.get('target') is None: view.pop('target', None)
    ext = {'EXT_structural_metadata': {'schema': {'id': 'rusty_tiles_large_source_v1', 'classes': {'source_primitive': b.SP_CLASS, 'source_triangle': b.ST_CLASS}},
                                       'propertyTables': [{'class': 'source_primitive', 'count': 1, 'properties': sp},
                                                          {'class': 'source_triangle', 'count': t, 'properties': st}]}}
    prim = {'attributes': {'POSITION': pos, '_FEATURE_ID_0': f0, '_FEATURE_ID_1': f1}, 'indices': idx, 'mode': 4,
            'extensions': {'EXT_mesh_features': {'featureIds': [
                {'featureCount': 1, 'attribute': 0, 'propertyTable': 0, 'label': 'source_primitive'},
                {'featureCount': t, 'attribute': 1, 'propertyTable': 1, 'label': 'source_triangle'}]}}}
    b.SERIAL = serial
    return d.finish(dict(scenes=[{'nodes': [0]}], nodes=[{'mesh': 0}], meshes=[{'primitives': [prim]}],
                         extensions=ext, extensionsUsed=['EXT_mesh_features', 'EXT_structural_metadata']), 0)


def coarse_glb(serial, xyz):
    """Coarse payload: source_primitive identity only, len(xyz)/9 triangles, distinct vertices."""
    d = b.Doc()
    t = len(xyz) // 9
    v = 3 * t
    pos = d.accessor(d.plain(struct.pack(f'<{3 * v}f', *xyz), target=34962), 5126, 'VEC3', v,
                     min=[min(xyz[k::3]) for k in range(3)], max=[max(xyz[k::3]) for k in range(3)])
    fid = d.accessor(d.plain(b'\0' * (4 * v), target=34962), 5126, 'SCALAR', v)
    cols = {k: {'values': d.plain(b'\0' * w)} for k, w in [('expanded_ordinal', 8), ('material_index', 4), ('mesh_index', 4), ('node_index', 4), ('primitive_index', 4)]}
    ext = {'EXT_structural_metadata': {'schema': {'id': 'rusty_tiles_large_hlod_v1', 'classes': {'source_primitive': b.SP_CLASS}},
                                       'propertyTables': [{'class': 'source_primitive', 'count': 1, 'properties': cols}]}}
    prim = {'attributes': {'POSITION': pos, '_FEATURE_ID_0': fid}, 'mode': 4,
            'extensions': {'EXT_mesh_features': {'featureIds': [{'featureCount': 1, 'attribute': 0, 'propertyTable': 0, 'label': 'source_primitive'}]}}}
    b.SERIAL = 10_000_000 + serial
    return d.finish(dict(scenes=[{'nodes': [0]}], nodes=[{'mesh': 0}], meshes=[{'primitives': [prim]}],
                         extensions=ext, extensionsUsed=['EXT_mesh_features', 'EXT_structural_metadata']), 0)


BOX = {'box': [0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0, 2]}
members, uri = [], {}


def content(name, blob):
    members.append((name, blob))
    return {'uri': name}


def leaf_tile(name, blob):
    return {'boundingVolume': BOX, 'geometricError': 0, 'refine': 'REPLACE', 'content': content(name, blob)}


rnd = random.Random(seed)
if kind == 'twin':
    half = n // 2
    cells = []
    for c in range(2):
        leaves = []
        for i in range(half):
            xyz = lattice(random.Random(seed * 1_000_003 + i), 3 * tri)
            serial = c * half + i
            leaves.append(leaf_tile(f'c{c}g{i}.glb', leaf_glb(serial, xyz, serial * 3 * tri)))
        cells.append({'boundingVolume': BOX, 'geometricError': 1, 'refine': 'REPLACE',
                      'content': content(f'coarse{c}.glb', coarse_glb(c, lattice(random.Random(seed + 7 + c), 3))), 'children': leaves})
    root = {'boundingVolume': BOX, 'geometricError': 200, 'refine': 'REPLACE', 'children': cells}
elif kind == 'chain':
    leaves, picked = [], []
    for i in range(n):
        r = random.Random(seed * 1_000_003 + i)
        once = lattice(r, 3 * (tri // 2))
        xyz = []
        for t in range(tri // 2):
            xyz += once[9 * t:9 * t + 9] * 2
        if len(picked) < 9 * COARSE:
            picked += once[:9 * max(1, COARSE // n + 1)]
        leaves.append(leaf_tile(f'g{i}.glb', leaf_glb(i, xyz, i * 3 * tri)))
    picked = picked[:9 * COARSE]
    coarse = []
    for t in range(len(picked) // 9):
        coarse += picked[9 * t:9 * t + 9] * 2
    blob = coarse_glb(0, coarse)
    members.append(('coarse.glb', blob))
    tile = {'boundingVolume': BOX, 'geometricError': 1, 'refine': 'REPLACE', 'content': {'uri': 'coarse.glb'}, 'children': leaves}
    for _ in range(K - 1):
        tile = {'boundingVolume': BOX, 'geometricError': 1, 'refine': 'REPLACE', 'content': {'uri': 'coarse.glb'}, 'children': [tile]}
    root = {'boundingVolume': BOX, 'geometricError': 200, 'refine': 'REPLACE', 'children': [tile]}
else:
    raise SystemExit(f'unknown kind {kind}')

ts = json.dumps({'asset': {'version': '1.1'}, 'geometricError': 200, 'root': root}).encode()
records = []
with zipfile.ZipFile(out, 'w') as z:
    for name, blob in [('tileset.json', ts)] + members:
        info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0)); info.external_attr = 0o100644 << 16
        z.writestr(info, blob); records.append((hashlib.md5(name.encode()).digest(), z.getinfo(name).header_offset))
    records.sort(key=lambda p: struct.unpack('<QQ', p[0]))
    info = zipfile.ZipInfo('@3dtilesIndex1@', (1980, 1, 1, 0, 0, 0)); info.external_attr = 0o100644 << 16
    z.writestr(info, b''.join(h + struct.pack('<Q', o) for h, o in records))
S = sum(i.compress_size for i in zipfile.ZipFile(out).infolist())
print(json.dumps(dict(kind=kind, leaves=n, triangles=tri, K=K, stored=S, budget=16_000_000 + 105 * S)))
