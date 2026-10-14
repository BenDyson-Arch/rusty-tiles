#!/usr/bin/env python3
"""F1b2 file/byte-bound measurements; consumer-authored, not a fidelity oracle."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import signal
import struct
import subprocess
import sys
import tempfile
import time
import zipfile

ROOT = Path(__file__).resolve().parents[3]
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('f1b_resource_measurement', ROOT / 'bench/architecture_audit/f1b1/resource_probe.py')
prior = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prior)
MIB = 1024 * 1024
CASES = ('64-distinct-resources', '64-aliased-resources', '64MiB-exact', '64MiB-plus-one', 'dependency32MiB-exact', 'dependency32MiB-plus-one', 'json1MiB-exact', 'json1MiB-plus-one')


def fixture(work, case):
    points = struct.pack('<9f', 0, 0, 0, 1, 0, 0, 0, 1, 0)
    doc = {'asset': {'version': '2.0'}, 'scene': 0, 'scenes': [{'nodes': [0]}], 'nodes': [{'mesh': 0}],
           'meshes': [{'primitives': [{'attributes': {'POSITION': 0}}]}],
           'buffers': [{'uri': 'b0.bin', 'byteLength': len(points)}],
           'bufferViews': [{'buffer': 0, 'byteLength': len(points)}],
           'accessors': [{'bufferView': 0, 'componentType': 5126, 'count': 3, 'type': 'VEC3', 'min': [0, 0, 0], 'max': [1, 1, 0]}]}
    (work / 'b0.bin').write_bytes(points)
    many = case.startswith('64-') or case.startswith('64MiB')
    if many:
        doc['buffers'].extend({'uri': f'b{i}.bin', 'byteLength': len(points)} for i in range(1, 32))
        doc['images'] = [{'uri': f'i{i}.png'} for i in range(32)]
        image = prior.png(1, 1)
        (work / 'i0.png').write_bytes(image)
        for i in range(1, 32):
            if case == '64-aliased-resources':
                os.link(work / 'b0.bin', work / f'b{i}.bin')
                os.link(work / 'i0.png', work / f'i{i}.png')
            else:
                (work / f'b{i}.bin').write_bytes(points)
                (work / f'i{i}.png').write_bytes(image)
    encoded = json.dumps(doc, separators=(',', ':')).encode()
    if case.startswith('json1MiB'):
        encoded += b' ' * (MIB - len(encoded) + int(case.endswith('plus-one')))
    if case.startswith('dependency32MiB'):
        with (work / 'b0.bin').open('r+b') as stream:
            stream.truncate(32 * MIB + int(case.endswith('plus-one')))
    if case.startswith('64MiB'):
        current = len(encoded) + sum(p.stat().st_size for p in work.iterdir())
        padding = 64 * MIB - current + int(case.endswith('plus-one'))
        for i in (0, 1, 2):
            path = work / f'b{i}.bin'
            amount = min(padding, 32 * MIB - path.stat().st_size)
            with path.open('r+b') as stream:
                stream.truncate(path.stat().st_size + amount)
            padding -= amount
        assert padding == 0
    source = work / 'source.gltf'
    source.write_bytes(encoded)
    identities = {}
    for path in work.iterdir():
        stat = path.stat()
        identities.setdefault((stat.st_dev, stat.st_ino), (path, stat.st_size))
    unique_external = [(path, length) for path, length in identities.values() if path != source]
    expected = {'source_bytes': len(encoded), 'external_files': len(unique_external), 'external_bytes': sum(length for _, length in unique_external), 'uri_requests': 64 if many else 1}
    return source, expected


def measure(binary, launcher, root, case, timeout):
    work = root / case
    work.mkdir()
    source, expected = fixture(work, case)
    output = work / 'result.3tz'
    rss, outlog, errlog = (work / name for name in ('rss.txt', 'stdout.json', 'stderr.txt'))
    samples = {field: {'peak': None, 'samples': 0, 'permission_denials': 0, 'vanished_races': 0} for field in ('threads', 'file_descriptors', 'converter_child_processes')}
    scratch = {'peak_files': 0, 'peak_bytes': 0, 'samples': 0, 'permission_denials': 0, 'vanished_races': 0}
    started = time.monotonic()
    termination = None
    with outlog.open('wb') as stdout, errlog.open('wb') as stderr:
        child = subprocess.Popen([str(launcher), str(rss), str(binary), '--json', 'mesh-local-to-3tz', '-i', str(source), '-o', str(output), '--leaf-triangles', '1'], stdout=stdout, stderr=stderr, start_new_session=True)
        while child.poll() is None:
            try:
                for pid in Path(f'/proc/{child.pid}/task/{child.pid}/children').read_text().split():
                    prior.sample_process(pid, samples)
            except (FileNotFoundError, ProcessLookupError, PermissionError):
                pass
            prior.sample_workspace(work, scratch)
            if time.monotonic() - started > timeout or max(outlog.stat().st_size, errlog.stat().st_size) > MIB:
                termination = 'timeout-or-log-bound'
                try:
                    os.killpg(child.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                try:
                    child.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait()
                break
            time.sleep(.002)
        child.wait()
    payload = json.loads(outlog.read_text())
    success = not case.endswith('plus-one')
    assert termination is None and (child.returncode == 0) == success, (case, payload, errlog.read_text())
    assert output.exists() == success
    assert not any(p.name.startswith(('.mesh-work-', '.tiles-')) for p in work.iterdir())
    if success:
        report = payload['meshReport']
        for key in ('source_bytes', 'external_files', 'external_bytes'):
            assert report[key] == expected[key], (case, key, report, expected)
        with zipfile.ZipFile(output) as archive:
            assert set(archive.namelist()) == {'tileset.json', 'conversion.json', 't/0.glb', '@3dtilesIndex1@'}
            assert json.loads(archive.read('conversion.json')) == report
        assert report['images'] == 0 and report['triangles'] == 1
    else:
        assert 'unsupported' in json.dumps(payload)
    return {'case': case, 'passed': True, 'expected': expected, 'exit_code': child.returncode, 'elapsed_seconds': time.monotonic() - started, 'peak_rss_kib_wait4': int(rss.read_text()), 'sampled_process_observations': samples, 'sampled_workspace': scratch, 'result': payload}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--timeout', type=float, default=90)
    args = parser.parse_args()
    binary = args.binary.resolve()
    identity = prior.digest(binary.read_bytes())
    receipt = {'binary_sha256': identity, 'platform': platform.platform(), 'probe_sha256': prior.digest(Path(__file__).read_bytes()), 'provenance': 'Consumer-authored finite file/byte workload, literal triangle and PNG. Shares only prior measurement helpers, not independent acceptance truth.', 'measurement_contract': 'Linux child wait4 maximum RSS excludes Python fixture author. FD/thread/scratch observations sampled at >=2ms plus inspection time, not hard maxima. Scratch excludes archive staging. Stable files; no filesystem hostile-race guarantee.', 'results': []}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='rusty-f1b2-resource-') as temporary:
        root = Path(temporary)
        code, launcher = root / 'measure.c', root / 'measure'
        code.write_text(prior.LAUNCHER)
        subprocess.run(['cc', '-O2', '-Wall', '-Wextra', '-Werror', str(code), '-o', str(launcher)], check=True, timeout=30)
        for case in CASES:
            result = measure(binary, launcher, root, case, args.timeout)
            receipt['results'].append(result)
            args.output.write_text(json.dumps(receipt, indent=2) + '\n')
            print(json.dumps({'case': case, 'passed': True, 'rss_kib': result['peak_rss_kib_wait4']}), flush=True)
    assert prior.digest(binary.read_bytes()) == identity


if __name__ == '__main__':
    main()
