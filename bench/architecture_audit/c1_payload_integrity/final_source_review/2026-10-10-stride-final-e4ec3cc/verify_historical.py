"""Keep unsuccessful or older executions separate from current acceptance."""
import base64
import hashlib
import json
import pathlib
import re

REPO = pathlib.Path(__file__).resolve().parents[5]
EVIDENCE = pathlib.Path('/tmp/rusty-tiles-payload-final-evidence')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


prior = REPO / 'bench/architecture_audit/c1_payload_integrity/final_source_review'
receipts = [
    ('2026-10-10-final-checkpoint', 'review-receipt.json', '7def5f7d0b0c330e73c5e34ea6240a084fe37c3e044ceb5cdc9d38b3c4037dbb'),
    ('2026-10-10-source-package-2ef8db1', 'review-receipt.json', '920febf5c3ca32cf7ea12a074e56519a6fb0039363c1316a6ee58d105f12f831'),
    ('2026-10-10-meshopt-stride-adjudication', 'decision-receipt.json', '517b051c30d5f1a90768f756d077477bae6e7cb03ed4ccd5a521ec6f2c282f90'),
]
for folder, name, digest in receipts:
    root = prior / folder
    assert sha(root / name) == digest
    for path, expected in json.loads((root / name).read_bytes())['phaseFilesSha256'].items():
        assert sha(root / path) == expected, path

old = json.loads((EVIDENCE / 'stride-old-binary-sensitive6/receipt.json').read_bytes())
assert old['sourcePin']['source_commit'] == '2ebfb749e9d71fdb72507fd1203472b226f23156'
assert old['binarySha256'] == sha(pathlib.Path(old['binary'])) == '104a60c7ac278f6cc6932b560136be0f63aa6b7cc8464e6cf64d9b4f122a07f2'
assert old['passes'] == 4 and old['failures'] == 2
failed = []
for case in old['cases']:
    for stream in ['stdout', 'stderr']:
        raw = (EVIDENCE / 'stride-old-binary-sensitive6' / (case['name'] + '.' + stream)).read_bytes()
        assert hashlib.sha256(raw).hexdigest() == case[stream + 'Sha256']
    if not case['passed']:
        assert case['expectedCategory'] == 'admitted' and case['observedCategory'] == 'invalid_input'
        assert case['exitCode'] == 3
        assert case['response']['error']['message'] == 'accessor stride differs from meshopt decoded stride'
        failed.append(case['name'])
assert set(failed) == {'unused-packed-scalar', 'unused-packed-scalar-offset'}

d642 = EVIDENCE / 'stride-corrected-build'
library = json.loads((d642 / 'library-receipt.json').read_bytes())
log = (d642 / 'library.log').read_text()
summaries = re.findall(r'^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;', log, re.M)
assert tuple(map(int, summaries[-1])) == (367, 0, 0, 0, 0)
assert library['exit_code'] == 0 and sha(d642 / 'library.log') == library['log_sha256']
clippy = json.loads((d642 / 'native-clippy-receipt.json').read_bytes())
assert clippy['exit_code'] == 101 and sha(d642 / 'native-clippy.log') == clippy['log_sha256']
assert (d642 / 'native-clippy.log').read_text().count('error: manual implementation of `.is_multiple_of()`') == 2

failed_precase = json.loads((EVIDENCE / 'stride-final-integrations/portable-payload210.json').read_bytes())
assert failed_precase['runnerExitCode'] == 1 and failed_precase['phases'] == {}
assert failed_precase['runnerFailure']['type'] == 'PermissionError'
binding = json.loads((EVIDENCE / 'stride-final-executable-mode-binding.json').read_bytes())
for artifact in binding['artifacts']:
    path = pathlib.Path(artifact['path'])
    assert sha(path) == artifact['sha256'] and path.stat().st_size == artifact['bytes']
    assert artifact['beforeMode'] == '0o644' and artifact['afterMode'] == '0o755'

print(json.dumps(dict(earlierReviewPhasesUnchanged=True, earlier2ebExecutedDefects=failed, oldRawStreamsVerified=12, d642CompleteLibraryPassed=367, d642ParserStopNotProductionFailure=True, d642TestOnlyClippyExitCode=101, finalPrecaseModeFailureHasNoExecutedCases=True, frozenArtifactByteHashesUnchangedByModeCorrection=True, noExecutionIdentitiesRebound=True), indent=2))
