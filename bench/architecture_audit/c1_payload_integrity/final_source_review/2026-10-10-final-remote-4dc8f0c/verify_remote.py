"""Nonauthor inspection of retained API metadata and uploaded bytes; no target execution."""
import base64
import csv
import email.parser
import hashlib
import io
import json
from pathlib import Path
import re
import subprocess
import zipfile

BASE = Path('/tmp/rusty-tiles-payload-final-evidence/remote-4dc8f0c')
REPO = Path('/tmp/rusty-tiles-c1-payload-foundation')
HEAD = '4dc8f0c04dd5a98420768d05c6787f3baa49bb75'
TREE = '57095507d251e4f2fcb826cee367fa72bd306158'
PREVIEW = '2a13c3268233d20218e867a4436d3e6b84770c7b'
MAIN, WHEEL = 38050915453, 38050915448
PINS = {}

def sha(data):
    return hashlib.sha256(data).hexdigest()

def read(path):
    path = Path(path)
    raw = path.read_bytes()
    PINS[str(path)] = {'bytes': len(raw), 'sha256': sha(raw)}
    return raw

def load(path):
    return json.loads(read(path))

def git(*args):
    return subprocess.check_output(['git', '-C', str(REPO), *args]).decode().strip()

assert git('rev-parse', 'HEAD') == HEAD
assert not git('status', '--porcelain')
assert git('rev-parse', 'HEAD^{tree}') == TREE
assert git('rev-parse', PREVIEW + '^{tree}') == TREE
assert HEAD in git('show', '-s', '--format=%P', PREVIEW).split()
pin = load(BASE / 'correction-source-pin.json')
assert pin['source_commit'] == HEAD and pin['source_tree'] == TREE
assert len(pin['production_sha256']) == 97 and len(pin['acceptance_inputs_sha256']) == 237
for group in ('production_sha256', 'acceptance_inputs_sha256'):
    for path, expected in pin[group].items():
        assert sha(read(REPO / path)) == expected, path
        committed = subprocess.check_output(['git', '-C', str(REPO), 'show', HEAD + ':' + path])
        assert sha(committed) == expected, path
extra = ['.github/workflows/wheel-acceptance.yml', 'scripts/blender_official.json',
         'scripts/download_blender.py', 'scripts/test_python_wheel.py', 'scripts/test_blender_wheel.py',
         'scripts/wheel_acceptance.py', 'bindings/python/tests/test_api.py',
         'bindings/python/tests/blender_acceptance.py']
for path in extra:
    raw = read(REPO / path)
    assert raw == subprocess.check_output(['git', '-C', str(REPO), 'show', HEAD + ':' + path])
snapshot = BASE / 'snapshot-20261010T122055Z'
runs = {}
for name, expected in [('main', MAIN), ('wheel', WHEEL)]:
    run = load(snapshot / (name + '-run.json'))
    assert run['id'] == expected and run['head_sha'] == HEAD
    assert run['head_commit']['tree_id'] == TREE
    assert run['status'] == 'completed' and run['conclusion'] == 'success'
    assert run['event'] == 'pull_request' and run['run_attempt'] == 1
    assert any(p['number'] == 153 and p['head']['sha'] == HEAD and p['base']['ref'] == 'develop'
               for p in run['pull_requests'])
    runs[expected] = run
jobs = []
for name, expected in [('main', MAIN), ('wheel', WHEEL)]:
    record = load(BASE / 'final-jobs-20261010T122131Z' / (name + '.json'))
    assert len(record['jobs']) == record['total_count']
    for job in record['jobs']:
        assert job['run_id'] == expected and job['head_sha'] == HEAD
        assert job['status'] == 'completed' and job['run_attempt'] == 1
        jobs.append(job)
assert len(jobs) == 23
passed = [j for j in jobs if j['conclusion'] == 'success']
skipped = [j for j in jobs if j['conclusion'] == 'skipped']
assert len(passed) == 21 and {j['name'] for j in skipped} == {
    'Release acceptance', 'Distribution Blender (Linux x86_64)'}
pr = load(snapshot / 'pr.json')
assert pr['number'] == 153 and pr['headRefOid'] == HEAD and pr['baseRefName'] == 'develop'
assert pr['state'] == 'OPEN' and pr['mergeable'] == 'MERGEABLE'
assert len(pr['statusCheckRollup']) == 23
assert {(c['name'], c['conclusion'].lower()) for c in pr['statusCheckRollup']} == {
    (j['name'], j['conclusion']) for j in jobs}
assert all(c['status'] == 'COMPLETED' for c in pr['statusCheckRollup'])
api_artifacts = {}
for name in ('main', 'wheel'):
    document = load(BASE / (name + '-artifacts-20261010T122102Z.json'))
    assert len(document['artifacts']) == document['total_count']
    for row in document['artifacts']:
        api_artifacts[row['id']] = row
artifacts = {}
for path in sorted((BASE / 'artifacts').glob('*/receipt.json')):
    r = load(path)
    assert r['id'] in api_artifacts
    assert r['artifactMetadata'] == api_artifacts[r['id']]
    meta = r['artifactMetadata']
    assert meta['name'] == r['name'] and meta['id'] == r['id']
    assert meta['workflow_run']['head_sha'] == HEAD and meta['workflow_run']['id'] == r['run']
    assert r['head'] == HEAD and r['run'] in (MAIN, WHEEL) and not meta['expired']
    raw = read(path.parent / 'artifact.zip')
    assert len(raw) == r['archiveBytes'] == meta['size_in_bytes']
    assert sha(raw) == r['archiveSha256']
    assert 'sha256:' + sha(raw) == r['publisherDigest'] == meta['digest']
    members = {m['path']: m for m in r['members']}
    with zipfile.ZipFile(io.BytesIO(raw)) as z:
        assert z.testzip() is None
        names = [n for n in z.namelist() if not n.endswith('/')]
        assert len(names) == len(set(names)) and set(names) == set(members)
        for name in names:
            data = z.read(name)
            m = members[name]
            assert len(data) == m['bytes'] and sha(data) == m['sha256']
            assert read(path.parent / 'files' / name) == data
    assert r['name'] not in artifacts
    artifacts[r['name']] = (r, path.parent / 'files')
assert len(artifacts) == 15

targets = {
    'x86_64-unknown-linux-gnu': ('manylinux_2_28_x86_64', 'Linux', ('x86_64', 'amd64'), 'ubuntu-22.04'),
    'aarch64-unknown-linux-gnu': ('manylinux_2_28_aarch64', 'Linux', ('aarch64', 'arm64'), 'ubuntu-22.04-arm'),
    'x86_64-apple-darwin': ('macosx_15_0_x86_64', 'macOS', ('x86_64', 'amd64'), 'macos-15-intel'),
    'aarch64-apple-darwin': ('macosx_15_0_arm64', 'macOS', ('aarch64', 'arm64'), 'macos-15'),
    'x86_64-pc-windows-msvc': ('win_amd64', 'Windows', ('amd64', 'x86_64'), 'windows-2022'),
}
wheel_results = {}
for target, (tag, system, machines, label) in targets.items():
    r, directory = artifacts['wheel-' + target]
    assert r['run'] == WHEEL
    wheels = list(directory.glob('*.whl'))
    assert len(wheels) == 1
    wheel = wheels[0]
    assert wheel.name == f'rusty_tiles-0.4.0-cp310-abi3-{tag}.whl'
    raw = read(wheel)
    digest = sha(raw)
    with zipfile.ZipFile(io.BytesIO(raw)) as z:
        assert z.testzip() is None
        names = z.namelist()
        assert len(names) == len(set(names))
        prefix = 'rusty_tiles-0.4.0.dist-info/'
        metadata = email.parser.BytesParser().parsebytes(z.read(prefix + 'METADATA'))
        assert metadata['Name'] == 'rusty-tiles' and metadata['Version'] == '0.4.0'
        detail = email.parser.BytesParser().parsebytes(z.read(prefix + 'WHEEL'))
        assert detail['Root-Is-Purelib'] == 'false'
        assert detail.get_all('Tag') == ['cp310-abi3-' + tag]
        record = list(csv.reader(io.StringIO(z.read(prefix + 'RECORD').decode())))
        assert len(record) == len(names) and {row[0] for row in record} == set(names)
        for name, hashed, size in record:
            if name == prefix + 'RECORD':
                assert hashed == '' and size == ''
            else:
                data = z.read(name)
                encoded = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).decode().rstrip('=')
                assert hashed == 'sha256=' + encoded and int(size) == len(data)
        extension = [n for n in names if n.startswith('rusty_tiles/') and n.endswith(('.so', '.pyd'))]
        assert len(extension) == 1
    job = next(j for j in jobs if j['name'] == 'Installed wheel (' + target + ')')
    assert job['run_id'] == WHEEL and job['conclusion'] == 'success' and job['labels'] == [label]
    evidence_receipt, evidence_dir = artifacts['evidence-wheel-' + target]
    assert evidence_receipt['run'] == WHEEL
    reports = []
    for version in ('3.10', '3.14'):
        report = load(evidence_dir / ('python-' + version + '.json'))
        assert report['wheel_sha256'] == digest and report['python'].startswith(version + '.')
        assert report['package_version'] == '0.4.0' and report['empty_path'] is True
        assert report['tests_run'] == 44 and report['ok'] is True
        assert report['failures'] == report['errors'] == report['skipped'] == 0
        assert 'runner_error' not in report
        assert report['machine'].lower() in machines and report['platform'].startswith(system)
        package = report['package_file'].replace('\\', '/')
        assert '/site-packages/rusty_tiles/__init__.py' in package and '/venv/' in package
        reports.append(report)
    wheel_results[target] = {'sha256': digest, 'bytes': len(raw), 'filename': wheel.name,
                             'artifact': r['id'], 'installedEvidenceArtifact': evidence_receipt['id'],
                             'job': job['id'], 'recordMembersVerified': len(record),
                             'nativeExtension': extension[0], 'installedReports': reports}

manifest = load(REPO / 'scripts/blender_official.json')
mapping = {'linux-x64': 'x86_64-unknown-linux-gnu', 'windows-x64': 'x86_64-pc-windows-msvc',
           'macos-x64': 'x86_64-apple-darwin', 'macos-arm64': 'aarch64-apple-darwin'}
blender_results = {}
for platform, target in mapping.items():
    r, directory = artifacts['evidence-official-blender-' + platform]
    assert r['run'] == WHEEL
    distribution = load(directory / 'blender-distribution.json')
    report = load(directory / 'blender-acceptance.json')
    spec = manifest['platforms'][platform]
    assert distribution['distribution'] == 'official-blender.org'
    assert distribution['distribution_platform'] == platform
    assert distribution['version'] == manifest['version'] == '4.5.14'
    assert distribution['archive_url'] == manifest['base_url'] + spec['archive']
    assert distribution['checksum_url'] == manifest['checksum_url']
    assert distribution['archive_sha256'] == spec['sha256']
    assert re.fullmatch('[a-f0-9]{64}', distribution['executable_sha256'])
    assert distribution['executable'].replace('\\', '/').endswith(spec['executable'])
    assert report['blender_distribution'] == distribution
    assert report['wheel_sha256'] == wheel_results[target]['sha256']
    assert report['blender_version'] == [4, 5, 14] and report['blender'] == '4.5.14 LTS'
    assert re.fullmatch('[a-f0-9]{12}', report['blender_build_hash'])
    assert report['package_version'] == '0.4.0' and report['empty_path'] is True
    assert report['tests_run'] == 44 and report['ok'] is True
    assert report['failures'] == report['errors'] == report['skipped'] == 0
    assert 'runner_error' not in report
    assert report['machine'].lower() in spec['machines']
    assert report['python'].startswith('3.11.')
    assert '/packages/rusty_tiles/__init__.py' in report['package_file'].replace('\\', '/')
    job = next(j for j in jobs if j['name'] == 'Official Blender (' + platform + ')')
    assert job['run_id'] == WHEEL and job['conclusion'] == 'success'
    for required in ("Verify and unpack official Blender", "Install and test using official Blender's embedded Python"):
        step = next(s for s in job['steps'] if s['name'] == required)
        assert step['status'] == 'completed' and step['conclusion'] == 'success'
    blender_results[platform] = {'job': job['id'], 'artifact': r['id'],
                                 'distribution': distribution, 'acceptance': report,
                                 'distributionAndExecutableBytesUploaded': False}

checkpoint_dir = BASE / '2026-10-10-c1-upload-checkpoint'
old = load(checkpoint_dir / 'verification.json')
read(checkpoint_dir / 'verify_uploaded.py')
read(checkpoint_dir / 'review.md')
read(checkpoint_dir / 'review-receipt.json')
# Recheck the frozen nonauthor C1 verifier against these immutable uploads without
# rewriting its historical output or claiming its then-pending gates were closed.
again = json.loads(subprocess.check_output(['python3', str(checkpoint_dir / 'verify_uploaded.py')]))
assert again == old
assert old['actualLibraryPassed'] == 367 and old['actualContextCalls'] == 82 and old['publicCases'] == 217
expected_names = {'c1-validation-evidence'} | {'wheel-' + t for t in targets} | {
    'evidence-wheel-' + t for t in targets} | {'evidence-official-blender-' + p for p in mapping}
assert set(artifacts) == expected_names
assert not git('status', '--porcelain') and git('rev-parse', 'HEAD') == HEAD
print(json.dumps({'head': HEAD, 'tree': TREE, 'actualC1ExecutionCommit': PREVIEW,
                  'mainRun': MAIN, 'wheelRun': WHEEL, 'jobsPassed': 21,
                  'expectedSkippedJobs': [j['name'] for j in skipped],
                  'selectedProductionHashesVerified': 97, 'selectedAcceptanceHashesVerified': 237,
                  'artifactArchivesPublisherDigestsAndAllMembersVerified': 15,
                  'wheels': wheel_results, 'installedPythonReportsVerified': 10,
                  'officialBlender': blender_results, 'officialBlenderReportsVerified': 4,
                  'c1CheckpointReverifiedWithoutRebinding': old,
                  'boundedC1FoundationRemoteMergeGatesMet': True,
                  'mergeExecuted': False, 'mergeAcceptedAsAlreadyCompleted': False,
                  'A2ReleaseMainTagPublishingAcceptance': False,
                  'reviewerRanCargoOrTarget': False, 'reviewerChangedRepository': False,
                  'inputPins': PINS}, indent=2))
