"""Separate static polygon fixture/oracle review; no target imports/execution."""
import copy
import hashlib
import io
import json
import pathlib
import struct
import tarfile
import zipfile


REPO = pathlib.Path(__file__).resolve().parents[5]
BASE = REPO / 'bench/architecture_audit/c1_payload_integrity/production_controls'
CORRECTION = BASE / 'polygon_correction/2026-10-10'


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def bundle(path):
    with tarfile.open(path) as archive:
        return {member.name: archive.extractfile(member).read() for member in archive.getmembers() if member.isfile()}


def member_archive(raw):
    with zipfile.ZipFile(io.BytesIO(raw)) as archive:
        assert archive.testzip() is None
        rows = [(hashlib.md5(info.filename.encode()).digest(), info.header_offset) for info in archive.infolist() if info.filename != '@3dtilesIndex1@']
        rows.sort(key=lambda row: struct.unpack('<QQ', row[0]))
        assert archive.read('@3dtilesIndex1@') == b''.join(digest + struct.pack('<Q', offset) for digest, offset in rows)
        return {info.filename: archive.read(info) for info in archive.infolist() if info.filename != '@3dtilesIndex1@'}


def extract_glb(raw):
    assert raw[:4] == b'glTF' and struct.unpack_from('<II', raw, 4) == (2, len(raw))
    length = struct.unpack_from('<I', raw, 12)[0]
    assert raw[16:20] == b'JSON'
    text = raw[20:20 + length]
    size = struct.unpack_from('<I', raw, 20 + length)[0]
    assert raw[24 + length:28 + length] == b'BIN\0'
    binary = raw[28 + length:28 + length + size]
    return text, json.loads(text), binary


original = bundle(BASE / 'fixtures.tar.gz')
corrected = bundle(CORRECTION / 'corrected-205-fixtures.tar.gz')
supplemental = bundle(CORRECTION / 'polygon-sensitive-5-fixtures.tar.gz')
old_manifest = json.loads(original['manifest.json'])
new_manifest = json.loads(corrected['manifest.json'])
extra_manifest = json.loads(supplemental['manifest.json'])
old = {case['name']: case for case in old_manifest['cases']}
new = {case['name']: case for case in new_manifest['cases']}
assert len(old) == len(new) == 205 and old.keys() == new.keys()
assert len(extra_manifest['cases']) == 5
changed = {'polygon-optional-reference-profile', 'polygon-reference-oob'}
unchanged_files = 0
for name in old.keys() - changed:
    assert old[name] == new[name], name
    old_files = {path: raw for path, raw in original.items() if path.startswith(name + '/')}
    new_files = {path: raw for path, raw in corrected.items() if path.startswith(name + '/')}
    assert old_files == new_files, name
    unchanged_files += len(old_files)
for manifest in [new_manifest, extra_manifest]:
    assert manifest['referencePins'] == old_manifest['referencePins']
    assert manifest['driverSha256'] == old_manifest['driverSha256'] == sha((BASE / 'driver.py').read_bytes())
    assert manifest['originalManifestSha256'] == sha(original['manifest.json'])
    assert manifest['generatorSha256'] == sha((CORRECTION / 'generate.py').read_bytes())
    primary = manifest['correctionPrimaryPins'][0]
    assert primary['sha256'] == sha((CORRECTION / 'references/EXT_mesh_polygon.md').read_bytes())

decoded = {}
for files, manifest in [(corrected, new_manifest), (supplemental, extra_manifest)]:
    for case in manifest['cases']:
        if files is corrected and case['name'] not in changed:
            continue
        supplied = files[case['path']]
        assert sha(supplied) == case['sha256'] and len(supplied) == case['bytes']
        members = member_archive(supplied)
        for name, raw in members.items():
            assert files[case['name'] + '/' + name] == raw
            assert case['memberSha256'][name] == sha(raw)
        text, doc, binary = extract_glb(members['tile.glb'])
        literal = files[case['name'] + '/literal-payload.json']
        assert json.loads(literal) == doc and sha(literal) == case['literalSha256']
        decoded[case['name']] = (case, literal, doc, binary)

positive, _, doc, binary = decoded['polygon-optional-reference-profile']
primitive = doc['meshes'][0]['primitives'][0]
assert primitive['mode'] == 4 and primitive['indices'] == 1
assert primitive['extensions']['EXT_mesh_polygon'] == dict(count=1, indicesOffsets=2, loopIndices=3, loopIndicesOffsets=4)
values = []
for accessor in doc['accessors']:
    view = doc['bufferViews'][accessor['bufferView']]
    code = {5126: 'f', 5123: 'H', 5125: 'I'}[accessor['componentType']]
    width = 3 if accessor['type'] == 'VEC3' else 1
    offset = view.get('byteOffset', 0) + accessor.get('byteOffset', 0)
    values.append(struct.unpack_from('<' + code * (accessor['count'] * width), binary, offset))
assert values == [(0., 0., 0., 1., 0., 0., 0., 1., 0.), (0, 1, 2), (0,), (0, 1, 2), (0,)]
positions = list(zip(values[0][0::3], values[0][1::3], values[0][2::3]))
area = sum(positions[i][0] * positions[(i + 1) % 3][1] - positions[(i + 1) % 3][0] * positions[i][1] for i in range(3))
assert area == 1 and len(set(values[3])) == 3 and set(values[3]) == set(values[1])
assert doc['accessors'][0]['min'] == [0, 0, 0] and doc['accessors'][0]['max'] == [1, 1, 0]
components = sum(accessor['count'] * (3 if accessor['type'] == 'VEC3' else 1) for accessor in doc['accessors'])
assert components == positive['independentPayloadComponents'] == 17
expected = copy.deepcopy(old['polygon-optional-reference-profile']['expectedReport'])
expected['payloads'] = [dict(uri='tile.glb', accessorsChecked=5, primitivesChecked=1, vertices=3)]
assert positive['expectedReport'] == expected

negative, _, negative_doc, negative_binary = decoded['polygon-reference-oob']
paired = copy.deepcopy(doc)
paired['meshes'][0]['primitives'][0]['extensions']['EXT_mesh_polygon']['indicesOffsets'] = 999
assert negative_doc == paired and negative_binary == binary
assert negative['expectedCategory'] == 'invalid_input' and negative['expectedExitCode'] == 3 and negative['expectedReport'] is None
original_negative = decoded['polygon-original-invalid-positive'][0]
assert supplemental[original_negative['path']] == original[old['polygon-optional-reference-profile']['path']]
for name, field, value in [('polygon-sensitive-mode-points', 'mode', 0), ('polygon-sensitive-missing-indices', 'indices', None), ('polygon-sensitive-offset-reference-wrong-scalar', 'indicesOffsets', 0)]:
    case, _, supplied_doc, supplied_binary = decoded[name]
    paired = copy.deepcopy(doc)
    primitive = paired['meshes'][0]['primitives'][0]
    if field == 'indices':
        primitive.pop(field)
    elif field == 'mode':
        primitive[field] = value
    else:
        primitive['extensions']['EXT_mesh_polygon'][field] = value
    assert supplied_doc == paired and supplied_binary == binary
    assert case['expectedCategory'] == 'invalid_input' and case['expectedExitCode'] == 3 and case['expectedReport'] is None
integral, integral_text, integral_doc, integral_binary = decoded['polygon-integral-notation']
assert integral_doc == doc and integral_binary == binary
assert b'"count":1.0,"indicesOffsets":2e0' in integral_text
assert integral['expectedReport'] == expected and integral['expectedCategory'] == 'admitted'
print(json.dumps(dict(unchangedOriginalCases=203, unchangedOriginalFiles=unchanged_files, correctedCases=2, supplementalCases=5, exactOriginalMalformedArchiveRetained=True, separateLittleEndianScalarsVerified=True, uniqueCCWLoopVerified=True, fullPositiveReportVerified=True, isolatedReferenceNegativeVerified=True, supplementalOnePropertySensitivityVerified=True, noFailureOverride=True, scope='Static independent supplied bytes/expectations only; no target execution'), indent=2))
