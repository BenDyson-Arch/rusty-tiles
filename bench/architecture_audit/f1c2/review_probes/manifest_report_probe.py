#!/usr/bin/env python3
"""Nonauthor CLI manifest report-location probe over an independently authored GLB."""
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
    payload = struct.pack('<9f', 0, 0, 0, 1, 0, 0, 0, 1, 0)
    doc = {'asset': {'version': '2.0'}, 'buffers': [{'byteLength': 36}],
           'bufferViews': [{'buffer': 0, 'byteLength': 36}],
           'accessors': [{'bufferView': 0, 'componentType': 5126, 'count': 3,
                          'type': 'VEC3', 'min': [0, 0, 0], 'max': [1, 1, 0]}],
           'meshes': [{'primitives': [{'attributes': {'POSITION': 0}}]}],
           'nodes': [{'mesh': 0}], 'scenes': [{'nodes': [0]}], 'scene': 0}
    text = json.dumps(doc, separators=(',', ':')).encode()
    text += b' ' * (-len(text) % 4)
    source = (struct.pack('<4sIIII', b'glTF', 2, 28 + len(text) + len(payload),
                          len(text), 0x4e4f534a) + text
              + struct.pack('<II', len(payload), 0x004e4942) + payload)
    with tempfile.TemporaryDirectory(prefix='w1-manifest-report-review-') as temp:
        root = Path(temp)
        input_path = root / 'source.glb'
        input_path.write_bytes(source)
        call = subprocess.run([str(binary), '--json', 'createTilesetJson', '-i', str(input_path)],
                              capture_output=True, text=True, timeout=90)
        assert call.returncode == 0, (call.stdout, call.stderr)
        result = json.loads(call.stdout)
        assert result['modelReport']['product'] == 'manifest'
        assert Path(result['output']) == root / 'tileset.json'
        assert sorted(p.name for p in root.iterdir()) == ['source.glb', 'tileset.json']
        location = result['conversionReport']
        if args.expect == 'defect':
            assert location == {'path': str(root / 'tileset.json' / 'conversion.json')}
            assert not Path(location['path']).exists()
        else:
            assert location is None, location
        receipt = {'binary_sha256': binary_hash, 'probe_sha256': sha(Path(__file__).read_bytes()),
                   'source_sha256': sha(source), 'expect': args.expect,
                   'conversionReport': location, 'sidecar_absent': True,
                   'inline_model_report_present': True,
                   'manifest_sha256': sha((root / 'tileset.json').read_bytes())}
    assert sha(binary.read_bytes()) == binary_hash
    args.output.write_text(json.dumps(receipt, indent=2) + '\n')


if __name__ == '__main__':
    main()
