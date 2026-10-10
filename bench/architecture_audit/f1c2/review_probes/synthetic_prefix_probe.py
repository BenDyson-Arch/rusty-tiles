#!/usr/bin/env python3
"""Nonauthor admission control: synthetic model root must not be a directory."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import tempfile


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--expect', choices=('defect', 'fixed'), required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    binary_hash = sha(binary.read_bytes())
    records = []
    with tempfile.TemporaryDirectory(prefix='w1-synthetic-prefix-review-') as temp:
        for suffix in ('gltf', 'glb'):
            root = Path(temp) / suffix
            root.mkdir()
            dependency_dir = root / ('source.' + suffix)
            dependency_dir.mkdir()
            payload = struct.pack('<9f', 0, 0, 0, 1, 0, 0, 0, 1, 0)
            (dependency_dir / 'data.bin').write_bytes(payload)
            doc = {'asset': {'version': '2.0'},
                   'buffers': [{'byteLength': 36}],
                   'bufferViews': [{'buffer': 0, 'byteLength': 36}],
                   'accessors': [{'bufferView': 0, 'componentType': 5126, 'count': 3,
                                  'type': 'VEC3', 'min': [0, 0, 0], 'max': [1, 1, 0]}],
                   'meshes': [{'primitives': [{'attributes': {'POSITION': 0}}]}],
                   'nodes': [{'mesh': 0}], 'scenes': [{'nodes': [0]}], 'scene': 0}
            uri = 'source.' + suffix + '/data.bin'
            if suffix == 'gltf':
                doc['buffers'][0]['uri'] = uri
                source = json.dumps(doc, separators=(',', ':')).encode()
            else:
                doc['buffers'].append({'byteLength': 36, 'uri': uri})
                text = json.dumps(doc, separators=(',', ':')).encode()
                text += b' ' * (-len(text) % 4)
                source = (struct.pack('<4sIIII', b'glTF', 2, 28 + len(text) + len(payload),
                                      len(text), 0x4e4f534a) + text
                          + struct.pack('<II', len(payload), 0x004e4942) + payload)
            input_path = root / ('original.' + suffix)
            input_path.write_bytes(source)
            output_parent = root / 'absent'
            call = subprocess.run([str(binary), '--json', 'glb-to-3tz', '-i', str(input_path),
                                   '-o', str(output_parent / 'out.3tz')],
                                  capture_output=True, text=True, timeout=90)
            result = json.loads(call.stdout)
            expected = 'io' if args.expect == 'defect' else 'unsupported'
            assert result['error']['kind'] == expected, result
            assert output_parent.exists() == (args.expect == 'defect')
            assert input_path.read_bytes() == source
            assert (dependency_dir / 'data.bin').read_bytes() == payload
            records.append({'kind': suffix, 'source_sha256': sha(source),
                            'error_kind': expected, 'output_parent_exists': output_parent.exists(),
                            'source_preserved': True})
    assert sha(binary.read_bytes()) == binary_hash
    args.output.write_text(json.dumps({'binary_sha256': binary_hash,
        'probe_sha256': sha(Path(__file__).read_bytes()), 'expect': args.expect,
        'records': records}, indent=2) + '\n')


if __name__ == '__main__':
    main()
