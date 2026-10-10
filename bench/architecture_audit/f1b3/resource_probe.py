#!/usr/bin/env python3
"""Concrete F1b3 PBR resource workloads; consumer measurements, not fidelity truth."""
import argparse
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

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[3]
PRIOR_PATH = ROOT / 'bench/architecture_audit/f1b1/resource_probe.py'
spec = importlib.util.spec_from_file_location('f1b1_measurement', PRIOR_PATH)
prior = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prior)
CASES = ('core-pbr-single-leaf', 'core-pbr-many-leaves', 'shared-six-attrs-4096-primitives', 'triangles-100001-refused')


def fixture(case):
    binary, views, accessors = bytearray(), [], []
    def add(raw, count, kind, **extra):
        binary.extend(b'\0' * (-len(binary) % 4))
        views.append({'buffer': 0, 'byteOffset': len(binary), 'byteLength': len(raw)})
        binary.extend(raw)
        accessors.append({'bufferView': len(views)-1, 'componentType': 5126, 'count': count, 'type': kind, **extra})
        return len(accessors)-1
    attrs = {'POSITION': add(struct.pack('<9f', 0,0,0, 1,0,0, 0,1,0), 3, 'VEC3', min=[0,0,0], max=[1,1,0]),
             'NORMAL': add(struct.pack('<9f', *([0,0,1]*3)), 3, 'VEC3'),
             'TANGENT': add(struct.pack('<12f', *([1,0,0,1]*3)), 3, 'VEC4'),
             'TEXCOORD_0': add(struct.pack('<6f', 0,0, 1,0, 0,1), 3, 'VEC2'),
             'TEXCOORD_1': add(struct.pack('<6f', .2,.3, .4,.5, .6,.7), 3, 'VEC2'),
             'COLOR_0': add(struct.pack('<12f', *([.25,.5,.75,1]*3)), 3, 'VEC4')}
    triangles = 4096 if case.startswith('shared-') else 100001 if case.endswith('refused') else 100000
    primitive = {'attributes': attrs, 'material': 0}
    if not case.startswith('shared-'):
        primitive['indices'] = add(struct.pack('<3H', 0,1,2) * triangles, triangles*3, 'SCALAR')
        accessors[-1]['componentType'] = 5123
    image = prior.png(1, 1)
    binary.extend(b'\0' * (-len(binary) % 4))
    views.append({'buffer': 0, 'byteOffset': len(binary), 'byteLength': len(image)})
    binary.extend(image)
    doc = {'asset': {'version': '2.0'}, 'scene': 0, 'scenes': [{'nodes': [0]}], 'nodes': [{'mesh': 0}],
           'buffers': [{'byteLength': len(binary)}], 'bufferViews': views, 'accessors': accessors,
           'meshes': [{'primitives': [primitive] * (4096 if case.startswith('shared-') else 1)}],
           'images': [{'bufferView': len(views)-1, 'mimeType': 'image/png'}], 'textures': [{'source': 0}],
           'materials': [{'pbrMetallicRoughness': {'baseColorTexture': {'index': 0}, 'metallicRoughnessTexture': {'index': 0, 'texCoord': 1}},
                          'normalTexture': {'index': 0, 'scale': 1.25}, 'occlusionTexture': {'index': 0, 'texCoord': 1, 'strength': .6},
                          'emissiveTexture': {'index': 0}, 'emissiveFactor': [.2,.3,.4]}]}
    metadata = json.dumps(doc, separators=(',', ':')).encode()
    metadata += b' ' * (-len(metadata) % 4)
    binary.extend(b'\0' * (-len(binary) % 4))
    raw = struct.pack('<4sII', b'glTF', 2, 28+len(metadata)+len(binary)) + struct.pack('<I4s', len(metadata), b'JSON') + metadata + struct.pack('<I4s', len(binary), b'BIN\0') + binary
    expected = {'triangles': triangles, 'source_bytes': len(raw), 'source_sha256': prior.digest(raw), 'image_bytes': len(image),
                'image_sha256': prior.digest(image), 'primitive_count': len(doc['meshes'][0]['primitives']), 'accessor_count_sum': sum(a['count'] for a in accessors)}
    return raw, 1000 if case.endswith('many-leaves') else 100000, expected


def inspect(output, expected, limit, payload):
    report = payload['meshReport']
    assert report['profile'] == 'f1d1-root-proxy-gltf-v1' and report['schema_version'] == 6
    assert report['approximation'] == {'kind': 'full_detail'}
    for key in ('triangles', 'source_bytes', 'image_bytes'):
        assert report[key] == expected[key], (key, report, expected)
    assert report['images'] == report['image_pixels'] == 1 and report['external_files'] == report['external_bytes'] == 0
    with zipfile.ZipFile(output) as archive:
        leaves = json.loads(archive.read('tileset.json'))['root']['children']
        names = archive.namelist()
        assert len(names) == len(set(names)) and set(names) == {'tileset.json', 'conversion.json', '@3dtilesIndex1@', 'textures/0.png'} | {leaf['content']['uri'] for leaf in leaves}
        assert prior.digest(archive.read('textures/0.png')) == expected['image_sha256']
        assert json.loads(archive.read('conversion.json')) == report and report['leaf_tiles'] == len(leaves)
        total = 0
        for leaf in leaves:
            raw = archive.read(leaf['content']['uri'])
            doc = json.loads(raw[20:20+struct.unpack_from('<I', raw, 12)[0]])
            count = sum(doc['accessors'][p['attributes']['POSITION']]['count']//3 for p in doc['meshes'][0]['primitives'])
            assert 0 < count <= limit and leaf['geometricError'] == 0
            total += count
        assert total == expected['triangles']
    return {'leaf_tiles': len(leaves), 'triangle_count_from_output_accessors': total, 'report': report}


def measure(binary, launcher, root, case, timeout):
    work = root / case
    work.mkdir()
    raw, limit, expected = fixture(case)
    source, output, rss, outlog, errlog = (work / p for p in ('source.glb', 'result.3tz', 'rss.txt', 'stdout.json', 'stderr.txt'))
    source.write_bytes(raw)
    del raw
    samples = {key: {'peak': None, 'samples': 0, 'permission_denials': 0, 'vanished_races': 0} for key in ('threads', 'file_descriptors', 'converter_child_processes')}
    scratch = {'peak_files': 0, 'peak_bytes': 0, 'samples': 0, 'permission_denials': 0, 'vanished_races': 0}
    launcher_reads = {'permission_denials': 0, 'vanished_races': 0}
    started, termination = time.monotonic(), None
    with outlog.open('wb') as stdout, errlog.open('wb') as stderr:
        child = subprocess.Popen([str(launcher), str(rss), str(binary), '--json', 'mesh-local-to-3tz', '-i', str(source), '-o', str(output), '--leaf-triangles', str(limit)], stdout=stdout, stderr=stderr, start_new_session=True)
        while child.poll() is None:
            try:
                for pid in Path(f'/proc/{child.pid}/task/{child.pid}/children').read_text().split():
                    prior.sample_process(pid, samples)
            except PermissionError:
                launcher_reads['permission_denials'] += 1
            except (FileNotFoundError, ProcessLookupError):
                launcher_reads['vanished_races'] += 1
            prior.sample_workspace(work, scratch)
            if time.monotonic()-started > timeout or max(outlog.stat().st_size, errlog.stat().st_size) > prior.LOG_LIMIT:
                termination = 'timeout-or-log-limit'
                try:
                    os.killpg(child.pid, signal.SIGTERM)
                    child.wait(timeout=2)
                except ProcessLookupError:
                    pass
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid, signal.SIGKILL)
                break
            time.sleep(.002)
        child.wait()
    payload = json.loads(outlog.read_text())
    success = not case.endswith('refused')
    leftovers = [p.name for p in work.iterdir() if p.name.startswith('.')]
    result = {'case': case, 'expected': expected, 'leaf_triangles': limit, 'exit_code': child.returncode, 'elapsed_seconds': time.monotonic()-started,
              'peak_rss_kib_wait4': int(rss.read_text()) if rss.exists() else None, 'sampled_process_observations': samples, 'sampled_workspace': scratch,
              'launcher_proc_observations': launcher_reads, 'termination': termination, 'scratch_remaining': leftovers, 'output_bytes': output.stat().st_size if output.exists() else 0,
              'result': payload, 'stderr_excerpt': errlog.read_bytes()[:4096].decode(errors='replace'), 'passed': False}
    try:
        assert termination is None and child.returncode == (0 if success else 2) and output.exists() == success and not leftovers
        if success:
            result['inventory'] = inspect(output, expected, limit, payload)
        else:
            assert payload['error']['kind'] == 'unsupported' and scratch['peak_files'] == scratch['peak_bytes'] == 0
        result['passed'] = True
    except (AssertionError, KeyError, ValueError, zipfile.BadZipFile) as error:
        result['failure'] = repr(error)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--case', action='append', choices=CASES)
    parser.add_argument('--timeout', type=float, default=90)
    args = parser.parse_args()
    binary, identity = args.binary.resolve(), prior.digest(args.binary.read_bytes())
    receipt = {'binary_path': str(binary), 'binary_sha256': identity, 'platform': platform.platform(), 'probe_sha256': prior.digest(Path(__file__).read_bytes()), 'prior_probe_sha256': prior.digest(PRIOR_PATH.read_bytes()),
               'provenance': 'Consumer-authored literal six-attribute/five-role GLB workload. Reuses F1b1 PNG encoder, wait4 launcher and /proc/workspace sampling helpers only; does not use acceptance oracle fixture/reader as fidelity truth.',
               'measurement_contract': 'Linux child wait4 high-water RSS after separate C launcher exec excludes Python fixture generation. Sampling every >=2ms plus inspection can miss FD/thread/child/scratch peaks; null means unobserved. Scratch measures .mesh-work-* files only, excludes archive staging and input/output. Concrete serial workloads establish neither hard RSS bounds nor universal performance. Refusal before staging is source-defined plus sampled evidence; cancellation remains separately tested.', 'results': []}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='rusty-f1b3-resources-') as temporary:
        root = Path(temporary)
        code, launcher = root / 'measure.c', root / 'measure'
        code.write_text(prior.LAUNCHER)
        subprocess.run(['cc', '-O2', '-Wall', '-Wextra', '-Werror', str(code), '-o', str(launcher)], check=True, timeout=30)
        for case in args.case or CASES:
            result = measure(binary, launcher, root, case, args.timeout)
            receipt['results'].append(result)
            receipt['binary_unchanged'] = prior.digest(binary.read_bytes()) == identity
            args.output.write_text(json.dumps(receipt, indent=2)+'\n')
            print(json.dumps({'case': case, 'passed': result['passed'], 'rss_kib': result['peak_rss_kib_wait4']}), flush=True)
    assert receipt['binary_unchanged'] and all(r['passed'] for r in receipt['results']), 'resource evidence contains failed cases'


if __name__ == '__main__':
    main()
