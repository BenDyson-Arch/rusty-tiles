"""H28 real-data comparison of two mesh archives (before, after).

usage: cmp_h28.py BEFORE.3tz AFTER.3tz WORKDIR OUT.json
Extracts both archives, decodes every GLB with the reference meshopt decoder (node),
and reports per-tile primitive/texture counts, archive bytes, whether every leaf
triangle's decoded corners are bit-identical by source identity, whether every proxy
face (by identity) is bit-identical, and which non-GLB members differ.
"""
import base64, collections, hashlib, json, os, struct, subprocess, sys, zipfile
import numpy as np

TOOLS = os.path.dirname(os.path.abspath(__file__))


def extract(archive, dest):
    with zipfile.ZipFile(archive) as z:
        z.extractall(dest)
        return sorted(n for n in z.namelist() if not n.endswith('/'))


def decoded_glbs(root, names):
    paths = [os.path.join(root, n) for n in names if n.endswith('.glb')]
    p = subprocess.run(['node', os.path.join(TOOLS, 'decode_many.cjs')], input='\n'.join(paths),
                       capture_output=True, text=True, check=True)
    for line in p.stdout.splitlines():
        yield json.loads(line)


def accessor(rec, index):
    doc = rec['document']
    a = doc['accessors'][index]
    raw = base64.b64decode(rec['views'][a['bufferView']])
    view = doc['bufferViews'][a['bufferView']]
    stride = view.get('byteStride') or (view.get('extensions', {}).get('EXT_meshopt_compression', {}).get('byteStride'))
    comp = {5120: 'i1', 5121: 'u1', 5122: 'i2', 5123: 'u2', 5125: 'u4', 5126: 'f4'}[a['componentType']]
    width = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4}[a['type']]
    size = np.dtype(comp).itemsize
    stride = stride or size * width
    count = a['count']
    off = a.get('byteOffset', 0)
    out = np.zeros((count, width), dtype=comp)
    buf = np.frombuffer(raw, dtype=np.uint8)
    for c in range(width):
        idx = off + np.arange(count) * stride + c * size
        out[:, c] = np.frombuffer(np.stack([buf[idx + b] for b in range(size)], axis=1).tobytes(), dtype=comp)
    return out


def table_columns(rec):
    doc = rec['document']
    meta = doc.get('extensions', {}).get('EXT_structural_metadata', {})
    classes = meta.get('schema', {}).get('classes', {})
    tables = []
    for t in meta.get('propertyTables', []):
        cols = {}
        for name, prop in t['properties'].items():
            comp = classes[t['class']]['properties'][name]['componentType']
            dt = {'UINT8': 'u1', 'UINT16': 'u2', 'UINT32': 'u4', 'UINT64': 'u8', 'FLOAT64': 'f8'}[comp]
            raw = base64.b64decode(rec['views'][prop['values']])
            cols[name] = np.frombuffer(raw[:t['count'] * np.dtype(dt).itemsize], dtype=dt)
        tables.append((t['class'], cols))
    return tables


def faces(rec):
    """Per published face: (kind, identity, corner bits) with corners as a 9-tuple of u32."""
    doc = rec['document']
    tables = table_columns(rec)
    out = []
    for node in doc['nodes']:
        if 'mesh' not in node:
            continue
        t = node.get('translation', [0., 0., 0.])
        s = node.get('scale', [1., 1., 1.])
        for prim in doc['meshes'][node['mesh']]['primitives']:
            attrs = prim['attributes']
            pos = accessor(rec, attrs['POSITION'])
            if pos.dtype == np.uint16:
                p = np.empty(pos.shape, dtype=np.float32)
                for a in range(3):
                    p[:, a] = (pos[:, a].astype(np.float64) * s[a] + t[a]).astype(np.float32)
            else:
                assert node.get('translation') is None and node.get('scale') is None
                p = pos.astype(np.float32)
            p[p == 0] = 0  # canonical zero
            bits = p.view(np.uint32)
            sets = prim['extensions']['EXT_mesh_features']['featureIds']
            ids = {}
            for fs in sets:
                if 'attribute' in fs:
                    v = accessor(rec, attrs['_FEATURE_ID_%d' % fs['attribute']])[:, 0].astype(np.int64)
                    ids[fs['label']] = (v, tables[fs['propertyTable']])
            prow, (_, pt) = ids['source_primitive']
            key = 'source_triangle' if 'source_triangle' in ids else 'proxy_source'
            trow, (_, tt) = ids[key]
            for f in range(len(p) // 3):
                r, q = prow[3 * f], trow[3 * f]
                ident = (int(pt['node_index'][r]), int(pt['mesh_index'][r]), int(pt['primitive_index'][r]),
                         int(pt['expanded_ordinal'][r]),
                         int(tt['triangle_index'][q]), int(tt['fill'][q]) if 'fill' in tt else 0)
                out.append((key, ident, tuple(int(x) for x in bits[3 * f:3 * f + 3].reshape(-1))))
    return out


def structure(rec):
    doc = rec['document']
    prims = sum(len(m['primitives']) for m in doc.get('meshes', []))
    texsets = sum(1 for m in doc.get('meshes', []) for p in m['primitives']
                  for s in p.get('extensions', {}).get('EXT_mesh_features', {}).get('featureIds', []) if 'texture' in s)
    classes = [t['class'] for t in doc.get('extensions', {}).get('EXT_structural_metadata', {}).get('propertyTables', [])]
    kind = 'leaf' if 'source_triangle' in classes else 'proxy'
    return {'kind': kind, 'primitives': prims, 'nodes': len(doc.get('nodes', [])), 'textures': len(doc.get('textures', [])),
            'images': len(doc.get('images', [])), 'materials': len(doc.get('materials', [])),
            'featureIdTextureSets': texsets, 'bytes': rec['bytes']}


def summary(rows):
    out = {}
    for kind in ('leaf', 'proxy'):
        rs = [r for r in rows.values() if r['kind'] == kind]
        if not rs:
            continue
        out[kind] = {'tiles': len(rs)}
        for k in ('primitives', 'textures', 'images', 'materials', 'featureIdTextureSets', 'bytes'):
            v = [r[k] for r in rs]
            out[kind][k] = {'total': sum(v), 'max': max(v), 'mean': round(sum(v) / len(v), 2)}
    return out


def main():
    before, after, work, report = sys.argv[1:5]
    result = {}
    trees = {}
    for label, archive in (('before', before), ('after', after)):
        root = os.path.join(work, label)
        names = extract(archive, root)
        rows, geometry = {}, {}
        for rec in decoded_glbs(root, names):
            name = os.path.relpath(rec['path'], root)
            rows[name] = structure(rec)
            geometry[name] = faces(rec)
        others = {n: hashlib.sha256(open(os.path.join(root, n), 'rb').read()).hexdigest()
                  for n in names if not n.endswith('.glb')}
        trees[label] = (rows, geometry, others)
        result[label] = {'archive': archive, 'archiveBytes': os.path.getsize(archive), 'members': len(names),
                         'summary': summary(rows), 'rootProxy': rows.get('content/f0-d0.glb')}
    (rb, gb, ob), (ra, ga, oa) = trees['before'], trees['after']
    assert set(rb) == set(ra), 'same tile members'
    leaf_same = proxy_same = 0
    leaf_diff, proxy_diff = [], []
    triangles = 0
    for name in sorted(rb):
        a = collections.Counter(gb[name])
        b = collections.Counter(ga[name])
        kind = rb[name]['kind']
        triangles += sum(a.values()) if kind == 'leaf' else 0
        if a == b:
            if kind == 'leaf':
                leaf_same += 1
            else:
                proxy_same += 1
        else:
            (leaf_diff if kind == 'leaf' else proxy_diff).append(name)
    result['comparison'] = {
        'leafTilesWithIdenticalDecodedTrianglesByIdentity': leaf_same, 'leafTilesDiffering': leaf_diff,
        'leafTrianglesCompared': triangles,
        'proxyTilesWithIdenticalDecodedFacesByIdentity': proxy_same, 'proxyTilesDiffering': proxy_diff,
        'nonGlbMembersDiffering': sorted(n for n in set(ob) | set(oa) if ob.get(n) != oa.get(n)),
    }
    json.dump(result, open(report, 'w'), indent=1)
    print(json.dumps({k: result[k] for k in ('comparison',)}, indent=1)[:3000])
    for label in ('before', 'after'):
        print(label, result[label]['archiveBytes'], json.dumps(result[label]['summary']), json.dumps(result[label]['rootProxy']))


main()
