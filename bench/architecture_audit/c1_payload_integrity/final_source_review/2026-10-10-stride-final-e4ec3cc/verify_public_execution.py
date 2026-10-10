"""Verify retained public CLI control receipts and lossless streams; no target run."""
import base64
import copy
import hashlib
import json
import pathlib
import sys
import tarfile


REPO = pathlib.Path(__file__).resolve().parents[5]
BASE = REPO / 'bench/architecture_audit/c1_payload_integrity/production_controls'
CORRECTION = BASE / 'polygon_correction/2026-10-10'
PIN = json.loads(pathlib.Path('/tmp/rusty-tiles-payload-final-evidence/stride-final-build/source-pin.json').read_bytes())
receipt_path = pathlib.Path(sys.argv[1])
receipt = json.loads(receipt_path.read_bytes())


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


assert receipt['runnerExitCode'] == 0 and 'runnerFailure' not in receipt
assert receipt['runnerSha256'] == sha((REPO / 'tests/payload_integrity_oracle.py').read_bytes())
assert receipt['adjudicationSha256'] == sha((CORRECTION / 'adjudication.md').read_bytes())
results = []
binary_hashes = {}
for label, filename, count in [('corrected205', 'corrected-205-fixtures.tar.gz', 205), ('polygon5', 'polygon-sensitive-5-fixtures.tar.gz', 5)]:
    phase = receipt['phases'][label]
    with tarfile.open(CORRECTION / filename) as bundle:
        supplied = {member.name: bundle.extractfile(member).read() for member in bundle.getmembers() if member.isfile()}
    manifest = json.loads(supplied['manifest.json'])
    assert phase['manifestSha256'] == sha(supplied['manifest.json'])
    assert phase['driverSha256'] == manifest['driverSha256'] == sha((BASE / 'driver.py').read_bytes())
    assert phase['passes'] == phase['expectedCaseCount'] == count and phase['failures'] == 0
    assert phase['fixturesUnchanged'] is True and phase['sourceAndBinaryUnchanged'] is True
    assert phase['sourcePin']['source_commit'] == PIN['source_commit']
    assert phase['sourcePin']['source_tree'] == PIN['source_tree']
    assert phase['sourcePin']['production_sha256'] == PIN['production_sha256']
    assert phase['sourcePin']['runner_sha256'] == receipt['runnerSha256']
    assert phase['sourcePin']['fixture_bundle_sha256'] == sha((CORRECTION / filename).read_bytes())
    for path, digest in phase['sourceSnapshot'].items():
        assert sha((REPO / path).read_bytes()) == digest, path
    assert phase['fixturesBefore'] == {path: sha(raw) for path, raw in supplied.items()}
    declared = {case['name']: case for case in manifest['cases']}
    assert len(phase['cases']) == len(declared) == count
    assert {case['name'] for case in phase['cases']} == declared.keys()
    full_reports = 0
    for row in phase['cases']:
        case = declared[row['name']]
        assert row['passed'] is True
        assert row['expectedCategory'] == case['expectedCategory']
        assert row['observedCategory'] == case['expectedCategory']
        assert row['exitCode'] == case['expectedExitCode']
        assert row['archiveSha256'] == case['sha256'] == sha(supplied[case['path']])
        stdout = base64.b64decode(row['stdoutBase64'], validate=True)
        stderr = base64.b64decode(row['stderrBase64'], validate=True)
        assert sha(stdout) == row['stdoutSha256'] and sha(stderr) == row['stderrSha256']
        response = json.loads(stdout)
        assert response == row['response']
        assert row['command'][1:3] == ['--json', 'validate']
        binary_path = row['command'][0]
        if binary_path not in binary_hashes:
            binary_hashes[binary_path] = sha(pathlib.Path(binary_path).read_bytes())
        assert binary_hashes[binary_path] == phase['binarySha256']
        expected = copy.deepcopy(case['expectedReport'])
        if expected is not None:
            expected['archive'] = row['command'][-1]
            assert response == expected == row['expectedReport']
            full_reports += 1
        else:
            assert row['expectedReport'] is None
            assert response['error']['code'] == case['expectedCategory']
            assert response['exitCode'] == case['expectedExitCode']
    results.append(dict(phase=label, cases=count, fullReportsVerified=full_reports,
                        rawStreamsHashChecked=count * 2, binarySha256=phase['binarySha256'],
                        sourceCommit=phase['sourcePin']['source_commit'],
                        allCategoriesAndExitsMatch=True, archiveAndInputIdentitiesMatch=True))
for binary_path, digest in binary_hashes.items():
    assert sha(pathlib.Path(binary_path).read_bytes()) == digest
print(json.dumps(dict(receipt=str(receipt_path), receiptSha256=sha(receipt_path.read_bytes()),
                      phases=results, sourceAndFixturesUnchanged=True,
                      scope='Verified retained exact-source public execution; no reviewer target execution'), indent=2))
