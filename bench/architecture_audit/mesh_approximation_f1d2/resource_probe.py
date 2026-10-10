#!/usr/bin/env python3
"""Bounded Linux resource observations for a frozen optimized F1d2 CLI.

The near-cap artifact gets independent structural and common-box checks, not an
exact replay of its smaller published scalar certificate. No Cargo operation.
"""
import argparse
from fractions import Fraction as F
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import signal
import subprocess
import sys
import time
import zipfile

REPOSITORY = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPOSITORY / 'tests'))
import f1d1_oracle as oracle

CAP = 16_777_216
LEAF_LIMIT = 2048
GRID_SIZE = 64
PROFILE = 'f1d2-adaptive-root-proxy-gltf-v1'


def sha256(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(chunk)
    return digest.hexdigest()


def sample(pid, output_parent):
    observed = dict(rss_kib=0, threads=0, file_descriptors=0,
                    scratch_files=0, scratch_bytes=0)
    try:
        for line in (Path('/proc') / str(pid) / 'status').read_text().splitlines():
            if line.startswith('VmRSS:'):
                observed['rss_kib'] = int(line.split()[1])
            elif line.startswith('Threads:'):
                observed['threads'] = int(line.split()[1])
        observed['file_descriptors'] = len(list((Path('/proc') / str(pid) / 'fd').iterdir()))
    except (FileNotFoundError, ProcessLookupError):
        pass
    if output_parent.exists():
        for path in output_parent.rglob('*'):
            if path.name == 'result.3tz':
                continue
            try:
                if path.is_file():
                    observed['scratch_files'] += 1
                    observed['scratch_bytes'] += path.stat().st_size
            except FileNotFoundError:
                pass
    return observed


def kill_group(pid):
    try:
        os.killpg(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass


def measure(binary, binary_hash, work, target, budget, poll_ms, timeout):
    oracle.require(not work.exists(), 'fresh resource case directory: ' + str(work))
    oracle.require(sha256(binary) == binary_hash, 'frozen binary before child launch')
    work.mkdir(parents=True)
    source = work / 'source.glb'
    source.write_bytes(oracle.fixture('grid', GRID_SIZE))
    output = work / 'output' / 'result.3tz'
    command = [str(binary), '--json', 'mesh-local-to-3tz', '-i', str(source),
               '-o', str(output), '--leaf-triangles', str(LEAF_LIMIT),
               '--root-proxy-triangles', str(target),
               '--max-proxy-error-metres', str(budget)]
    peaks = {key: 0 for key in sample(-1, output.parent)}
    samples = 0
    usage = None
    timed_out = False
    with (work / 'stdout.log').open('wb') as stdout, (work / 'stderr.log').open('wb') as stderr:
        start = time.monotonic()
        process = subprocess.Popen(command, stdout=stdout, stderr=stderr,
                                   start_new_session=True)
        try:
            while True:
                observed = sample(process.pid, output.parent)
                samples += 1
                for key, value in observed.items():
                    peaks[key] = max(peaks[key], value)
                pid, status, observed_usage = os.wait4(process.pid, os.WNOHANG)
                if pid:
                    process.returncode = os.waitstatus_to_exitcode(status)
                    usage = observed_usage
                    break
                if time.monotonic() - start >= timeout:
                    timed_out = True
                    kill_group(process.pid)
                    _, status, usage = os.wait4(process.pid, 0)
                    process.returncode = os.waitstatus_to_exitcode(status)
                    break
                time.sleep(poll_ms / 1000)
        finally:
            if process.returncode is None:
                kill_group(process.pid)
                _, status, usage = os.wait4(process.pid, 0)
                process.returncode = os.waitstatus_to_exitcode(status)
        elapsed = time.monotonic() - start
    # Store timeout and actual exit before acceptance checks. Failed controls
    # remain reviewable and never become successful measurement receipts.
    return dict(command=command, source_path=str(source), source_sha256=sha256(source),
                source_bytes=source.stat().st_size, archive_path=str(output),
                triangle_limit=target, requested_error_metres=budget,
                exit_code=process.returncode, elapsed_seconds=elapsed,
                peak_rss_kib_wait4=usage.ru_maxrss,
                user_cpu_seconds=usage.ru_utime, system_cpu_seconds=usage.ru_stime,
                poll_interval_ms=poll_ms, samples=samples, sampled_peaks=peaks,
                stdout=(work / 'stdout.log').read_text(),
                stderr=(work / 'stderr.log').read_text(), timeout_seconds=timeout,
                timed_out=timed_out, timeout_sent_sigkill_to_process_group=timed_out,
                child_reaped=True)


def inspect_large_success(receipt):
    """Linear artifact checks; deliberately no 16M Fraction face-pair replay."""
    source_path = Path(receipt['source_path'])
    output = Path(receipt['archive_path'])
    source = source_path.read_bytes()
    response = json.loads(receipt['stdout'])
    oracle.require(receipt['exit_code'] == 0 and response['ok'], 'successful near-cap child')
    with zipfile.ZipFile(output) as stream:
        names = stream.namelist()
        oracle.require(len(names) == len(set(names)), 'unique archive entries')
        members = {name: stream.read(name) for name in names if name != '@3dtilesIndex1@'}
    manifest = json.loads(members['tileset.json'])
    report = json.loads(members['conversion.json'])
    oracle.require(report == response['meshReport'], 'published and CLI report agreement')
    oracle.require(report['schema_version'] == 7 and report['profile'] == PROFILE,
                   'schema7/F1d2 profile')
    root = manifest['root']
    budget = receipt['requested_error_metres']
    oracle.require(root['refine'] == 'REPLACE' and root['geometricError'] == budget,
                   'declared root error and REPLACE refinement')
    oracle.require(root['children'] and all(c['geometricError'] == 0 and not c.get('children')
                   for c in root['children']), 'unchanged complete leaf hierarchy')
    oracle.require(manifest['geometricError'] >= budget, 'omission error covers root budget')
    leaves = oracle.leaf_oracle.inspect_members(source, members, LEAF_LIMIT)
    truth, source_names = oracle.leaf_oracle.expected_source(source)
    oracle.require(len(truth) == 2 * GRID_SIZE ** 2, 'independently decoded source count')
    regions = {key[:3] for key in truth}
    oracle.require(len(regions) == 1, 'one independently decoded fixture region')
    doc, raw, _ = oracle.decode(members[root['content']['uri']])
    rows = oracle.proxy_rows(doc, raw)
    oracle.require(len(rows) == 1, 'one proxy membership region')
    tuples = list(zip(*(rows[0][key] for key in oracle.ARRAY_KEYS)))
    oracle.require(tuples == sorted(truth), 'complete unique sorted proxy membership')
    node = tuples[0][0]
    oracle.require((rows[0]['source_node_name_present'], rows[0]['source_node_name']) == source_names[node],
                   'exact optional source label')
    allowed = {tuple(p) for record in truth.values() for p in record['positions']}
    material = truth[tuples[0]]['material']
    faces = []
    for primitive in doc['meshes'][0]['primitives']:
        attrs = primitive['attributes']
        oracle.require(set(attrs) == {'POSITION', '_FEATURE_ID_0'}, 'positions-only proxy')
        position_accessor = doc['accessors'][attrs['POSITION']]
        oracle.require(position_accessor['componentType'] == 5126 and position_accessor['type'] == 'VEC3',
                       'unquantized decoded float32 proxy')
        positions = oracle.accessor(doc, raw, attrs['POSITION'])
        ids = oracle.accessor(doc, raw, attrs['_FEATURE_ID_0'])
        oracle.require(len(ids) == len(positions) and all(i == 0 for i in ids), 'uniform fixture region')
        features = primitive['extensions']['EXT_mesh_features']['featureIds']
        oracle.require(len(features) == 1 and features[0]['label'] == 'proxy_region'
                       and features[0]['attribute'] == 0 and features[0]['propertyTable'] == 0,
                       'proxy feature meaning')
        actual_material = doc['materials'][primitive['material']] if 'material' in primitive else None
        oracle.require(actual_material == material, 'exact proxy PBR factors and omission')
        indices = oracle.accessor(doc, raw, primitive['indices']) if 'indices' in primitive else list(range(len(positions)))
        oracle.require(len(indices) % 3 == 0, 'complete proxy faces')
        for offset in range(0, len(indices), 3):
            corners = indices[offset:offset + 3]
            oracle.require(all(type(i) is int and 0 <= i < len(positions) for i in corners), 'valid decoded proxy indices')
            face = [positions[i] for i in corners]
            oracle.require(all(tuple(p) in allowed for p in face), 'original decoded positions only')
            faces.append(face)
    count = len(faces)
    oracle.require(0 < count <= receipt['triangle_limit'] and count < len(truth), 'actual requested reduction')
    approximation = report['approximation']
    oracle.require(approximation['kind'] == 'root_proxy'
                   and approximation['appearance'] == 'opaque-untextured-factors'
                   and approximation['regions'] == 1 and approximation['triangles'] == count
                   and approximation['triangle_limit'] == receipt['triangle_limit']
                   and approximation['geometric_error_metres'] == budget, 'typed approximation counts/policy')
    oracle.require('comparison_pairs' not in approximation and 'certified_error_metres' not in approximation,
                   'removed F1d1 report fields')
    certificate = approximation['certificate']
    oracle.require(set(certificate) == {'error_metres', 'patch_face_tests', 'accepted_patches', 'max_depth'},
                   'exact schema7 certificate fields')
    error = certificate['error_metres']
    oracle.require(type(error) in (int, float) and math.isfinite(error) and 0 <= error <= budget,
                   'finite reported certificate within budget')
    base = 2 * len(truth) * count
    tests = certificate['patch_face_tests']
    accepted = certificate['accepted_patches']
    depth = certificate['max_depth']
    oracle.require(type(tests) is int and base <= tests <= CAP, 'actual tests bounded by admitted base and cap')
    oracle.require(type(accepted) is int and len(truth) + count <= accepted <= tests,
                   'possible complete-cover leaf count')
    oracle.require(type(depth) is int and 0 <= depth <= 24, 'finite accepted depth')
    oracle.require(depth == 0 and tests == base and accepted == len(truth) + count,
                   'wide-budget control performs exactly compulsory root scans')
    oracle.require(base >= F(9, 10) * CAP, 'successful root baseline near cap')
    oracle.require(report['triangles'] == len(truth) and report['leaf_tiles'] == leaves['leaves'],
                   'independent source/leaf counts')
    all_points = [p for record in truth.values() for p in record['positions']] + [p for face in faces for p in face]
    oracle.require(all(0 <= F(p[0]) <= GRID_SIZE and F(p[1]) == 0 and 0 <= F(p[2]) <= GRID_SIZE
                       for p in all_points), 'both nonempty exact supports inside common64m box')
    squared_diameter = 2 * GRID_SIZE ** 2
    oracle.require(squared_diameter <= F(budget) ** 2, 'independent common-box proof of requested budget')
    expected_inventory = {'tileset.json', 'conversion.json', root['content']['uri']}
    expected_inventory.update(c['content']['uri'] for c in root['children'])
    oracle.require(set(members) == expected_inventory, 'exact archive inventory')
    entries = sorted(path.name for path in output.parent.iterdir())
    oracle.require(entries == ['result.3tz'], 'no retained workspace or staging')
    return dict(source_triangles=len(truth), proxy_triangles=count, leaves=leaves['leaves'],
                base_patch_face_tests=base, certificate=certificate,
                base_ceiling_fraction=base / CAP, actual_ceiling_fraction=tests / CAP,
                archive_sha256=sha256(output), archive_bytes=output.stat().st_size,
                member_sha256={name: oracle.digest(data) for name, data in members.items()},
                output_parent_entries_after_completion=entries,
                independent_geometry_reference={
                    'kind': 'exact_common_box_diameter',
                    'squared_diameter_metres2': squared_diameter,
                    'requested_budget_squared_metres2': str(F(budget) ** 2),
                    'requested_budget_proved': True,
                    'published_smaller_certificate_independently_proved': False,
                    'scope': 'Both decoded nonempty supports lie in the same closed box. Their whole-support Hausdorff distance is <=sqrt(8192)<100m. The reported smaller scalar and precise operational counts are not independently replayed at this size.'})


def inspect_refusal(receipt, required_text):
    oracle.require(receipt['exit_code'] != 0, 'refusal child exits nonzero')
    response = json.loads(receipt['stdout'])
    error = response['error']
    oracle.require(response['ok'] is False and error['kind'] == 'unsupported'
                   and required_text in error['message'], 'actual typed expected refusal')
    output = Path(receipt['archive_path'])
    oracle.require(not output.parent.exists(), 'refusal before output parent/workspace')
    oracle.require(not error.get('retainedPaths') and not error.get('secondaryDiagnostics'),
                   'no retained paths or secondary diagnostics')
    return dict(output_parent_absent=True, error=error)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--artifact-dir', type=Path, required=True)
    parser.add_argument('--production-source', required=True)
    parser.add_argument('--expected-binary-sha256', required=True)
    parser.add_argument('--json-output', type=Path, required=True)
    parser.add_argument('--poll-ms', type=int, default=10)
    parser.add_argument('--timeout-seconds', type=int, default=180)
    args = parser.parse_args()
    oracle.require(platform.system() == 'Linux', 'wait4/procfs evidence is Linux only')
    oracle.require(1 <= args.poll_ms <= 100 and 1 <= args.timeout_seconds <= 180,
                   'bounded resource supervisor settings')
    binary = args.binary.resolve(strict=True)
    binary_hash = sha256(binary)
    oracle.require(binary_hash == args.expected_binary_sha256, 'frozen optimized binary identity')
    args.artifact_dir = args.artifact_dir.resolve()
    oracle.require(not args.artifact_dir.exists(), 'fresh entire resource artifact directory')
    drivers = [Path(__file__), REPOSITORY / 'tests/f1d1_oracle.py',
               REPOSITORY / 'tests/f1d2_certificate.py', REPOSITORY / 'tests/f1c2_oracle.py']
    sources = [REPOSITORY / path for path in ('src/mesh_archive/approximation/certificate.rs',
        'src/mesh_archive/approximation.rs', 'src/mesh_archive.rs', 'src/runtime.rs',
        'src/main.rs', 'src/report.rs', 'Cargo.lock', 'docs/architecture/f1d2-certificate-contract.md')]
    source_hashes = {str(path.relative_to(REPOSITORY)): sha256(path) for path in sources}
    result = dict(production_source_commit=args.production_source,
                  binary_path=str(binary), binary_sha256=binary_hash,
                  source_sha256=source_hashes,
                  driver_sha256={str(path.relative_to(REPOSITORY)): sha256(path) for path in drivers},
                  platform=platform.platform(), python=sys.version, cpu_count=os.cpu_count(),
                  invocation=[sys.executable, *sys.argv], cases=[], status='pending',
                  limits=[
                      'Three fixed serial process observations on this Linux host; no total-RSS, allocation, cancellation-latency or performance guarantee.',
                      'wait4 ru_maxrss may include inherited Python launcher residency before exec; it is not pure converter steady-state residency.',
                      'Thread/descriptor/procfs RSS/scratch peaks are sampled and may miss short peaks. ru_maxrss is the child process high-water observation.',
                      'All converter children launch before large artifact decoding to reduce parent-decoder inheritance bias, without claiming its removal.',
                      'The large success independently proves the requested100m budget by common-box diameter. Its smaller published certificate is explicitly not re-certified here.',
                      'Timeout uses process-group SIGKILL and synchronous wait4 reap; sampling is cooperative and the timeout is not a hard latency guarantee.'])
    failure = None
    try:
        # Measure all converter processes before loading the large published GLBs.
        specs = [('accepted-near-base-cap', 1024, 100.),
                 ('refused-above-base-cap', 4096, 100.),
                 ('dynamic-cap-control', 1024, 6.)]
        for label, target, budget in specs:
            receipt = measure(binary, binary_hash, args.artifact_dir / label,
                              target, budget, args.poll_ms, args.timeout_seconds)
            receipt['label'] = label
            result['cases'].append(receipt)
            oracle.require(not receipt['timed_out'], 'resource child timed out; killed/reaped: ' + label)
        result['cases'][0]['artifact'] = inspect_large_success(result['cases'][0])
        result['cases'][1]['refusal'] = inspect_refusal(result['cases'][1], 'base face-pair work limit')
        result['cases'][2]['refusal'] = inspect_refusal(result['cases'][2], 'patch-face work limit')
        result['cases'][2]['scope'] = 'Actual typed dynamic work refusal after in-memory proof, no workspace. Failed attempt has no published certificate/counters; exact consumed count is established separately by unit/source controls.'
        oracle.require(sha256(binary) == binary_hash, 'frozen binary after all children')
        oracle.require(source_hashes == {str(path.relative_to(REPOSITORY)): sha256(path) for path in sources},
                       'source files unchanged during measurement')
        result['status'] = 'pass'
    except Exception as error:
        failure = error
        result['status'] = 'failed'
        result['failure'] = dict(kind=type(error).__name__, message=str(error))
    args.json_output.parent.mkdir(parents=True, exist_ok=True)
    args.json_output.write_text(json.dumps(result, indent=2, allow_nan=False) + '\n')
    if failure:
        print(str(failure), file=sys.stderr)
        raise SystemExit(1)


if __name__ == '__main__':
    main()
