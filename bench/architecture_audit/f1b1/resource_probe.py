#!/usr/bin/env python3
"""Finite F1b1 resource evidence; wait4 RSS, sampled /proc/workspace observations.

This is a measurement workload, not the independent texture/geometry acceptance
oracle. The encoder author wrote this probe; fixture provenance is recorded.
"""
import argparse
import copy
import hashlib
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
import zlib

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'tests'))
import f1b_oracle as oracle

MIB = 1024 * 1024
SOURCE_LIMIT = 32 * MIB
LOG_LIMIT = MIB
LAUNCHER = r'''
#define _DEFAULT_SOURCE
#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <unistd.h>
static void interrupted(int sig) { (void)sig; }
int main(int argc, char **argv) {
    if (argc < 3) return 125;
    struct sigaction action = {0};
    action.sa_handler = interrupted;
    sigemptyset(&action.sa_mask);
    if (sigaction(SIGTERM, &action, NULL) || sigaction(SIGINT, &action, NULL)) return 125;
    pid_t pid = fork();
    if (pid == -1) return 125;
    if (pid == 0) {
        execv(argv[2], argv + 2);
        perror("exec converter");
        _exit(125);
    }
    int status;
    struct rusage usage;
    while (wait4(pid, &status, 0, &usage) == -1) {
        if (errno != EINTR) return 125;
    }
    FILE *out = fopen(argv[1], "w");
    if (!out) return 125;
    fprintf(out, "%ld\n", usage.ru_maxrss);
    if (fclose(out) != 0) return 125;
    return WIFEXITED(status) ? WEXITSTATUS(status) : 128 + WTERMSIG(status);
}
'''


def digest(data):
    return hashlib.sha256(data).hexdigest()


def chunk(kind, payload):
    return struct.pack('>I', len(payload)) + kind + payload + struct.pack('>I', zlib.crc32(kind + payload) & 0xffffffff)


def png(width, height, encoded_bytes=None):
    """Own literal RGBA8 PNG: one row at a time, no Python pixel-object list."""
    compressor = zlib.compressobj()
    row = b'\0' + b'\x31\x79\xba\xff' * width
    compressed = bytearray()
    for _ in range(height):
        compressed.extend(compressor.compress(row))
    compressed.extend(compressor.flush())
    header = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0))
    tail = chunk(b'IDAT', bytes(compressed)) + chunk(b'IEND', b'')
    if encoded_bytes is not None:
        padding = encoded_bytes - len(header) - len(tail) - 12
        assert padding >= 6
        header += chunk(b'tEXt', b'probe\0' + b'x' * (padding - 6))
    result = header + tail
    assert encoded_bytes is None or len(result) == encoded_bytes
    return result


def with_images(count, width=1, height=1, encoded_bytes=None, select_all=False):
    doc, binary = oracle.geometry.decode_glb(oracle.fixture(1, transformed=False))
    binary = bytearray(binary)
    binary.extend(b'\0' * (-len(binary) % 4))
    raw = png(width, height, encoded_bytes)
    view = len(doc['bufferViews'])
    doc['bufferViews'].append({'buffer': 0, 'byteOffset': len(binary), 'byteLength': len(raw)})
    binary.extend(raw)
    # Every source image is admitted, including unselected ones. Repeated views
    # isolate decoded-pixel/owned-byte amplification from container byte size.
    doc['images'] = [{'bufferView': view, 'mimeType': 'image/png'} for _ in range(count)]
    doc['textures'] = [{'source': i if select_all else 0, 'sampler': 0} for i in range(count if select_all else 1)]
    doc['materials'] = [{'pbrMetallicRoughness': {'baseColorTexture': {'index': i}}} for i in range(len(doc['textures']))]
    prototype = doc['meshes'][0]['primitives'][0]
    doc['meshes'][0]['primitives'] = []
    for i in range(len(doc['materials'])):
        primitive = copy.deepcopy(prototype)
        primitive['material'] = i
        doc['meshes'][0]['primitives'].append(primitive)
    return oracle.encode_glb(doc, bytes(binary))


def padded_source(target):
    doc, binary = oracle.geometry.decode_glb(with_images(1))
    # JSON's byteLength digit count changes when the BIN grows. Adjust until
    # the independently framed, still-valid GLB has the exact requested size.
    binary = bytearray(binary)
    for _ in range(8):
        raw = oracle.encode_glb(doc, bytes(binary))
        difference = target - len(raw)
        if difference == 0:
            return raw
        assert difference % 4 == 0
        if difference > 0:
            binary.extend(b'\0' * difference)
        else:
            del binary[difference:]
    raise AssertionError('exact source-size fixture did not converge')


CASES = (
    'tiny', 'images-32', 'images-33-rejected', 'square-2048',
    'edge-4096-by-1024', 'pixels-2049-by-2048-rejected', 'edge-4097-rejected',
    'aggregate-4-by-4mip', 'aggregate-5-by-4mip-rejected',
    'owned-bytes-32mib', 'owned-bytes-over-32mib-rejected',
    'source-32mib', 'source-32mib-plus-one-rejected',
    'shared-uv-one-leaf', 'shared-uv-4096-leaves',
)


def fixture(case):
    limit = 4096
    succeeds = not case.endswith('-rejected')
    if case == 'tiny':
        raw, limit = oracle.fixture(8), 3
    elif case in ('images-32', 'images-33-rejected'):
        raw = with_images(32 if succeeds else 33)
    elif case == 'square-2048':
        raw = with_images(1, 2048, 2048)
    elif case == 'edge-4096-by-1024':
        raw = with_images(1, 4096, 1024)
    elif case == 'pixels-2049-by-2048-rejected':
        raw = with_images(1, 2049, 2048)
    elif case == 'edge-4097-rejected':
        raw = with_images(1, 4097, 1)
    elif case.startswith('aggregate-'):
        raw = with_images(4 if succeeds else 5, 2048, 2048)
    elif case.startswith('owned-bytes-'):
        raw = with_images(32, encoded_bytes=MIB if succeeds else MIB + 1, select_all=True)
    elif case.startswith('source-'):
        raw = padded_source(SOURCE_LIMIT)
        if not succeeds:
            # Metadata admission precedes parsing. A GLB cannot have an odd
            # chunk-aligned total length; this adds exactly one excess byte.
            raw += b'\0'
    elif case.startswith('shared-uv-'):
        raw = oracle.shared_uv_fixture(4096, 300000)
        limit = 4096 if case.endswith('one-leaf') else 1
    else:
        raise AssertionError(case)
    return raw, limit, succeeds


def manifest(raw, case):
    valid_raw = raw[:-1] if case == 'source-32mib-plus-one-rejected' else raw
    doc, binary = oracle.geometry.decode_glb(valid_raw)
    primitives = doc['meshes'][0]['primitives']
    material_ids = {p['material'] for p in primitives if 'material' in p}
    texture_ids = {doc['materials'][i]['pbrMetallicRoughness']['baseColorTexture']['index'] for i in material_ids}
    image_ids = sorted({doc['textures'][i]['source'] for i in texture_ids})
    images = []
    for image in doc['images']:
        view = doc['bufferViews'][image['bufferView']]
        start = view.get('byteOffset', 0)
        data = binary[start:start + view['byteLength']]
        assert data.startswith(b'\x89PNG') or case == 'tiny'
        width, height = struct.unpack_from('>II', data, 16)
        images.append({'bytes': len(data), 'width': width, 'height': height, 'sha256': digest(data)})
    triangles = sum(doc['accessors'][p.get('indices', p['attributes']['POSITION'])]['count'] // 3 for p in primitives)
    return {'source_bytes': len(raw), 'source_sha256': digest(raw),
            'source_image_count': len(images), 'source_images': images,
            'owned_source_image_bytes': sum(i['bytes'] for i in images),
            'admitted_source_pixels': sum(i['width'] * i['height'] for i in images),
            'selected_image_ids': image_ids, 'selected_triangles': triangles,
            'declared_accessor_count_sum': sum(a['count'] for a in doc['accessors']),
            'primitive_count': len(primitives)}


def sample_process(pid, observations):
    for field, read in (
        ('threads', lambda: next(int(line.split()[1]) for line in Path(f'/proc/{pid}/status').read_text().splitlines() if line.startswith('Threads:'))),
        ('file_descriptors', lambda: len(list(Path(f'/proc/{pid}/fd').iterdir()))),
        ('converter_child_processes', lambda: len(Path(f'/proc/{pid}/task/{pid}/children').read_text().split())),
    ):
        try:
            value = read()
            item = observations[field]
            item['samples'] += 1
            item['peak'] = value if item['peak'] is None else max(item['peak'], value)
        except PermissionError:
            observations[field]['permission_denials'] += 1
        except (FileNotFoundError, ProcessLookupError):
            observations[field]['vanished_races'] += 1


def sample_workspace(work, observations):
    size = count = 0
    for directory in work.glob('.mesh-work-*'):
        for path in directory.rglob('*'):
            try:
                if path.is_file():
                    size += path.stat().st_size
                    count += 1
            except (FileNotFoundError, ProcessLookupError):
                observations['vanished_races'] += 1
            except PermissionError:
                observations['permission_denials'] += 1
    observations['samples'] += 1
    observations['peak_files'] = max(observations['peak_files'], count)
    observations['peak_bytes'] = max(observations['peak_bytes'], size)


def inventory(output, expected, limit):
    with zipfile.ZipFile(output) as archive:
        names = archive.namelist()
        assert len(names) == len(set(names)), 'duplicate archive members'
        tileset = json.loads(archive.read('tileset.json'))
        leaves = tileset['root']['children']
        actual_report = json.loads(archive.read('conversion.json'))
        image_names = {f'textures/{i}.png' for i in expected['selected_image_ids']}
        leaf_names = {leaf['content']['uri'] for leaf in leaves}
        assert set(names) == image_names | leaf_names | {'tileset.json', 'conversion.json', '@3dtilesIndex1@'}
        for image_id in expected['selected_image_ids']:
            assert digest(archive.read(f'textures/{image_id}.png')) == expected['source_images'][image_id]['sha256']
        total = 0
        for leaf in leaves:
            data = archive.read(leaf['content']['uri'])
            json_size, kind = struct.unpack_from('<II', data, 12)
            assert kind == 0x4e4f534a
            doc = json.loads(data[20:20 + json_size])
            triangles = sum(doc['accessors'][p['attributes']['POSITION']]['count'] // 3 for p in doc['meshes'][0]['primitives'])
            assert 0 < triangles <= limit
            total += triangles
            assert leaf['geometricError'] == 0
        assert total == expected['selected_triangles']
        assert actual_report['triangles'] == total and actual_report['source_bytes'] == expected['source_bytes']
        assert actual_report['images'] == len(image_names) and actual_report['leaf_tiles'] == len(leaves)
        return {'leaf_tiles': len(leaves), 'image_members': len(image_names), 'member_count': len(names),
                'triangle_count_from_output_accessors': total, 'report': actual_report,
                'scope': 'inventory/image-byte/count checks only; independent fidelity oracle is tests/f1b_oracle.py'}


def measure(binary, case, root, launcher, timeout):
    work = root / case
    work.mkdir()
    source, limit, expected_success = fixture(case)
    source_manifest = manifest(source, case)
    input_path = work / 'source.glb'
    input_path.write_bytes(source)
    del source
    output = work / 'result.3tz'
    rss_path = work / 'peak-rss.txt'
    stdout_path, stderr_path = work / 'stdout.json', work / 'stderr.txt'
    observations = {field: {'peak': None, 'samples': 0, 'permission_denials': 0, 'vanished_races': 0}
                    for field in ('threads', 'file_descriptors', 'converter_child_processes')}
    workspace = {'peak_files': 0, 'peak_bytes': 0, 'samples': 0, 'permission_denials': 0, 'vanished_races': 0}
    command = [str(binary), '--json', 'mesh-local-to-3tz', '-i', str(input_path), '-o', str(output), '--leaf-triangles', str(limit)]
    started = time.monotonic()
    termination = None
    child_pids = set()
    launcher_denials = launcher_races = 0
    with stdout_path.open('wb') as stdout, stderr_path.open('wb') as stderr:
        child = subprocess.Popen([str(launcher), str(rss_path), *command], stdout=stdout, stderr=stderr, start_new_session=True)
        while child.poll() is None:
            try:
                pids = Path(f'/proc/{child.pid}/task/{child.pid}/children').read_text().split()
                for pid in pids:
                    child_pids.add(int(pid))
                    sample_process(pid, observations)
            except PermissionError:
                launcher_denials += 1
            except (FileNotFoundError, ProcessLookupError):
                launcher_races += 1
            sample_workspace(work, workspace)
            if time.monotonic() - started > timeout:
                termination = 'timeout'
            elif stdout_path.stat().st_size > LOG_LIMIT or stderr_path.stat().st_size > LOG_LIMIT:
                termination = 'log-limit'
            if termination:
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
            time.sleep(0.002)
        child.wait()
    elapsed = time.monotonic() - started
    payload = None
    stdout_bytes, stderr_bytes = stdout_path.stat().st_size, stderr_path.stat().st_size
    if stdout_bytes <= LOG_LIMIT:
        try:
            payload = json.loads(stdout_path.read_text())
        except (json.JSONDecodeError, UnicodeDecodeError):
            pass
    scratch_remaining = [p.name for p in work.iterdir() if p.name.startswith('.')]
    result = {'case': case, 'manifest': source_manifest, 'leaf_triangles': limit,
              'expected_success': expected_success, 'exit_code': child.returncode,
              'elapsed_seconds': elapsed, 'termination': termination,
              'peak_rss_kib_wait4': int(rss_path.read_text()) if rss_path.exists() else None,
              'sampled_process_observations': observations, 'sampled_workspace': workspace,
              'launcher_proc_permission_denials': launcher_denials, 'launcher_proc_vanished_races': launcher_races,
              'observed_converter_pid_count': len(child_pids),
              'stdout_bytes': stdout_bytes, 'stderr_bytes': stderr_bytes,
              'stderr_excerpt': stderr_path.read_bytes()[:4096].decode(errors='replace'),
              'output_exists': output.exists(), 'output_bytes': output.stat().st_size if output.exists() else 0,
              'scratch_remaining': scratch_remaining, 'result': payload}
    passed = termination is None and (child.returncode == 0) == expected_success and output.exists() == expected_success and not scratch_remaining
    if expected_success and passed:
        try:
            result['inventory'] = inventory(output, source_manifest, limit)
        except (AssertionError, KeyError, ValueError, zipfile.BadZipFile) as error:
            result['inventory_failure'] = str(error)
            passed = False
    elif not expected_success:
        passed = passed and payload is not None and 'unsupported' in json.dumps(payload).lower()
    result['passed'] = passed
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('/home/bend/.cache/rusty-tiles-f1b-reviewed-candidate'))
    parser.add_argument('--output', type=Path, default=Path(__file__).with_name('resource-initial.json'))
    parser.add_argument('--case', action='append', choices=CASES)
    parser.add_argument('--timeout', type=float, default=90)
    args = parser.parse_args()
    binary = args.binary.resolve()
    identity = digest(binary.read_bytes())
    report = {'binary_sha256': identity, 'binary_path': str(binary),
              'platform': platform.platform(), 'machine': platform.machine(), 'python': sys.version,
              'cpu_count': os.cpu_count(), 'probe_sha256': digest(Path(__file__).read_bytes()),
              'fixture_oracle_sha256': digest((ROOT / 'tests/f1b_oracle.py').read_bytes()),
              'provenance': 'Measurement probe authored by F1b encoder author. Geometry/UV helpers and small fixture imported from separately authored tests/f1b_oracle.py. Large PNG bytes independently framed from struct/zlib streaming literal scanlines here.',
              'measurement_contract': {'rss': 'Linux wait4 maximum resident set of converter child after separate C-launcher exec; excludes Python fixture generation RSS',
                                       'sampling': '>=2ms plus inspection time; FD/thread/process/workspace observations are samples, not established maxima; null means unobserved',
                                       'scratch': 'Producer .mesh-work-* file bytes only; excludes archive staging and input/output files',
                                       'execution': 'One serial converter per case; no converter worker or external encoder claim beyond sampled observations',
                                       'timeout_seconds': args.timeout, 'log_stop_threshold_bytes': LOG_LIMIT,
                                       'logs': 'stdout/stderr redirected to files; sampled stop threshold may overshoot; at most 4096 stderr bytes retained in JSON'},
              'results': []}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='rusty-f1b1-resources-') as root:
        root = Path(root)
        code, launcher = root / 'measure-child.c', root / 'measure-child'
        code.write_text(LAUNCHER)
        compiled = subprocess.run(['cc', '-O2', '-Wall', '-Wextra', '-Werror', str(code), '-o', str(launcher)], capture_output=True, timeout=30)
        assert compiled.returncode == 0, compiled.stderr.decode(errors='replace')[:4096]
        report['compiler'] = subprocess.run(['cc', '--version'], capture_output=True, text=True, timeout=10).stdout.splitlines()[0]
        report['launcher_source_sha256'] = digest(LAUNCHER.encode())
        for case in args.case or CASES:
            result = measure(binary, case, root, launcher, args.timeout)
            report['results'].append(result)
            report['binary_unchanged'] = digest(binary.read_bytes()) == identity
            args.output.write_text(json.dumps(report, indent=2) + '\n')
            print(json.dumps({'case': case, 'passed': result['passed'], 'seconds': round(result['elapsed_seconds'], 3),
                              'rss_kib_wait4': result['peak_rss_kib_wait4'], 'exit_code': result['exit_code']}), flush=True)
    assert report['binary_unchanged'] and all(item['passed'] for item in report['results']), 'resource evidence contains failed cases'


if __name__ == '__main__':
    main()
