#!/usr/bin/env python3
"""Bounded placed root-proxy proof using public Cesium traversal and queries.

The independent F1c1 Decimal80 WGS84 reference defines the frame and cameras.
The unchanged F1d1 viewer supplies public picking and natural SSE16 traversal.
Its plan function is overridden in this test process only; no production or
stable oracle/viewer file is modified.
"""
import argparse
import copy
from decimal import Decimal, localcontext
import gzip
import json
import math
from pathlib import Path
import subprocess
import sys
import tempfile
import zipfile

REPOSITORY = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPOSITORY / 'tests'))
import f1c1_oracle as placement
import f1d1_oracle as oracle
import f1d1_viewer as viewer

FROZEN_BINARY_SHA256 = '2098adf353e3d0c4eddde66b977e7a38514f7de1e3abd379051c35c2fb3d86e6'
FROZEN_SOURCE_COMMIT = '0eeb906f7f043eb608a5223e2817703c6836ea4b'
ANCHOR = [153.02, -27.47, 25.0]


def inspect_placement(manifest, report, reference, expectation):
    matrix = manifest['root']['transform']
    oracle.require(matrix == report['root_transform'], 'exact manifest/report root matrix agreement')
    oracle.require(report['coordinates'] == expectation['coordinates'] and
                   report['placement'] == expectation['placement'], 'independent explicit WGS84 placement report')
    oracle.require(len(matrix) == 16 and all(math.isfinite(x) for x in matrix), 'finite complete root matrix')
    errors = []
    with localcontext() as context:
        context.prec = 80
        for index, (actual, expected) in enumerate(zip(matrix, reference)):
            error = abs(Decimal.from_float(float(actual)) - expected)
            limit = Decimal('1e-8') if index in (12, 13, 14) else Decimal('2e-15')
            oracle.require(error <= limit, 'independent ECEF/ENU root matrix component '+str(index))
            errors.append(float(error))
    return {'maximum_basis_component_error': max(errors[:12]),
            'maximum_origin_component_error_metres': max(errors[12:15]),
            'basis_component_limit': 2e-15, 'origin_component_limit_metres': 1e-8}


def placed_plan(source, reference):
    plan = viewer.plan(source)
    for target in plan['targets']:
        target['world'] = [float(x) for x in placement.decimal_world(reference, target['world'])]
    for camera in plan['camera'].values():
        camera['destination'] = [float(x) for x in placement.decimal_world(reference, camera['destination'])]
        for name in ('direction', 'up'):
            camera[name] = [float(x) for x in placement.decimal_world(reference, camera[name], vector=True)]
    return plan


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--binary-sha256', help='Require exact binary identity; used for frozen replay')
    parser.add_argument('--source-commit', required=True, help='Externally established production source identity; no Git inference')
    parser.add_argument('--artifact-dir', type=Path, required=True)
    parser.add_argument('--cesium-dir', type=Path, required=True)
    parser.add_argument('--node-modules', type=Path, required=True)
    parser.add_argument('--chromium', type=Path, required=True)
    parser.add_argument('--json-output', type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    binary_hash = viewer.file_sha256(binary)
    if args.binary_sha256:
        oracle.require(binary_hash == args.binary_sha256, 'exact requested converter binary SHA256')
    args.artifact_dir.mkdir(parents=True, exist_ok=True)
    source_path = args.artifact_dir / 'source.glb'
    archive_path = args.artifact_dir / 'result.3tz'
    oracle.require(not source_path.exists() and not archive_path.exists(), 'fresh probe artifact paths')
    source = oracle.fixture('viewer')
    source_path.write_bytes(source)
    command = [str(binary), 'mesh-local-to-3tz', '--input', str(source_path), '--output', str(archive_path),
               '--leaf-triangles', '16', '--root-proxy-triangles', '48',
               '--max-proxy-error-metres', '8', '--anchor', *map(str, ANCHOR)]
    conversion = subprocess.run(command, text=True, capture_output=True, timeout=60)
    oracle.require(conversion.returncode == 0, 'placed candidate conversion '+conversion.stdout+conversion.stderr)
    with zipfile.ZipFile(archive_path) as archive:
        oracle.require(len(archive.namelist()) == len(set(archive.namelist())), 'no duplicate archive members')
        members = {name: archive.read(name) for name in archive.namelist() if name != '@3dtilesIndex1@'}
    manifest, report = (json.loads(members[name]) for name in ('tileset.json', 'conversion.json'))
    expectation, reference = placement.placement_reference({'anchor': ANCHOR})
    accuracy = inspect_placement(manifest, report, reference, expectation)
    # Sensitive controls mutate both matrix copies, avoiding a self-consistency
    # check accidentally becoming the only placement oracle.
    controls = []
    for name in ('missing-root-placement', 'wrong-height', 'swapped-east-north'):
        bad_manifest, bad_report = copy.deepcopy(manifest), copy.deepcopy(report)
        matrix = bad_manifest['root']['transform']
        if name == 'missing-root-placement':
            matrix[:] = [int(row == column) for column in range(4) for row in range(4)]
        elif name == 'wrong-height':
            for row in range(3):
                matrix[12+row] += 5*float(reference[8+row])
        else:
            matrix[:3], matrix[4:7] = matrix[4:7], matrix[:3]
        bad_report['root_transform'] = matrix[:]
        try:
            inspect_placement(bad_manifest, bad_report, reference, expectation)
        except oracle.leaf_oracle.OracleError:
            controls.append(name)
        else:
            raise oracle.leaf_oracle.OracleError('insensitive independent placement control '+name)

    # Capture an independent plan before replacing the viewer's local-only plan.
    plan = placed_plan(source, reference)
    original_plan, original_argv = viewer.plan, sys.argv[:]
    with tempfile.TemporaryDirectory(prefix='f1d1-placed-receipt-') as temporary:
        raw_receipt = Path(temporary) / 'browser.json'
        try:
            viewer.plan = lambda unused_source: copy.deepcopy(plan)
            sys.argv = [str(REPOSITORY / 'tests/f1d1_viewer.py'), '--source', str(source_path),
                        '--archive', str(archive_path), '--cesium-dir', str(args.cesium_dir),
                        '--node-modules', str(args.node_modules), '--chromium', str(args.chromium),
                        '--leaf-limit', '16', '--triangle-limit', '48', '--max-error', '8',
                        '--json-output', str(raw_receipt)]
            viewer.main()
            observed = json.loads(raw_receipt.read_text())
        finally:
            viewer.plan, sys.argv = original_plan, original_argv

    runs, picks = [], []
    for run in observed['browser']['runs']:
        correct_label = (run['stage'], run['label']) in (('coarse', 'proxy_region'), ('fine', 'source_triangle'))
        oracle.require(len(run['samples']) == 6, 'six independently targeted primitive instances')
        runs.append({key: run[key] for key in ('label', 'stage', 'visible', 'maximumScreenSpaceError', 'modelMatrixIsIdentity')}
                    | {'matching_public_queries': sum(sample['matches'] for sample in run['samples'])})
        if correct_label:
            picks.append({'label': run['label'], 'stage': run['stage'], 'samples': run['samples']})
    # Retain complete arrays/tuples for the twelve successful queries, without
    # repeating properties for the intentionally absent-label observations.
    oracle.require(len(picks) == 2 and all(sample['matches'] for run in picks for sample in run['samples']),
                   'six complete coarse arrays and six exact fine triangle tuples')
    drivers = [Path(__file__), *(REPOSITORY/'tests'/name for name in
               ('f1a_oracle.py', 'f1b_oracle.py', 'f1b2_oracle.py', 'f1b3_oracle.py',
                'f1c1_oracle.py', 'f1c2_oracle.py', 'f1d1_oracle.py', 'f1d1_viewer.py')),
               REPOSITORY/'tests/fixtures/f1d1_viewer.cjs']
    result = {
        'mode': 'candidate-published-WGS84-root-proxy', 'production_source_commit': args.source_commit,
        'binary': str(binary), 'binary_sha256': binary_hash,
        'frozen_requested_binary_match': binary_hash == FROZEN_BINARY_SHA256,
        'frozen_requested_source_match': args.source_commit == FROZEN_SOURCE_COMMIT,
        'conversion_command': command, 'conversion_stdout': conversion.stdout,
        'source': str(source_path), 'source_sha256': oracle.digest(source),
        'archive': str(archive_path), 'archive_sha256': oracle.digest(archive_path.read_bytes()),
        'placement_request': {'anchor': ANCHOR}, 'emitted_root_transform': manifest['root']['transform'],
        'independent_root_transform_decimal': list(map(str, reference)), 'placement_accuracy': accuracy,
        'independent_matrix_controls_rejected': controls, 'artifact': observed['artifact'],
        'browser': {key: observed['browser'][key] for key in
                    ('version', 'browserVersion', 'camera', 'pageErrors', 'failedRequests', 'externalRequests')}
                   | {'runs': runs, 'successful_queries': picks},
        'browser_executable': observed['browser_executable'],
        'browser_executable_sha256': observed['browser_executable_sha256'],
        'cesium_js_sha256': observed['cesium_js_sha256'], 'packages': observed['packages'],
        'driver_sha256': {str(path.relative_to(REPOSITORY)): viewer.file_sha256(path) for path in drivers},
        'invocation': [sys.executable, *sys.argv],
        'scope': 'One Brisbane WGS84 anchor, default identity orientation and zero offset, independently positioned ECEF cameras/targets, unchanged identity modelMatrix and natural SSE16 coarse-to-fine traversal. Six coarse complete membership-array queries and six fine original-triangle queries. Local stored geometry/bounds checked independently. No exact world Hausdorff, general placement/topology or textured appearance claim.'}
    encoded = (json.dumps(result, indent=2, allow_nan=False)+'\n').encode()
    args.json_output.parent.mkdir(parents=True, exist_ok=True)
    args.json_output.write_bytes(gzip.compress(encoded, mtime=0) if args.json_output.suffix == '.gz' else encoded)
    print(json.dumps({'result': str(args.json_output), 'source_sha256': result['source_sha256'],
                      'archive_sha256': result['archive_sha256'], 'proxy_faces': observed['artifact']['proxy_triangles'],
                      'regions': observed['artifact']['regions'], 'coarse_picks': 6, 'fine_picks': 6,
                      'placement_accuracy': accuracy, 'controls_rejected': controls}, indent=2))


if __name__ == '__main__':
    main()
