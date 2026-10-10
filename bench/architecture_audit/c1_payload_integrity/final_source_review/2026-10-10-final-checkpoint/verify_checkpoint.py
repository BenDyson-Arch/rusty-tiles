"""Verify final source equivalence, CI receipt provenance and lossless retention."""
import gzip
import hashlib
import io
import json
import pathlib
import subprocess
import tarfile

REPO = pathlib.Path(__file__).resolve().parents[5]
EVIDENCE = pathlib.Path('/tmp/rusty-tiles-payload-final-evidence')
RETAINED = REPO / 'bench/architecture_audit/c1_payload_integrity/candidate_evidence/local-2ebfb74'
CONTROLS = REPO / 'bench/architecture_audit/c1_payload_integrity/production_controls'


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def git_bytes(commit, path):
    return subprocess.check_output(['git', 'show', f'{commit}:{path}'], cwd=REPO)


pin_path = EVIDENCE / 'final-build/source-pin.json'
pin = json.loads(pin_path.read_bytes())
previous = json.loads((EVIDENCE / 'context-build/source-pin.json').read_bytes())
assert pin['source_commit'] == '2ebfb749e9d71fdb72507fd1203472b226f23156'
assert pin['source_tree'] == '1881bb5a90cf94f261925a44fd80720bf16084fb'
assert set(pin['production_sha256']) == set(previous['production_sha256'])
changed = [path for path, digest in pin['production_sha256'].items() if digest != previous['production_sha256'][path]]
assert changed == ['src/lib.rs']
old_lib = git_bytes(previous['source_commit'], 'src/lib.rs')
new_lib = git_bytes(pin['source_commit'], 'src/lib.rs')
assert new_lib == old_lib.replace(b'/compiled_private_controls.rs', b'/compiled_private_controls_clippy.rs')
old_test = (CONTROLS / 'compiled_private_controls.rs').read_bytes()
new_test = (CONTROLS / 'compiled_private_controls_clippy.rs').read_bytes()
assert sha(old_test) == '403c05f33da985f6e9c165d95a480f0b77d171f342f5a371311f72b8f194dbc7'
assert sha(new_test) == 'dfa9ce48083905b4058710925f11a4690125381fc1e20836145b4b33400da227'
assert new_test == old_test.replace(b'            } else if count == 2 {\n                2\n', b'')

for flavor in ['portable', 'native']:
    receipt = json.loads((EVIDENCE / f'final-build/{flavor}-cli-receipt.json').read_bytes())
    raw = pathlib.Path(receipt['binary']).read_bytes()
    assert sha(raw) == receipt['binary_sha256'] and len(raw) == receipt['bytes']
    assert receipt['source_commit'] == pin['source_commit']
    assert receipt['source_pin_sha256'] == sha(pin_path.read_bytes())
    assert receipt['build_exit_code'] == 0 and receipt['productionAndSelectedInputsUnchanged'] is True
    assert receipt['build_log_sha256'] == sha((EVIDENCE / f'final-build/{flavor}-cli-build.log').read_bytes())

harness = json.loads((EVIDENCE / 'final-build/context-harness-receipt.json').read_bytes())
unit = json.loads((EVIDENCE / 'final-build/receipt.json').read_bytes())
assert harness['sourceCommit'] == pin['source_commit'] and harness['sourceTree'] == pin['source_tree']
assert harness['unitSha256'] == unit['sha256']
assert harness['logSha256'] == unit['log_sha256']
assert harness['runnerSha256'] == sha((REPO / 'tests/payload_context_receipt.py').read_bytes())
assert harness['passes'] == len(harness['executions']) == 82
assert harness['executions'] == json.loads((EVIDENCE / 'final-build/context-executions.json').read_bytes())['executions']
assert len(harness['sourceAndInputSha256']) == 270
for path, digest in harness['sourceAndInputSha256'].items():
    assert sha((REPO / path).read_bytes()) == digest, path
for path, digest in pin['production_sha256'].items():
    assert harness['sourceAndInputSha256'][path] == digest

index_path = RETAINED / 'index.json'
index = json.loads(index_path.read_bytes())
assert index['sourceCommit'] == pin['source_commit'] and index['sourceTree'] == pin['source_tree']
assert index['sourcePinSha256'] == sha(pin_path.read_bytes())
assert len(index['records']) == 45
streams = 0
for record in index['records']:
    packed = (RETAINED / record['file']).read_bytes()
    assert len(packed) == record['gzipBytes'] and sha(packed) == record['gzipSha256']
    raw = gzip.decompress(packed)
    assert len(raw) == record['bytes'] and sha(raw) == record['sha256']
    source = pathlib.Path(record['sourcePath'])
    if source.is_file():
        assert raw == source.read_bytes()
    else:
        assert record['file'] == 'historical-original-205-streams.tar.gz'
        with tarfile.open(fileobj=io.BytesIO(raw)) as archive:
            members = [member for member in archive.getmembers() if member.isfile()]
            assert len(members) == 410
            assert {member.name for member in members} == {path.name for path in source.iterdir() if path.is_file() and path.suffix in ['.stdout', '.stderr']}
            for member in members:
                assert archive.extractfile(member).read() == (source / member.name).read_bytes()
            streams += len(members)
assert streams == 410

print(json.dumps(dict(sourceCommit=pin['source_commit'], sourceTree=pin['source_tree'],
                      sourcePinSha256=sha(pin_path.read_bytes()), onlyProductionChangeFromCa9='test cfg include',
                      compiledControlEquivalentBranchCollapse=True,
                      contextHarnessActualUnitLogAnd82RowsMatch=True, contextHarness270InputsVerified=True,
                      retainedIndexSha256=sha(index_path.read_bytes()), losslessRecordsVerified=45,
                      gzipTotalBytes=sum(row['gzipBytes'] for row in index['records']), historicalRawStreamsVerified=streams,
                      portableAndNativeBuildArtifactsVerified=True,
                      scope='Local exact checkpoint and retention verification; no remote/platform acceptance or historical artifact rebinding'), indent=2))
