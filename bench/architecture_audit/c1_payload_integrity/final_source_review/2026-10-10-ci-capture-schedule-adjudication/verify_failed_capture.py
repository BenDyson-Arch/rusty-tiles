"""Inspect original corrupt capture and unchanged emitter; never repair or run target."""
import hashlib
import json
import pathlib
import re

REPO = pathlib.Path('/tmp/rusty-tiles-c1-payload-foundation')
EXTERNAL = pathlib.Path('/tmp/rusty-tiles-payload-final-evidence/remote-5fdf6fb')
uploaded = EXTERNAL / 'failed-main-c1-upload/c1-library.log'
api = EXTERNAL / 'rust-job-api-complete.log'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


assert sha(uploaded) == '669eb8f547b8b868292a3ffcb9ce28feceef85505beef5706976ec25ba857023'
assert sha(api) == 'a6ef3061719aa8dc4d790abd3a41424d34ca8379f6fca19709dedd7f80334d7f'
decoder = json.JSONDecoder()
complete = []
corrupt = []
raw = uploaded.read_text()
for line in raw.splitlines():
    start = line.find('{"actualReportSha256"')
    if start >= 0:
        try:
            complete.append(decoder.raw_decode(line[start:])[0])
        except json.JSONDecodeError as error:
            corrupt.append(dict(error=str(error), originalLine=line))
assert len(complete) == 77 and len(corrupt) == 3
assert raw.count('{"actualReportSha256"') == 80
assert '"accessorElements"test payload_context_acceptance::actual_c1_batch_table_selected_limits' in corrupt[0]['originalLine']
assert re.search(r'^test result: ok\. 367 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;', raw, re.M)
assert 'JSONDecodeError: Expecting' in api.read_text()
paths = [
    'bench/architecture_audit/c1_payload_integrity/production_controls/context_controls/2026-10-10/compiled_context_controls.rs',
    'tests/payload_context_receipt.py', 'src/validate.rs',
    'src/runtime/tests.rs', 'src/runtime/directory.rs', '.github/workflows/ci.yml',
]
context = (REPO / paths[0]).read_text()
assert 'eprintln!(' in context and 'json!({' in context
assert not any(token in context for token in ['thread::', '.spawn(', 'Command::new', 'tokio::'])
runtime = (REPO / 'src/runtime/tests.rs').read_text()
directory = (REPO / 'src/runtime/directory.rs').read_text()
assert 'thread::scope(' in runtime and 'ready.wait();' in runtime and 'handle.join().unwrap()' in runtime
assert '.spawn()' in directory and 'assert!(child.wait().unwrap().success());' in directory
print(json.dumps(dict(originalUploadSha256=sha(uploaded), originalUploadBytes=uploaded.stat().st_size,
    originalApiLogSha256=sha(api), fullLibraryPassed=367, diagnosticIntactRecords=77,
    diagnosticCorruptMarkerLines=corrupt, intactStartMarkers=80,
    accepted82RecordReceipt=False, reconstructedOrDroppedRecords=False,
    selectedContextEmitterHasNoSpawnedWriters=True,
    ownedScopedThreadAndChildConcurrencyRemainsInTests=True,
    observedPrecodeInputsSha256={path: sha(REPO / path) for path in paths},
    decision='CI-only outer harness serialization is justified; actual corrected capture remains required'), indent=2))
