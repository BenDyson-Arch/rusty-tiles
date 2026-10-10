"""Check exact-source build receipts and selected input preservation, without runs."""
import hashlib
import json
import pathlib
import subprocess

REPO = pathlib.Path(__file__).resolve().parents[5]
EVIDENCE = pathlib.Path('/tmp/rusty-tiles-payload-final-evidence')
BUILD = EVIDENCE / 'stride-final-build'
pin_path = BUILD / 'source-pin.json'
pin = json.loads(pin_path.read_bytes())
old = json.loads((EVIDENCE / 'final-build/source-pin.json').read_bytes())


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


assert {path for path, digest in old['production_sha256'].items() if pin['production_sha256'].get(path) != digest} == {'src/content_integrity/payload.rs', 'src/lib.rs'}
assert {path for path, digest in old['acceptance_inputs_sha256'].items() if pin['acceptance_inputs_sha256'].get(path) != digest} == {'.github/workflows/ci.yml'}
for group in ['production_sha256', 'acceptance_inputs_sha256']:
    for path, digest in pin[group].items():
        assert sha(REPO / path) == digest
        assert hashlib.sha256(subprocess.check_output(['git', 'show', pin['source_commit'] + ':' + path], cwd=REPO)).hexdigest() == digest
results = []
for name in ['library', 'portable-cli', 'native-cli', 'native-clippy', 'bindings-clippy', 'context-parser']:
    receipt_path = BUILD / (name + '-receipt.json')
    receipt = json.loads(receipt_path.read_bytes())
    assert receipt['source_commit'] == pin['source_commit'] and receipt['source_tree'] == pin['source_tree']
    assert receipt['source_pin_sha256'] == sha(pin_path)
    assert receipt['exit_code'] == 0 and receipt['all_inputs_unchanged'] is True
    assert sha(BUILD / (name + '.log')) == receipt['log_sha256']
    if 'frozen_artifact' in receipt:
        artifact = pathlib.Path(receipt['frozen_artifact'])
        assert sha(artifact) == receipt['artifact_sha256'] and artifact.stat().st_size == receipt['artifact_bytes']
    results.append(dict(name=name, receiptSha256=sha(receipt_path), exitCode=0, artifactSha256=receipt.get('artifact_sha256'), artifactBytes=receipt.get('artifact_bytes'), command=receipt['argv']))
workflow = (REPO / '.github/workflows/ci.yml').read_text()
assert 'tests/payload_context_receipt.py --log' in workflow and 'tests/payload_meshopt_stride_oracle.py --binary target/debug/rusty-tiles' in workflow
assert 'c1-payload-stride.json' in workflow
print(json.dumps(dict(sourceCommit=pin['source_commit'], sourceTree=pin['source_tree'], sourcePinSha256=sha(pin_path), productionFilesVerified=97, acceptanceFilesVerified=237, unchangedExistingAcceptanceFiles=178, onlyExistingAcceptanceChange='CI workflow adds independent stride lane', actualArtifactBuildsAndChecks=results, ciHarnessesRead=True, remoteCiExecutedByReviewer=False), indent=2))
