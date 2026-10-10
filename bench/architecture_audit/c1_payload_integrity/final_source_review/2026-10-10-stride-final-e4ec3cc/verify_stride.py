"""Independently inspect selected stride inputs and actual receipts; no target run."""
import base64
import copy
import hashlib
import io
import json
import math
import pathlib
import re
import struct
import subprocess
import sys
import tarfile
import zipfile

REPO = pathlib.Path(__file__).resolve().parents[5]
BASE = REPO / 'bench/architecture_audit/c1_payload_integrity/production_controls'
EVIDENCE = pathlib.Path('/tmp/rusty-tiles-payload-final-evidence')
PIN = json.loads((EVIDENCE / 'stride-final-build/source-pin.json').read_bytes())


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


original_source = subprocess.check_output(['git', 'show', '2ebfb749e9d71fdb72507fd1203472b226f23156:src/content_integrity/payload.rs'], cwd=REPO)
expected_source = original_source
for removed in [b'    meshopt_stride: Option<usize>,\n', b'                meshopt_stride: None,\n', b'            plan.meshopt_stride = Some(stride);\n', b'            if plans[v].meshopt_stride.is_some_and(|s| s != stride) {\n                return Err(invalid("accessor stride differs from meshopt decoded stride").into());\n            }\n']:
    assert expected_source.count(removed) == 1
    expected_source = expected_source.replace(removed, b'')
assert (REPO / 'src/content_integrity/payload.rs').read_bytes() == expected_source
assert b'meshopt and bufferView stride disagree' in expected_source

lane = BASE / 'stride_controls/2026-10-10'
original = lane / 'compiled_stride_controls.rs'
formatted = lane / 'compiled_stride_controls_formatted.rs'
compiled = lane / 'compiled_stride_controls_clippy.rs'
rustfmt_stdout = subprocess.check_output(['rustfmt', '--edition', '2021', '--emit', 'stdout', str(original)], cwd=REPO)
prefix = (str(original) + ':\n\n').encode()
assert rustfmt_stdout.startswith(prefix)
assert rustfmt_stdout[len(prefix):] == formatted.read_bytes()
expected_compiled = formatted.read_bytes().replace(b'json.len() % 4 != 0', b'!json.len().is_multiple_of(4)').replace(b'bin.len() % 4 != 0', b'!bin.len().is_multiple_of(4)')
assert expected_compiled == compiled.read_bytes()
assert sha(original.read_bytes()) == '64fd2e6c8660c064daa7496c85dc4e71af69f09c42075c5df2637265c593dd10'
assert sha(compiled.read_bytes()) == 'bc7ac12a5a9022e826e9bbd04af01567971185da9b8471969a0c02a161b8fa8f'
assert b'[(23, 24, 48), (24, 23, 48), (24, 24, 47)]' in expected_compiled
with tarfile.open(REPO / 'bench/architecture_audit/c1_payload_integrity/primary_controls/raw-evidence.tar.gz') as archive:
    published_core = archive.extractfile('pinned-Specification.adoc').read()
assert sha(published_core) == '10aebb6a8362a155b196448912bf6250bf046f150431fa73c6cd657c44185aff'
new_core = (lane / 'primary/glTF-core.adoc').read_bytes()
assert new_core == published_core.replace(b'`inverseBindMatrices` **MUST** greater', b'`inverseBindMatrices` **MUST** be greater')

manifests = {}
static_rows = []
integrity_counts = {}
for dirname, label, count in [('stride_controls', 'stride6', 6), ('stride_parent_control', 'parent1', 1)]:
    root = BASE / dirname / '2026-10-10'
    integrity = json.loads((root / 'integrity.json').read_bytes())
    for path, identity in integrity['files'].items():
        raw = (root / path).read_bytes()
        assert len(raw) == identity['bytes'] and sha(raw) == identity['sha256'], path
    integrity_counts[label] = len(integrity['files'])
    manifest_raw = (root / 'fixtures/manifest.json').read_bytes()
    manifest = json.loads(manifest_raw)
    manifests[label] = (root, manifest, manifest_raw)
    assert len(manifest['cases']) == count
    assert manifest['runnerSha256'] == sha((root / 'runner.py').read_bytes())
    for primary in json.loads((root / 'primary/pins.json').read_bytes()):
        raw = (root / 'primary' / primary['file']).read_bytes()
        assert len(raw) == primary['bytes'] and sha(raw) == primary['sha256']
    source = (root / 'primary/meshopt_decoder.test.js').read_text()
    section = source.split('decodeVertexBuffer: function () {', 1)[1].split('decodeVertexBuffer_More:', 1)[0]
    literal = {}
    for name in ['encoded', 'expected']:
        text = re.search(r'var ' + name + r' = new Uint8Array\(\[([\s\S]*?)\]\);', section).group(1)
        literal[name] = bytes(int(token.strip(), 0) for token in text.split(',') if token.strip())
    assert literal['encoded'] == (root / 'fixtures/golden-compressed.bin').read_bytes()
    assert literal['expected'] == (root / 'fixtures/golden-expected.bin').read_bytes()
    assert len(literal['encoded']) == 85 and len(literal['expected']) == 48
    floats = struct.unpack('<12f', literal['expected'])
    assert all(math.isfinite(value) for value in floats)
    for case in manifest['cases']:
        path = root / 'fixtures' / case['path']
        raw = path.read_bytes()
        assert sha(raw) == case['sha256'] and len(raw) == case['bytes']
        with zipfile.ZipFile(io.BytesIO(raw)) as archive:
            assert archive.testzip() is None and len(archive.infolist()) == 3
            records = sorted([(hashlib.md5(member.filename.encode()).digest(), member.header_offset) for member in archive.infolist() if member.filename != '@3dtilesIndex1@'], key=lambda row: struct.unpack('<QQ', row[0]))
            assert archive.read('@3dtilesIndex1@') == b''.join(digest + struct.pack('<Q', offset) for digest, offset in records)
            glb = archive.read('tile.glb')
            for member, digest in case['memberSha256'].items():
                assert sha(archive.read(member)) == digest
        assert struct.unpack_from('<4sII', glb) == (b'glTF', 2, len(glb))
        length, kind = struct.unpack_from('<I4s', glb, 12)
        assert kind == b'JSON' and length % 4 == 0
        text = (path.parent / 'literal-payload.json').read_bytes()
        assert sha(text) == case['literalSha256'] and glb[20:20 + length].rstrip(b' ') == text
        document = json.loads(text)
        position = 20 + length
        bin_length, kind = struct.unpack_from('<I4s', glb, position)
        binary = glb[position + 8:]
        assert kind == b'BIN\0' and bin_length == len(binary) == 88
        assert binary[:85] == literal['encoded'] and binary[85:] == bytes(3)
        view = document['bufferViews'][0]
        codec = view['extensions']['EXT_meshopt_compression']
        assert codec['byteStride'] * codec['count'] == view['byteLength'] == 48
        assert codec['mode'] == 'ATTRIBUTES' and codec['filter'] == 'NONE'
        assert len(document['bufferViews']) == case['physicalViews'] == 1
        components = sum(accessor['count'] * {'VEC3': 3, 'SCALAR': 1}[accessor['type']] for accessor in document['accessors'])
        assert components == case['components']
        window = case['scalarWindow']
        if window is not None:
            scalar = document['accessors'][1]
            stride = view.get('byteStride', 4)
            end = scalar.get('byteOffset', 0) + (scalar['count'] - 1) * stride + 4
            assert window['effectiveStride'] == stride and window['end'] == end
            if end <= 48:
                values = [struct.unpack_from('<f', literal['expected'], scalar.get('byteOffset', 0) + index * stride)[0] for index in range(scalar['count'])]
                assert values == window['rawValues'] and all(math.isfinite(value) for value in values)
            else:
                assert window['rawValues'] is None
        if label == 'parent1':
            assert 'bufferView' not in document['accessors'][0]
            assert view['byteStride'] == 8 and window['end'] == 28
            assert document['meshes'][0]['primitives'][0]['attributes'] == {'POSITION': 0}
            assert components == 16
            assert document['accessors'][0]['min'] == document['accessors'][0]['max'] == [0, 0, 0]
        else:
            positions = [floats[index:index + 3] for index in range(0, 12, 3)]
            assert document['accessors'][0]['min'] == [min(row[index] for row in positions) for index in range(3)]
            assert document['accessors'][0]['max'] == [max(row[index] for row in positions) for index in range(3)]
        report = case['expectedReport'] or case.get('hypotheticalGateRemovedReport')
        if report is not None:
            assert report['ok'] is True and report['entries'] == 3 and report['tiles'] == report['contentReferences'] == 1
            assert report['payloads'] == [dict(uri='tile.glb', accessorsChecked=len(document['accessors']), primitivesChecked=1, vertices=4)]
            assert len(report['checks']) == 13 and len(report['limits']) == 15 and len(report['notInspected']) == 6
        static_rows.append(dict(lane=label, case=case['name'], components=components, scalarWindowEnd=window['end'] if window else None, physicalDecodedBytes=48))

receipt_path = pathlib.Path(sys.argv[1])
receipt = json.loads(receipt_path.read_bytes())
assert receipt['coordinatorRunnerSha256'] == sha((REPO / 'tests/payload_meshopt_stride_oracle.py').read_bytes())
reports = 0
for label, count in [('stride6', 6), ('parent1', 1)]:
    root, manifest, manifest_raw = manifests[label]
    phase = receipt['phases'][label]
    assert phase['runnerExitCode'] == 0 and phase['passes'] == count and phase['failures'] == 0
    assert phase['fixturesAndReferencesUnchanged'] and phase['artifactIdentitiesUnchanged']
    assert phase['sourcePin']['source_commit'] == PIN['source_commit'] and phase['sourcePin']['source_tree'] == PIN['source_tree']
    assert phase['sourcePin']['production_sha256'] == PIN['production_sha256']
    assert phase['manifestSha256'] == sha(manifest_raw) and phase['runnerSha256'] == manifest['runnerSha256']
    assert sha(pathlib.Path(phase['binary']).read_bytes()) == phase['binarySha256']
    for path, digest in phase['fixturesBefore'].items():
        assert sha((root / 'fixtures' / path).read_bytes()) == digest
    declared = {case['name']: case for case in manifest['cases']}
    assert len(phase['cases']) == count and {case['name'] for case in phase['cases']} == declared.keys()
    for row in phase['cases']:
        case = declared[row['name']]
        stdout = base64.b64decode(row['stdoutBase64'], validate=True)
        stderr = base64.b64decode(row['stderrBase64'], validate=True)
        assert sha(stdout) == row['stdoutSha256'] and sha(stderr) == row['stderrSha256']
        response = json.loads(stdout)
        assert response == row['response'] and row['passed'] is True
        assert row['observedCategory'] == row['expectedCategory'] == case['expectedCategory']
        assert row['exitCode'] == case['expectedExitCode'] and row['archiveSha256'] == case['sha256']
        expected = copy.deepcopy(case['expectedReport'])
        if expected is not None:
            expected['archive'] = row['command'][-1]
            assert response == expected == row['expectedReport']
            reports += 1
        else:
            assert response['error']['code'] == case['expectedCategory'] and response['exitCode'] == case['expectedExitCode']
            if 'expectedErrorReport' in case:
                assert response == case['expectedErrorReport'] and stderr == b''

print(json.dumps(dict(sourceCommit=PIN['source_commit'], receiptSha256=sha(receipt_path.read_bytes()), frozenOriginalIntegrityFiles=integrity_counts, exactSixLineProductionRemoval=True, originalToFormattedRustfmtEquivalent=True, twoPaddingPredicatesEquivalent=True, independentLiteralPrimaryGoldensVerified=True, consumedPrimaryRulesIdenticalToPublishedRevision=True, onlyPrimaryRevisionDelta='Three bytes be in unrelated inverseBindMatrices sentence', cases=static_rows, actualPublicCases=7, positiveCompleteReports=reports, rawStreamsHashChecked=14, unmaskedParentCompleteTypedErrorVerified=True, limitations='Embedded/placeholder private controls establish actual24/23 components and48/47 decoded boundaries; zero resolver calls do not independently prove external-resource preflight.'), indent=2))
