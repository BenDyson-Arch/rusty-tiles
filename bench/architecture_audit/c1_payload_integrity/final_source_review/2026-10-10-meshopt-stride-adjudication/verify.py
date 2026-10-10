"""Verify retained pre-correction stride controls; no target execution."""
import base64
import copy
import gzip
import hashlib
import json
import math
import pathlib
import struct
import subprocess
import tarfile
import zipfile

REPO = pathlib.Path(__file__).resolve().parents[5]
PHASE = pathlib.Path(__file__).resolve().parent
PROBE = pathlib.Path('/tmp/rusty-tiles-payload-final-evidence/meshopt-stride-adjudication')
BASE = REPO / 'bench/architecture_audit/c1_payload_integrity'
REFERENCE = BASE / 'primary_controls/2026-10-10-pinned-reference-verification'


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


receipt = json.loads((PROBE / 'receipt.json').read_bytes())
assert receipt['source_commit'] == '2ebfb749e9d71fdb72507fd1203472b226f23156'
binary = pathlib.Path('/tmp/rusty-tiles-payload-final-evidence/final-build/portable-cli-2ebfb74-104a60c7ac278f6cc6932b560136be0f63aa6b7cc8464e6cf64d9b4f122a07f2')
assert sha(binary.read_bytes()) == receipt['binary_sha256']
with tarfile.open(BASE / 'production_controls/fixtures.tar.gz') as bundle:
    manifest = json.loads(bundle.extractfile('manifest.json').read())
primary_pins = []
for pin in manifest['referencePins']:
    if pin.get('member') == 'pinned-Specification.adoc':
        with tarfile.open(REPO / pin['bundle']) as bundle:
            raw = bundle.extractfile(pin['member']).read()
    elif pin.get('path') in [str((REFERENCE / name).relative_to(REPO)) for name in ['EXT_meshopt_compression.md', 'golden-compressed.bin', 'golden-expected.bin']]:
        raw = (REPO / pin['path']).read_bytes()
    elif pin.get('path') in ['docs/architecture/c1-payload-integrity-contract.md', 'bench/architecture_audit/c1_payload_integrity/prereq_review/2026-10-10-whole-slice-implementation-ready.md']:
        raw = (REPO / pin['path']).read_bytes()
    else:
        continue
    assert len(raw) == pin['bytes'] and sha(raw) == pin['sha256']
    primary_pins.append(pin)
assert len(primary_pins) == 6
decoded = (REFERENCE / 'golden-expected.bin').read_bytes()
compressed = (REFERENCE / 'golden-compressed.bin').read_bytes()
values = [struct.unpack_from('<f', decoded, offset)[0] for offset in range(0, 48, 4)]
assert all(math.isfinite(value) for value in values)
assert values == [value[0] for value in receipt['golden_decoded_float_oracle']['rawValues']]
positions = [values[offset:offset + 3] for offset in range(0, 12, 3)]
template = next(case for case in manifest['cases'] if case['name'] == 'meshopt-none-placeholder-gltf')['expectedReport']
assert template is not None
documents = {}
rows = []
for row in receipt['cases']:
    name = row['name']
    archive_path = PROBE / (name + '.3tz')
    assert sha(archive_path.read_bytes()) == row['archive_sha256']
    stdout = base64.b64decode(row['stdout_base64'], validate=True)
    stderr = base64.b64decode(row['stderr_base64'], validate=True)
    assert stdout == (PROBE / (name + '.stdout')).read_bytes()
    assert stderr == (PROBE / (name + '.stderr')).read_bytes()
    response = json.loads(stdout)
    with zipfile.ZipFile(archive_path) as archive:
        assert archive.namelist() == ['tileset.json', 'tile.gltf', 'compressed.bin', '@3dtilesIndex1@']
        assert archive.read('compressed.bin') == compressed
        document = json.loads(archive.read('tile.gltf'))
        documents[name] = document
        view = document['bufferViews'][0]
        codec = view['extensions']['EXT_meshopt_compression']
        assert codec == {'buffer': 1, 'byteOffset': 0, 'byteLength': 85, 'byteStride': 12, 'count': 4, 'mode': 'ATTRIBUTES', 'filter': 'NONE'}
        assert view['byteLength'] == codec['byteStride'] * codec['count'] == len(decoded)
        assert document['meshes'][0]['primitives'] == [{'attributes': {'POSITION': 0}, 'mode': 0}]
        position = document['accessors'][0]
        assert position['min'] == [min(value[c] for value in positions) for c in range(3)]
        assert position['max'] == [max(value[c] for value in positions) for c in range(3)]
    expected_report = copy.deepcopy(template)
    expected_report['archive'] = str(archive_path)
    expected_report['payloads'][0]['accessorsChecked'] = len(document['accessors'])
    if name == 'absent-parent-scalar-range-over':
        assert row['expected'] == 'invalid_input' and row['exit_code'] == 3
        assert response['error']['code'] == 'invalid_input'
        assert 'byte range outside actual resource' == response['error']['message']
        independent = 'invalid_input'
    elif name == 'absent-parent-unused-packed-scalar':
        assert row['expected'] == 'admitted' and row['exit_code'] == 3
        assert response['error'] == {'code': 'invalid_input', 'message': 'accessor stride differs from meshopt decoded stride'}
        independent = 'admitted'
    else:
        assert row['expected'] == 'admitted' and row['exit_code'] == 0
        assert response == expected_report
        independent = 'admitted'
    rows.append(dict(name=name, independentExpected=independent, observedExit=row['exit_code'],
                     archiveSha256=row['archive_sha256'], stdoutSha256=sha(stdout), stderrSha256=sha(stderr),
                     expectedCompleteReport=expected_report if independent == 'admitted' else None))
assert len(rows) == 4
baseline = documents['absent-parent-baseline']
packed = copy.deepcopy(baseline)
packed['accessors'].append({'bufferView': 0, 'componentType': 5126, 'count': 12, 'type': 'SCALAR'})
assert documents['absent-parent-unused-packed-scalar'] == packed
explicit = copy.deepcopy(packed)
explicit['bufferViews'][0]['byteStride'] = 12
explicit['accessors'][1]['count'] = 4
assert documents['explicit-parent-unused-scalar'] == explicit
over = copy.deepcopy(packed)
over['accessors'][1]['count'] = 13
assert documents['absent-parent-scalar-range-over'] == over
baseline_source = subprocess.check_output(['git', 'show', '0eebdeebf013591460e6ac77485ad91b7401aada:src/validate/payload.rs'], cwd=REPO)
assert b'accessor stride differs from meshopt decoded stride' in baseline_source
retained = BASE / 'candidate_evidence/2026-10-10-original-stride-probe'
index = json.loads((retained / 'index.json').read_bytes())
assert index['source_commit'] == receipt['source_commit'] and len(index['records']) == 13
for row in index['records']:
    packed = (retained / row['path']).read_bytes()
    raw = gzip.decompress(packed)
    assert len(packed) == row['stored_bytes'] and sha(packed) == row['stored_sha256']
    assert len(raw) == row['raw_bytes'] and sha(raw) == row['raw_sha256']
    assert raw == pathlib.Path(row['original_path']).read_bytes()

print(json.dumps(dict(sourceCommit=receipt['source_commit'], binarySha256=receipt['binary_sha256'],
                      probeReceiptSha256=sha((PROBE / 'receipt.json').read_bytes()), primaryPins=primary_pins,
                      fourArchiveIdentitiesVerified=True, eightRawStreamsVerified=True, finitePackedScalars=12,
                      codecDecodedBytes=48, packedAccessorBytes=48, rangeOverBytes=52,
                      expectedComponentWorkForCorrectedPackedCase=24,
                      twoActualPositiveCompleteReportsVerified=True, exactMutationBackgroundVerified=True,
                      inheritedBaselineRuleConfirmed=True, executedFalseInvalidInput=True, cases=rows,
                      thirteenLosslessProbeRecordsVerified=True,
                      retainedProbeIndexSha256=sha((retained / 'index.json').read_bytes()),
                      decision='Approve minimal owner correction before merge; no corrected-source acceptance yet'), indent=2))
