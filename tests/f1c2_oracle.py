#!/usr/bin/env python3
"""Independent F1c2 source/metadata oracle. Standard library; no Rust imports.

Source truth uses authored node/mesh/primitive indices and TRIANGLES ordinals.
It never reconstructs source identities from emitted coordinates or row IDs.
All generated fixtures, source and artifact digests accompany executed results.
"""
import argparse
from collections import Counter
import copy
import hashlib
import json
import math
from pathlib import Path
import struct
import subprocess
import tempfile
import zipfile

PROFILE = 'f1c2-source-identity-gltf-v1'
LABELS = ('source_primitive', 'source_triangle')
KEYS = ('node_index', 'mesh_index', 'primitive_index', 'triangle_index')
SCHEMAS = (
    dict(node_index='UINT32', mesh_index='UINT32', primitive_index='UINT32',
         node_name_present='UINT8', node_name='STRING'),
    dict(node_index='UINT32', mesh_index='UINT32', primitive_index='UINT32', triangle_index='UINT32'),
)
IDENTITY = (1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.)
PLACED = ['--anchor', '153', '-27', '130', '--orientation-xyzw', '0', '0', '0', '1',
          '--scene-offset', '11', '13', '17']


class OracleError(Exception):
    pass


def require(ok, message):
    if not ok:
        raise OracleError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def glb(doc, binary):
    doc = copy.deepcopy(doc)
    payload = bytes(binary) + b'\0' * (-len(binary) % 8)
    doc['buffers'] = [{'byteLength': len(payload)}]
    text = json.dumps(doc, ensure_ascii=False, separators=(',', ':')).encode()
    text += b' ' * (-(20 + len(text) + 8) % 8)
    total = 12 + 8 + len(text) + 8 + len(payload)
    return (struct.pack('<4sII', b'glTF', 2, total) + struct.pack('<I4s', len(text), b'JSON') +
            text + struct.pack('<I4s', len(payload), b'BIN\0') + payload)


def decode(data):
    require(data[:4] == b'glTF' and struct.unpack_from('<II', data, 4) == (2, len(data)), 'GLB envelope')
    n, kind = struct.unpack_from('<I4s', data, 12)
    require(kind == b'JSON' and n % 4 == 0, 'GLB JSON chunk')
    doc = json.loads(data[20:20+n])
    size, kind = struct.unpack_from('<I4s', data, 20+n)
    start = 28+n
    require(kind == b'BIN\0' and start+size == len(data), 'GLB BIN chunk')
    return doc, data[start:start+size], start


def terminal_overpadding(data):
    """Raw fixture construction: four extra BIN bytes outside declared buffer."""
    doc, payload, _ = decode(data)
    payload += b'\0'*8
    doc['buffers'][0]['byteLength'] = len(payload)-4
    text = json.dumps(doc, ensure_ascii=False, separators=(',', ':')).encode()
    text += b' '*(-len(text) % 4)
    return (struct.pack('<4sII', b'glTF', 2, 28+len(text)+len(payload)) +
            struct.pack('<I4s', len(text), b'JSON') + text +
            struct.pack('<I4s', len(payload), b'BIN\0') + payload)


def view(doc, binary, i):
    item = doc['bufferViews'][i]
    require(item.get('buffer', 0) == 0, 'view owns embedded buffer')
    start, n = item.get('byteOffset', 0), item['byteLength']
    require(n >= 1 and 0 <= start <= start+n <= doc['buffers'][0]['byteLength'] <= len(binary), 'nonempty view range')
    return binary[start:start+n]


def accessor(doc, binary, i):
    item = doc['accessors'][i]
    fmt, size = {5121: ('B', 1), 5123: ('H', 2), 5125: ('I', 4), 5126: ('f', 4)}[item['componentType']]
    width = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4}[item['type']]
    raw = view(doc, binary, item['bufferView'])
    stride = doc['bufferViews'][item['bufferView']].get('byteStride', width*size)
    offset = item.get('byteOffset', 0)
    require(offset + max(0, item['count']-1)*stride + width*size <= len(raw), 'accessor range')
    values = [struct.unpack_from('<'+fmt*width, raw, offset+j*stride) for j in range(item['count'])]
    if item.get('normalized'):
        divisor = {5121: 255., 5123: 65535.}[item['componentType']]
        values = [tuple(x/divisor for x in value) for value in values]
    return [v[0] for v in values] if width == 1 else values


def fixture(variant='instances', count=None):
    doc = {'asset': {'version': '2.0'}, 'scene': 1, 'scenes': [{'nodes': [0]}, {'nodes': [4, 2, 5, 7, 3, 1]}],
           'nodes': [{'mesh': 0, 'name': 'UNSELECTED'}, {'mesh': 0},
                     {'mesh': 0, 'name': '', 'translation': [8, 0, 0]},
                     {'mesh': 0, 'name': 'duplicate', 'translation': [16, 0, 0]},
                     {'mesh': 0, 'name': 'duplicate', 'translation': [24, 0, 0], 'scale': [-1, 1, 1]},
                     {'children': [6], 'translation': [0, 0, 8]},
                     {'mesh': 1, 'name': ' exact ♥ é e\u0301 ', 'translation': [32, 0, 0]},
                     {'mesh': 0, 'name': 'coincident'}],
           'meshes': [{'primitives': []}, {'primitives': []}], 'bufferViews': [], 'accessors': [],
           'materials': [{'pbrMetallicRoughness': {'baseColorFactor': [.2, .4, .6, 1.]}, 'doubleSided': True},
                         {'pbrMetallicRoughness': {'baseColorFactor': [.8, .2, .1, 1.]}, 'doubleSided': True}]}
    binary = bytearray()

    def store(values, shape, component=5126):
        binary.extend(b'\0' * (-len(binary) % 4))
        start = len(binary)
        fmt = {5126: 'f', 5125: 'I'}[component]
        rows = [(value,) if not isinstance(value, (tuple, list)) else value for value in values]
        for row in rows:
            binary.extend(struct.pack('<'+fmt*len(row), *row))
        v = len(doc['bufferViews'])
        doc['bufferViews'].append({'buffer': 0, 'byteOffset': start, 'byteLength': len(binary)-start})
        a = {'bufferView': v, 'componentType': component, 'count': len(values), 'type': shape}
        if shape == 'VEC3':
            a['min'] = [min(row[c] for row in rows) for c in range(3)]
            a['max'] = [max(row[c] for row in rows) for c in range(3)]
        i = len(doc['accessors'])
        doc['accessors'].append(a)
        return i

    positions = store([(-1, 0, -1), (1, 0, -1), (-1, 0, 1), (1, 0, 1)], 'VEC3')
    indices = store([0, 1, 2, 1, 3, 2], 'SCALAR', 5125)
    p = {'attributes': {'POSITION': positions}, 'indices': indices, 'material': 0}
    doc['meshes'][0]['primitives'] = [p, dict(copy.deepcopy(p), material=1)]
    other = store([(2, 0, -1), (4, 0, -1), (2, 0, 1), (4, 0, -1), (4, 0, 1), (2, 0, 1)], 'VEC3')
    color = store([(1, .25, .5, 1), (.5, .75, .25, .75), (.25, .5, .75, .5),
                   (.125, .25, .5, .25), (.5, .125, .25, .625), (.625, .5, .125, .875)], 'VEC4')
    doc['meshes'][0]['primitives'].append({'attributes': {'POSITION': other, 'COLOR_0': color}, 'material': 0})
    doc['meshes'][1]['primitives'] = [copy.deepcopy(p)]
    if variant in ('viewer', 'empty-names'):
        doc['nodes'] = [{'mesh': 0}, {'mesh': 0, 'name': '', 'translation': [10, 0, 0], 'scale': [-1, 1, 1]},
                        {'mesh': 0, 'name': ' exact ♥ é e\u0301 ', 'translation': [20, 0, 0]}]
        doc['scenes'] = [{'nodes': [2, 1, 0]}]
        doc['scene'] = 0
        # Two separated primitives of identical material, unlike geometry
        # grouping by source primitive. Both indexed and unindexed triangles.
        doc['meshes'][0]['primitives'] = [p, {'attributes': {'POSITION': other}, 'material': 1}]
        doc['meshes'] = doc['meshes'][:1]
        if variant == 'empty-names':
            doc['nodes'][2].pop('name')
    elif variant in ('large-id', 'name-budget'):
        doc['nodes'] = [{'mesh': 0, 'name': 'é'*2048 if variant == 'name-budget' else 'large'}]
        doc['scenes'] = [{'nodes': [0]}]
        doc['scene'] = 0
        doc['meshes'] = doc['meshes'][:1]
        p = copy.deepcopy(p)
        p['indices'] = store([0, 1, 2]*(count if variant == 'large-id' else 1), 'SCALAR', 5125)
        doc['meshes'][0]['primitives'] = [p] if variant == 'large-id' else [copy.deepcopy(p) for _ in range(count)]
        # Replace, rather than retain, the old index accessor: admission requires
        # every UINT32 accessor to be used as primitive indices.
        doc['accessors'].pop(indices)
        for primitive in doc['meshes'][0]['primitives']:
            primitive['indices'] -= primitive['indices'] > indices
            primitive['attributes'] = {k: a-(a > indices) for k, a in primitive['attributes'].items()}
    elif variant == 'first-scene':
        doc.pop('scene')
        doc['scenes'] = doc['scenes'][:1]
    elif variant != 'instances':
        raise OracleError('unknown source variant')
    return glb(doc, binary)


def matrix(node):
    if 'matrix' in node:
        return node['matrix']
    require('rotation' not in node, 'reference fixture uses analytic translation/reflection, no quaternion decoder')
    t, s = node.get('translation', [0, 0, 0]), node.get('scale', [1, 1, 1])
    return [s[0], 0, 0, 0, 0, s[1], 0, 0, 0, 0, s[2], 0, *t, 1]


def multiply(a, b):
    return [sum(a[k*4+r]*b[c*4+k] for k in range(4)) for c in range(4) for r in range(4)]


def point(m, p):
    return tuple(sum(m[c*4+r]*p[c] for c in range(3))+m[12+r] for r in range(3))


def expected_source(data):
    doc, binary, _ = decode(data)
    truth = {}
    names = {}

    def visit(n, parent):
        node = doc['nodes'][n]
        world = multiply(parent, matrix(node))
        for child in node.get('children', []):
            visit(child, world)
        if 'mesh' not in node:
            return
        mesh = node['mesh']
        names[n] = (int('name' in node), node.get('name', ''))
        sign = world[0]*(world[5]*world[10]-world[9]*world[6])-world[4]*(world[1]*world[10]-world[9]*world[2])+world[8]*(world[1]*world[6]-world[5]*world[2])
        for primitive_index, primitive in enumerate(doc['meshes'][mesh]['primitives']):
            positions = accessor(doc, binary, primitive['attributes']['POSITION'])
            colors = accessor(doc, binary, primitive['attributes']['COLOR_0']) if 'COLOR_0' in primitive['attributes'] else None
            indices = accessor(doc, binary, primitive['indices']) if 'indices' in primitive else list(range(len(positions)))
            for triangle in range(len(indices)//3):
                ids = indices[triangle*3:triangle*3+3]
                if sign < 0:
                    ids[1], ids[2] = ids[2], ids[1]
                corners = [point(world, positions[i]) for i in ids]
                truth[(n, mesh, primitive_index, triangle)] = {
                    'positions': corners,
                    'colors': [colors[i] for i in ids] if colors else None,
                    'material': doc['materials'][primitive['material']],
                }

    for root in doc['scenes'][doc.get('scene', 0)]['nodes']:
        visit(root, IDENTITY)
    return truth, names


def tables(doc, binary):
    ext = doc['extensions']['EXT_structural_metadata']
    require(len(ext['propertyTables']) == 2, 'exactly two property tables')
    rows = []
    for t, expected in enumerate(SCHEMAS):
        table = ext['propertyTables'][t]
        require(table['class'] == LABELS[t] and table['count'] > 0, 'table class/count')
        schema = ext['schema']['classes'][LABELS[t]]['properties']
        require(set(schema) == set(expected) == set(table['properties']), 'exact schema/column fields')
        columns = {}
        for name, component in expected.items():
            declaration = schema[name]
            require(declaration.get('required') is True, 'required schema column '+name)
            require(declaration['type'] == ('STRING' if component == 'STRING' else 'SCALAR'), 'schema type '+name)
            prop = table['properties'][name]
            raw = view(doc, binary, prop['values'])
            require(doc['bufferViews'][prop['values']].get('byteOffset', 0) % 8 == 0, 'new view eight-byte alignment')
            if component == 'STRING':
                require(prop.get('stringOffsetType', 'UINT32') == 'UINT32', 'string offset component')
                offsets = view(doc, binary, prop['stringOffsets'])
                require(doc['bufferViews'][prop['stringOffsets']].get('byteOffset', 0) % 8 == 0, 'offset view alignment')
                require(len(offsets) == 4*(table['count']+1), 'count+1 string offsets')
                offsets = struct.unpack('<'+'I'*(table['count']+1), offsets)
                empty_sentinel = all(offset == 0 for offset in offsets) and raw == b'\0'
                require(offsets[0] == 0 and (offsets[-1] == len(raw) or empty_sentinel), 'complete exact string storage or exact empty sentinel')
                require(all(a <= b for a, b in zip(offsets, offsets[1:])), 'monotone string offsets')
                columns[name] = [raw[a:b].decode('utf8') for a, b in zip(offsets, offsets[1:])]
            else:
                require(declaration.get('componentType') == component, 'numeric schema component '+name)
                size, fmt = (4, 'I') if component == 'UINT32' else (1, 'B')
                require(len(raw) == table['count']*size, 'numeric column byte length')
                columns[name] = struct.unpack('<'+fmt*table['count'], raw)
        table_rows = [{name: values[i] for name, values in columns.items()} for i in range(table['count'])]
        keys = [tuple(row[k] for k in KEYS[:3+t]) for row in table_rows]
        require(keys == sorted(set(keys)), 'dense unique sorted source keys')
        rows.append(table_rows)
    return rows


def inspect_members(source, members, leaf_limit):
    truth, names = expected_source(source)
    remaining = set(truth)
    manifest = json.loads(members['tileset.json'])
    report = json.loads(members['conversion.json'])
    require(report['schema_version'] == 5 and report['profile'] == PROFILE, 'report profile/schema')
    leaves = []

    def visit(tile):
        if 'children' in tile:
            for child in tile['children']:
                visit(child)
        elif 'content' in tile:
            leaves.append(tile['content']['uri'])
    visit(manifest['root'])
    total_name_bytes = 0
    source_rows = Counter()
    for uri in leaves:
        doc, binary, start = decode(members[uri])
        require(start % 8 == 0 and len(binary) % 8 == 0, 'absolute BIN/padding alignment')
        require(0 <= len(binary)-doc['buffers'][0]['byteLength'] <= 3, 'core GLB terminal padding limit')
        require(all(x == 0 for x in binary[doc['buffers'][0]['byteLength']:]), 'terminal BIN padding zeros')
        require(set(LABELS) <= set(doc['extensions']['EXT_structural_metadata']['schema']['classes']), 'schema classes')
        require({'EXT_mesh_features', 'EXT_structural_metadata'} <= set(doc['extensionsUsed']), 'extensionsUsed')
        require(not {'EXT_mesh_features', 'EXT_structural_metadata'} & set(doc.get('extensionsRequired', [])), 'extensions optional')
        rows = tables(doc, binary)
        used = [set(), set()]
        leaf_triangles = 0
        for primitive in doc['meshes'][0]['primitives']:
            attrs = primitive['attributes']
            positions = accessor(doc, binary, attrs['POSITION'])
            colors = accessor(doc, binary, attrs['COLOR_0']) if 'COLOR_0' in attrs else None
            ids = []
            sets = primitive['extensions']['EXT_mesh_features']['featureIds']
            require(len(sets) == 2, 'two feature ID sets per encoded primitive')
            for t in range(2):
                require(set(sets[t]) == {'featureCount', 'attribute', 'propertyTable', 'label'} and
                        {k: v for k, v in sets[t].items() if k != 'featureCount'} ==
                        {'attribute': t, 'propertyTable': t, 'label': LABELS[t]}, 'exact feature set '+str(t))
                layout = doc['accessors'][attrs['_FEATURE_ID_'+str(t)]]
                require(layout['type'] == 'SCALAR' and layout['componentType'] == 5126 and not layout.get('normalized'), 'feature accessor scalar float')
                require(layout['count'] == len(positions), 'feature accessor corner count')
                require(doc['bufferViews'][layout['bufferView']].get('byteOffset', 0) % 8 == 0, 'feature view alignment')
                values = accessor(doc, binary, attrs['_FEATURE_ID_'+str(t)])
                require(all(math.isfinite(x) and int(x) == x and 0 <= x < len(rows[t]) for x in values), 'feature IDs integral/in-range')
                require(sets[t]['featureCount'] == len(set(values)), 'per-primitive unique feature cardinality')
                ids.append([int(x) for x in values])
            indices = accessor(doc, binary, primitive['indices']) if 'indices' in primitive else list(range(len(positions)))
            for j in range(0, len(indices), 3):
                corners = indices[j:j+3]
                require(len(corners) == 3, 'TRIANGLES index multiplicity')
                row_ids = []
                for t in range(2):
                    require(len({ids[t][i] for i in corners}) == 1, 'uniform triangle corner IDs')
                    row_ids.append(ids[t][corners[0]])
                    used[t].add(row_ids[t])
                rp, rt = rows[0][row_ids[0]], rows[1][row_ids[1]]
                key = tuple(rt[k] for k in KEYS)
                require(tuple(rp[k] for k in KEYS[:3]) == key[:3], 'primitive/triangle associations agree')
                require(key in remaining, 'source identity missing/duplicate/invented '+str(key))
                require((rp['node_name_present'], rp['node_name']) == names[key[0]], 'exact optional source name')
                original = truth[key]
                # Complete corners with cyclic winding are compared together;
                # reflections must retain the original triangle ordinal.
                wanted = original['positions']
                actual = [positions[i] for i in corners]
                rotations = [shift for shift in range(3) if all(max(abs(a-b) for a, b in zip(actual[(k+shift)%3], wanted[k])) <= 1e-6 for k in range(3))]
                require(rotations, 'oriented source geometry/key association '+str(key))
                require(doc['materials'][primitive['material']] == original['material'], 'source material/key association')
                require((colors is None) == (original['colors'] is None), 'source companion layout')
                if colors:
                    shift = rotations[0]
                    require(all(tuple(colors[corners[(k+shift)%3]]) == tuple(original['colors'][k]) for k in range(3)), 'source color/key corner association')
                remaining.remove(key)
                source_rows[key] += 1
                leaf_triangles += 1
        require(0 < leaf_triangles <= leaf_limit, 'leaf triangle ceiling')
        require(used == [set(range(len(table))) for table in rows], 'all dense property rows actually referenced')
        total_name_bytes += sum(len(row['node_name'].encode()) for row in rows[0])
    require(not remaining, 'omitted source triangles/instances '+str(sorted(remaining)[:5]))
    require(total_name_bytes <= 8*1024*1024, 'archive emitted name byte cap')
    return {'source_sha256': digest(source), 'triangles': len(truth), 'leaves': len(leaves),
            'source_primitive_instances': len({k[:3] for k in truth}), 'emitted_name_bytes': total_name_bytes,
            'keys_sha256': digest(json.dumps(sorted(source_rows)).encode()), 'report': report}


def corruptions(members):
    leaf = next(name for name in members if name.endswith('.glb'))
    original, payload, _ = decode(members[leaf])
    variants = {}
    for label in ('swap-feature-sets', 'wrong-table', 'accessor-component', 'accessor-count', 'presence', 'string-offset', 'triangle-ordinal', 'triangle-id', 'source-column-swap', 'reflected-id-swap'):
        doc, binary = copy.deepcopy(original), bytearray(payload[:original['buffers'][0]['byteLength']])
        p = doc['meshes'][0]['primitives'][0]
        table = doc['extensions']['EXT_structural_metadata']['propertyTables']
        if label == 'swap-feature-sets':
            p['attributes']['_FEATURE_ID_0'], p['attributes']['_FEATURE_ID_1'] = p['attributes']['_FEATURE_ID_1'], p['attributes']['_FEATURE_ID_0']
        elif label == 'wrong-table':
            p['extensions']['EXT_mesh_features']['featureIds'][0]['propertyTable'] = 1
        elif label == 'accessor-component':
            doc['accessors'][p['attributes']['_FEATURE_ID_1']]['componentType'] = 5125
        elif label == 'accessor-count':
            doc['accessors'][p['attributes']['_FEATURE_ID_1']]['count'] -= 1
        elif label in ('presence', 'string-offset', 'triangle-ordinal'):
            t, column, field, fmt, value = {
                'presence': (0, 'node_name_present', 'values', 'B', 2),
                'string-offset': (0, 'node_name', 'stringOffsets', 'I', 1),
                'triangle-ordinal': (1, 'triangle_index', 'values', 'I', 999999),
            }[label]
            v = doc['bufferViews'][table[t]['properties'][column][field]]
            struct.pack_into('<'+fmt, binary, v.get('byteOffset', 0), value)
        elif label == 'triangle-id':
            a = doc['accessors'][p['attributes']['_FEATURE_ID_1']]
            v = doc['bufferViews'][a['bufferView']]
            start = v.get('byteOffset', 0)+a.get('byteOffset', 0)
            struct.pack_into('<f', binary, start, float(len(table[1]['properties'])+100000))
        elif label == 'source-column-swap':
            props = table[1]['properties']
            props['node_index'], props['mesh_index'] = props['mesh_index'], props['node_index']
        elif label == 'reflected-id-swap':
            rows = tables(doc, binary)[1]
            keys = [tuple(row[k] for k in KEYS) for row in rows]
            swaps = {i: keys.index(key[:3]+(1-key[3],)) for i, key in enumerate(keys)
                     if key[0] == 4 and key[3] in (0, 1) and key[:3]+(1-key[3],) in keys}
            require(swaps, 'reflected ordinal sensitivity has both source triangles')
            for primitive in doc['meshes'][0]['primitives']:
                a = doc['accessors'][primitive['attributes']['_FEATURE_ID_1']]
                v = doc['bufferViews'][a['bufferView']]
                stride = v.get('byteStride', 4)
                start = v.get('byteOffset', 0)+a.get('byteOffset', 0)
                for i in range(a['count']):
                    value = int(struct.unpack_from('<f', binary, start+i*stride)[0])
                    if value in swaps:
                        struct.pack_into('<f', binary, start+i*stride, swaps[value])
        changed = dict(members)
        changed[leaf] = glb(doc, binary)
        variants[label] = changed
    changed = dict(members)
    doc = json.loads(changed['tileset.json'])
    # Missing one leaf is a source multiplicity control, not codec corruption.
    if doc['root'].get('children'):
        doc['root']['children'].pop()
        changed['tileset.json'] = json.dumps(doc).encode()
        variants['missing-leaf'] = changed
    return variants


def synthetic_members(source):
    """Independently author two shared tables, sparse per-material row ID sets.

    Used only to establish decoder/real consumer feasibility and sensitivity.
    Synthetic outputs are never labeled candidate-conversion acceptance.
    """
    truth, names = expected_source(source)
    primitive_keys = sorted({key[:3] for key in truth})
    triangle_keys = sorted(truth)
    binary = bytearray()
    doc = {'asset': {'version': '2.0'}, 'scene': 0, 'scenes': [{'nodes': [0]}], 'nodes': [{'mesh': 0}],
           'meshes': [{'primitives': []}], 'accessors': [], 'bufferViews': [], 'materials': [],
           'extensionsUsed': ['EXT_mesh_features', 'EXT_structural_metadata']}

    def storage(raw):
        binary.extend(b'\0'*(-len(binary) % 8))
        v = len(doc['bufferViews'])
        doc['bufferViews'].append({'buffer': 0, 'byteOffset': len(binary), 'byteLength': len(raw)})
        binary.extend(raw)
        return v

    def array(values, shape):
        rows = [(v,) if not isinstance(v, (list, tuple)) else v for v in values]
        raw = b''.join(struct.pack('<'+'f'*len(row), *row) for row in rows)
        a = {'bufferView': storage(raw), 'componentType': 5126, 'count': len(values), 'type': shape}
        if shape == 'VEC3':
            a.update(min=[min(row[c] for row in rows) for c in range(3)], max=[max(row[c] for row in rows) for c in range(3)])
        i = len(doc['accessors'])
        doc['accessors'].append(a)
        return i

    groups = {}
    for key in reversed(triangle_keys):
        record = truth[key]
        group = (json.dumps(record['material'], sort_keys=True), record['colors'] is not None)
        groups.setdefault(group, []).append(key)
    for (material, colors), keys in groups.items():
        positions = [p for key in keys for p in truth[key]['positions']]
        primitive_ids = [primitive_keys.index(key[:3]) for key in keys for _ in range(3)]
        triangle_ids = [triangle_keys.index(key) for key in keys for _ in range(3)]
        attrs = {'POSITION': array(positions, 'VEC3'),
                 '_FEATURE_ID_0': array(primitive_ids, 'SCALAR'), '_FEATURE_ID_1': array(triangle_ids, 'SCALAR')}
        if colors:
            attrs['COLOR_0'] = array([p for key in keys for p in truth[key]['colors']], 'VEC4')
        mat = len(doc['materials'])
        doc['materials'].append(json.loads(material))
        doc['meshes'][0]['primitives'].append({'attributes': attrs, 'material': mat,
            'extensions': {'EXT_mesh_features': {'featureIds': [
                {'featureCount': len(set(ids)), 'attribute': t, 'propertyTable': t, 'label': LABELS[t]}
                for t, ids in enumerate((primitive_ids, triangle_ids))]}}})
    tables_out = []
    schema = {'id': 'independent_f1c2_feasibility', 'classes': {}}
    for t, keys in enumerate((primitive_keys, triangle_keys)):
        columns = {name: [key[KEYS.index(name)] for key in keys] for name in KEYS[:3+t]}
        if t == 0:
            columns['node_name_present'] = [names[key[0]][0] for key in keys]
            columns['node_name'] = [names[key[0]][1] for key in keys]
        props, declarations = {}, {}
        for name, component in SCHEMAS[t].items():
            values = columns[name]
            if component == 'STRING':
                strings = [value.encode() for value in values]
                offsets = [0]
                for raw in strings:
                    offsets.append(offsets[-1]+len(raw))
                props[name] = {'values': storage(b''.join(strings) or b'\0'),
                               'stringOffsets': storage(struct.pack('<'+'I'*len(offsets), *offsets)), 'stringOffsetType': 'UINT32'}
                declarations[name] = {'type': 'STRING', 'required': True}
            else:
                fmt = 'I' if component == 'UINT32' else 'B'
                props[name] = {'values': storage(struct.pack('<'+fmt*len(values), *values))}
                declarations[name] = {'type': 'SCALAR', 'componentType': component, 'required': True}
        schema['classes'][LABELS[t]] = {'properties': declarations}
        tables_out.append({'class': LABELS[t], 'count': len(keys), 'properties': props})
    doc['extensions'] = {'EXT_structural_metadata': {'schema': schema, 'propertyTables': tables_out}}
    # A single source-independent local-Z box includes all source Y-up positions
    # after the standard glTF axis map B(x,y,z)=(x,-z,y).
    points = [(p[0], -p[2], p[1]) for record in truth.values() for p in record['positions']]
    low = [min(p[c] for p in points) for c in range(3)]
    high = [max(p[c] for p in points) for c in range(3)]
    center = [(a+b)/2 for a, b in zip(low, high)]
    half = [(b-a)/2 for a, b in zip(low, high)]
    box = center+[half[0], 0, 0, 0, half[1], 0, 0, 0, half[2]]
    manifest = {'asset': {'version': '1.1'}, 'geometricError': 1000,
                'root': {'transform': list(IDENTITY), 'boundingVolume': {'box': box}, 'geometricError': 1000, 'refine': 'REPLACE',
                         'children': [{'boundingVolume': {'box': box}, 'geometricError': 0, 'content': {'uri': 't/0.glb'}}]}}
    return {'t/0.glb': glb(doc, binary), 'tileset.json': json.dumps(manifest).encode(),
            'conversion.json': json.dumps({'schema_version': 5, 'profile': PROFILE}).encode()}


def self_test():
    data = fixture()
    members = synthetic_members(data)
    positive = inspect_members(data, members, 100)
    records = []
    for label, changed in corruptions(members).items():
        try:
            inspect_members(data, changed, 100)
        except (OracleError, UnicodeError, KeyError, IndexError, struct.error) as error:
            records.append({'name': label, 'rejected': True, 'reason': str(error)})
        else:
            raise OracleError('insensitive independent self-test '+label)
    empty_source = fixture('empty-names')
    empty_members = synthetic_members(empty_source)
    empty_positive = inspect_members(empty_source, empty_members, 100)
    leaf = 't/0.glb'
    doc, raw, _ = decode(empty_members[leaf])
    prop = doc['extensions']['EXT_structural_metadata']['propertyTables'][0]['properties']['node_name']
    for label, length, byte in (('empty-sentinel-nonzero', 1, 1), ('empty-sentinel-zero-view', 0, 0),
                                ('empty-sentinel-extra-zero', 2, 0)):
        changed_doc, changed_raw = copy.deepcopy(doc), bytearray(raw)
        v = changed_doc['bufferViews'][prop['values']]
        v['byteLength'] = length
        changed_raw[v['byteOffset']] = byte
        changed = dict(empty_members, **{leaf: glb(changed_doc, changed_raw)})
        try:
            inspect_members(empty_source, changed, 100)
        except OracleError as error:
            records.append({'name': label, 'rejected': True, 'reason': str(error)})
        else:
            raise OracleError('insensitive empty string storage control '+label)
    return {'mode': 'independent-synthetic-decoder-and-control-self-test', 'positive': positive,
            'empty_names_positive': empty_positive, 'controls': records,
            'scope': 'Oracle and representation feasibility only; no candidate binary execution.'}


def run(binary, *, large=False):
    records, controls, refusals = [], [], []
    with tempfile.TemporaryDirectory(prefix='f1c2-identity-oracle-') as temporary:
        root = Path(temporary)
        cases = [('instances', None), ('viewer', None), ('empty-names', None), ('first-scene', None)]
        if large:
            cases += [('large-id', 100000), ('name-budget', 2048)]
        for variant, count in cases:
            data = fixture(variant, count)
            source = root/(variant+'.glb')
            source.write_bytes(data)
            ceilings = (1, 5, 100) if count is None else (100000,)
            keys = None
            for ceiling in ceilings:
                for placement in ('local', 'wgs84') if count is None else ('local',):
                    archive = root/(variant+'-'+str(ceiling)+'-'+placement+'.3tz')
                    command = [str(binary), '--json', 'mesh-local-to-3tz', '-i', str(source), '-o', str(archive), '--leaf-triangles', str(ceiling)]
                    if placement == 'wgs84':
                        command += PLACED
                    completed = subprocess.run(command, text=True, capture_output=True, timeout=120)
                    require(completed.returncode == 0, 'candidate conversion '+variant+' '+str(ceiling)+' '+placement+': '+completed.stdout+completed.stderr)
                    with zipfile.ZipFile(archive) as stream:
                        members = {n: stream.read(n) for n in stream.namelist() if n != '@3dtilesIndex1@'}
                    inspected = inspect_members(data, members, ceiling)
                    require(json.loads(completed.stdout)['meshReport'] == inspected['report'], 'CLI/published report parity')
                    require(keys is None or keys == inspected['keys_sha256'], 'partition/placement source key stability')
                    keys = inspected['keys_sha256']
                    inspected.update(variant=variant, leaf_limit=ceiling, placement=placement, archive_sha256=digest(archive.read_bytes()))
                    records.append(inspected)
                    if variant == 'instances' and ceiling == 100 and placement == 'local':
                        for label, changed in corruptions(members).items():
                            try:
                                inspect_members(data, changed, ceiling)
                            except (OracleError, UnicodeError, KeyError, IndexError, struct.error) as error:
                                controls.append({'name': label, 'rejected': True, 'reason': str(error)})
                            else:
                                raise OracleError('insensitive corruption control '+label)
        # These controls retain a valid source envelope and test source admission
        # before staging. Multiple parents are excluded by the inherited profile.
        doc, raw, _ = decode(fixture())
        cases = [('name-4097-bytes', 'unsupported'), ('malformed-name', 'invalid_input'), ('multiple-parents', 'invalid_input'),
                 ('ambiguous-scenes', 'unsupported'), ('terminal-four-padding', 'invalid_input')]
        for label, kind in cases:
            changed = copy.deepcopy(doc)
            if label == 'name-4097-bytes':
                changed['nodes'][1]['name'] = 'é'*2048+'x'
            elif label == 'malformed-name':
                changed['nodes'][1]['name'] = 7
            elif label == 'multiple-parents':
                changed['nodes'][1]['children'] = [6]
            elif label == 'ambiguous-scenes':
                changed.pop('scene')
            source = root/(label+'.glb')
            data = glb(changed, raw[:doc['buffers'][0]['byteLength']])
            source.write_bytes(terminal_overpadding(data) if label == 'terminal-four-padding' else data)
            output = root/label/'result.3tz'
            completed = subprocess.run([str(binary), '--json', 'mesh-local-to-3tz', '-i', str(source), '-o', str(output), '--leaf-triangles', '100'], text=True, capture_output=True, timeout=60)
            summary = json.loads(completed.stdout)
            require(completed.returncode != 0 and summary['error']['kind'] == kind, 'typed source refusal '+label+': '+completed.stdout)
            require(not output.parent.exists(), 'source refusal creates no staging/output parent')
            refusals.append({'name': label, 'kind': kind, 'source_sha256': digest(source.read_bytes()), 'output_parent_created': False})
        if large:
            source = root/'name-budget-over.glb'
            source.write_bytes(fixture('name-budget', 2049))
            output = root/'name-budget-over'/'result.3tz'
            completed = subprocess.run([str(binary), '--json', 'mesh-local-to-3tz', '-i', str(source), '-o', str(output), '--leaf-triangles', '100000'], text=True, capture_output=True, timeout=120)
            summary = json.loads(completed.stdout)
            require(completed.returncode != 0 and summary['error']['kind'] == 'unsupported', 'emitted name budget refusal '+completed.stdout)
            require(not output.parent.exists(), 'emitted name budget refused before staging')
            refusals.append({'name': 'name-budget-8MiB-plus4096', 'kind': 'unsupported', 'source_sha256': digest(source.read_bytes()), 'output_parent_created': False})
    return {'binary': str(binary), 'binary_sha256': digest(binary.read_bytes()), 'positive_cases': records,
            'sensitive_controls': controls, 'source_refusals': refusals,
            'scope': 'Finite authored static local source/identity/metadata cases; inherited prior PBR/placement/closure and lifecycle evidence remains independently required.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--self-test', action='store_true')
    parser.add_argument('--large', action='store_true')
    parser.add_argument('--fixture', type=Path)
    parser.add_argument('--variant', choices=('instances', 'viewer', 'empty-names', 'first-scene', 'large-id', 'name-budget'), default='instances')
    parser.add_argument('--count', type=int)
    parser.add_argument('--json-output', type=Path)
    args = parser.parse_args()
    result = {'oracle_sha256': digest(Path(__file__).read_bytes())}
    if args.self_test:
        result['self_test'] = self_test()
    if args.fixture:
        data = fixture(args.variant, args.count)
        args.fixture.parent.mkdir(parents=True, exist_ok=True)
        args.fixture.write_bytes(data)
        truth, names = expected_source(data)
        result['fixture'] = {'path': str(args.fixture), 'sha256': digest(data), 'triangles': len(truth), 'node_names': names}
    if args.binary:
        result['candidate'] = run(args.binary.resolve(strict=True), large=args.large)
    encoded = json.dumps(result, indent=2, allow_nan=False)+'\n'
    if args.json_output:
        args.json_output.parent.mkdir(parents=True, exist_ok=True)
        args.json_output.write_text(encoded)
    else:
        print(encoded, end='')


if __name__ == '__main__':
    main()
