#!/usr/bin/env python3
"""Nonauthor W1 emitted alias boundary and measured finite process resources."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import time
import zipfile


def digest(path):
    with open(path, 'rb') as source:
        result = hashlib.file_digest(source, 'sha256')
    return result.hexdigest()


def execute(command, directory, measurements):
    started = time.monotonic()
    with subprocess.Popen(command,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) as child:
        scratch_peak, descriptor_peak = 0, 0
        while True:
            waited, status, usage = os.wait4(child.pid, os.WNOHANG)
            if waited:
                child.returncode = os.waitstatus_to_exitcode(status)
                break
            scratch = 0
            for path in directory.rglob('*'):
                if any(part.startswith('.') for part in path.relative_to(directory).parts) and path.is_file():
                    try:
                        scratch += path.stat().st_size
                    except FileNotFoundError:
                        pass
            scratch_peak = max(scratch_peak, scratch)
            try:
                descriptor_peak = max(descriptor_peak, len(list(Path(f'/proc/{child.pid}/fd').iterdir())))
            except FileNotFoundError:
                pass
            time.sleep(.01)
        stdout, stderr = child.communicate(timeout=10)
    measurements.update(elapsed_seconds=time.monotonic()-started, sampled_scratch_bytes=scratch_peak,
                        sampled_open_descriptors=descriptor_peak,
                        max_rss_kib=usage.ru_maxrss)
    return child.returncode, json.loads(stdout), stderr


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    binary_hash = digest(binary)
    records = []
    with tempfile.TemporaryDirectory(prefix='w1-emitted-alias-review-') as temporary:
        root = Path(temporary)
        source = {'asset': {'version': '2.0', 'generator': 'review'}, 'buffers':
            [{'uri': 'a.bin', 'byteLength': 36}, {'uri': 'b.bin', 'byteLength': 36}],
            'bufferViews': [{'buffer': 0, 'byteLength': 36}],
            'accessors': [{'bufferView': 0, 'componentType': 5126, 'type': 'VEC3', 'count': 3,
                           'min': [0, 0, 0], 'max': [1, 1, 0]}],
            'meshes': [{'primitives': [{'attributes': {'POSITION': 0}}]}],
            'nodes': [{'mesh': 0}], 'scenes': [{'nodes': [0]}], 'scene': 0}
        source_bytes = json.dumps(source, separators=(',', ':')).encode()
        if len(source_bytes) % 2:
            source['asset']['generator'] += 'x'
            source_bytes = json.dumps(source, separators=(',', ':')).encode()
        size = (64*1024*1024-len(source_bytes))//2
        for case, payload_size in [('exact-64MiB', size), ('over-64MiB-by-two', size+1)]:
            directory = root/case
            directory.mkdir()
            input_path = directory/'source.gltf'
            input_path.write_bytes(source_bytes)
            a = directory/'a.bin'
            with a.open('wb') as target:
                target.write(struct.pack('<9f', 0, 0, 0, 1, 0, 0, 0, 1, 0))
                target.truncate(payload_size)
            os.link(a, directory/'b.bin')
            output = directory/'out.3tz'
            if case.startswith('over'):
                output.write_bytes(b'KEEP')
            measurements = {}
            code, report, stderr = execute([str(binary), '--json', 'glb-to-3tz', '-i', str(input_path),
                                           '-o', str(output), '--force'], directory, measurements)
            emitted_bytes = len(source_bytes)+2*payload_size
            if code == 0:
                assert emitted_bytes == 64*1024*1024
                result = report['modelReport']
                assert result['model_payload_bytes'] == emitted_bytes
                assert result['model_payload_files'] == 3 and result['external_files'] == 1
                assert result['external_bytes'] == payload_size
                payload_hash = digest(a)
                with zipfile.ZipFile(output) as archive:
                    assert archive.read('model/source.gltf') == source_bytes
                    for name in ('model/a.bin', 'model/b.bin'):
                        with archive.open(name) as member:
                            assert hashlib.file_digest(member, 'sha256').hexdigest() == payload_hash
            else:
                assert emitted_bytes == 64*1024*1024+2
                assert report['error']['kind'] == 'unsupported'
                assert 'emitted alias' in report['error']['message']
                assert output.read_bytes() == b'KEEP'
                assert measurements['sampled_scratch_bytes'] == 0
            assert not any(path.name.startswith('.') for path in directory.iterdir())
            records.append({'case': case, 'logical_emitted_bytes': emitted_bytes, 'unique_external_bytes': payload_size,
                            'source_sha256': hashlib.sha256(source_bytes).hexdigest(),
                            'artifact_sha256': digest(output), 'status': code, 'result': report,
                            'measurements': measurements})
    assert digest(binary) == binary_hash
    args.output.write_text(json.dumps({'binary_sha256': binary_hash, 'probe_sha256': digest(Path(__file__)),
        'records': records, 'limits': ['Linux wait4 max RSS for these executions; descriptors/scratch sampled at10ms',
                                    'no whole-process bound inferred; two distinct hard-link alias names']}, indent=2)+'\n')


if __name__ == '__main__':
    main()
