#!/usr/bin/env python3
"""Install eight tiny independent authored cases into a fresh artifact folder.

Uses retained model artifact bytes, not the production writer. Adds a correctly
framed source-equivalence corruption, forbidden-root-occupancy control, and present
TILE_TRANSFORM metadata consumer-support control for each subdivision scheme.
"""
import argparse
import base64
import copy
import hashlib
import json
from pathlib import Path
import struct

HERE = Path(__file__).resolve().parent
MODEL = HERE.parent/'a2_semantics'


def jsbytes(x):
    return json.dumps(x, sort_keys=True, separators=(',', ':'), allow_nan=False).encode()


def mutate(data, action):
    magic, version, size, binary_size = struct.unpack_from('<4sIQQ', data)
    assert (magic, version) == (b'subt', 1)
    doc = json.loads(data[24:24+size])
    binary = bytearray(data[24+size:])
    action(doc, binary)
    encoded = jsbytes(doc)
    encoded += b' '*(-len(encoded) % 8)
    return struct.pack('<4sIQQ', b'subt', 1, len(encoded), len(binary))+encoded+binary


def main():
    p = argparse.ArgumentParser()
    p.add_argument('directory', type=Path)
    args = p.parse_args()
    args.directory.mkdir(exist_ok=False)
    manifest = {'cases': {}, 'model_inputs': {}, 'driver_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}
    for scheme in ['quadtree', 'octree']:
        path = MODEL/(scheme+'-source-order.json')
        raw = path.read_bytes()
        manifest['model_inputs'][path.name] = hashlib.sha256(raw).hexdigest()
        fixture = json.loads(raw)
        original = {name: base64.b64decode(value) for name, value in fixture['members_base64'].items()}
        for control in ['candidate', 'double-child-transform', 'root-external-available']:
            members = copy.deepcopy(original)
            suffix = '-0' if scheme == 'octree' else ''
            child_uri = 'n0-link-1-0-0'+suffix+'.json'
            if control == 'double-child-transform':
                doc = json.loads(members[child_uri])
                doc['root']['transform'][12] += 2
                members[child_uri] = jsbytes(doc)
            elif control == 'root-external-available':
                name = 'n0-0-0-0'+suffix+'.subtree'
                def available(doc, binary):
                    d = doc['contentAvailability'][-1]
                    offset = doc['bufferViews'][d['bitstream']]['byteOffset']
                    binary[offset] |= 1
                    d['availableCount'] += 1
                members[name] = mutate(members[name], available)
                members['n0-link-0-0-0'+suffix+'.json'] = members[child_uri]
            case = scheme+'-'+control
            folder = args.directory/case
            folder.mkdir()
            (folder/'source.json').write_bytes(jsbytes(fixture['source']))
            (folder/'tileset.json').write_bytes(members['n0.json'])
            for name, data in sorted(members.items()):
                target = folder/name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(data)
            manifest['cases'][case] = {'scheme': scheme.upper(), 'kind': control,
                'member_sha256': {str(p.relative_to(folder)): hashlib.sha256(p.read_bytes()).hexdigest()
                                  for p in sorted(folder.rglob('*')) if p.is_file()}}
        # One available implicit root, identity manifest transform, present
        # TILE_TRANSFORM metadata. A conforming application of the semantic
        # would translate its payload; the intended client's behavior is measured.
        members = copy.deepcopy(original)
        doc = json.loads(members['n2.json'])
        doc['root']['transform'] = [1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1]
        doc['schema']['classes']['a2Tile']['properties']['placement'] = {
            'type': 'MAT4', 'componentType': 'FLOAT64', 'semantic': 'TILE_TRANSFORM'}
        doc['root']['implicitTiling']['subtrees']['uri'] = 'transform-{level}-{x}-{y}'+('-{z}' if scheme == 'octree' else '')+'.subtree'
        transform = [1,0,0,0,0,1,0,0,0,0,1,0,9,10,11,1]
        def placement(subtree, binary):
            binary.extend(b'\0'*(-len(binary)%8))
            index = len(subtree['bufferViews'])
            subtree['bufferViews'].append({'buffer':0, 'byteOffset':len(binary), 'byteLength':128})
            binary.extend(struct.pack('<16d', *transform))
            subtree['buffers'][0]['byteLength'] = len(binary)
            subtree['propertyTables'][0]['properties']['placement'] = {'values': index}
        data = mutate(members['n2-0-0-0'+('-0' if scheme == 'octree' else '')+'.subtree'], placement)
        case = scheme+'-metadata-transform'
        folder = args.directory/case
        folder.mkdir()
        (folder/'tileset.json').write_bytes(jsbytes(doc))
        (folder/('transform-0-0-0'+('-0' if scheme == 'octree' else '')+'.subtree')).write_bytes(data)
        for name, raw in members.items():
            if name.endswith('.glb'):
                (folder/name).write_bytes(raw)
        manifest['cases'][case] = {'scheme': scheme.upper(), 'kind':'metadata-transform',
            'expected_local_transform': transform,
            'member_sha256': {str(p.relative_to(folder)): hashlib.sha256(p.read_bytes()).hexdigest()
                              for p in sorted(folder.rglob('*')) if p.is_file()}}
    (args.directory/'manifest.json').write_text(json.dumps(manifest, indent=2, sort_keys=True)+'\n')
    print(json.dumps({'cases': len(manifest['cases']), 'directory': str(args.directory)}))


if __name__ == '__main__':
    main()
