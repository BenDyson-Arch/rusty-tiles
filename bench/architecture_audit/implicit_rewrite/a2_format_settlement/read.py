#!/usr/bin/env python3
"""Bounded independent raw reader: no product/client/model oracle imports.

Only the eight authored candidates' declared five-node profile is supported.
No claim that profile acceptance proves general 3D Tiles conformance.
"""
import argparse
from fractions import Fraction as F
import hashlib
import json
from pathlib import Path
import struct

IDENTITY = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]


class Rejected(Exception):
    pass


def need(condition, reason):
    if not condition:
        raise Rejected(reason)


def mul(a, b):
    return [sum(F(a[k*4+r])*F(b[c*4+k]) for k in range(4))
            for c in range(4) for r in range(4)]


def point(a, p):
    return [sum(F(a[k*4+r])*F((list(p)+[1])[k]) for k in range(4)) for r in range(3)]


def box(a, b):
    return point(a, b[:3]) + [sum(F(a[k*4+r])*F(b[3+c*3+k]) for k in range(3))
                            for c in range(3) for r in range(3)]


def inverse(a):
    rows = [[F(a[c*4+r]) for c in range(4)]+[F(r == c) for c in range(4)] for r in range(4)]
    for c in range(4):
        pivot = next(r for r in range(c, 4) if rows[r][c])
        rows[c], rows[pivot] = rows[pivot], rows[c]
        v = rows[c][c]
        rows[c] = [x/v for x in rows[c]]
        for r in range(4):
            if r != c:
                v = rows[r][c]
                rows[r] = [x-v*y for x, y in zip(rows[r], rows[c])]
    return [rows[r][4+c] for c in range(4) for r in range(4)]


def inside(p, b):
    # The fixture boxes are exact nonsingular oriented boxes, not an envelope.
    frame = [b[3], b[4], b[5], 0, b[6], b[7], b[8], 0,
             b[9], b[10], b[11], 0, b[0], b[1], b[2], 1]
    return all(abs(x) <= 1 for x in point(inverse(frame), p))


def subtree(raw):
    need(len(raw) >= 24, 'subtree_header')
    magic, version, jlen, blen = struct.unpack_from('<4sIQQ', raw)
    need((magic, version) == (b'subt', 1), 'subtree_header')
    need(jlen % 8 == blen % 8 == 0 and len(raw) == 24+jlen+blen, 'subtree_lengths')
    doc = json.loads(raw[24:24+jlen])
    binary = raw[24+jlen:]
    need(len(doc['buffers']) == 1 and doc['buffers'][0]['byteLength'] == blen, 'internal_buffer')
    views = []
    for view in doc['bufferViews']:
        offset, length = view.get('byteOffset', 0), view['byteLength']
        need(view['buffer'] == 0 and 0 <= offset <= offset+length <= blen, 'buffer_view')
        views.append(binary[offset:offset+length])
    return doc, views


def bits(desc, views, count):
    if 'constant' in desc:
        need(desc['constant'] in (0, 1), 'constant_bit')
        return [desc['constant']]*count
    raw = views[desc['bitstream']]
    need(len(raw)*8 >= count, 'availability_size')
    values = [(raw[i//8] >> (i % 8)) & 1 for i in range(count)]
    need(desc.get('availableCount', sum(values)) == sum(values), 'available_count')
    return values


def values(table, key, views, size):
    raw = views[table['properties'][key]['values']]
    need(len(raw) == table['count']*size*8, 'metadata_size')
    flat = [F(x) for x in struct.unpack('<'+'d'*(len(raw)//8), raw)]
    return [flat[i:i+size] for i in range(0, len(flat), size)]


def uri(template, level, ordinal, dims):
    coords = {'level': level, 'x': ordinal & 1, 'y': (ordinal >> 1) & 1, 'z': (ordinal >> 2) & 1}
    return template.format(**coords)


def payload(raw):
    magic, version, length = struct.unpack_from('<4sII', raw)
    need((magic, version, length) == (b'glTF', 2, len(raw)), 'glb_header')
    jlen, jtype = struct.unpack_from('<I4s', raw, 12)
    need(jtype == b'JSON', 'glb_json')
    doc = json.loads(raw[20:20+jlen])
    offset = 20+jlen
    blen, btype = struct.unpack_from('<I4s', raw, offset)
    need(btype == b'BIN\0' and offset+8+blen == len(raw), 'glb_binary')
    accessor = doc['accessors'][0]
    need(accessor['componentType'] == 5126 and accessor['type'] == 'VEC3' and accessor['count'] == 1,
         'fixture_position_profile')
    view = doc['bufferViews'][accessor['bufferView']]
    pos = offset+8+view.get('byteOffset', 0)+accessor.get('byteOffset', 0)
    x, y, z = map(F, struct.unpack_from('<3f', raw, pos))
    return [x, -z, y]  # Core glTF y-up to tileset z-up.


def read_case(folder, item, client, overrides=None):
    overrides = overrides or {}
    def read(name):
        return overrides[name] if name in overrides else (folder/name).read_bytes()
    if item['kind'] == 'metadata-transform':
        document = json.loads((folder/'tileset.json').read_bytes())
        root = document['root']
        dims = 3 if item['scheme'] == 'OCTREE' else 2
        doc, views = subtree((folder/uri(root['implicitTiling']['subtrees']['uri'], 0, 0, dims)).read_bytes())
        need(bits(doc['tileAvailability'], views, 1) == [1], 'transform_tile_available')
        table = doc['propertyTables'][doc['tileMetadata']]
        matrix = values(table, 'placement', views, 16)[0]
        need(matrix == list(map(F, item['expected_local_transform'])), 'transform_metadata')
        prop = document['schema']['classes'][table['class']]['properties']['placement']
        need(prop == {'type': 'MAT4', 'componentType': 'FLOAT64', 'semantic': 'TILE_TRANSFORM'}, 'transform_schema')
        need(client['semantic_present'] == item['expected_local_transform'] and not client['semantic_applied'],
             'client_transform_control')
        return {'independent_semantic_matrix_present': True, 'client_ignores_semantic': True}
    source = json.loads((folder/'source.json').read_bytes())
    nodes = {n['id']: n for n in source['explicit_nodes']}
    need(len(nodes) == 5, 'five_node_bound')
    expected, chains = {}, {}
    def explicit(nid, frame, ancestors):
        expected[nid] = mul(frame, nodes[nid]['transform'])
        chains[nid] = ancestors+[nid]
        for child in nodes[nid]['children']:
            explicit(child, expected[nid], ancestors+[nid])
    explicit('n0', IDENTITY, [])
    actual, owned, active_documents = {}, [], set()
    dims = 3 if item['scheme'] == 'OCTREE' else 2
    fanout = 2**dims
    top_schema = json.loads(read('tileset.json'))['schema']['classes']
    def visit(nid, document_name, frame):
        need(document_name not in active_documents, 'external_document_cycle')
        active_documents.add(document_name)
        raw = read(document_name)
        need(raw.lstrip().startswith(b'{'), 'resource_role_external_json')
        document = json.loads(raw)
        need(document['asset']['version'] == '1.1' and isinstance(document['root'], dict), 'external_tileset_document')
        need(all(top_schema.get(k) == v for k, v in document['schema']['classes'].items()), 'external_schema_classes')
        need(nid not in actual and len(actual) < 5, 'owned_document_bound')
        root, original = document['root'], nodes[nid]
        actual[nid] = mul(frame, root.get('transform', IDENTITY))
        need(actual[nid] == expected[nid], 'source_world_frame_mismatch')
        need(root['boundingVolume']['box'] == original['bounds'] and root['geometricError'] == original['error'],
             'source_root_fields')
        need(root['refine'] == 'REPLACE' and 'children' not in root and 'metadata' not in root, 'root_fields')
        depth = 2 if original['children'] else 1
        implicit = root['implicitTiling']
        need(implicit['subtreeLevels'] == implicit['availableLevels'] == depth, 'finite_depth')
        need(len(original['children']) <= fanout, 'ordinal_capacity')
        doc, views = subtree((folder/uri(implicit['subtrees']['uri'], 0, 0, dims)).read_bytes())
        count = 1+fanout if depth == 2 else 1
        tilebits = bits(doc['tileAvailability'], views, count)
        need(tilebits == [1]+([int(i < len(original['children'])) for i in range(fanout)] if depth == 2 else []),
             'source_order_tile_addresses')
        need(not any(bits(doc['childSubtreeAvailability'], views, fanout**depth)), 'terminal_subtrees')
        contents = root.get('contents', [root['content']] if 'content' in root else [])
        avail = [bits(desc, views, count) for desc in doc.get('contentAvailability', [])]
        need(len(avail) == len(contents), 'slot_layer_count')
        need(len(contents) == len(original['payloads'])+int(bool(original['children'])), 'all_source_slots')
        for slot, descriptor in enumerate(contents):
            # Role comes from independently authored source ownership, not suffix.
            # Available resources are checked by actual GLB bytes / JSON document.
            is_json = slot == len(original['payloads'])
            need(not (is_json and avail[slot][0]), 'forbidden_root_external_occupancy')
            wanted = ([0]+tilebits[1:]) if is_json else [1]+[0]*(count-1)
            need(avail[slot] == wanted, 'slot_ownership')
            if not is_json:
                pid = original['payloads'][slot]
                data = (folder/uri(descriptor['uri'], 0, 0, dims)).read_bytes()
                need(data == (folder/(pid+'.glb')).read_bytes(), 'payload_alias')
                local = payload(data)
                need(local == list(map(F, source['points'][pid])), 'decoded_source_position')
                world = point(actual[nid], local)
                for ancestor in chains[nid]:
                    need(inside(point(inverse(actual[ancestor]), world), nodes[ancestor]['bounds']),
                         'ancestor_content_escape')
                owned.append(pid)
        table = doc['propertyTables'][doc['tileMetadata']]
        properties = document['schema']['classes'][table['class']]['properties']
        need(properties['bounds']['semantic'] == 'TILE_BOUNDING_BOX' and
             properties['error']['semantic'] == 'TILE_GEOMETRIC_ERROR', 'authoritative_metadata_semantics')
        need(table['count'] == sum(tilebits), 'available_metadata_rows')
        boxes, errors = values(table, 'bounds', views, 12), values(table, 'error', views, 1)
        need(boxes[0] == list(map(F, original['bounds'])) and errors[0][0] == original['error'], 'root_metadata')
        for ordinal, childid in enumerate(original['children']):
            child = nodes[childid]
            need(boxes[ordinal+1] == box(child['transform'], child['bounds']) and errors[ordinal+1][0] == child['error'],
                 'terminal_parent_frame_metadata')
            slot = len(original['payloads'])
            name = uri(contents[slot]['uri'], 1, ordinal, dims)
            visit(childid, name, actual[nid])
        active_documents.remove(document_name)
    visit('n0', 'tileset.json', IDENTITY)
    need(set(actual) == set(nodes) and len(owned) == 6 and len(set(owned)) == 6, 'all_source_ownership')
    for row in client['roots']:
        need(list(map(F, row['computed_transform'])) == expected[row['source_id']], 'independent_client_frames')
        need(len(row['root_available_uris']) == len(nodes[row['source_id']]['payloads']), 'client_all_source_slots')
    for row in client['terminal_links']:
        need(list(map(F, row['computed_transform'])) == expected[row['source_parent']], 'client_link_parent_frame')
        child = nodes[row['source_child']]
        ordinal = nodes[row['source_parent']]['children'].index(row['source_child'])
        wanted = {'level': 1, 'x': ordinal & 1, 'y': (ordinal >> 1) & 1}
        if dims == 3:
            wanted['z'] = (ordinal >> 2) & 1
        need(row['coordinates'] == wanted, 'client_ordinal_address')
        need(list(map(F, row['semantic_box'])) == box(child['transform'], child['bounds']) and
             row['geometric_error'] == child['error'], 'client_authoritative_terminal_fields')
    return {'five_source_roots_exact': True, 'six_payload_slots_exact': True,
            'four_terminal_links_exact': True, 'all_content_inside_own_and_ancestor_boxes': True,
            'client_frames_equal_independent_fraction_reference': True}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('directory', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    manifest = json.loads((args.directory/'manifest.json').read_bytes())
    client = json.loads((args.directory/'client-results.json').read_bytes())
    result = {'scope': 'finite_interoperability_profile_not_general_conformance', 'cases': {}, 'reader_only_controls': {},
              'driver_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              'manifest_sha256': hashlib.sha256((args.directory/'manifest.json').read_bytes()).hexdigest(),
              'client_results_sha256': hashlib.sha256((args.directory/'client-results.json').read_bytes()).hexdigest()}
    need(len(manifest['cases']) == 8, 'eight_case_bound')
    for name, item in manifest['cases'].items():
        folder = args.directory/name
        for member, digest in item['member_sha256'].items():
            need(hashlib.sha256((folder/member).read_bytes()).hexdigest() == digest, 'captured_member_identity')
        expected_rejection = {'double-child-transform': 'source_world_frame_mismatch',
                              'root-external-available': 'forbidden_root_external_occupancy'}.get(item['kind'])
        try:
            outcome = read_case(folder, item, client['cases'][name])
            need(expected_rejection is None, 'missed_control')
            result['cases'][name] = {'accepted': True, **outcome}
        except Rejected as error:
            need(str(error) == expected_rejection, 'unexpected_rejection:'+str(error))
            result['cases'][name] = {'accepted': False, 'expected_sensitive_rejection': str(error)}
        if item['kind'] == 'candidate':
            root = json.loads((folder/'tileset.json').read_bytes())['root']
            dims = 3 if item['scheme'] == 'OCTREE' else 2
            child_name = uri(root['contents'][-1]['uri'], 1, 0, dims)
            cyclic = json.loads((folder/child_name).read_bytes())
            cyclic['root']['contents'][-1]['uri'] = root['contents'][-1]['uri']
            controls = [
                ('actual-json-cycle', json.dumps(cyclic).encode(), 'external_document_cycle'),
                ('json-link-has-glb-bytes', (folder/'p0.glb').read_bytes(), 'resource_role_external_json'),
            ]
            for control, corrupt_bytes, reason in controls:
                try:
                    read_case(folder, item, client['cases'][name], {child_name: corrupt_bytes})
                    need(False, 'missed_reader_control')
                except Rejected as error:
                    need(str(error) == reason, 'wrong_reader_control:'+str(error))
                    result['reader_only_controls'][name+'-'+control] = {'expected_sensitive_rejection': reason}
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True)+'\n')
    print(json.dumps({'cases': 8, 'candidates': 2, 'sensitive_controls': 4,
                      'transform_support_controls': 2, 'reader_only_cycle_type_controls': 4}))


if __name__ == '__main__':
    main()
