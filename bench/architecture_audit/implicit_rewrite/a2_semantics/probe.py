#!/usr/bin/env python3
"""Small independent A2 design probe. Standard library; never runs converter.

Hand-assigned emitted slots are intentionally separate from explicit-tree truth.
Exact Fraction arithmetic has no epsilon. Not a general 3D Tiles validator.
"""
import argparse
import base64
import copy
from fractions import Fraction as F
import hashlib
import itertools
import json
from pathlib import Path
import struct
import time

HERE = Path(__file__).resolve().parent
IDENTITY = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]


def require(value, message):
    if not value:
        raise ValueError(message)


def matrix(delta):
    out = IDENTITY.copy()
    out[12:15] = delta
    return out


def multiply(a, b):
    return [sum(F(a[r+4*k])*F(b[k+4*c]) for k in range(4))
            for c in range(4) for r in range(4)]


def point(m, p):
    return [sum(F(m[r+4*k])*F((list(p)+[1])[k]) for k in range(4))
            for r in range(3)]


def box(lo, hi):
    center = [(F(a)+F(b))/2 for a, b in zip(lo, hi)]
    axes = [0]*9
    for i in range(3):
        axes[4*i] = (F(hi[i])-F(lo[i]))/2
    return center+axes


def shifted(b, delta):
    return [F(b[i])+F(delta[i]) for i in range(3)]+list(b[3:])


def corners(b, m=IDENTITY):
    return sorted(tuple(point(m, [F(b[i])+sum(s[k]*F(b[3+k*3+i])
                       for k in range(3)) for i in range(3)]))
                  for s in itertools.product((-1, 1), repeat=3))


def slot(parent, child, delta, scheme):
    """Finite profile: entire translated child box fits its center-selected cell.

    Exact midpoint centers choose high; this is profile policy, not OGC law.
    """
    b = shifted(child, delta)
    n = 3 if scheme == 'OCTREE' else 2
    selected = sum((F(b[i]) >= F(parent[i])) << i for i in range(n))
    for i in range(3):
        h = F(parent[3+4*i])/(2 if i < n else 1)
        c = F(parent[i]) + (h if selected & (1 << i) else -h) if i < n else F(parent[i])
        require(abs(F(b[i])-c)+F(b[3+4*i]) <= h, 'child outside local cell')
    return selected


def json_bytes(x):
    return json.dumps(x, separators=(',', ':'), sort_keys=True,
                      default=lambda v: float(v) if isinstance(v, F) else v,
                      allow_nan=False).encode()


def payload(p):
    # glTF y-up -> tiles z-up: (x,y,z) -> (x,-z,y).
    v = [p[0], p[2], -p[1]]
    data = struct.pack('<3f', *v)
    doc = {'asset': {'version': '2.0'}, 'scene': 0, 'scenes': [{'nodes': [0]}],
           'nodes': [{'mesh': 0}], 'meshes': [{'primitives': [{'mode': 0,
                     'attributes': {'POSITION': 0}}]}], 'buffers': [{'byteLength': 12}],
           'bufferViews': [{'buffer': 0, 'byteLength': 12}],
           'accessors': [{'bufferView': 0, 'componentType': 5126, 'count': 1,
                          'type': 'VEC3', 'min': v, 'max': v}]}
    encoded = json_bytes(doc)
    encoded += b' '*((-len(encoded)) % 4)
    return struct.pack('<4sII', b'glTF', 2, 28+len(encoded)+len(data)) + \
        struct.pack('<I4s', len(encoded), b'JSON') + encoded + \
        struct.pack('<I4s', len(data), b'BIN\0') + data


def payload_point(data):
    magic, version, length = struct.unpack_from('<4sII', data)
    require((magic, version, length) == (b'glTF', 2, len(data)), 'payload header')
    size, kind = struct.unpack_from('<I4s', data, 12)
    require(kind == b'JSON', 'payload JSON')
    doc = json.loads(data[20:20+size])
    ac = doc['accessors'][doc['meshes'][0]['primitives'][0]['attributes']['POSITION']]
    require((ac['componentType'], ac['type'], ac['count']) == (5126, 'VEC3', 1), 'payload profile')
    view = doc['bufferViews'][ac['bufferView']]
    n, kind = struct.unpack_from('<I4s', data, 20+size)
    require(kind == b'BIN\0', 'payload binary')
    x, y, z = struct.unpack_from('<3f', data, 28+size+view.get('byteOffset', 0)+ac.get('byteOffset', 0))
    return [F(x), -F(z), F(y)]


def subtree(tile_bits, content_bits, rows, branches):
    binary = bytearray()
    views = []
    def view(data):
        binary.extend(b'\0'*(-len(binary) % 8))
        views.append({'buffer': 0, 'byteOffset': len(binary), 'byteLength': len(data)})
        binary.extend(data)
        return len(views)-1
    def bits(values):
        data = bytearray((len(values)+7)//8)
        for i, v in enumerate(values):
            data[i//8] |= int(v) << (i % 8)
        return {'bitstream': view(data), 'availableCount': sum(values)}
    doc = {'tileAvailability': bits(tile_bits),
           'childSubtreeAvailability': {'constant': 0},
           'tileMetadata': 0}
    if content_bits:
        doc['contentAvailability'] = [bits(v) for v in content_bits]
    properties = {}
    for key, width in [('bounds', 12), ('error', 1)]:
        values = [r[key] if width == 1 else r[key][i]
                  for r in rows for i in range(width)]
        properties[key] = {'values': view(struct.pack('<'+'d'*len(values), *values))}
    doc['propertyTables'] = [{'class': 'a2Tile', 'count': len(rows), 'properties': properties}]
    doc['bufferViews'] = views
    doc['buffers'] = [{'byteLength': len(binary)}]
    encoded = json_bytes(doc)
    encoded += b' '*(-len(encoded) % 8)
    binary.extend(b'\0'*(-len(binary) % 8))
    return struct.pack('<4sIQQ', b'subt', 1, len(encoded), len(binary))+encoded+binary


def decode_subtree(data, root, schema):
    magic, version, js, bs = struct.unpack_from('<4sIQQ', data)
    require((magic, version) == (b'subt', 1) and js % 8 == bs % 8 == 0, 'subtree header')
    require(24+js+bs == len(data), 'subtree length')
    doc = json.loads(data[24:24+js])
    binary = data[24+js:]
    def view(i):
        v = doc['bufferViews'][i]
        require(v['buffer'] == 0 and v.get('byteOffset', 0) % 8 == 0, 'view alignment')
        lo = v.get('byteOffset', 0)
        hi = lo+v['byteLength']
        require(hi <= doc['buffers'][0]['byteLength'] <= bs, 'view bounds')
        return binary[lo:hi]
    branches = 8 if root['implicitTiling']['subdivisionScheme'] == 'OCTREE' else 4
    levels = root['implicitTiling']['subtreeLevels']
    require(levels in (1, 2), 'design fixture levels')
    length = sum(branches**l for l in range(levels))
    def stream(d, n):
        if 'constant' in d:
            require(d['constant'] in (0, 1), 'constant')
            return [d['constant']]*n
        data = view(d['bitstream'])
        require(len(data) == (n+7)//8, 'stream size')
        values = [(data[i//8] >> (i % 8)) & 1 for i in range(n)]
        require(all(not ((data[i//8] >> (i % 8)) & 1) for i in range(n, len(data)*8)), 'trailing bits')
        require(d.get('availableCount', sum(values)) == sum(values), 'availableCount')
        return values
    tiles = stream(doc['tileAvailability'], length)
    require(tiles[0] == 1, 'root availability')
    require(all(not tiles[i] or tiles[0] for i in range(1, length)), 'available parent')
    contents = root.get('contents', [root['content']] if 'content' in root else [])
    streams = [stream(v, length) for v in doc.get('contentAvailability', [])]
    require(len(streams) == len(contents), 'slot count')
    require(all(not v or tiles[i] for s in streams for i, v in enumerate(s)), 'content outside tile')
    children = stream(doc['childSubtreeAvailability'], branches**levels)
    require(not any(children), 'unexpected child subtree')
    table = doc['propertyTables'][doc['tileMetadata']]
    require(table['count'] == sum(tiles), 'metadata row count')
    definitions = schema['classes'][table['class']]['properties']
    decoded = {}
    for name, prop in table['properties'].items():
        definition = definitions[name]
        if 'semantic' not in definition:
            continue
        require(definition['componentType'] == 'FLOAT64', 'component type')
        width = definition.get('count', 1)
        values = view(prop['values'])
        require(len(values) == 8*width*table['count'], 'property byte count')
        values = [F(v) for v in struct.unpack('<'+'d'*(len(values)//8), values)]
        decoded[definition['semantic']] = [values[i*width:(i+1)*width] for i in range(table['count'])]
    return tiles, streams, decoded


SCHEMA = {'classes': {'a2Tile': {'properties': {
    'bounds': {'type': 'SCALAR', 'componentType': 'FLOAT64', 'array': True, 'count': 12, 'semantic': 'TILE_BOUNDING_BOX'},
    'error': {'type': 'SCALAR', 'componentType': 'FLOAT64', 'semantic': 'TILE_GEOMETRIC_ERROR'}}}}}


def fixture(scheme, address_policy='center-hand-assigned'):
    # This finite source is independently authored, never emitted by Rust.
    # Child n1 local midpoint is 1; root-derived previous grid midpoint is 4.
    nodes = [
        {'id': 'n0', 'bounds': box([0, 0, 0], [16, 16, 8]), 'error': 19,
         'refine': 'REPLACE', 'transform': [0, 1, 0, 0, -1, 0, 0, 0, 0, 0, 1, 0, 100, 200, 300, 1],
         'payloads': ['p0', 'p1'], 'children': ['n1', 'n3']},
        {'id': 'n1', 'bounds': box([0, 0, 0], [2, 2, 2]), 'error': 7,
         'transform': matrix([2, 2, 1]), 'payloads': ['p2'], 'children': ['n2']},
        {'id': 'n2', 'bounds': box([0, 0, 0], [0.5, 0.5, 0.5]), 'error': 3,
         'transform': matrix([1.25, 1.25, 1.25]), 'payloads': ['p3', 'p4'], 'children': []},
        {'id': 'n3', 'bounds': box([0, 0, 0], [2, 2, 2]), 'error': 11,
         'transform': matrix([10, 10, 5]), 'payloads': [], 'children': ['n4']},
        {'id': 'n4', 'bounds': box([0, 0, 0], [0.5, 0.5, 0.5]), 'error': 0,
         'transform': matrix([0.25, 0.25, 0.25]), 'payloads': ['p5'], 'children': []}]
    # Fixed hand-assigned identities. No slot function builds emitted output.
    assignments = {'n0': [('n1', 0), ('n3', 7 if scheme == 'OCTREE' else 3)],
                   'n1': [('n2', 7 if scheme == 'OCTREE' else 3)],
                   'n2': [], 'n3': [('n4', 0)], 'n4': []}
    if address_policy == 'source-child-order':
        # Distinct child identities with coincident boxes and centers. Each
        # retains its own different descendant/content ownership.
        nodes[3]['transform'] = nodes[1]['transform'].copy()
        assignments = {n['id']: list(zip(n['children'], range(len(n['children'])))) for n in nodes}
    points = {'p0': [1, 1, 1], 'p1': [15, 15, 7], 'p2': [0.5, 0.5, 0.5],
              'p3': [0.125, 0.125, 0.125], 'p4': [0.375, 0.375, 0.375], 'p5': [0.25, 0.25, 0.25]}
    members = {name+'.glb': payload(p) for name, p in points.items()}
    by_id = {n['id']: n for n in nodes}
    branches = 8 if scheme == 'OCTREE' else 4
    for n in nodes:
        links = assignments[n['id']]
        levels = 2 if links else 1
        templates = [{'uri': f'{p}-{{level}}-{{x}}-{{y}}'+('-{z}' if branches == 8 else '')+'.glb'} for p in n['payloads']]
        if links:
            templates.append({'uri': n['id']+'-link-{level}-{x}-{y}'+('-{z}' if branches == 8 else '')+'.json'})
        root = {'boundingVolume': {'box': n['bounds']}, 'geometricError': n['error'], 'refine': 'REPLACE',
                'transform': n['transform'], 'implicitTiling': {'subdivisionScheme': scheme, 'subtreeLevels': levels,
                'availableLevels': levels, 'subtrees': {'uri': n['id']+'-{level}-{x}-{y}'+('-{z}' if branches == 8 else '')+'.subtree'}}}
        if templates:
            root['contents'] = templates
        tiles = [1]+[int(any(s == i for _, s in links)) for i in range(branches)] if links else [1]
        flags = [[1]+[0]*branches if links else [1] for _ in n['payloads']]
        if links:
            flags.append([0]+tiles[1:])
        rows = [{'bounds': n['bounds'], 'error': n['error']}]
        for child_id, child_slot in sorted(links, key=lambda v: v[1]):
            c = by_id[child_id]
            rows.append({'bounds': shifted(c['bounds'], c['transform'][12:15]), 'error': c['error']})
        zero = '-0-0-0'+('-0' if branches == 8 else '')
        members[n['id']+zero+'.subtree'] = subtree(tiles, flags, rows, branches)
        for p, template in zip(n['payloads'], templates):
            members[resolve(template['uri'], 0, 0, 0, 0)] = members[p+'.glb']
        members[n['id']+'.json'] = json_bytes({'asset': {'version': '1.1'}, 'geometricError': n['error'], 'schema': SCHEMA, 'root': root})
        for child_id, child_slot in links:
            uri = resolve(templates[-1]['uri'], 1, child_slot & 1, (child_slot >> 1) & 1, (child_slot >> 2) & 1)
            # Alias document content is installed after all documents are built.
            members[uri] = child_id
    for name, data in list(members.items()):
        if isinstance(data, str):
            members[name] = members[data+'.json']
    return {'scheme': scheme, 'address_policy': address_policy, 'explicit_nodes': nodes, 'points': points}, members


def resolve(uri, level, x, y, z):
    return uri.format(level=level, x=x, y=y, z=z)


def oracle(source, members):
    by_id = {n['id']: n for n in source['explicit_nodes']}
    expected = {}
    def explicit(nid, frame, refine):
        n = by_id[nid]
        refine = n.get('refine', refine)
        require(refine == 'REPLACE', 'effective refinement')
        frame = multiply(frame, n['transform'])
        expected[nid] = (frame, corners(n['bounds'], frame), F(n['error']))
        for pid in n['payloads']:
            p = point(frame, source['points'][pid])
            require(all(min(v[i] for v in expected[nid][1]) <= p[i] <= max(v[i] for v in expected[nid][1]) for i in range(3)), 'source payload bound')
        for child_id in n['children']:
            explicit(child_id, frame, refine)
    explicit('n0', IDENTITY, None)
    seen = set()
    observed = {}
    links_seen = []
    def emitted(nid, data, parent_frame):
        require(nid not in seen, 'external cycle/duplicate owner')
        seen.add(nid)
        n = by_id[nid]
        doc = json.loads(data)
        require(doc['asset']['version'] == '1.1', 'generated version')
        root = doc['root']
        require(root['refine'] == 'REPLACE' and 'children' not in root and 'metadata' not in root, 'implicit root fields')
        require(F(doc['geometricError']) == F(n['error']), 'document error')
        frame = multiply(parent_frame, root['transform'])
        require(frame == expected[nid][0], 'external transform composition')
        uri = resolve(root['implicitTiling']['subtrees']['uri'], 0, 0, 0, 0)
        tiles, streams, meta = decode_subtree(members[uri], root, doc['schema'])
        boxes = meta['TILE_BOUNDING_BOX']
        errors = meta['TILE_GEOMETRIC_ERROR']
        require(corners(boxes[0], frame) == expected[nid][1], 'root semantic box')
        require(errors[0][0] == expected[nid][2], 'root semantic error')
        templates = root.get('contents', [])
        require(len(templates) == len(n['payloads'])+bool(n['children']), 'ordered slot inventory')
        require(sum(tiles) == len(n['children'])+1, 'tile inventory')
        for i, pid in enumerate(n['payloads']):
            require(streams[i][0] == 1 and sum(streams[i]) == 1, 'original root-only slot')
            content_uri = resolve(templates[i]['uri'], 0, 0, 0, 0)
            require(members[content_uri] == members[pid+'.glb'], 'payload byte identity')
            observed[pid] = point(frame, payload_point(members[content_uri]))
            require(observed[pid] == point(expected[nid][0], source['points'][pid]), 'payload world position')
        if n['children']:
            require(streams[-1][0] == 0 and streams[-1][1:] == tiles[1:], 'terminal-only external links')
            require(root['implicitTiling']['availableLevels'] == root['implicitTiling']['subtreeLevels'] == 2, 'internal levels')
        else:
            require(root['implicitTiling']['availableLevels'] == root['implicitTiling']['subtreeLevels'] == 1, 'leaf levels')
        for child_id in n['children']:
            c = by_id[child_id]
            selected = n['children'].index(child_id) if source.get('address_policy') == 'source-child-order' else slot(n['bounds'], c['bounds'], c['transform'][12:15], source['scheme'])
            index = 1+selected
            require(tiles[index] == 1 and streams[-1][index] == 1, 'correct local slot')
            row = sum(tiles[:index])
            require(corners(boxes[row], frame) == expected[child_id][1], 'terminal box frame/rank')
            require(errors[row][0] == F(c['error']), 'terminal authored error')
            require(row > 0, 'terminal rank')
            uri = resolve(templates[-1]['uri'], 1, selected & 1, (selected >> 1) & 1, (selected >> 2) & 1)
            links_seen.append([nid, child_id, selected, row])
            emitted(child_id, members[uri], frame)
    emitted('n0', members['n0.json'], IDENTITY)
    require(seen == set(by_id), 'source ownership inventory')
    return {'nodes': len(seen), 'payloads': len(observed), 'links': links_seen,
            'world_points': {k: [str(v) for v in p] for k, p in sorted(observed.items())}}


def mutate_subtree(members, name, operation):
    data = members[name]
    _, _, js, _ = struct.unpack_from('<4sIQQ', data)
    doc = json.loads(data[24:24+js])
    binary = bytearray(data[24+js:])
    operation(doc, binary)
    encoded = json_bytes(doc)
    encoded += b' '*(-len(encoded) % 8)
    members[name] = struct.pack('<4sIQQ', b'subt', 1, len(encoded), len(binary))+encoded+binary


def semantic_cases():
    """Exact, independently authored precedence examples; narrow supported types.

    This is specification modeling, not an imported metadata implementation.
    """
    parent_refine = 'REPLACE'
    original = {'bounds': box([0, 0, 0], [8, 8, 8]), 'error': 17,
                'transform': matrix([2, 3, 4])}
    effective = dict(original, refine=parent_refine)
    props = {'shape': {'semantic': 'TILE_BOUNDING_BOX'},
             'selection': {'semantic': 'TILE_GEOMETRIC_ERROR', 'noData': -1},
             'strategy': {'semantic': 'TILE_REFINE'},
             'placement': {'semantic': 'TILE_TRANSFORM'}}
    values = {'shape': box([1, 1, 1], [7, 7, 7]), 'selection': 5,
              'strategy': 1, 'placement': matrix([9, 10, 11])}
    names = {'TILE_BOUNDING_BOX': 'bounds', 'TILE_GEOMETRIC_ERROR': 'error',
             'TILE_REFINE': 'refine', 'TILE_TRANSFORM': 'transform'}
    def apply(values):
        out = effective.copy()
        for key, value in values.items():
            definition = props[key]
            if 'noData' in definition and value == definition['noData']:
                continue
            semantic = definition['semantic']
            out[names[semantic]] = ['ADD', 'REPLACE'][value] if semantic == 'TILE_REFINE' else value
        return out
    out = apply(values)
    require(out['error'] == 5 and out['refine'] == 'REPLACE' and out['transform'][12:15] == [9, 10, 11], 'semantic override')
    require(out['bounds'] != original['bounds'], 'semantic bounding override')
    missing = apply({'selection': -1})
    require(missing == effective, 'noData must retain explicit properties')
    additive = apply({'strategy': 0})
    require(additive['refine'] == 'ADD', 'metadata ADD must not be ignored')
    # Errors are not inherited: the original authored error is a required field.
    require(effective['error'] == 17 and effective['refine'] == parent_refine, 'only refinement inheritance')
    return [{'case': 'custom_property_names_use_semantics', 'effective_error': 5,
             'effective_transform_translation': [9, 10, 11]},
            {'case': 'noData_retains_original', 'effective_error': 17},
            {'case': 'inherited_replace', 'effective_refine': 'REPLACE'},
            {'case': 'metadata_add_override', 'profile_disposition': 'Unsupported'},
            {'case': 'missing_geometric_error', 'profile_disposition': 'InvalidInput'}]


def overlap_case():
    parent = box([0, 0, 0], [16, 16, 8])
    override = box([2, 2, 1], [8.25, 8.25, 3])
    root = {'implicitTiling': {'subdivisionScheme': 'QUADTREE', 'subtreeLevels': 2},
            'contents': [{'uri': 'p-{level}-{x}-{y}.glb'}]}
    encoded = subtree([1, 1, 0, 0, 0], [[0, 1, 0, 0, 0]],
                      [{'bounds': parent, 'error': 9}, {'bounds': override, 'error': 2}], 4)
    _, _, decoded = decode_subtree(encoded, root, SCHEMA)
    effective = decoded['TILE_BOUNDING_BOX'][1]
    authored_point = [F('8.125'), F('8.125'), F(2)]
    lo = [effective[i]-effective[3+4*i] for i in range(3)]
    hi = [effective[i]+effective[3+4*i] for i in range(3)]
    require(all(lo[i] <= authored_point[i] <= hi[i] for i in range(3)), 'override must bound content')
    require(all(0 <= authored_point[i] <= [16, 16, 8][i] for i in range(3)), 'parent content coherence')
    require(authored_point[0] > 8 and authored_point[1] > 8, 'nominal-cell control sensitivity')
    require(effective == override, 'metadata must replace nominal bounds without clipping')
    return {'case': 'overlapping_metadata_box', 'content': [str(v) for v in authored_point],
            'nominal_child_xy_max': 8, 'semantic_child_xy_max': '33/4',
            'parent_content_coherent': True, 'nominal_clipping_control_rejected': True,
            'production_consumer_executed': False}


def run():
    started = time.monotonic()
    result = {'kind': 'independent-design-model-only', 'production_executed': False,
              'fixtures': {}, 'controls': [], 'profile_cases': [],
              'semantic_precedence_cases': semantic_cases(),
              'overlap_case': overlap_case(), 'source_order_fixtures': {}}
    for scheme in ['QUADTREE', 'OCTREE']:
        source, members = fixture(scheme)
        result['fixtures'][scheme] = oracle(source, members)
        ordered_source, ordered_members = fixture(scheme, 'source-child-order')
        result['source_order_fixtures'][scheme] = oracle(ordered_source, ordered_members)
        require(sum(len(v) for v in ordered_members.values()) < 1024*1024, 'source-order fixture byte ceiling')
        corrupt_ordered = ordered_members.copy()
        tail = '-0' if scheme == 'OCTREE' else ''
        a, b = 'n0-link-1-0-0'+tail+'.json', 'n0-link-1-1-0'+tail+'.json'
        corrupt_ordered[a], corrupt_ordered[b] = corrupt_ordered[b], corrupt_ordered[a]
        try:
            oracle(ordered_source, corrupt_ordered)
        except ValueError as error:
            result['controls'].append({'scheme': scheme, 'control': 'source_order_external_owner_swap',
                                       'rejected': True, 'cause': str(error)})
        else:
            raise ValueError('source-order owner control insensitive')
        require(sum(len(v) for v in members.values()) < 1024*1024, 'fixture byte limit')
        zero = '-0-0-0'+('-0' if scheme == 'OCTREE' else '')
        subtree_name = 'n0'+zero+'.subtree'
        controls = ['wrong_frame', 'wrong_error', 'metadata_rank_swap', 'slot_swap',
                    'external_double_translation', 'payload_swap', 'root_external_available',
                    'version_1_0', 'child_subtree_available', 'content_slot_swap']
        for control in controls:
            changed = copy.deepcopy(members)
            def operation(doc, binary):
                table = doc['propertyTables'][0]
                def offset(property_name):
                    return doc['bufferViews'][table['properties'][property_name]['values']]['byteOffset']
                if control in ('wrong_frame', 'wrong_error', 'metadata_rank_swap'):
                    key = 'error' if control == 'wrong_error' else 'bounds'
                    off = offset(key)
                    width = 8 if key == 'error' else 96
                    if control == 'metadata_rank_swap':
                        binary[off+width:off+3*width] = binary[off+2*width:off+3*width]+binary[off+width:off+2*width]
                    else:
                        pos = off+width
                        value = struct.unpack_from('<d', binary, pos)[0]
                        struct.pack_into('<d', binary, pos, value+1)
                elif control == 'slot_swap':
                    # Keep tile count and metadata row count. Move first child 0 -> 1.
                    for d in [doc['tileAvailability'], doc['contentAvailability'][-1]]:
                        off = doc['bufferViews'][d['bitstream']]['byteOffset']
                        binary[off] ^= (1 << 1) | (1 << 2)
                elif control == 'root_external_available':
                    d = doc['contentAvailability'][-1]
                    binary[doc['bufferViews'][d['bitstream']]['byteOffset']] |= 1
                    d['availableCount'] += 1
                elif control == 'child_subtree_available':
                    doc['childSubtreeAvailability'] = {'constant': 1}
                elif control == 'content_slot_swap':
                    doc['contentAvailability'][0], doc['contentAvailability'][-1] = doc['contentAvailability'][-1], doc['contentAvailability'][0]
            if control == 'external_double_translation':
                for name in list(changed):
                    if name.startswith('n0-link-1-0-0'):
                        doc = json.loads(changed[name])
                        doc['root']['transform'][12] += 2
                        changed[name] = json_bytes(doc)
            elif control == 'payload_swap':
                for name in list(changed):
                    if name.startswith('p0-0-0-0'):
                        changed[name] = members['p1.glb']
            elif control == 'version_1_0':
                doc = json.loads(changed['n0.json'])
                doc['asset']['version'] = '1.0'
                changed['n0.json'] = json_bytes(doc)
            else:
                mutate_subtree(changed, subtree_name, operation)
            try:
                oracle(source, changed)
            except (ValueError, KeyError) as error:
                result['controls'].append({'scheme': scheme, 'control': control, 'rejected': True, 'cause': str(error)})
            else:
                raise ValueError('insensitive control '+control)
        # Old global root-grid choice differs from the same emitted root choice.
        p = source['explicit_nodes'][1]
        c = source['explicit_nodes'][2]
        actual = slot(p['bounds'], c['bounds'], c['transform'][12:15], scheme)
        world_child_center = shifted(c['bounds'], [3.25, 3.25, 2.25])
        old = sum((world_child_center[i] >= [4, 4, 2][i]) << i for i in range(3 if scheme == 'OCTREE' else 2))
        result['profile_cases'].append({'case': 'deep_tight_local_slot', 'scheme': scheme,
                                        'local_slot': actual, 'global_cell_slot': old, 'disagree': actual != old})
        require(actual != old, 'tight slot control not sensitive')
    parent = box([0, 0, 0], [4, 4, 4])
    result['profile_cases'].append({'case': 'midpoint_zero_extent_high',
        'slot': slot(parent, box([2, 2, 2], [2, 2, 2]), [0, 0, 0], 'OCTREE')})
    try:
        slot(parent, box([1.5, 1.5, 1.5], [2.5, 2.5, 2.5]), [0, 0, 0], 'OCTREE')
    except ValueError as error:
        result['profile_cases'].append({'case': 'midpoint_positive_extent_crossing', 'unsupported': True, 'cause': str(error)})
    else:
        raise ValueError('midpoint crossing must refuse finite profile')
    result['limits'] = {'fixtures': 4, 'nodes_per_fixture': 5, 'control_count': 22,
                        'max_fixture_bytes': 1048576, 'max_seconds': 10}
    require(time.monotonic()-started < 10, 'probe time budget')
    result['driver_sha256'] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    root = HERE.parents[3]
    result['inspected_source_sha256'] = {
        str(p): hashlib.sha256((root/p).read_bytes()).hexdigest()
        for p in map(Path, ['src/convert_implicit.rs', 'src/implicit.rs',
                           'src/implicit/tileset.rs', 'src/metadata.rs',
                           'src/point_cloud/tiles.rs', 'src/vector/pipeline/encoding.rs',
                           'src/vector/pipeline/store.rs', 'tests/convert_implicit.rs'])}
    result['fixture_sha256'] = {
        scheme: hashlib.sha256(json_bytes({'source': fixture(scheme)[0],
                  'members_base64': {k: base64.b64encode(v).decode()
                  for k, v in sorted(fixture(scheme)[1].items())}})+b'\n').hexdigest()
        for scheme in ['QUADTREE', 'OCTREE']}
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--write', action='store_true', help='write deterministic results and authored fixtures in this subtree')
    args = parser.parse_args()
    result = run()
    if args.write:
        for scheme in ['QUADTREE', 'OCTREE']:
            source, members = fixture(scheme)
            (HERE/(scheme.lower()+'.json')).write_bytes(json_bytes({'source': source,
                'members_base64': {k: base64.b64encode(v).decode() for k, v in sorted(members.items())}})+b'\n')
            source, members = fixture(scheme, 'source-child-order')
            (HERE/(scheme.lower()+'-source-order.json')).write_bytes(json_bytes({'source': source,
                'members_base64': {k: base64.b64encode(v).decode() for k, v in sorted(members.items())}})+b'\n')
        (HERE/'results.json').write_text(json.dumps(result, indent=2, sort_keys=True)+'\n')
    else:
        expected = json.loads((HERE/'results.json').read_text())
        require(expected == result, 'recorded result mismatch')
        for scheme in ['QUADTREE', 'OCTREE']:
            stored = json.loads((HERE/(scheme.lower()+'.json')).read_text())
            members = {k: base64.b64decode(v) for k, v in stored['members_base64'].items()}
            require(oracle(stored['source'], members) == result['fixtures'][scheme], 'stored fixture mismatch')
            stored = json.loads((HERE/(scheme.lower()+'-source-order.json')).read_text())
            members = {k: base64.b64decode(v) for k, v in stored['members_base64'].items()}
            require(oracle(stored['source'], members) == result['source_order_fixtures'][scheme], 'stored source-order fixture mismatch')
    print(json.dumps({'status': 'pass', 'fixtures': len(result['fixtures'])+len(result['source_order_fixtures']),
                      'rejected_controls': len(result['controls']), 'production_executed': False}, sort_keys=True))


if __name__ == '__main__':
    main()
