"""Hostile r2 archive: N leaves of T nondegenerate triangles with distinct, pseudo-random lattice
positions (x = 1 + b/128 per component, b random 0..127: one varying byte, cheap in the meshopt
vertex codec) and a source_triangle table (3 new originals per triangle). Every leaf point, edge
and face becomes one SQLite scratch row with a random key.
usage: leafrand.py N_LEAVES TRIANGLES SEED OUT.3tz"""
import sys, json, zipfile, hashlib, struct, pathlib, random
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
sys.argv, args = sys.argv[:1], sys.argv[1:]
import bombs_lib as b
n, tri, seed, out = int(args[0]), int(args[1]), int(args[2]), pathlib.Path(args[3])
b.TMP.mkdir(parents=True, exist_ok=True)

def leaf(i):
    rnd = random.Random(seed * 1_000_003 + i)
    d = b.Doc()
    v = 3 * tri
    xyz = [1 + rnd.randrange(128) / 128 for _ in range(3 * v)]
    pos = d.accessor(d.meshopt(b.raw(struct.pack(f'<{3 * v}f', *xyz), v, 12), v, 12, 'ATTRIBUTES', 34962), 5126, 'VEC3', v,
                     min=[min(xyz[k::3]) for k in range(3)], max=[max(xyz[k::3]) for k in range(3)])
    f0 = d.accessor(d.meshopt(b.enc('zeros', v, 4), v, 4, 'ATTRIBUTES', 34962), 5126, 'SCALAR', v)
    rows = struct.pack(f'<{v}f', *[float(c // 3) for c in range(v)])
    f1 = d.accessor(d.meshopt(b.raw(rows, v, 4), v, 4, 'ATTRIBUTES', 34962), 5126, 'SCALAR', v)
    idx = d.accessor(d.meshopt(b.enc('tri-seq', tri, 4), 3 * tri, 4, 'TRIANGLES', 34963), 5125, 'SCALAR', 3 * tri)
    sp = {k: {'values': d.plain(b'\0' * w)} for k, w in [('expanded_ordinal', 8), ('material_index', 4), ('mesh_index', 4), ('node_index', 4), ('primitive_index', 4)]}
    st = {k: {'values': d.plain(b'\0' * tri)} for k in ['attribute_flags', 'degenerate']}
    for k in ['material_index', 'mesh_index', 'node_index', 'primitive_index']:
        st[k] = {'values': d.meshopt(b.enc('zeros', tri, 4), tri, 4, 'ATTRIBUTES', None)}
    st['expanded_ordinal'] = {'values': d.meshopt(b.enc('zeros', tri, 8), tri, 8, 'ATTRIBUTES', None)}
    st['triangle_index'] = {'values': d.meshopt(b.raw(struct.pack(f'<{tri}Q', *range(tri)), tri, 8), tri, 8, 'ATTRIBUTES', None)}
    for j in range(3):
        # distinct original vertex ids per leaf: offset by leaf index
        st[f'vertex_{j}'] = {'values': d.meshopt(b.raw(struct.pack(f'<{tri}Q', *range(i * v + j, i * v + v, 3)), tri, 8), tri, 8, 'ATTRIBUTES', None)}
    for view in d.views:
        if view.get('target') is None: view.pop('target', None)
    ext = {'EXT_structural_metadata': {'schema': {'id': 'rusty_tiles_large_source_v1', 'classes': {'source_primitive': b.SP_CLASS, 'source_triangle': b.ST_CLASS}},
                                       'propertyTables': [{'class': 'source_primitive', 'count': 1, 'properties': sp},
                                                          {'class': 'source_triangle', 'count': tri, 'properties': st}]}}
    prim = {'attributes': {'POSITION': pos, '_FEATURE_ID_0': f0, '_FEATURE_ID_1': f1}, 'indices': idx, 'mode': 4,
            'extensions': {'EXT_mesh_features': {'featureIds': [
                {'featureCount': 1, 'attribute': 0, 'propertyTable': 0, 'label': 'source_primitive'},
                {'featureCount': tri, 'attribute': 1, 'propertyTable': 1, 'label': 'source_triangle'}]}}}
    b.SERIAL = i
    return d.finish(dict(scenes=[{'nodes': [0]}], nodes=[{'mesh': 0}], meshes=[{'primitives': [prim]}],
                         extensions=ext, extensionsUsed=['EXT_mesh_features', 'EXT_structural_metadata']), 0)

children = [{'boundingVolume': {'box': [0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0, 2]}, 'geometricError': 0, 'refine': 'REPLACE', 'content': {'uri': f'g{i}.glb'}} for i in range(n)]
ts = json.dumps({'asset': {'version': '1.1'}, 'geometricError': 200,
                 'root': {'boundingVolume': {'box': [0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0, 2]}, 'geometricError': 200, 'refine': 'REPLACE', 'children': children}}).encode()
records = []
with zipfile.ZipFile(out, 'w') as z:
    for name, blob in [('tileset.json', ts)] + [(f'g{i}.glb', leaf(i)) for i in range(n)]:
        info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0)); info.external_attr = 0o100644 << 16
        z.writestr(info, blob); records.append((hashlib.md5(name.encode()).digest(), z.getinfo(name).header_offset))
    records.sort(key=lambda p: struct.unpack('<QQ', p[0]))
    info = zipfile.ZipInfo('@3dtilesIndex1@', (1980, 1, 1, 0, 0, 0)); info.external_attr = 0o100644 << 16
    z.writestr(info, b''.join(h + struct.pack('<Q', o) for h, o in records))
S = sum(i.compress_size for i in zipfile.ZipFile(out).infolist())
print(json.dumps(dict(leaves=n, triangles=tri, stored=S, rows_upper=n * tri * 7, row_units_upper=n * tri * 7 * 128, per_byte_rows_units=round(n * tri * 7 * 128 / S, 1), budget=16_000_000 + 105 * S)))
