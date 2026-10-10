#!/usr/bin/env python3
"""Reviewer-authored planar GLB through a frozen CLI and independent reader.

The literal fixture is separate from the producer/evidence owners' fixtures.
Artifacts are temporary; the receipt retains source/archive/member identities.
"""
import argparse
from fractions import Fraction as F
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import zipfile

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / 'tests'))
import f1d1_oracle as oracle


def digest(raw): return hashlib.sha256(raw).hexdigest()


def source():
    positions = [(float(x), float(y), 0.) for y in range(5) for x in range(5)]
    indices = []
    for y in range(4):
        for x in range(4):
            a = 5*y+x
            indices.extend((a, a+1, a+6, a, a+6, a+5))
    coordinates = b''.join(struct.pack('<3f', *p) for p in positions)
    triangles = struct.pack('<'+'I'*len(indices), *indices)
    binary = coordinates + triangles
    document = {'asset': {'version': '2.0'}, 'scene': 0, 'scenes': [{'nodes': [0]}],
                'nodes': [{'mesh': 0}], 'meshes': [{'primitives': [
                    {'attributes': {'POSITION': 0}, 'indices': 1}]}],
                'buffers': [{'byteLength': len(binary)}],
                'bufferViews': [{'buffer': 0, 'byteOffset': 0, 'byteLength': len(coordinates)},
                                {'buffer': 0, 'byteOffset': len(coordinates), 'byteLength': len(triangles)}],
                'accessors': [{'bufferView': 0, 'componentType': 5126, 'count': 25,
                               'type': 'VEC3', 'min': [0,0,0], 'max': [4,4,0]},
                              {'bufferView': 1, 'componentType': 5125, 'count': len(indices), 'type': 'SCALAR'}]}
    metadata = json.dumps(document, separators=(',', ':')).encode()
    metadata += b' ' * (-len(metadata) % 4)
    assert len(binary) % 4 == 0
    return (struct.pack('<4sII', b'glTF', 2, 28+len(metadata)+len(binary))
            + struct.pack('<I4s', len(metadata), b'JSON') + metadata
            + struct.pack('<I4s', len(binary), b'BIN\0') + binary)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    binary_sha = digest(binary.read_bytes())
    raw = source()
    with tempfile.TemporaryDirectory(prefix='f1d2-nonauthor-replay-') as temporary:
        work = Path(temporary)
        input_path, output = work/'source.glb', work/'result.3tz'
        input_path.write_bytes(raw)
        command = [str(binary), '--json', 'mesh-local-to-3tz', '-i', str(input_path),
                   '-o', str(output), '--leaf-triangles', '16',
                   '--root-proxy-triangles', '16', '--max-proxy-error-metres', '0.5']
        completed = subprocess.run(command, capture_output=True, text=True, timeout=120)
        assert completed.returncode == 0, completed.stdout + completed.stderr
        with zipfile.ZipFile(output) as archive:
            assert len(archive.namelist()) == len(set(archive.namelist()))
            members = {name: archive.read(name) for name in archive.namelist() if name != '@3dtilesIndex1@'}
        actual = oracle.inspect_members(raw, members, 16, 16, .5)
        oracle.inspect_cli_report(completed.stdout, actual['report'])
        report = json.loads(completed.stdout)['meshReport']
        certificate = report['approximation']['certificate']
        historical = F(actual['historical_whole_face_squared'])
        assert F(certificate['error_metres'])**2 < historical
        assert F(.5)**2 < historical
        controls = oracle.corruption_controls(raw, members, 16, 16, .5)
        cli_controls = []
        payload = json.loads(completed.stdout)
        for label, change in [('stale-certificate', lambda p: p['meshReport']['approximation']['certificate'].update(error_metres=0)),
                              ('missing-report', lambda p: p.pop('meshReport')),
                              ('false-success', lambda p: p.update(ok=False)),
                              ('wrong-profile', lambda p: p['meshReport'].update(profile='f1d1-root-proxy-gltf-v1'))]:
            altered = json.loads(json.dumps(payload)); change(altered)
            try: oracle.inspect_cli_report(json.dumps(altered), actual['report'])
            except oracle.leaf_oracle.OracleError: cli_controls.append(label)
            else: raise AssertionError('insensitive CLI report comparison: ' + label)
        # The parity helper must distinguish True from integer 1. This extra
        # synthetic report control is solely a serialization comparison check.
        one_depth = json.loads(json.dumps(actual['report']))
        one_depth['approximation']['certificate']['max_depth'] = 1
        boolean = {'ok': True, 'meshReport': json.loads(json.dumps(one_depth))}
        boolean['meshReport']['approximation']['certificate']['max_depth'] = True
        try: oracle.inspect_cli_report(json.dumps(boolean), one_depth)
        except oracle.leaf_oracle.OracleError: cli_controls.append('synthetic-boolean-one-collision')
        else: raise AssertionError('boolean/integer CLI equality collision')
        assert digest(binary.read_bytes()) == binary_sha
        print(json.dumps({'scope': __doc__.strip(), 'production_commit': '4553533e64c1494e706888a2c8e3a38a6867ce56',
                         'driver_sha256': digest(Path(__file__).read_bytes()),
                         'oracle_sha256': {p.name: digest(p.read_bytes()) for p in
                                          (ROOT/'tests/f1d1_oracle.py', ROOT/'tests/f1d2_certificate.py', ROOT/'tests/f1c2_oracle.py')},
                         'binary_path': str(binary), 'binary_sha256': binary_sha,
                         'source_sha256': digest(raw), 'source_bytes': len(raw),
                         'archive_sha256': digest(output.read_bytes()), 'exit_code': completed.returncode,
                         'stderr': completed.stderr, 'artifact': actual, 'controls': controls,
                         'cli_report_corruptions_rejected': cli_controls,
                         'cli_report_equals_published_report': True, 'passed': True}, indent=2))


if __name__ == '__main__': main()
