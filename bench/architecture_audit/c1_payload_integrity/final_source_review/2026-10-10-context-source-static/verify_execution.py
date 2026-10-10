"""Verify retained coordinator execution; never execute a rusty-tiles target."""
import copy
import hashlib
import json
import pathlib
import re
import subprocess


REPO = pathlib.Path(__file__).resolve().parents[5]
BASE = REPO / 'bench/architecture_audit/c1_payload_integrity/production_controls/context_controls/2026-10-10'
BUILD = pathlib.Path('/tmp/rusty-tiles-payload-final-evidence/context-build')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


receipt = json.loads((BUILD / 'receipt.json').read_bytes())
pin = json.loads((BUILD / 'source-pin.json').read_bytes())
assert sha(BUILD / 'source-pin.json') == receipt['source_pin_sha256']
later_worktree_changes = []
for section in ['production_sha256', 'acceptance_inputs_sha256']:
    for path, digest in pin[section].items():
        if sha(REPO / path) != digest:
            # Root may begin a separately dated correction after the completed
            # source was reviewed. Verify the immutable recorded commit, without
            # rebinding this execution to those later working-tree bytes.
            raw = subprocess.check_output(['git', 'show', pin['source_commit'] + ':' + path], cwd=REPO)
            assert hashlib.sha256(raw).hexdigest() == digest, path
            later_worktree_changes.append(path)
artifact = pathlib.Path(receipt['frozen_unit_artifact'])
assert sha(artifact) == receipt['sha256'] and artifact.stat().st_size == receipt['bytes']
assert sha(BUILD / 'library.log') == receipt['log_sha256']
assert sha(BUILD / 'context-executions.json') == receipt['context_execution_receipt_sha256']
log = (BUILD / 'library.log').read_text()
assert 'test result: ok. 365 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;' in log
retained = json.loads((BUILD / 'context-executions.json').read_bytes())
assert retained['sourcePinSha256'] == receipt['source_pin_sha256']
assert retained['sourceCommit'] == pin['source_commit'] == receipt['source_commit']
assert retained['unitSha256'] == receipt['sha256']
assert retained['logSha256'] == receipt['log_sha256']
logged = []
decoder = json.JSONDecoder()
for line in log.splitlines():
    if 'independentContextControl' in line:
        row, _ = decoder.raw_decode(line[line.index('{'):])
        logged.append(row)
assert logged == retained['executions']
manifest = json.loads((BASE / 'fixtures/manifest.json').read_bytes())
cases = {case['name']: case for case in manifest['cases']}
baseline = dict(jsonBytes=65536, jsonDepth=32, memberBytes=262144,
                archiveStoredBytes=1048576, archiveEntries=256,
                accessorElements=4096, documentDecodedBytes=262144,
                hierarchyVisits=128, hierarchyDepth=32, references=1024,
                totalPayloadElements=16, totalBytesRead=4194304,
                sourceArchiveBytes=1048576, centralDirectoryBytes=65536,
                documentItems=8192)
expected_keys = set()
for case in manifest['cases']:
    facts = ['bytes', 'depth', 'nodes'] if case['target'] else ['totalPayloadElements']
    expected_keys.update((case['name'], fact, outcome) for fact in facts for outcome in ['admitted', 'resource_limit'])
observed_keys = set()
for row in logged:
    name, fact, outcome = row['independentContextControl'], row['fact'], row['outcome']
    key = (name, fact, outcome)
    assert key not in observed_keys
    observed_keys.add(key)
    case = cases[name]
    exact = case['metrics'][name][fact] if case['target'] else case['aggregateEquality']
    selected = copy.deepcopy(baseline)
    selected[dict(bytes='jsonBytes', depth='jsonDepth', nodes='documentItems').get(fact, fact)] = exact - (outcome == 'resource_limit')
    assert row['selectedLimits'] == selected
    assert row['archiveSha256'] == case['sha256'] and row['readOnly'] is True
    if outcome == 'admitted':
        report = copy.deepcopy(case['expectedReport'])
        report['archive'] = str(BASE / case['path'])
        report['limits'] = selected
        encoded = json.dumps(report, separators=(',', ':'), sort_keys=True).encode()
        assert hashlib.sha256(encoded).hexdigest() == row['actualReportSha256'], key
    else:
        assert row['actualReportSha256'] is None
assert expected_keys == observed_keys and len(logged) == 82
tests = re.findall(r'test payload_context_acceptance::(\w+) \.\.\. ok', log)
assert len(set(tests)) == 13
print(json.dumps(dict(sourceCommit=pin['source_commit'], sourcePinSha256=receipt['source_pin_sha256'],
                      productionFilesVerified=len(pin['production_sha256']), acceptanceFilesVerified=len(pin['acceptance_inputs_sha256']),
                      unitSha256=receipt['sha256'], unitBytes=receipt['bytes'],
                      libraryPassed=365, libraryFailed=0, libraryIgnored=0, libraryFiltered=0,
                      distinctContextTests=len(set(tests)), actualC1Executions=len(logged),
                      admitted=sum(row['outcome'] == 'admitted' for row in logged),
                      resourceLimit=sum(row['outcome'] == 'resource_limit' for row in logged),
                      allManifestPairsPresent=True, allSelectedFactsMatch=True,
                      positiveCompleteReportHashesMatch=True, rawLogMatchesRetainedRecords=True,
                      laterWorktreeChangesNotRebound=later_worktree_changes,
                      scope='Verified retained coordinator execution; reviewer did not execute target'), indent=2))
