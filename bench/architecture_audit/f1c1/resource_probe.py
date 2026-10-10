#!/usr/bin/env python3
"""Pinned F1c1 placement resource measurements; stdlib only, Linux /proc + wait4.

Reuses the independent F1b3 literal source generator only. Archive/bounds checks
and measurements here are separate from that prior probe's acceptance reader.
This evidence measures concrete workloads, not universal performance or PBR truth.
"""
import argparse
import hashlib
import importlib.util
import itertools
import json
import math
import os
from pathlib import Path
import platform
import signal
import struct
import sys
import tempfile
import time
import zipfile

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[3]
GENERATOR = ROOT / 'bench/architecture_audit/f1b3/resource_probe.py'
CASES = ('core-pbr-single-leaf', 'core-pbr-many-leaves', 'shared-six-attrs-4096-primitives')
PLACED = ['--anchor', '153', '-27', '42', '--orientation-xyzw', '.2', '-.3', '.4', str(math.sqrt(.71)), '--scene-offset', '10.125', '-20.25', '30.375']
LOG_LIMIT = 1024 * 1024


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def load_generator():
    spec = importlib.util.spec_from_file_location('f1c1_source_generator', GENERATOR)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.fixture


def sample(pid, work, observations):
    observations['samples'] += 1
    try:
        status = Path(f'/proc/{pid}/status').read_text()
        fields = dict(line.split(':', 1) for line in status.splitlines() if ':' in line)
        current = {'rss_kib': int(fields.get('VmRSS', '0 kB').split()[0]),
                   'vm_hwm_kib': int(fields.get('VmHWM', '0 kB').split()[0]),
                   'threads': int(fields.get('Threads', '0')),
                   'file_descriptors': len(list(Path(f'/proc/{pid}/fd').iterdir())),
                   'child_processes': len(Path(f'/proc/{pid}/task/{pid}/children').read_text().split())}
        for key, value in current.items():
            observations['peak'][key] = max(observations['peak'].get(key, 0), value)
    except PermissionError:
        observations['permission_denials'] += 1
    except (FileNotFoundError, ProcessLookupError):
        observations['vanished_races'] += 1
    try:
        files = []
        for entry in work.rglob('*'):
            if entry.name.startswith('.tiles-stage-') and entry.is_file():
                files.append(entry)
            elif any(part.startswith('.mesh-work-') for part in entry.parts) and entry.is_file():
                files.append(entry)
        observations['peak_scratch_files'] = max(observations['peak_scratch_files'], len(files))
        observations['peak_scratch_bytes'] = max(observations['peak_scratch_bytes'], sum(p.stat().st_size for p in files))
    except PermissionError:
        observations['permission_denials'] += 1
    except FileNotFoundError:
        observations['vanished_races'] += 1


def execute_in_worker(binary, command, work, timeout):
    stdout, stderr = work / 'stdout.json', work / 'stderr.txt'
    observations = {'samples': 0, 'peak': {}, 'permission_denials': 0, 'vanished_races': 0,
                    'peak_scratch_files': 0, 'peak_scratch_bytes': 0}
    started, termination = time.monotonic(), None
    # Linux wait4 can include inherited launcher HWM. This fresh interpreter
    # never generates fixtures or reads the binary, keeping that floor explicit.
    status = dict(line.split(':', 1) for line in Path('/proc/self/status').read_text().splitlines() if ':' in line)
    launcher_memory = {key: int(status[key].split()[0]) for key in ('VmRSS', 'VmHWM')}
    with stdout.open('wb') as out, stderr.open('wb') as err:
        pid = os.posix_spawn(str(binary), command, dict(os.environ),
                             file_actions=[(os.POSIX_SPAWN_DUP2, out.fileno(), 1),
                                           (os.POSIX_SPAWN_DUP2, err.fileno(), 2)])
        while True:
            sample(pid, work, observations)
            finished, status, usage = os.wait4(pid, os.WNOHANG)
            if finished:
                break
            if time.monotonic() - started > timeout or max(stdout.stat().st_size, stderr.stat().st_size) > LOG_LIMIT:
                termination = 'timeout-or-log-limit'
                try:
                    os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                _, status, usage = os.wait4(pid, 0)
                break
            time.sleep(.002)
    result = {'command': command, 'exit_code': os.waitstatus_to_exitcode(status),
              'elapsed_seconds': time.monotonic() - started,
              'peak_rss_kib_wait4': usage.ru_maxrss, 'launcher_memory_kib_at_spawn': launcher_memory,
              'rss_accounting': 'wait4 includes inherited fresh-worker launcher floor; sampled vm_hwm_kib is observed converter /proc VmHWM, with transient peaks possibly missed',
              'sampled_observations': observations,
              'termination': termination, 'stderr_excerpt': stderr.read_bytes()[:4096].decode(errors='replace')}
    try:
        result['payload'] = json.loads(stdout.read_text())
    except (ValueError, UnicodeDecodeError) as error:
        result['payload_parse_error'] = repr(error)
    return result


def execute(binary, command, work, timeout):
    # One new interpreter per case prevents fixture generation/inspection and
    # binary hashing from contaminating the converter launcher HWM.
    parameters = work / 'measurement-parameters.json'
    measured = work / 'measurement-result.json'
    parameters.write_text(json.dumps({'binary': str(binary), 'command': command,
                                      'work': str(work), 'timeout': timeout,
                                      'result': str(measured)}))
    started = time.monotonic()
    with (work/'worker-stdout.txt').open('wb') as out, (work/'worker-stderr.txt').open('wb') as err:
        worker = os.posix_spawn(sys.executable,
                                [sys.executable, '-B', str(Path(__file__).resolve()), '--measurement-worker', str(parameters)],
                                dict(os.environ), setsid=True,
                                file_actions=[(os.POSIX_SPAWN_DUP2, out.fileno(), 1),
                                              (os.POSIX_SPAWN_DUP2, err.fileno(), 2)])
        try:
            while True:
                finished, status, _usage = os.wait4(worker, os.WNOHANG)
                if finished:
                    break
                if time.monotonic()-started > timeout+30:
                    os.killpg(worker, signal.SIGKILL)
                    os.wait4(worker, 0)
                    raise AssertionError('measurement worker timed out')
                time.sleep(.002)
            require(os.waitstatus_to_exitcode(status) == 0, 'measurement worker failed: '+(work/'worker-stderr.txt').read_text()[:4096])
        finally:
            # Converter inherits the worker process group. Remove any unexpected
            # descendant after timeout/failure as well as successful observation.
            try:
                os.killpg(worker, signal.SIGKILL)
            except ProcessLookupError:
                pass
    result = json.loads(measured.read_text())
    require(result['launcher_memory_kib_at_spawn']['VmHWM'] <= 65536,
            'fresh measurement worker exceeds64MiB launch-floor ceiling')
    return result


def measurement_worker(parameters):
    specification = json.loads(Path(parameters).read_text())
    result = execute_in_worker(Path(specification['binary']), specification['command'],
                               Path(specification['work']), specification['timeout'])
    Path(specification['result']).write_text(json.dumps(result))


def world(matrix, p):
    return [matrix[12+r] + sum(matrix[c*4+r]*p[c] for c in range(3)) for r in range(3)]


def finite(values):
    return all(isinstance(v, (int, float)) and math.isfinite(v) for v in values)


def inspect(output, expected, limit, payload, placed):
    report = payload['meshReport']
    require(report['profile'] == 'f1c2-source-identity-gltf-v1' and report['schema_version'] == 5, 'report profile/version')
    require(report['coordinates'] == ('wgs84-ecef' if placed else 'local-gltf'), 'coordinate mode')
    require(report['placement']['kind'] == ('wgs84' if placed else 'local'), 'placement kind')
    require(report['source_coordinates'] == 'local-gltf', 'source interpretation')
    for key in ('triangles', 'source_bytes', 'image_bytes'):
        require(report[key] == expected[key], 'report count: ' + key)
    require(report['images'] == report['image_pixels'] == 1, 'selected image counts')
    require(report['external_files'] == report['external_bytes'] == 0, 'external counts')
    hashes, total, world_vertex_count = {}, 0, 0
    with zipfile.ZipFile(output) as archive:
        manifest = json.loads(archive.read('tileset.json'))
        root = manifest['root']
        matrix = root['transform']
        require(len(matrix) == 16 and finite(matrix) and matrix == report['root_transform'], 'finite reported matrix')
        require([matrix[i] for i in (3, 7, 11, 15)] == [0, 0, 0, 1], 'affine bottom row')
        require(finite(root['boundingVolume']['box']), 'finite root bounds')
        require(math.isfinite(root['geometricError']) and root['geometricError'] >= 1, 'finite routing error')
        require(root['geometricError'] == manifest['geometricError'] == report['routing_geometric_error_metres'], 'routing metric')
        leaves = root['children']
        names = archive.namelist()
        expected_names = {'tileset.json', 'conversion.json', '@3dtilesIndex1@', 'textures/0.png'} | {leaf['content']['uri'] for leaf in leaves}
        require(len(names) == len(set(names)) and set(names) == expected_names, 'exact archive inventory')
        require(json.loads(archive.read('conversion.json')) == report, 'published/returned report equality')
        require(report['leaf_tiles'] == len(leaves), 'leaf count')
        image = archive.read('textures/0.png')
        require(digest(image) == expected['image_sha256'], 'source image unchanged')
        hashes['textures/0.png'] = digest(image)
        root_box = root['boundingVolume']['box']
        for leaf in leaves:
            require('transform' not in leaf and leaf['geometricError'] == 0, 'leaf transform/error')
            box = leaf['boundingVolume']['box']
            require(len(box) == 12 and finite(box), 'finite leaf bounds')
            require(all(box[i] >= 0 for i in (3, 7, 11)), 'nonnegative half extents')
            require(all(box[i] == 0 for i in (4, 5, 6, 8, 9, 10)), 'local axis-aligned bounds')
            require(all(root_box[i]-root_box[3+4*i] <= box[i]-box[3+4*i] and box[i]+box[3+4*i] <= root_box[i]+root_box[3+4*i] for i in range(3)), 'parent enclosure')
            corners = [world(matrix, [box[i]+signs[i]*box[3+4*i] for i in range(3)]) for signs in itertools.product((-1, 1), repeat=3)]
            low = [min(p[i] for p in corners) for i in range(3)]
            high = [max(p[i] for p in corners) for i in range(3)]
            name = leaf['content']['uri']
            raw = archive.read(name)
            hashes[name] = digest(raw)
            require(struct.unpack_from('<4sII', raw) == (b'glTF', 2, len(raw)), 'GLB framing')
            json_length = struct.unpack_from('<I', raw, 12)[0]
            require(raw[16:20] == b'JSON' and raw[24+json_length:28+json_length] == b'BIN\0', 'GLB chunks')
            document = json.loads(raw[20:20+json_length])
            require(document['nodes'] == [{'mesh': 0}] and document['scenes'] == [{'nodes': [0]}] and document['scene'] == 0, 'identity emitted glTF node chain')
            binary_start = 28+json_length
            leaf_count = 0
            for primitive in document['meshes'][0]['primitives']:
                require('indices' not in primitive, 'resource reader expects expanded geometry')
                accessor = document['accessors'][primitive['attributes']['POSITION']]
                require(accessor['componentType'] == 5126 and accessor['type'] == 'VEC3' and accessor['count'] % 3 == 0, 'position layout')
                view = document['bufferViews'][accessor['bufferView']]
                stride = view.get('byteStride', 12)
                start = binary_start + view.get('byteOffset', 0) + accessor.get('byteOffset', 0)
                leaf_count += accessor['count']//3
                for index in range(accessor['count']):
                    x, y, z = struct.unpack_from('<3f', raw, start + index*stride)
                    p = [x, -z, y]
                    require(finite(p) and all(box[i]-box[3+4*i] <= p[i] <= box[i]+box[3+4*i] for i in range(3)), 'exact stored local enclosure')
                    wp = world(matrix, p)
                    require(finite(wp) and all(low[i]-1e-6 <= wp[i] <= high[i]+1e-6 for i in range(3)), 'finite transformed corner-AABB enclosure')
                    world_vertex_count += 1
            require(0 < leaf_count <= limit, 'leaf triangle budget')
            total += leaf_count
    require(total == expected['triangles'], 'complete triangle count')
    return {'leaf_tiles': len(leaves), 'triangles': total, 'world_vertices_checked': world_vertex_count,
            'unchanged_content_sha256': hashes,
            'bounds_contract': 'Exact decoded local enclosure and parent enclosure; finite complete affine world transform plus transformed-corner AABB within1e-6m. Full world OBB/triangle/PBR truth is independent acceptance, not this resource reader.'}


def run_case(args, binary, root, case, source, expected, limit, placed, expected_error=None, placement_args=None):
    name = case + ('-placed' if placed else '-local')
    work = root / name
    work.mkdir()
    output = work / 'output-parent' / 'result.3tz'
    command = [str(binary), '--json', 'mesh-local-to-3tz', '-i', str(source), '-o', str(output), '--leaf-triangles', str(limit)]
    command += placement_args if placement_args is not None else PLACED if placed else []
    result = execute(binary, command, work, args.timeout)
    result.update(case=case, placed=placed, expected_source=expected, leaf_triangles=limit, passed=False)
    try:
        require(result['termination'] is None, 'bounded execution')
        require(result['peak_rss_kib_wait4'] <= args.max_rss_kib, 'RSS workload ceiling')
        require(result['sampled_observations']['peak_scratch_bytes'] <= args.max_scratch_bytes, 'sampled scratch ceiling')
        require(result['sampled_observations']['peak'].get('file_descriptors', 0) <= 128, 'sampled descriptor ceiling')
        require(result['sampled_observations']['peak'].get('threads', 0) <= 32, 'sampled thread ceiling')
        require(result['sampled_observations']['peak'].get('child_processes', 0) == 0, 'no sampled converter subprocesses')
        require(not any(p.name.startswith(('.mesh-work-', '.tiles-stage-')) for p in work.rglob('*')), 'private work cleanup')
        if expected_error:
            require(result['payload']['ok'] is False and result['payload']['error']['kind'] == expected_error, 'typed refusal')
            require(result['exit_code'] == {'unsupported': 2, 'invalid_request': 2, 'invalid_input': 3}[expected_error], 'refusal exit status')
            require(not output.parent.exists(), 'refusal before output-parent creation')
            require(result['sampled_observations']['peak_scratch_files'] == 0, 'no sampled staging')
        else:
            require(result['exit_code'] == 0 and result['payload']['ok'] is True, 'successful conversion')
            require(0 < output.stat().st_size <= args.max_output_bytes, 'finite output-size ceiling')
            result['output_bytes'] = output.stat().st_size
            result['inspection'] = inspect(output, expected, limit, result['payload'], placed)
        result['passed'] = True
    except (AssertionError, KeyError, ValueError, IndexError, struct.error, zipfile.BadZipFile) as error:
        result['failure'] = repr(error)
    require(digest(binary.read_bytes()) == args.binary_sha256, 'binary pin changed during probe')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--binary-sha256', required=True, help='Expected frozen converter hash; verified before any invocation')
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--case', action='append', choices=CASES)
    parser.add_argument('--timeout', type=float, default=90)
    parser.add_argument('--max-rss-kib', type=int, default=524288)
    parser.add_argument('--max-scratch-bytes', type=int, default=512*1024*1024)
    parser.add_argument('--max-output-bytes', type=int, default=64*1024*1024)
    args = parser.parse_args()
    require(sys.platform.startswith('linux') and hasattr(os, 'posix_spawn'), 'Linux POSIX spawn/wait4 measurement required')
    require(all(v > 0 for v in (args.timeout, args.max_rss_kib, args.max_scratch_bytes, args.max_output_bytes)), 'positive measurement ceilings')
    binary = args.binary.resolve()
    require(digest(binary.read_bytes()) == args.binary_sha256, 'binary does not match requested frozen pin')
    fixture = load_generator()
    receipt = {'binary_path': str(binary), 'binary_sha256': args.binary_sha256, 'probe_sha256': digest(Path(__file__).read_bytes()),
               'generator_sha256': digest(GENERATOR.read_bytes()), 'png_generator_sha256': digest((GENERATOR.parent.parent/'f1b1/resource_probe.py').read_bytes()),
               'platform': platform.platform(), 'measurement_limits': vars(args) | {'binary': str(binary), 'output': str(args.output)},
               'provenance': 'Reuses only F1b3 literal fixture() and its transitive F1b1 PNG generator. No prior measurement, inspector or production geometry functions are called.',
               'measurement_contract': 'A fresh stdlib Python worker per case never hashes binaries or generates/inspects fixtures. Its recorded current RSS/HWM is capped at64MiB. Linux wait4 is raw child high-water accounting including the inherited fresh-worker launch floor; it is not pure converter RSS below that floor. Separately sampled converter /proc VmHWM is an observation lower bound.2ms minimum /proc/workspace polling. Sampled peaks can miss transient FD/thread/child/scratch peaks; absence of samples is not zero. Scratch includes .mesh-work files and .tiles-stage files, excludes input/output/logs. Three concrete pairings do not prove asymptotic constant overhead or a hard universal RSS cap. Unchanged GLB/image hashes prove byte identity for these workloads; independent oracle owns complete placement and material truth.',
               'results': [], 'comparisons': []}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    def save(result):
        receipt['results'].append(result)
        args.output.write_text(json.dumps(receipt, indent=2)+'\n')
        print(json.dumps({'case': result['case'], 'placed': result['placed'], 'passed': result['passed'], 'rss_kib': result['peak_rss_kib_wait4']}), flush=True)
    with tempfile.TemporaryDirectory(prefix='rusty-f1c1-resources-') as temporary:
        root = Path(temporary)
        for case in args.case or CASES:
            raw, limit, expected = fixture(case)
            source = root / (case+'.glb')
            source.write_bytes(raw)
            del raw
            local = run_case(args, binary, root, case, source, expected, limit, False)
            save(local)
            placed = run_case(args, binary, root, case, source, expected, limit, True)
            save(placed)
            equal = local.get('inspection', {}).get('unchanged_content_sha256') == placed.get('inspection', {}).get('unchanged_content_sha256') and local['passed'] and placed['passed']
            comparison = {'case': case, 'content_bytes_identical': equal,
                          'placed_minus_local_rss_kib': placed['peak_rss_kib_wait4']-local['peak_rss_kib_wait4'],
                          'placed_minus_local_elapsed_seconds': placed['elapsed_seconds']-local['elapsed_seconds']}
            receipt['comparisons'].append(comparison)
        raw, limit, expected = fixture('triangles-100001-refused')
        source = root / 'triangles-100001.glb'
        source.write_bytes(raw)
        del raw
        for placed in (False, True):
            save(run_case(args, binary, root, 'triangles-100001-refused', source, expected, limit, placed, 'unsupported'))
        missing = root / 'missing-source.glb'
        save(run_case(args, binary, root, 'placement-budget-before-missing-source', missing, None, 1, True, 'unsupported', ['--anchor', '0', '0', '1000000000']))
        save(run_case(args, binary, root, 'invalid-quaternion-before-missing-source', missing, None, 1, True, 'invalid_request', ['--anchor', '0', '0', '0', '--orientation-xyzw', '0', '0', '0', '0']))
    receipt['binary_unchanged'] = digest(binary.read_bytes()) == args.binary_sha256
    receipt['passed'] = receipt['binary_unchanged'] and all(r['passed'] for r in receipt['results']) and all(c['content_bytes_identical'] for c in receipt['comparisons'])
    args.output.write_text(json.dumps(receipt, indent=2)+'\n')
    require(receipt['passed'], 'resource evidence contains failed cases')


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == '--measurement-worker':
        measurement_worker(sys.argv[2])
    else:
        main()
