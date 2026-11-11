#!/usr/bin/env python3
"""Dated independent polygon-oracle correction; preserves original failed lane.

Retains203 original byte-identical controls and adds a primary-valid indexed
triangle plus sensitive paired negatives. Does not build or import production.
"""
import argparse
import copy
import hashlib
import io
import json
from pathlib import Path
import struct
import tarfile
import zipfile

HERE = Path(__file__).resolve().parent
ORIGINAL = HERE.parents[1]
REPO = HERE.parents[5]


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def packed(value):
    return json.dumps(value, separators=(',', ':'), allow_nan=False).encode()


def glb(raw, binary):
    raw += b' ' * (-len(raw) % 4)
    binary += b'\0' * (-len(binary) % 4)
    chunks = struct.pack('<I4s', len(raw), b'JSON') + raw + struct.pack('<I4s', len(binary), b'BIN\0') + binary
    return struct.pack('<4sII', b'glTF', 2, len(chunks) + 12) + chunks


def archive(members):
    stream = io.BytesIO()
    records = []
    with zipfile.ZipFile(stream, 'w', compression=zipfile.ZIP_STORED) as target:
        for name, raw in members:
            records.append((hashlib.md5(name.encode()).digest(), stream.tell()))
            target.writestr(zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0)), raw)
        records.sort(key=lambda row: struct.unpack('<QQ', row[0]))
        target.writestr(zipfile.ZipInfo('@3dtilesIndex1@', (1980, 1, 1, 0, 0, 0)),
                        b''.join(digest + struct.pack('<Q', offset) for digest, offset in records))
    raw = stream.getvalue()
    with zipfile.ZipFile(io.BytesIO(raw)) as check:
        assert check.testzip() is None
        for name, value in members:
            assert check.read(name) == value
        for _, offset in records:
            assert raw[offset:offset + 4] == b'PK\x03\x04'
    return raw


def original_manifest_and_files():
    pin = json.loads((ORIGINAL / 'fixture-bundle.json').read_text())
    raw = (ORIGINAL / 'fixtures.tar.gz').read_bytes()
    assert sha(raw) == pin['bundleSha256']
    with tarfile.open(fileobj=io.BytesIO(raw), mode='r:gz') as bundle:
        files = {member.name: bundle.extractfile(member).read()
                 for member in bundle.getmembers() if member.isfile()}
    assert sha(files['manifest.json']) == pin['manifestSha256']
    return json.loads(files['manifest.json']), files


def references():
    manifest, _ = original_manifest_and_files()
    pin = json.loads((HERE / 'references/retrieval-receipt.json').read_text())
    raw = (HERE / 'references/EXT_mesh_polygon.md').read_bytes()
    assert sha(raw) == pin['sha256'] and len(raw) == pin['bytes']
    return manifest['referencePins'] + [pin]


def generate(directory):
    directory.mkdir(parents=True, exist_ok=False)
    original, files = original_manifest_and_files()
    omitted = 'polygon-optional-reference-profile'
    replaced = {omitted, 'polygon-reference-oob'}
    cases = [copy.deepcopy(case) for case in original['cases'] if case['name'] not in replaced]
    assert len(cases) == 203
    for case in cases:
        prefix = case['name'] + '/'
        for name, raw in files.items():
            if name.startswith(prefix):
                target = directory / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(raw)
        assert sha((directory / case['path']).read_bytes()) == case['sha256']

    positions = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
    data = struct.pack('<9f', *(value for point in positions for value in point))
    data += struct.pack('<3H', 0, 1, 2) + b'\0' * 2
    data += struct.pack('<I', 0) + struct.pack('<3H', 0, 1, 2) + b'\0' * 2 + struct.pack('<I', 0)
    assert len(data) == 60
    assert [struct.unpack_from('<3f', data, offset) for offset in [0, 12, 24]] == positions
    assert struct.unpack_from('<3H', data, 36) == (0, 1, 2)
    assert struct.unpack_from('<I', data, 44) == (0,)
    assert struct.unpack_from('<3H', data, 48) == (0, 1, 2)
    assert struct.unpack_from('<I', data, 56) == (0,)
    area2 = sum(positions[i][0] * positions[(i + 1) % 3][1] - positions[(i + 1) % 3][0] * positions[i][1] for i in range(3))
    assert area2 == 1 and len(set((0, 1, 2))) == 3
    doc = dict(asset=dict(version='2.0'), buffers=[dict(byteLength=60)],
               bufferViews=[dict(buffer=0, byteOffset=offset, byteLength=length, target=target)
                            for offset, length, target in [(0, 36, 34962), (36, 6, 34963),
                                                           (44, 4, 34963), (48, 6, 34963), (56, 4, 34963)]],
               accessors=[dict(bufferView=0, componentType=5126, count=3, type='VEC3', min=[0, 0, 0], max=[1, 1, 0]),
                          dict(bufferView=1, componentType=5123, count=3, type='SCALAR'),
                          dict(bufferView=2, componentType=5125, count=1, type='SCALAR'),
                          dict(bufferView=3, componentType=5123, count=3, type='SCALAR'),
                          dict(bufferView=4, componentType=5125, count=1, type='SCALAR')],
               meshes=[dict(primitives=[dict(attributes=dict(POSITION=0), indices=1, mode=4,
                      extensions=dict(EXT_mesh_polygon=dict(count=1, indicesOffsets=2, loopIndices=3, loopIndicesOffsets=4)))])],
               extensionsUsed=['EXT_mesh_polygon'])
    source = packed(doc)
    old_case = next(case for case in original['cases'] if case['name'] == omitted)
    root_raw = files[omitted + '/tileset.json']
    expected = copy.deepcopy(old_case['expectedReport'])
    expected['payloads'] = [dict(uri='tile.glb', accessorsChecked=5, primitivesChecked=1, vertices=3)]

    def emit(name, text, category, rationale, payload=None):
        case = directory / name
        case.mkdir()
        content = glb(text, data) if payload is None else payload
        members = [('tileset.json', root_raw), ('tile.glb', content)]
        for member, raw in members:
            (case / member).write_bytes(raw)
        (case / 'literal-payload.json').write_bytes(text)
        raw = archive(members)
        (case / 'input.3tz').write_bytes(raw)
        cases.append(dict(name=name, path=name + '/input.3tz', expectedCategory=category,
                          expectedExitCode=0 if category == 'admitted' else 3, rationale=rationale,
                          expectedReport=expected if category == 'admitted' else None,
                          independentPayloadComponents=17 if category == 'admitted' else None,
                          scalarOracle=dict(positionValues=positions, rawMin=[0, 0, 0], rawMax=[1, 1, 0],
                                            triangleIndices=[0, 1, 2], polygonIndexOffsets=[0],
                                            loopIndices=[0, 1, 2], polygonLoopOffsets=[0], signedArea2=area2,
                                            components=9 + 3 + 1 + 3 + 1),
                          sha256=sha(raw), bytes=len(raw), literalSha256=sha(text),
                          memberSha256={n: sha(v) for n, v in members}))

    emit('polygon-optional-reference-profile', source, 'admitted',
         'Pinned EXT_mesh_polygon mode4 plus explicit TRIANGLES indices; one nondegenerate CCW exterior ring0,1,2 and offsets0/count1.')
    broken_reference = copy.deepcopy(doc)
    broken_reference['meshes'][0]['primitives'][0]['extensions']['EXT_mesh_polygon']['indicesOffsets'] = 999
    emit('polygon-reference-oob', packed(broken_reference), 'invalid_input',
         'Sensitive corrected paired fixture: indexed TRIANGLES and valid loops; isolated indicesOffsets999 exceeds five actual accessors.')
    assert len(cases) == 205
    supplemental_start = len(cases)
    # Retain the exact old archive as a newly classified primary-invalid control;
    # original positive expectation/failure stays immutable in its own lane.
    invalid_name = 'polygon-original-invalid-positive'
    case = directory / invalid_name
    case.mkdir()
    for member in ['input.3tz', 'tileset.json', 'tile.glb', 'literal-payload.json']:
        (case / member).write_bytes(files[omitted + '/' + member])
    declared = copy.deepcopy(old_case)
    declared.update(name=invalid_name, path=invalid_name + '/input.3tz', expectedCategory='invalid_input',
                    expectedExitCode=3, expectedReport=None, independentPayloadComponents=None,
                    rationale='Exact original malformed POINTS/no-indices/repeated-loop fixture, adjudicated against pinned draft; original failed execution remains204/1.')
    cases.append(declared)
    for suffix, field, value in [('mode-points', 'mode', 0), ('missing-indices', 'indices', None),
                                  ('offset-reference-wrong-scalar', 'indicesOffsets', 0)]:
        broken = copy.deepcopy(doc)
        primitive = broken['meshes'][0]['primitives'][0]
        if field == 'indices' and value is None:
            primitive.pop(field)
        elif field in ['mode', 'indices']:
            primitive[field] = value
        else:
            primitive['extensions']['EXT_mesh_polygon'][field] = value
        emit('polygon-sensitive-' + suffix, packed(broken), 'invalid_input',
             'One-property sensitive pair from independently valid polygon; actual finite owner reference/profile gate.')
    decimal = source.replace(b'"count":1,"indicesOffsets":2', b'"count":1.0,"indicesOffsets":2e0')
    assert decimal != source
    emit('polygon-integral-notation', decimal, 'admitted', 'Actual polygon count/reference properties admit exact mathematical integer decimal/exponent notation.')
    manifest = dict(schemaVersion=1, scope=original['scope'],
                    status='New independently corrected expectations/inputs only; no new target execution claimed.',
                    cases=cases[:supplemental_start], referencePins=original['referencePins'], driverSha256=original['driverSha256'],
                    correctionPrimaryPins=references()[len(original['referencePins']):], generatorSha256=sha(Path(__file__).read_bytes()),
                    originalManifestSha256=sha(files['manifest.json']),
                    originalLaneResult='205 executed;204 pass/1 malformed positive-oracle failure. Preserved without retroactive green.',
                    unchangedOriginalArchives=203,
                    changedExistingCases=['polygon-optional-reference-profile', 'polygon-reference-oob'])
    (directory / 'manifest.json').write_text(json.dumps(manifest, indent=2, sort_keys=True) + '\n')
    supplemental = copy.deepcopy(manifest)
    supplemental.update(cases=cases[supplemental_start:], unchangedOriginalArchives=0,
                        scope='Five additional primary-justified polygon sensitivity controls; no full topology/conformance claim.')
    (directory / 'supplemental-manifest.json').write_text(json.dumps(supplemental, indent=2, sort_keys=True) + '\n')
    print(json.dumps(dict(correctedCases=supplemental_start, unchangedOriginalArchives=203,
                          supplementalCases=len(cases) - supplemental_start)))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--generate', type=Path, required=True)
    generate(parser.parse_args().generate)
    return 0


if __name__ == '__main__':
    main()
