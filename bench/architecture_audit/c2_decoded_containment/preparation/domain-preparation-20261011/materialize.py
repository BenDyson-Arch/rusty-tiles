#!/usr/bin/env python3
"""Unexecuted literal input preparation; no repository import or transform math.

Writes local glTF resources and optional modern b3dm, plus unmodified context and
literal expected records. It does not produce 3TZ indexes or target observations.
Root must separately pin any materialization and execute exact-source acceptance.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import struct

HERE = Path(__file__).resolve().parent


def encoded_positions(case):
    component = case['componentType']
    fmt, width = {5126: ('f', 4), 5120: ('b', 1), 5121: ('B', 1),
                  5122: ('h', 2), 5123: ('H', 2)}[component]
    stride = (3 * width + 3) // 4 * 4
    data = bytearray()
    for position in case['positions']:
        data.extend(struct.pack('<' + fmt * 3, *position))
        data.extend(bytes(stride - 3 * width))
    return bytes(data), stride


def frame_glb(document, data):
    document = copy.deepcopy(document)
    if document.get('buffers'):
        document['buffers'][0].pop('uri', None)
    text = json.dumps(document, separators=(',', ':'), allow_nan=False).encode()
    text += b' ' * (-len(text) % 4)
    binary = data + bytes(-len(data) % 4)
    body = struct.pack('<II', len(text), 0x4E4F534A) + text
    if data:
        body += struct.pack('<II', len(binary), 0x004E4942) + binary
    return b'glTF' + struct.pack('<II', 2, 12 + len(body)) + body


def frame_b3dm(glb, rtc):
    table = json.dumps({'BATCH_LENGTH': 0, 'RTC_CENTER': rtc},
                       separators=(',', ':'), allow_nan=False).encode()
    table += b' ' * (-(28 + len(table)) % 8)
    tail = bytes(-(28 + len(table) + len(glb)) % 8)
    size = 28 + len(table) + len(glb) + len(tail)
    return b'b3dm' + struct.pack('<6I', 1, size, len(table), 0, 0, 0) + table + glb + tail


def materialize(case, folder):
    folder.mkdir()
    viewless = case.get('viewless', False)
    data, stride = encoded_positions(case)
    accessor = {'componentType': case['componentType'], 'type': 'VEC3',
                'count': len(case['positions']),
                'min': case.get('declared_min', [min(p[i] for p in case['positions']) for i in range(3)]),
                'max': case.get('declared_max', [max(p[i] for p in case['positions']) for i in range(3)])}
    if case.get('normalized'):
        accessor['normalized'] = True
    document = {'asset': {'version': '2.0'}, 'accessors': [accessor],
                'meshes': [{'primitives': [{'attributes': {'POSITION': 0}, 'mode': 0}]}],
                'nodes': case['nodes'], 'scenes': case['scenes']}
    if 'scene' in case:
        document['scene'] = case['scene']
    if not viewless:
        accessor['bufferView'] = 0
        document['buffers'] = [{'byteLength': len(data), 'uri': 'geometry.bin'}]
        document['bufferViews'] = [{'buffer': 0, 'byteLength': len(data), 'target': 34962}]
        if stride != 12:
            document['bufferViews'][0]['byteStride'] = stride
    else:
        data = b''
    if case['componentType'] != 5126:
        document['extensionsUsed'] = ['KHR_mesh_quantization']
        document['extensionsRequired'] = ['KHR_mesh_quantization']
    if 'indices' in case:
        offset = len(data)
        indices = struct.pack('<' + 'H' * len(case['indices']), *case['indices'])
        data += indices
        document['buffers'][0]['byteLength'] = len(data)
        document['bufferViews'].append({'buffer': 0, 'byteOffset': offset,
                                      'byteLength': len(indices), 'target': 34963})
        document['accessors'].append({'bufferView': 1, 'componentType': 5123,
                                     'type': 'SCALAR', 'count': len(case['indices'])})
        document['meshes'][0]['primitives'][0]['indices'] = 1
    (folder / 'scene.gltf').write_text(json.dumps(document, indent=2, allow_nan=False) + '\n')
    if data:
        (folder / 'geometry.bin').write_bytes(data)
    glb = frame_glb(document, data)
    (folder / 'scene.glb').write_bytes(glb)
    if case.get('kind') == 'b3dm':
        (folder / 'scene.b3dm').write_bytes(frame_b3dm(glb, case['rtc']))
    (folder / 'context-and-literal-truth.json').write_text(json.dumps(case, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = json.loads((HERE / 'witnesses.json').read_text())
    for case in manifest['cases']:
        materialize(case, args.output / case['id'])
    records = []
    for path in sorted(args.output.rglob('*')):
        if path.is_file():
            data = path.read_bytes()
            records.append({'path': str(path.relative_to(args.output)), 'bytes': len(data),
                            'sha256': hashlib.sha256(data).hexdigest()})
    (args.output / 'materialization-pin.json').write_text(json.dumps({
        'record': 'Input materialization only; no target execution or acceptance',
        'source_manifest_sha256': hashlib.sha256((HERE / 'witnesses.json').read_bytes()).hexdigest(),
        'records': records}, indent=2) + '\n')


if __name__ == '__main__':
    main()
