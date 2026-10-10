#!/usr/bin/env python3
"""Nonauthor W1 bounds and exact-byte review; no production/test helper imports."""
import argparse
import copy
from decimal import Decimal, localcontext
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import tempfile
import zipfile


def digest(data):
    return hashlib.sha256(data).hexdigest()


def d(value):
    return Decimal.from_float(float(value))


def identity():
    return [[Decimal(r == c) for c in range(4)] for r in range(4)]


def product(a, b):
    return [[sum((a[r][k]*b[k][c] for k in range(4)), Decimal(0))
             for c in range(4)] for r in range(4)]


def local(node, normalized):
    if 'matrix' in node:
        return [[d(node['matrix'][c*4+r]) for c in range(4)] for r in range(4)]
    x, y, z, w = map(d, node.get('rotation', [0, 0, 0, 1]))
    if normalized:
        norm = (x*x+y*y+z*z+w*w).sqrt()
        x, y, z, w = [v/norm for v in (x, y, z, w)]
    rotation = [[1-2*(y*y+z*z), 2*(x*y-z*w), 2*(x*z+y*w)],
                [2*(x*y+z*w), 1-2*(x*x+z*z), 2*(y*z-x*w)],
                [2*(x*z-y*w), 2*(y*z+x*w), 1-2*(x*x+y*y)]]
    result = identity()
    for r in range(3):
        for c in range(3):
            result[r][c] = rotation[r][c]*d(node.get('scale', [1, 1, 1])[c])
        result[r][3] = d(node.get('translation', [0, 0, 0])[r])
    return result


def points(doc, payload, normalized):
    result = []

    def visit(index, parent):
        node = doc['nodes'][index]
        matrix = product(parent, local(node, normalized))
        if 'mesh' in node:
            for primitive in doc['meshes'][node['mesh']]['primitives']:
                accessor = doc['accessors'][primitive['attributes']['POSITION']]
                view = doc['bufferViews'][accessor['bufferView']]
                start = view.get('byteOffset', 0)+accessor.get('byteOffset', 0)
                positions = [struct.unpack_from('<3f', payload, start+i*view.get('byteStride', 12))
                             for i in range(accessor['count'])]
                indices = range(len(positions))
                if 'indices' in primitive:
                    accessor = doc['accessors'][primitive['indices']]
                    view = doc['bufferViews'][accessor['bufferView']]
                    start = view.get('byteOffset', 0)+accessor.get('byteOffset', 0)
                    indices = struct.unpack_from('<'+'H'*accessor['count'], payload, start)
                for index in indices:
                    point = [*map(d, positions[index]), Decimal(1)]
                    p = [sum((matrix[r][c]*point[c] for c in range(4)), Decimal(0)) for r in range(3)]
                    result.append([p[0], -p[2], p[1]])
        for child in node.get('children', []):
            visit(child, matrix)

    for index in doc['scenes'][doc.get('scene', 0)]['nodes']:
        visit(index, identity())
    return result


def bounds_contains(box, values):
    center = list(map(d, box[:3]))
    halves = list(map(d, [box[3], box[7], box[11]]))
    return all(center[a]-halves[a] <= p[a] <= center[a]+halves[a]
               for p in values for a in range(3))


def exact_box(values, bake=False):
    if bake:
        values = [[d(struct.unpack('<f', struct.pack('<f', float(v)))[0]) for v in p] for p in values]
    lower = [min(p[a] for p in values) for a in range(3)]
    upper = [max(p[a] for p in values) for a in range(3)]
    center = [float((lo+hi)/2) for lo, hi in zip(lower, upper)]
    halves = [float((hi-lo)/2) for lo, hi in zip(lower, upper)]
    return [*center, halves[0], 0, 0, 0, halves[1], 0, 0, 0, halves[2]]


def glb(doc, payload):
    text = json.dumps(doc, separators=(',', ':')).encode()
    text += b' '*((-len(text)) % 4)
    payload += b'\0'*((-len(payload)) % 4)
    return (struct.pack('<4sIIII', b'glTF', 2, 28+len(text)+len(payload), len(text), 0x4e4f534a)
            + text + struct.pack('<II', len(payload), 0x004e4942) + payload)


def fixture(positions):
    payload = b''.join(struct.pack('<3f', *p) for p in positions)
    doc = {'asset': {'version': '2.0'}, 'buffers': [{'byteLength': len(payload)}],
           'bufferViews': [{'buffer': 0, 'byteLength': len(payload)}],
           'accessors': [{'bufferView': 0, 'componentType': 5126, 'count': len(positions), 'type': 'VEC3',
                          'min': [min(p[a] for p in positions) for a in range(3)],
                          'max': [max(p[a] for p in positions) for a in range(3)]}],
           'meshes': [{'primitives': [{'attributes': {'POSITION': 0}}]}],
           'nodes': [{'mesh': 0}], 'scenes': [{'nodes': [0]}], 'scene': 0}
    return doc, payload


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    binary_hash = digest(binary.read_bytes())
    records = []
    with localcontext() as context, tempfile.TemporaryDirectory(prefix='w1-nonauthor-') as temporary:
        context.prec = 100
        directory = Path(temporary)
        for case in ('f32-bake', 'raw-quaternion', 'matrix-selected-scene', 'indexed-repeats', 'cancellation'):
            positions = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
            if case == 'raw-quaternion':
                positions = [(800000, 300000, -600000), (800001, 300000, -600000), (800000, 300001, -600000)]
            doc, payload = fixture(positions)
            if case == 'f32-bake':
                doc['nodes'][0]['translation'] = [.1, .2, -.3]
            elif case == 'raw-quaternion':
                doc['nodes'][0].update(rotation=[0, 0, .7071069, .7071069], translation=[100000, -200000, 300000])
            elif case == 'matrix-selected-scene':
                doc['nodes'] = [{'mesh': 0, 'translation': [100, 0, 0]},
                                {'children': [2], 'matrix': [0, 2, 0, 0, -3, 0, 0, 0, 0, 0, 4, 0, 10, 20, 30, 1]},
                                {'mesh': 0, 'translation': [.1, -.2, .3], 'scale': [-1, 2, 1]}]
                doc['scenes'] = [{'nodes': [0]}, {'nodes': [1]}]
                doc['scene'] = 1
            elif case == 'indexed-repeats':
                payload += struct.pack('<6H', 0, 1, 2, 0, 1, 2)
                doc['buffers'][0]['byteLength'] = len(payload)
                doc['bufferViews'].append({'buffer': 0, 'byteOffset': 36, 'byteLength': 12})
                doc['accessors'].append({'bufferView': 1, 'componentType': 5123, 'count': 6, 'type': 'SCALAR'})
                doc['meshes'][0]['primitives'][0]['indices'] = 1
            else:
                doc['nodes'] = [{'children': [1], 'translation': [1e15, 0, 0]},
                                {'mesh': 0, 'translation': [-1e15, 0, 0]}]
            source = glb(doc, payload)
            input_path = directory/(case+'.glb')
            input_path.write_bytes(source)
            output = directory/(case+'.3tz')
            call = subprocess.run([str(binary), '--json', 'glb-to-3tz', '-i', str(input_path), '-o', str(output)],
                                  capture_output=True, text=True, timeout=90)
            assert call.returncode == 0, (case, call.stdout, call.stderr)
            with zipfile.ZipFile(output) as archive:
                assert archive.read('model/source.glb') == source
                manifest = json.loads(archive.read('tileset.json'))
                report = json.loads(archive.read('conversion.json'))
            box = manifest['root']['boundingVolume']['box']
            assert box == report['bounding_box']
            assert report['triangles'] == (2 if case == 'indexed-repeats' else 1)
            values = points(doc, payload, False)+points(doc, payload, True)
            assert bounds_contains(box, values), case
            controls = []
            if case == 'f32-bake':
                assert not bounds_contains(exact_box(points(doc, payload, True), True), values)
                controls.append('normalized-f32-bounds-rejected')
            elif case == 'raw-quaternion':
                assert not bounds_contains(exact_box(points(doc, payload, True)), values)
                controls.append('normalized-only-bounds-rejected')
            elif case == 'matrix-selected-scene':
                wrong = copy.deepcopy(doc)
                wrong['scene'] = 0
                assert not bounds_contains(exact_box(points(wrong, payload, False)), values)
                controls.append('wrong-scene-bounds-rejected')
            records.append({'case': case, 'source_sha256': digest(source), 'artifact_sha256': digest(output.read_bytes()),
                            'box': box, 'corners_checked': len(values), 'controls': controls})
    assert digest(binary.read_bytes()) == binary_hash
    args.output.write_text(json.dumps({'binary_sha256': binary_hash, 'probe_sha256': digest(Path(__file__).read_bytes()),
        'decimal_precision': 100, 'cases': records,
        'limits': ['Local only; mathematical decoded-f64 raw/normalized node semantics; no GPU precision claim']}, indent=2)+'\n')


if __name__ == '__main__':
    main()
