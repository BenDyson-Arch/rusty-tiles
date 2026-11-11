"""Hostile classes of the D11 coarse-drop recount (PR #309 review 3; the reviewer's gridbomb.py
and scanbomb.py). One coarse tile (G = ERR) declaring dropped components over two leaves (G =
LEAFG), both leaves cut from one list of triangles.

- `grid N`: N big triangles whose boxes each span the whole extent, so the recount enters each
  in about 27,000 of its distance-grid cells, plus one tiny triangle on the last big one that
  the coarse payload drops (`droppedComponents: 1`; below the leaves' G, so not a coarse drop).
  Defaults LEAFG 0.01, ERR 0.02.
- `scan N D`: N stacked horizontal triangles 1e-5 m apart in a 3 cm square plus one far
  triangle (the stack fills one or two grid cells), D vertical dropped triangles each spanning
  the top 24 planes (successive sample points are answered by different planes, so the
  last-face cache misses), and filler triangles hung off plane vertices so the children exceed
  the 8,192 cap (a coarse drop). LEAFG 0, ERR 1e-6.

Both reproduce review 3's archives byte for byte (meshopt streams from `enc.mjs`). The
time-statement archives are `grid 2000`, `grid 8000` and `scan 4100 4000` (S = 183,055, 683,051
and 471,742; SHA-256 `0438e14a…`, `df1e917e…`, `a137f000…`). `rows_control.py check` runs both
classes at CI size with plain buffers.
usage: recount.py grid N OUT.3tz [LEAFG] [ERR]
       recount.py scan N D OUT.3tz"""
import hashlib, json, os, pathlib, struct, sys, zipfile
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import bombs_lib as b

BOX = {'box': [0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0, 2]}


def finish_dropped(d, extra, dropped):
    """`Doc.finish` with `asset.extras.rustyTiles.droppedComponents`."""
    doc = dict(asset={'version': '2.0', 'generator': f'bomb {b.SERIAL}', 'extras': {'rustyTiles': {'droppedComponents': dropped}}},
               scene=0, buffers=[{'byteLength': 0}], bufferViews=d.views, accessors=d.acc, **extra)
    used = set(doc.get('extensionsUsed', []))
    if d.fallback:
        doc['buffers'].append({'byteLength': d.fallback, 'extensions': {'EXT_meshopt_compression': {'fallback': True}}})
        used.add('EXT_meshopt_compression')
        doc['extensionsRequired'] = sorted(set(doc.get('extensionsRequired', [])) | {'EXT_meshopt_compression'})
    if used:
        doc['extensionsUsed'] = sorted(used)
    return b.glb(doc, bytes(d.bin))


def leaf_glb(serial, xyz, first_vertex):
    """Identity leaf: len(xyz)/9 triangles, distinct vertices, source_triangle rows."""
    d = b.Doc()
    t = len(xyz) // 9
    v = 3 * t
    pos = d.accessor(d.meshopt(b.raw(struct.pack(f'<{3 * v}f', *xyz), v, 12), v, 12, 'ATTRIBUTES', 34962), 5126, 'VEC3', v,
                     min=[min(xyz[k::3]) for k in range(3)], max=[max(xyz[k::3]) for k in range(3)])
    f0 = d.accessor(d.meshopt(b.enc('zeros', v, 4), v, 4, 'ATTRIBUTES', 34962), 5126, 'SCALAR', v)
    f1 = d.accessor(d.meshopt(b.raw(struct.pack(f'<{v}f', *[float(c // 3) for c in range(v)]), v, 4), v, 4, 'ATTRIBUTES', 34962),
                    5126, 'SCALAR', v)
    idx = d.accessor(d.meshopt(b.enc('tri-seq', t, 4), 3 * t, 4, 'TRIANGLES', 34963), 5125, 'SCALAR', 3 * t)
    sp = {k: {'values': d.plain(b'\0' * w)} for k, w in [('expanded_ordinal', 8), ('material_index', 4), ('mesh_index', 4),
                                                         ('node_index', 4), ('primitive_index', 4)]}
    st = {k: {'values': d.plain(b'\0' * t)} for k in ['attribute_flags', 'degenerate']}
    for k in ['material_index', 'mesh_index', 'node_index', 'primitive_index']:
        st[k] = {'values': d.meshopt(b.enc('zeros', t, 4), t, 4, 'ATTRIBUTES', None)}
    st['expanded_ordinal'] = {'values': d.meshopt(b.enc('zeros', t, 8), t, 8, 'ATTRIBUTES', None)}
    st['triangle_index'] = {'values': d.meshopt(b.raw(struct.pack(f'<{t}Q', *range(t)), t, 8), t, 8, 'ATTRIBUTES', None)}
    for j in range(3):
        st[f'vertex_{j}'] = {'values': d.meshopt(b.raw(struct.pack(f'<{t}Q', *range(first_vertex + j, first_vertex + v, 3)), t, 8),
                                                 t, 8, 'ATTRIBUTES', None)}
    for view in d.views:
        if view.get('target') is None:
            view.pop('target', None)
    ext = {'EXT_structural_metadata': {'schema': {'id': 'rusty_tiles_large_source_v1',
                                                  'classes': {'source_primitive': b.SP_CLASS, 'source_triangle': b.ST_CLASS}},
                                       'propertyTables': [{'class': 'source_primitive', 'count': 1, 'properties': sp},
                                                          {'class': 'source_triangle', 'count': t, 'properties': st}]}}
    prim = {'attributes': {'POSITION': pos, '_FEATURE_ID_0': f0, '_FEATURE_ID_1': f1}, 'indices': idx, 'mode': 4,
            'extensions': {'EXT_mesh_features': {'featureIds': [
                {'featureCount': 1, 'attribute': 0, 'propertyTable': 0, 'label': 'source_primitive'},
                {'featureCount': t, 'attribute': 1, 'propertyTable': 1, 'label': 'source_triangle'}]}}}
    b.SERIAL = serial
    return d.finish(dict(scenes=[{'nodes': [0]}], nodes=[{'mesh': 0}], meshes=[{'primitives': [prim]}],
                         extensions=ext, extensionsUsed=['EXT_mesh_features', 'EXT_structural_metadata']), 0)


def coarse_glb(xyz, dropped):
    """Coarse payload: source_primitive identity only, len(xyz)/9 triangles, distinct vertices."""
    d = b.Doc()
    t = len(xyz) // 9
    v = 3 * t
    pos = d.accessor(d.plain(struct.pack(f'<{3 * v}f', *xyz), target=34962), 5126, 'VEC3', v,
                     min=[min(xyz[k::3]) for k in range(3)], max=[max(xyz[k::3]) for k in range(3)])
    fid = d.accessor(d.plain(b'\0' * (4 * v), target=34962), 5126, 'SCALAR', v)
    cols = {k: {'values': d.plain(b'\0' * w)} for k, w in [('expanded_ordinal', 8), ('material_index', 4), ('mesh_index', 4),
                                                           ('node_index', 4), ('primitive_index', 4)]}
    ext = {'EXT_structural_metadata': {'schema': {'id': 'rusty_tiles_large_hlod_v1', 'classes': {'source_primitive': b.SP_CLASS}},
                                       'propertyTables': [{'class': 'source_primitive', 'count': 1, 'properties': cols}]}}
    prim = {'attributes': {'POSITION': pos, '_FEATURE_ID_0': fid}, 'mode': 4,
            'extensions': {'EXT_mesh_features': {'featureIds': [{'featureCount': 1, 'attribute': 0, 'propertyTable': 0,
                                                                  'label': 'source_primitive'}]}}}
    b.SERIAL = 10_000_000
    return finish_dropped(d, dict(scenes=[{'nodes': [0]}], nodes=[{'mesh': 0}], meshes=[{'primitives': [prim]}],
                                  extensions=ext, extensionsUsed=['EXT_mesh_features', 'EXT_structural_metadata']), dropped)


def write(out, kids, coarse, dropped, leaf_g, err):
    """Two leaves (the first and second half of `kids`) under one coarse tile, under a root."""
    members = []

    def content(name, blob):
        members.append((name, blob))
        return {'uri': name}

    half = len(kids) // 2
    xa = [x for f in kids[:half] for x in f]
    xb = [x for f in kids[half:] for x in f]
    leaves = [{'boundingVolume': BOX, 'geometricError': leaf_g, 'refine': 'REPLACE', 'content': content(name, blob)}
              for name, blob in [('a.glb', leaf_glb(0, xa, 0)), ('b.glb', leaf_glb(1, xb, 3 * half))]]
    tile = {'boundingVolume': BOX, 'geometricError': err, 'refine': 'REPLACE',
            'content': content('coarse.glb', coarse_glb([x for f in coarse for x in f], dropped)), 'children': leaves}
    root = {'boundingVolume': BOX, 'geometricError': 200, 'refine': 'REPLACE', 'children': [tile]}
    ts = json.dumps({'asset': {'version': '1.1'}, 'geometricError': 200, 'root': root}).encode()
    records = []
    with zipfile.ZipFile(out, 'w') as z:
        for name, blob in [('tileset.json', ts)] + members:
            info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
            info.external_attr = 0o100644 << 16
            z.writestr(info, blob)
            records.append((hashlib.md5(name.encode()).digest(), z.getinfo(name).header_offset))
        records.sort(key=lambda p: struct.unpack('<QQ', p[0]))
        info = zipfile.ZipInfo('@3dtilesIndex1@', (1980, 1, 1, 0, 0, 0))
        info.external_attr = 0o100644 << 16
        z.writestr(info, b''.join(h + struct.pack('<Q', o) for h, o in records))
    return sum(i.compress_size for i in zipfile.ZipFile(out).infolist())


def grid(n):
    """N big triangles spanning the extent, and one tiny triangle on the last one (dropped)."""
    big = []
    for i in range(n):
        t = 0.1 + 0.3 * i / n
        big.append([0., t, 0., 1., 1. - t, 1., t, 1., 0.5])
    last = big[-1]
    c = [(last[k] + last[3 + k] + last[6 + k]) / 3 for k in range(3)]
    e1 = [(last[3 + k] - last[k]) * 1e-4 for k in range(3)]
    e2 = [(last[6 + k] - last[k]) * 1e-4 for k in range(3)]
    tiny = c + [c[k] + e1[k] for k in range(3)] + [c[k] + e2[k] for k in range(3)]
    return big + [tiny], big


def scan(n, d):
    """N stacked planes, a far triangle, D dropped vertical triangles and cap filler."""
    f32 = lambda x: struct.unpack('<f', struct.pack('<f', x))[0]
    z = lambda k: f32(k * 1e-5)
    far = [1., 1., 1., 0.999, 1., 1., 1., 0.999, 1.]
    planes = [[0., 0., z(k), 0.03, 0., z(k), 0., 0.03, z(k)] for k in range(n)]
    lo, hi = n - 1 - 24, n - 1
    drops = []
    for j in range(d):
        x = 0.001 + 0.0001 * (j % 100)
        y = 0.001 + 0.0001 * (j // 100)
        drops.append([x, y, z(lo), x + 0.00002, y, z(hi), x, y + 0.00002, z(lo)])
    filler = []
    for k in range(max(0, 8193 - (n + 1 + d))):
        p = planes[k % n]
        off = 0.001 * (1 + k // n)
        filler.append([p[0], p[1], p[2], p[0] - off, p[1], p[2] - off, p[0], p[1] - off, p[2] - off])
    return [far] + planes + drops + filler, [far] + planes


def main():
    argv = sys.argv[1:]
    b.TMP.mkdir(parents=True, exist_ok=True)
    if argv[0] == 'grid':
        n, out = int(argv[1]), pathlib.Path(argv[2])
        leaf_g = float(argv[3]) if len(argv) > 3 else 0.01
        err = float(argv[4]) if len(argv) > 4 else 0.02
        kids, coarse = grid(n)
        stored = write(out, kids, coarse, 1, leaf_g, err)
        info = dict(N=n, leafG=leaf_g, err=err)
    elif argv[0] == 'scan':
        n, d, out = int(argv[1]), int(argv[2]), pathlib.Path(argv[3])
        kids, coarse = scan(n, d)
        stored = write(out, kids, coarse, d, 0.0, 1e-6)
        info = dict(N=n, D=d, children=len(kids))
    else:
        sys.exit(__doc__)
    print(json.dumps(dict(cls=argv[0], **info, stored=stored, budget=16_000_000 + 224 * stored)))


if __name__ == '__main__':
    main()
