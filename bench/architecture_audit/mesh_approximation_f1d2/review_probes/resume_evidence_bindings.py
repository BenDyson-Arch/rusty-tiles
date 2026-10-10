#!/usr/bin/env python3
"""Nonauthor final-receipt binding checks; no producer build or geometry claim."""
import hashlib
import json
from pathlib import Path
import sys
import tarfile
import tomllib
import zipfile

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[4]
FINAL = Path('/tmp/rusty-tiles-f1d2-final-evidence')
OLD = Path('/home/bend/.cache/rusty-tiles-f1d2-evidence')


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), allow_nan=False)


def verify_artifact(case):
    command = case['command']
    source = Path(command[command.index('-i') + 1])
    output = Path(command[command.index('-o') + 1])
    artifact = case['artifact']
    assert case['exit_code'] == 0
    assert sha(source) == artifact['source_sha256']
    assert sha(output) == case['archive_sha256']
    with zipfile.ZipFile(output) as stream:
        assert len(stream.namelist()) == len(set(stream.namelist()))
        for name, expected in artifact['member_sha256'].items():
            assert hashlib.sha256(stream.read(name)).hexdigest() == expected
        published = json.loads(stream.read('conversion.json'))
    assert canonical(published) == canonical(artifact['report'])
    response = json.loads(case['stdout'])
    assert response['ok'] is True
    assert canonical(response['meshReport']) == canonical(published)
    assert artifact['independent_certificate']['proved'] is True
    assert all(c.get('accepted', not c.get('rejected', False)) is False for c in case['controls'])
    return {'source': str(source), 'source_sha256': sha(source),
            'archive': str(output), 'archive_sha256': sha(output),
            'verified_members': len(artifact['member_sha256']),
            'rejected_controls': len(case['controls']),
            'certificate': published['approximation']['certificate']}


def main():
    receipt = {'scope': __doc__, 'nonauthor': True, 'driver_sha256': sha(__file__),
               'production_commit': '4553533e64c1494e706888a2c8e3a38a6867ce56',
               'receipts': {}, 'artifacts': {}, 'refusals': {}}
    for name in ('portable-oracle.json', 'native-oracle.json'):
        path = FINAL / name
        data = json.loads(path.read_text())
        receipt['receipts'][str(path)] = sha(path)
        for filename, expected in data['driver_sha256'].items():
            assert sha(ROOT / 'tests' / filename) == expected
        execution = data['execution']
        assert sha(execution['binary_path']) == execution['binary_sha256']
        cases = [*execution['baseline']['successful_artifacts'],
                 execution['useful_tightening'], execution['nonzero_artifact']]
        receipt['artifacts'][name] = [verify_artifact(c) for c in cases]
        refused = [*execution['baseline']['admission_refusals'],
                   execution['nonzero_budget_refusal']]
        verified = []
        for case in refused:
            response = json.loads(case['stdout'])
            assert response['ok'] is False and case['exit_code'] != 0
            assert case['output_parent_absent'] is True
            command = case['command']
            output = Path(command[command.index('-o') + 1])
            assert not output.parent.exists()
            if 'replace_preservation' in case:
                replace = case['replace_preservation']
                second = json.loads(replace['stdout'])
                assert second['ok'] is False and replace['exit_code'] != 0
                assert second['error']['kind'] == response['error']['kind'] == 'unsupported'
                assert second['error']['message'] == response['error']['message']
                command = replace['command']
                destination = Path(command[command.index('-o') + 1])
                assert sha(destination) == replace['destination_sha256']
                assert replace['directory_unchanged'] is True
            verified.append({'case': case.get('case', 'nonzero-budget-proposal'),
                             'kind': response['error']['kind'],
                             'message': response['error']['message']})
        receipt['refusals'][name] = verified

    for name in ('placed-browser.json',):
        path = FINAL / name
        data = json.loads(path.read_text())
        receipt['receipts'][str(path)] = sha(path)
        for filename, expected in data['driver_sha256'].items():
            assert sha(ROOT / filename) == expected
        assert sha(data['binary']) == data['binary_sha256']
        assert sha(data['source']) == data['source_sha256']
        assert sha(data['archive']) == data['archive_sha256']
        assert data['production_source_commit'] == receipt['production_commit']
        assert data['browser']['pageErrors'] == []
        assert data['browser']['failedRequests'] == []
        assert data['browser']['externalRequests'] == []
        assert len(data['browser']['successful_queries']) == 2
        assert all(len(run['samples']) == 6 and all(s['matches'] for s in run['samples'])
                   for run in data['browser']['successful_queries'])
        assert all(run['maximumScreenSpaceError'] == 16 for run in data['browser']['runs'])
        assert set(data['independent_matrix_controls_rejected']) == {
            'missing-root-placement', 'wrong-height', 'swapped-east-north'}
        with zipfile.ZipFile(data['archive']) as stream:
            for filename, expected in data['artifact']['member_sha256'].items():
                assert hashlib.sha256(stream.read(filename)).hexdigest() == expected

    resource_path = OLD / 'resource-release.json'
    resource = json.loads(resource_path.read_text())
    receipt['receipts'][str(resource_path)] = sha(resource_path)
    assert resource['production_source_commit'] == receipt['production_commit']
    assert sha(resource['binary_path']) == resource['binary_sha256']
    for filename, expected in resource['source_sha256'].items():
        assert sha(ROOT / filename) == expected
    for filename, expected in resource['driver_sha256'].items():
        assert sha(ROOT / filename) == expected
    assert len(resource['cases']) == 3
    for case in resource['cases']:
        assert sha(case['source_path']) == case['source_sha256']
        assert case['child_reaped'] is True and case['timed_out'] is False
        response = json.loads(case['stdout'])
        if case['exit_code'] == 0:
            artifact = case['artifact']
            assert sha(case['archive_path']) == artifact['archive_sha256']
            reference = artifact['independent_geometry_reference']
            assert reference['requested_budget_proved'] is True
            assert reference['published_smaller_certificate_independently_proved'] is False
            with zipfile.ZipFile(case['archive_path']) as stream:
                for filename, expected in artifact['member_sha256'].items():
                    assert hashlib.sha256(stream.read(filename)).hexdigest() == expected
                published = json.loads(stream.read('conversion.json'))
            assert canonical(response['meshReport']) == canonical(published)
        else:
            assert response['ok'] is False and response['error']['kind'] == 'unsupported'
            assert not Path(case['archive_path']).parent.exists()

    wheel_path = FINAL / 'wheel-api.json'
    wheel = json.loads(wheel_path.read_text())
    receipt['receipts'][str(wheel_path)] = sha(wheel_path)
    wheels = list((FINAL / 'wheels').glob('*.whl'))
    assert len(wheels) == 1 and sha(wheels[0]) == wheel['wheel_sha256']
    assert wheel['empty_path'] is True and wheel['ok'] is True
    assert (wheel['tests_run'], wheel['failures'], wheel['errors'], wheel['skipped']) == (44, 0, 0, 0)
    receipt['wheel'] = {'path': str(wheels[0]), 'sha256': sha(wheels[0]), 'api': wheel}
    source_path = FINAL / 'source-artifacts.json'
    pins = json.loads(source_path.read_text())
    receipt['receipts'][str(source_path)] = sha(source_path)
    assert pins['source_commit'] == receipt['production_commit']
    for filename, expected in pins['production_sha256'].items():
        assert sha(ROOT / filename) == expected
    for filename, expected in pins['final_driver_sha256'].items():
        assert sha(ROOT / filename) == expected
    assert sha(pins['wheel']['path']) == pins['wheel']['sha256'] == wheel['wheel_sha256']
    with zipfile.ZipFile(pins['wheel']['path']) as stream:
        assert set(stream.namelist()) == set(pins['wheel']['members_sha256'])
        for filename, expected in pins['wheel']['members_sha256'].items():
            assert hashlib.sha256(stream.read(filename)).hexdigest() == expected
    receipt['verified_production_files'] = len(pins['production_sha256'])
    receipt['final_driver_sha256'] = pins['final_driver_sha256']
    package_path = FINAL / 'package-checks.json'
    package = json.loads(package_path.read_text())
    receipt['receipts'][str(package_path)] = sha(package_path)
    assert package['production_source_commit'] == receipt['production_commit']
    assert package['exit_code'] == 0
    assert sha(package['package_path']) == package['package_sha256']
    with tarfile.open(package['package_path']) as stream:
        def member(filename):
            return stream.extractfile('rusty-tiles-0.4.0/' + filename).read()
        for filename, expected in package['verified_production_sha256'].items():
            assert hashlib.sha256(member(filename)).hexdigest() == expected == sha(ROOT / filename)
        assert member('Cargo.toml.orig') == (ROOT / 'Cargo.toml').read_bytes()
        for filename, expected in package['required_docs_and_oracles_sha256'].items():
            assert hashlib.sha256(member(filename)).hexdigest() == expected
        original = tomllib.loads((ROOT / 'Cargo.lock').read_text())
        normalized = tomllib.loads(member('Cargo.lock').decode())
        original = {(p['name'], p['version']): p for p in original['package']}
        normalized = {(p['name'], p['version']): p for p in normalized['package']}
        assert not set(normalized) - set(original)
        assert all(normalized[key] == original[key] for key in normalized)
        removed = list(map(list, sorted(set(original) - set(normalized))))
        assert removed == package['normalized_metadata']['Cargo.lock']['removed_package_keys']
    receipt['package'] = {'path': package['package_path'], 'sha256': package['package_sha256'],
                          'verified_root_production_files': len(package['verified_production_sha256']),
                          'removed_workspace_only_lock_packages': removed,
                          'scope': package['package_verification_scope']}
    coordinator_path = ROOT / 'bench/architecture_audit/mesh_approximation_f1d2/coordinator-checks.json'
    coordinator = json.loads(coordinator_path.read_text())
    receipt['receipts'][str(coordinator_path)] = sha(coordinator_path)
    for check in coordinator['checks'].values():
        assert check['exit_code'] == 0
        path = Path(check['invocation_log_path'])
        assert sha(path) == check['sha256'] and path.stat().st_size == check['bytes']
    assert 'remaining_check' not in coordinator
    receipt['coordinator_logs_verified'] = len(coordinator['checks'])
    receipt['passed'] = True
    print(json.dumps(receipt, indent=2))


if __name__ == '__main__':
    main()
