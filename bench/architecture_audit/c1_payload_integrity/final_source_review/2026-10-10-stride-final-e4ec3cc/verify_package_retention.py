"""Verify frozen extracted-source compilation without Cargo or target execution."""
import copy
import gzip
import hashlib
import io
import json
import pathlib
import subprocess
import tarfile
import tomllib

REPO = pathlib.Path(__file__).resolve().parents[5]
EVIDENCE = pathlib.Path('/tmp/rusty-tiles-payload-final-evidence')
PACKAGE = EVIDENCE / 'stride-final-source-package'
RETAINED = REPO / 'bench/architecture_audit/c1_payload_integrity/candidate_evidence/local-e4ec3cc'
COMMIT = 'e4ec3cc6ecf95eeba5c610dc7decb4c83a15696a'


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def git_bytes(path):
    return subprocess.check_output(['git', 'show', f'{COMMIT}:{path}'], cwd=REPO)


pin_path = EVIDENCE / 'stride-final-build/source-pin.json'
pin = json.loads(pin_path.read_bytes())
for group in ['production_sha256', 'acceptance_inputs_sha256']:
    for name, digest in pin[group].items():
        assert sha(git_bytes(name)) == digest
        assert sha((REPO / name).read_bytes()) == digest
receipt_path = PACKAGE / 'receipt.json'
receipt = json.loads(receipt_path.read_bytes())
assert receipt['source_commit'] == COMMIT
assert receipt['source_tree'] == 'dd6bf3dc696f9adf1a44f5fe9e5b0c9ea873813b'
assert receipt['unchanged_local_execution_source'] == pin['source_commit']
assert receipt['source_pin_sha256'] == sha(pin_path.read_bytes())
assert receipt['argv'][-5:] == ['cargo', 'package', '--locked', '-p', 'rusty-tiles']
assert receipt['exit_code'] == 0
log = (PACKAGE / 'package-verify.log').read_bytes()
assert sha(log) == receipt['log_sha256']
assert b'Verifying rusty-tiles v0.4.0' in log
assert b'Compiling rusty-tiles v0.4.0 (/tmp/rusty-tiles-f1d2-final-target/package/rusty-tiles-0.4.0)' in log
assert b'Finished `dev` profile' in log

archive_path = PACKAGE / f"rusty-tiles-e4ec3cc-{receipt['archive_sha256']}.crate"
archive_raw = archive_path.read_bytes()
assert sha(archive_raw) == receipt['archive_sha256']
assert len(archive_raw) == receipt['archive_bytes'] == 5792085
with tarfile.open(fileobj=io.BytesIO(archive_raw)) as archive:
    members = archive.getmembers()
    assert all(member.isfile() and member.name.startswith('rusty-tiles-0.4.0/') for member in members)
    assert len(members) == len({member.name for member in members}) == 1965
    files = {member.name.removeprefix('rusty-tiles-0.4.0/'): archive.extractfile(member).read() for member in members}
vcs = json.loads(files['.cargo_vcs_info.json'])
assert vcs == {'git': {'sha1': COMMIT}, 'path_in_vcs': ''}
assert files['Cargo.toml.orig'] == git_bytes('Cargo.toml')

tree = subprocess.check_output(['git', 'ls-tree', '-r', '-z', COMMIT], cwd=REPO)
blobs = {}
for entry in tree.split(b'\0'):
    if entry:
        metadata, name = entry.split(b'\t', 1)
        mode, kind, oid = metadata.split()
        if kind == b'blob':
            blobs[name.decode()] = oid
original_names = set(files) - {'.cargo_vcs_info.json', 'Cargo.toml.orig', 'Cargo.toml', 'Cargo.lock'}
assert original_names <= blobs.keys()
oids = list(dict.fromkeys(blobs[name] for name in original_names))
batch = subprocess.check_output(['git', 'cat-file', '--batch'], cwd=REPO, input=b''.join(oid + b'\n' for oid in oids))
contents = {}
cursor = 0
for expected_oid in oids:
    end = batch.index(b'\n', cursor)
    oid, kind, size = batch[cursor:end].split()
    assert oid == expected_oid and kind == b'blob'
    cursor = end + 1
    size = int(size)
    contents[oid] = batch[cursor:cursor + size]
    cursor += size
    assert batch[cursor:cursor + 1] == b'\n'
    cursor += 1
assert cursor == len(batch)
for name in original_names:
    assert files[name] == contents[blobs[name]], name

exact_selected = []
omitted = []
for group in ['production_sha256', 'acceptance_inputs_sha256']:
    for name, digest in pin[group].items():
        if name in ['Cargo.toml', 'Cargo.lock']:
            continue
        if name.startswith(('bindings/', '.github/')):
            assert name not in files
            omitted.append(name)
            continue
        assert name in files and sha(files[name]) == digest, name
        exact_selected.append(name)
assert len(exact_selected) == 329
assert len([name for name in files if name.startswith('src/')]) == 92
assert 'src/validate/json.rs' not in files and 'src/validate/payload.rs' not in files


def dependencies(table):
    return {name: {'version': value} if isinstance(value, str) else value for name, value in table.items()}


original_manifest = tomllib.loads(files['Cargo.toml.orig'].decode())
normalized = tomllib.loads(files['Cargo.toml'].decode())
expected = copy.deepcopy(original_manifest)
workspace = expected.pop('workspace')
assert expected['package']['version'] == {'workspace': True}
expected['package']['version'] = workspace['package']['version']
expected['package'].update(build='build.rs', autolib=False, autobins=False, autoexamples=False, autotests=False, autobenches=False)
for kind in ['dependencies', 'dev-dependencies', 'build-dependencies']:
    expected[kind] = dependencies(expected[kind])
for target in expected['target'].values():
    for kind in ['dependencies', 'dev-dependencies', 'build-dependencies']:
        if kind in target:
            target[kind] = dependencies(target[kind])
expected['lib'] = {'name': 'rusty_tiles', 'path': 'src/lib.rs'}
expected['test'] = [{'name': pathlib.PurePosixPath(name).stem, 'path': name}
                    for name in sorted(files) if name.startswith('tests/') and name.count('/') == 1 and name.endswith('.rs')]
assert normalized == expected

original_lock = tomllib.loads(git_bytes('Cargo.lock').decode())
normalized_lock = tomllib.loads(files['Cargo.lock'].decode())
assert {key: value for key, value in original_lock.items() if key != 'package'} == {key: value for key, value in normalized_lock.items() if key != 'package'}
key = lambda row: (row['name'], row['version'], row.get('source'))
old = {key(row): row for row in original_lock['package']}
new = {key(row): row for row in normalized_lock['package']}
assert new.keys() <= old.keys()
assert all(old[name] == row for name, row in new.items())
removed = sorted(name[0] for name in old.keys() - new.keys())
assert removed == ['portable-atomic', 'pyo3', 'pyo3-build-config', 'pyo3-ffi', 'pyo3-macros', 'pyo3-macros-backend', 'rusty-tiles-python', 'target-lexicon']

root_key = ('rusty-tiles', '0.4.0', None)
reachable = set()
pending = [root_key]
while pending:
    current = pending.pop()
    if current in reachable:
        continue
    reachable.add(current)
    for dependency in old[current].get('dependencies', []):
        parts = dependency.split()
        candidates = [name for name in old if name[0] == parts[0] and (len(parts) == 1 or name[1] == parts[1])]
        assert len(candidates) == 1, dependency
        pending.append(candidates[0])
assert reachable == new.keys()

inventory = json.loads((PACKAGE / 'archive-inventory.json').read_bytes())
assert inventory['frozenArchive'] == str(archive_path)
assert inventory['archiveSha256'] == sha(archive_raw)
assert len(inventory['records']) == len(files)
assert {row['path']: row for row in inventory['records']} == {name: dict(path=name, bytes=len(raw), sha256=sha(raw), **({'trackedSource': 'byte-identical e4ec3cc'} if name in original_names else {})) for name, raw in files.items()}
retained_index_path = RETAINED / 'index.json'
retained_index = json.loads(retained_index_path.read_bytes())
assert retained_index['sourceCommit'] == COMMIT and len(retained_index['records']) == 82
assert sha(retained_index_path.read_bytes()) == '0a5e2b4be92b413420bad856d284aa77d73a0ce24456c4e98877b640da445739'
for row in retained_index['records']:
    packed = (RETAINED / row['file']).read_bytes()
    raw = gzip.decompress(packed)
    assert len(packed) == row['gzipBytes'] and sha(packed) == row['gzipSha256']
    assert len(raw) == row['bytes'] and sha(raw) == row['sha256']
    assert raw == pathlib.Path(row['sourcePath']).read_bytes()

print(json.dumps(dict(sourceCommit=COMMIT, sourceTree=receipt['source_tree'],
                      originalLocalExecutionSource=pin['source_commit'], sourcePinSha256=sha(pin_path.read_bytes()),
                      all97ProductionAnd237AcceptanceInputsMatch=True, packageDefaultVerificationExitCode=0,
                      logSha256=sha(log), archiveSha256=sha(archive_raw), archiveBytes=len(archive_raw),
                      archiveMembers=len(members), originalMembersByteIdenticalToCommit=len(original_names),
                      actualSrcFiles=92, selectedSourceAndControlMembersByteIdentical=329,
                      omittedSeparatePackageAndWorkflowPaths=omitted, vcsCleanExactCommit=True,
                      manifestOrigExact=True, normalizedManifestSemanticChanges=False,
                      lockedPackageTablesUnchanged=288, removedUnreachablePythonPackageTables=removed,
                      retainedIndexSha256=sha(retained_index_path.read_bytes()), losslessPhaseRecordsVerified=82,
                      decision='Bounded extracted root source archive compilation accepted',
                      scope='Default dev-profile compilation, not rerun library/public controls, native-feature source build, installed wheel, Blender, remote CI, publication, full A2 or release acceptance'), indent=2))
