"""Adversarial 3TZ archives for the totalPayloadElements budget (PR #304 reviews).
usage: bombs_lib.py KIND N_GLBS OUT.3tz [pad_bytes_per_glb] [extra]
KIND:
  comp64    one unreferenced VEC4 u8 accessor of ~1M elements per GLB (meshopt zeros, 64 comps/byte)
  hlodseq   HLOD-identity primitive, n triangles with distinct vertices (indexed, tri-seq)
  hloddup   HLOD-identity primitive, n repeated triangles (0,1,2), instanced by `extra` nodes
  hugecount accessor claiming count 2^32-1 with a 1-byte stream (refusal before work)
Streams come from the upstream meshoptimizer encoder (enc.mjs, vertex codec v0)."""
import json, struct, subprocess, sys, zipfile, hashlib, pathlib, os

HERE = pathlib.Path(__file__).resolve().parent
# Encoded streams are cached by kind, count and stride (or raw-input hash).
TMP = pathlib.Path(os.environ.get('RT_HOSTILE_CACHE', '/tmp/rt-hostile-enc'))


def enc(kind, n, stride):
    out = TMP / f'{kind}-{n}-{stride}'
    if not out.exists():
        subprocess.run(['node', str(HERE / 'enc.mjs'), kind, str(n), str(stride), str(out)], check=True,
                       stdout=subprocess.DEVNULL)
    return out.read_bytes()


def glb(doc, binary):
    binary += b'\0' * (-len(binary) % 4)
    doc['buffers'][0]['byteLength'] = len(binary)
    j = json.dumps(doc, separators=(',', ':')).encode()
    j += b' ' * (-len(j) % 4)
    return (struct.pack('<III', 0x46546C67, 2, 28 + len(j) + len(binary)) + struct.pack('<II', len(j), 0x4E4F534A) + j
            + struct.pack('<II', len(binary), 0x004E4942) + binary)


class Doc:
    def __init__(self):
        self.bin = bytearray()
        self.views, self.acc = [], []
        self.fallback = 0

    def plain(self, data, stride=None, target=None):
        self.bin += b'\0' * (-len(self.bin) % 8)
        v = {'buffer': 0, 'byteOffset': len(self.bin), 'byteLength': len(data)}
        if stride: v['byteStride'] = stride
        if target: v['target'] = target
        self.bin += data
        self.views.append(v)
        return len(self.views) - 1

    def meshopt(self, data, count, stride, mode, target):
        self.bin += b'\0' * (-len(self.bin) % 8)
        off = len(self.bin)
        self.bin += data
        length = count * stride
        v = {'buffer': 1, 'byteOffset': self.fallback, 'byteLength': length, 'target': target,
             'extensions': {'EXT_meshopt_compression': {'buffer': 0, 'byteOffset': off, 'byteLength': len(data),
                                                        'byteStride': stride, 'count': count, 'mode': mode,
                                                        'filter': 'NONE'}}}
        if mode == 'ATTRIBUTES': v['byteStride'] = stride
        self.fallback += length + (-length % 8)
        self.views.append(v)
        return len(self.views) - 1

    def accessor(self, view, comp, typ, count, **kw):
        self.acc.append(dict(bufferView=view, componentType=comp, type=typ, count=count, **kw))
        return len(self.acc) - 1

    def finish(self, extra, pad):
        if pad: self.plain(os.urandom(pad))
        doc = dict(asset={'version': '2.0', 'generator': f'bomb {SERIAL}'}, scene=0, buffers=[{'byteLength': 0}], bufferViews=self.views,
                   accessors=self.acc, **extra)
        used = set(doc.get('extensionsUsed', []))
        if self.fallback:
            doc['buffers'].append({'byteLength': self.fallback, 'extensions': {'EXT_meshopt_compression': {'fallback': True}}})
            used.add('EXT_meshopt_compression')
            doc['extensionsRequired'] = sorted(set(doc.get('extensionsRequired', [])) | {'EXT_meshopt_compression'})
        if used: doc['extensionsUsed'] = sorted(used)
        return glb(doc, bytes(self.bin))


SERIAL = 0
TRI = struct.pack('<9f', 0, 0, 0, 1, 0, 0, 0, 1, 0)
SP_CLASS = {'properties': {k: {'componentType': t, 'required': True, 'type': 'SCALAR'} for k, t in
                           [('expanded_ordinal', 'UINT64'), ('material_index', 'UINT32'), ('mesh_index', 'UINT32'),
                            ('node_index', 'UINT32'), ('primitive_index', 'UINT32')]}}


def comp64(pad, extra):
    d = Doc()
    pos = d.accessor(d.plain(TRI, target=34962), 5126, 'VEC3', 3, min=[0, 0, 0], max=[1, 1, 0])
    m = 999_996
    d.accessor(d.meshopt(enc('zeros', m, 4), m, 4, 'ATTRIBUTES', 34962), 5121, 'VEC4', m)
    return d.finish(dict(scenes=[{'nodes': [0]}], nodes=[{'mesh': 0}],
                         meshes=[{'primitives': [{'attributes': {'POSITION': pos}, 'mode': 4}]}]), pad)


def hlod(n, kind, instances, pad):
    d = Doc()
    v = 3 * n if kind == 'tri-seq' else 3
    pos = d.accessor(d.meshopt(enc('zeros', v, 12), v, 12, 'ATTRIBUTES', 34962), 5126, 'VEC3', v,
                     min=[0, 0, 0], max=[0, 0, 0])
    fid = d.accessor(d.meshopt(enc('zeros', v, 4), v, 4, 'ATTRIBUTES', 34962), 5126, 'SCALAR', v)
    idx = d.accessor(d.meshopt(enc(kind, n, 4), 3 * n, 4, 'TRIANGLES', 34963), 5125, 'SCALAR', 3 * n)
    cols = {}
    for k, w in [('expanded_ordinal', 8), ('material_index', 4), ('mesh_index', 4), ('node_index', 4),
                 ('primitive_index', 4)]:
        cols[k] = {'values': d.plain(b'\0' * w)}
    ext = {'EXT_structural_metadata': {'schema': {'id': 'rusty_tiles_large_hlod_v1',
                                                  'classes': {'source_primitive': SP_CLASS}},
                                       'propertyTables': [{'class': 'source_primitive', 'count': 1,
                                                           'properties': cols}]}}
    prim = {'attributes': {'POSITION': pos, '_FEATURE_ID_0': fid}, 'indices': idx, 'mode': 4,
            'extensions': {'EXT_mesh_features': {'featureIds': [
                {'featureCount': 1, 'attribute': 0, 'propertyTable': 0, 'label': 'source_primitive'}]}}}
    return d.finish(dict(scenes=[{'nodes': list(range(instances))}], nodes=[{'mesh': 0}] * instances,
                         meshes=[{'primitives': [prim]}], extensions=ext,
                         extensionsUsed=['EXT_mesh_features', 'EXT_structural_metadata']), pad)


def raw(data, n, stride):
    import hashlib as h
    out = TMP / ('raw-' + h.sha256(data).hexdigest()[:16] + f'-{n}-{stride}')
    if not out.exists():
        pathlib.Path(str(out) + '.raw').write_bytes(data)
        subprocess.run(['node', str(HERE / 'enc.mjs'), 'raw', str(n), str(stride), str(out)], check=True, stdout=subprocess.DEVNULL)
    return out.read_bytes()


ST_CLASS = {'properties': {k: {'componentType': t, 'required': True, 'type': 'SCALAR'} for k, t in
                           [('attribute_flags', 'UINT8'), ('degenerate', 'UINT8'), ('expanded_ordinal', 'UINT64'),
                            ('material_index', 'UINT32'), ('mesh_index', 'UINT32'), ('node_index', 'UINT32'),
                            ('primitive_index', 'UINT32'), ('triangle_index', 'UINT64'), ('vertex_0', 'UINT64'),
                            ('vertex_1', 'UINT64'), ('vertex_2', 'UINT64')]}}


def hlodleaf(n, pad):
    """Leaf-like identity: n triangles, distinct vertices, a source_triangle table whose rows
    name 3n distinct original vertices (one SQLite insert each)."""
    d = Doc()
    v = 3 * n
    pos = d.accessor(d.meshopt(enc('zeros', v, 12), v, 12, 'ATTRIBUTES', 34962), 5126, 'VEC3', v,
                     min=[0, 0, 0], max=[0, 0, 0])
    f0 = d.accessor(d.meshopt(enc('zeros', v, 4), v, 4, 'ATTRIBUTES', 34962), 5126, 'SCALAR', v)
    rows = struct.pack(f'<{v}f', *[float(c // 3) for c in range(v)])
    f1 = d.accessor(d.meshopt(raw(rows, v, 4), v, 4, 'ATTRIBUTES', 34962), 5126, 'SCALAR', v)
    idx = d.accessor(d.meshopt(enc('tri-seq', n, 4), 3 * n, 4, 'TRIANGLES', 34963), 5125, 'SCALAR', 3 * n)
    sp = {k: {'values': d.plain(b'\0' * w)} for k, w in [('expanded_ordinal', 8), ('material_index', 4),
                                                         ('mesh_index', 4), ('node_index', 4), ('primitive_index', 4)]}
    st = {}
    for k, t in [('attribute_flags', 1), ('degenerate', 1)]:
        st[k] = {'values': d.plain(b'\0' * n)}
    for k in ['material_index', 'mesh_index', 'node_index', 'primitive_index']:
        st[k] = {'values': d.meshopt(enc('zeros', n, 4), n, 4, 'ATTRIBUTES', None)}
    st['expanded_ordinal'] = {'values': d.meshopt(enc('zeros', n, 8), n, 8, 'ATTRIBUTES', None)}
    st['triangle_index'] = {'values': d.meshopt(raw(struct.pack(f'<{n}Q', *range(n)), n, 8), n, 8, 'ATTRIBUTES', None)}
    for j in range(3):
        st[f'vertex_{j}'] = {'values': d.meshopt(raw(struct.pack(f'<{n}Q', *range(j, 3 * n, 3)), n, 8), n, 8, 'ATTRIBUTES', None)}
    for view in d.views:
        if view.get('target') is None: view.pop('target', None)
    ext = {'EXT_structural_metadata': {'schema': {'id': 'rusty_tiles_large_source_v1',
                                                  'classes': {'source_primitive': SP_CLASS, 'source_triangle': ST_CLASS}},
                                       'propertyTables': [{'class': 'source_primitive', 'count': 1, 'properties': sp},
                                                          {'class': 'source_triangle', 'count': n, 'properties': st}]}}
    prim = {'attributes': {'POSITION': pos, '_FEATURE_ID_0': f0, '_FEATURE_ID_1': f1}, 'indices': idx, 'mode': 4,
            'extensions': {'EXT_mesh_features': {'featureIds': [
                {'featureCount': 1, 'attribute': 0, 'propertyTable': 0, 'label': 'source_primitive'},
                {'featureCount': n, 'attribute': 1, 'propertyTable': 1, 'label': 'source_triangle'}]}}}
    return d.finish(dict(scenes=[{'nodes': [0]}], nodes=[{'mesh': 0}], meshes=[{'primitives': [prim]}],
                         extensions=ext, extensionsUsed=['EXT_mesh_features', 'EXT_structural_metadata']), pad)


def hugecount(pad, extra):
    d = Doc()
    pos = d.accessor(d.plain(TRI, target=34962), 5126, 'VEC3', 3, min=[0, 0, 0], max=[1, 1, 0])
    z = enc('zeros', 16, 4)
    view = d.meshopt(z, 16, 4, 'ATTRIBUTES', 34962)
    d.views[view]['extensions']['EXT_meshopt_compression']['count'] = 2 ** 32 - 1
    d.views[view]['byteLength'] = 4 * (2 ** 32 - 1)
    d.accessor(view, 5121, 'VEC4', 2 ** 32 - 1)
    return d.finish(dict(scenes=[{'nodes': [0]}], nodes=[{'mesh': 0}],
                         meshes=[{'primitives': [{'attributes': {'POSITION': pos}, 'mode': 4}]}]), pad)


def main():
    kind, n_glbs, out = sys.argv[1], int(sys.argv[2]), pathlib.Path(sys.argv[3])
    pad = int(sys.argv[4]) if len(sys.argv) > 4 else 0
    extra = int(sys.argv[5]) if len(sys.argv) > 5 else 0
    TMP.mkdir(parents=True, exist_ok=True)
    make = {'comp64': lambda: comp64(pad, extra),
            'hlodseq': lambda: hlod(extra or 262_144, 'tri-seq', 1, pad),
            'hloddup': lambda: hlod(100_000, 'tri-repeat', extra or 1, pad),
            'hugecount': lambda: hugecount(pad, extra),
            'hlodleaf': lambda: hlodleaf(extra or 100_000, pad)}[kind]
    data = make()
    children = [{'boundingVolume': {'box': [0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0, 2]}, 'geometricError': 0,
                 'refine': 'REPLACE', 'content': {'uri': f'g{i}.glb'}} for i in range(n_glbs)]
    ts = json.dumps({'asset': {'version': '1.1'}, 'geometricError': 1,
                     'root': {'boundingVolume': {'box': [0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0, 2]}, 'geometricError': 1,
                              'refine': 'REPLACE', 'children': children}}).encode()
    # Distinct payload bytes per GLB (asset.generator differs) so no byte-identity shortcut applies.
    global SERIAL
    members = [('tileset.json', ts)]
    for i in range(n_glbs):
        SERIAL = i
        members.append((f'g{i}.glb', make()))
    records = []
    with zipfile.ZipFile(out, 'w') as z:
        for name, blob in members:
            info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0)); info.external_attr = 0o100644 << 16
            info.compress_type = zipfile.ZIP_STORED
            z.writestr(info, blob); records.append((hashlib.md5(name.encode()).digest(), z.getinfo(name).header_offset))
        records.sort(key=lambda p: struct.unpack('<QQ', p[0]))
        info = zipfile.ZipInfo('@3dtilesIndex1@', (1980, 1, 1, 0, 0, 0)); info.external_attr = 0o100644 << 16
        z.writestr(info, b''.join(h + struct.pack('<Q', o) for h, o in records))
    stored = sum(i.compress_size for i in zipfile.ZipFile(out).infolist())
    print(json.dumps(dict(kind=kind, glbs=n_glbs, glb_bytes=len(data), stored=stored,
                          budget_dev=max(16_000_000, 16 * stored), budget_br=max(16_000_000, 25 * stored))))



