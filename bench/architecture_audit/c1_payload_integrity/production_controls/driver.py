#!/usr/bin/env python3
"""Independent finite payload controls. Standard library only; never builds a target.

Generate with --generate DIR; execute only a supplied hash-pinned final CLI using
--binary BIN --binary-sha256 HASH --source-pin FILE --source-pin-sha256 HASH
--run-dir NEW_DIR. Expectations are declared before target execution.
"""
import argparse
import base64
import copy
import hashlib
import io
import json
import math
import os
from pathlib import Path
import re
import struct
import subprocess
import sys
import tarfile
import time
import zipfile

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
AUDIT = HERE.parent
PRIMARY = AUDIT / 'primary_controls'
REFS = PRIMARY / '2026-10-10-pinned-reference-verification'
CHECKS = ['archiveIndex', 'archiveCrc', 'archiveStoredRecordLayout', 'contentHashes',
          'tilesetSchema', 'payloadEnvelope', 'bufferRanges', 'accessorValues',
          'primitiveIndices', 'hierarchyBounds', 'geometricErrorOrder',
          'resourceReferences', 'recordedBudgets']
EXCLUSIONS = ['decodedContentBounds', 'geometricErrorAccuracy', 'materialAndImageSemantics',
              'metadata and feature semantics', 'metadataSemantics',
              'scene transforms, skins, animations, morph targets and rendered bounds']
LIMITS = dict(jsonBytes=8388608, jsonDepth=64, memberBytes=67108864,
              archiveStoredBytes=1073741824, archiveEntries=65536, accessorElements=4000000,
              documentDecodedBytes=67108864, hierarchyVisits=65536, hierarchyDepth=128,
              references=262144, totalPayloadElements=16000000, totalBytesRead=2147483648,
              sourceArchiveBytes=1073741824, centralDirectoryBytes=16777216, documentItems=65536)
CODES = {'admitted': 0, 'invalid_input': 3, 'unsupported': 2, 'resource_limit': 3, 'io': 1}
BASE = '{"asset":{"version":"2.0"},"buffers":[{"byteLength":12}],"bufferViews":[{"buffer":0,"byteLength":12}],"accessors":[{"bufferView":0,"componentType":5126,"count":1,"type":"VEC3","min":[0,0,0],"max":[0,0,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":0}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0}'
VIEWLESS = '{"asset":{"version":"2.0"},"accessors":[{"componentType":5126,"count":1,"type":"VEC3","min":[0,0,0],"max":[0,0,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":0}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0}'


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def packed(doc):
    return json.dumps(doc, separators=(',', ':'), ensure_ascii=True, allow_nan=False).encode()


def glb(text, binary=None, extra=b''):
    raw = text.encode() if isinstance(text, str) else text
    raw += b' ' * (-len(raw) % 4)
    chunks = struct.pack('<I4s', len(raw), b'JSON') + raw
    if binary is not None:
        binary += b'\0' * (-len(binary) % 4)
        chunks += struct.pack('<I4s', len(binary), b'BIN\0') + binary
    chunks += extra
    return struct.pack('<4sII', b'glTF', 2, 12 + len(chunks)) + chunks


def b3dm(payload, feature=b'{"BATCH_LENGTH":0}', batch=b''):
    feature += b' ' * (-(28 + len(feature)) % 8)
    if batch:
        batch += b' ' * (-len(batch) % 8)
    body = feature + batch + payload
    body += b'\0' * (-(28 + len(body)) % 8)
    return struct.pack('<4s6I', b'b3dm', 1, 28 + len(body), len(feature), 0, len(batch), 0) + body


def archive(members):
    stream = io.BytesIO()
    with zipfile.ZipFile(stream, 'w', compression=zipfile.ZIP_STORED) as target:
        records = []
        for name, raw in members:
            records.append((hashlib.md5(name.encode()).digest(), stream.tell()))
            target.writestr(zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0)), raw)
        records.sort(key=lambda row: struct.unpack('<QQ', row[0]))
        target.writestr(zipfile.ZipInfo('@3dtilesIndex1@', (1980, 1, 1, 0, 0, 0)),
                        b''.join(digest + struct.pack('<Q', offset) for digest, offset in records))
    raw = stream.getvalue()
    with zipfile.ZipFile(io.BytesIO(raw)) as check:
        assert check.testzip() is None
        for name, expected in members:
            assert check.read(name) == expected
        for _, offset in records:
            assert raw[offset:offset + 4] == b'PK\x03\x04'
    return raw


def tile(uri):
    return dict(boundingVolume=dict(box=[0, 0, 0, 10, 0, 0, 0, 10, 0, 0, 0, 10]),
                geometricError=0, content=dict(uri=uri))


def tileset(uri='tile.glb'):
    return dict(asset=dict(version='1.1'), geometricError=0, root=tile(uri))


def scalar_oracle(raw, component_type, shape, count, offset=0, stride=None, normalized=False):
    """Little-endian reference, including column alignment and signed clamp.

    Never imports project helpers. Returns raw extrema separately from normalized
    values: normalized accessor bounds are raw integers under published glTF.
    """
    code, size = {5120: ('b', 1), 5121: ('B', 1), 5122: ('h', 2),
                  5123: ('H', 2), 5125: ('I', 4), 5126: ('f', 4)}[component_type]
    rows, columns = {'SCALAR': (1, 1), 'VEC2': (2, 1), 'VEC3': (3, 1), 'VEC4': (4, 1),
                     'MAT2': (2, 2), 'MAT3': (3, 3), 'MAT4': (4, 4)}[shape]
    column = ((rows * size + 3) // 4) * 4 if columns > 1 else rows * size
    effective = column * columns if stride is None else stride
    values = [[struct.unpack_from('<' + code, raw, offset + i * effective + col * column + row * size)[0]
               for col in range(columns) for row in range(rows)] for i in range(count)]
    assert all(math.isfinite(v) for record in values for v in record)
    converted = copy.deepcopy(values)
    if normalized:
        divisor = {5120: 127, 5121: 255, 5122: 32767, 5123: 65535}[component_type]
        converted = [[max(v / divisor, -1) if component_type in (5120, 5122) else v / divisor
                      for v in record] for record in values]
    return dict(method='Independent struct.unpack_from little-endian component/column offsets',
                rawValues=values, normalizedValues=converted,
                rawMin=[min(v[k] for v in values) for k in range(rows * columns)],
                rawMax=[max(v[k] for v in values) for k in range(rows * columns)],
                occupiedBytes=(columns - 1) * column + rows * size,
                scalarComponents=count * rows * columns)


def numeric_oracle(token):
    """Independent exact mathematical range oracle, no binary floats/big exponent expansion."""
    match = re.fullmatch(r'(-?)(0|[1-9][0-9]*)(?:\.([0-9]+))?(?:[eE]([+-]?[0-9]+))?', token)
    if not match:
        return 'invalid_input'
    sign, whole, fractional, exponent = match.groups()
    digits = (whole + (fractional or '')).lstrip('0')
    if not digits:
        return 0
    if sign:
        return 'invalid_input'
    # Tokens in this finite independent lane have <=400 exponent digits. Avoid
    # expansion; Python arbitrary integer here is a mathematical oracle only.
    power = int(exponent or '0') - len(fractional or '')
    if power < 0:
        cancelled = len(digits) - len(digits.rstrip('0'))
        if -power > cancelled:
            return 'invalid_input'
        digits = digits[:power]
        power = 0
    if len(digits) + power > 20:
        return 'overflow'
    value = int(digits) * 10 ** power
    return value if value <= 18446744073709551615 else 'overflow'


def reference_pins():
    receipt = json.loads((REFS / 'fetch-receipt.json').read_text())
    pins = []
    for ref in receipt['references']:
        raw = (REFS / ref['file']).read_bytes()
        assert len(raw) == ref['bytes'] and sha(raw) == ref['sha256'], ref['file']
        pins.append(dict(path=str((REFS / ref['file']).relative_to(REPO)),
                         bytes=len(raw), sha256=sha(raw), url=ref['url'], revision=ref['revision']))
    adjudication = json.loads((PRIMARY / 'adjudication.json').read_text())
    # Core/raw accessor source is retained in the immutable primary bundle.
    with tarfile.open(PRIMARY / 'raw-evidence.tar.gz', 'r:gz') as bundle:
        for name, expected in [('primary-source.adoc', adjudication['selectedPrimary']['sourceSha256']),
                               ('accessor.schema.json', adjudication['selectedPrimary']['accessorSchema']['sha256'])]:
            matches = [member for member in bundle.getmembers() if member.isfile() and Path(member.name).name.endswith(name)]
            if not matches and name == 'primary-source.adoc':
                matches = [member for member in bundle.getmembers() if member.isfile() and member.name.endswith('.adoc')]
            assert len(matches) == 1, (name, [m.name for m in matches])
            raw = bundle.extractfile(matches[0]).read()
            assert sha(raw) == expected, name
            pins.append(dict(bundle=str((PRIMARY / 'raw-evidence.tar.gz').relative_to(REPO)),
                             member=matches[0].name, bytes=len(raw), sha256=sha(raw),
                             revision='8e798b02d254cea97659a333cfcb20875b62bdd4'))
    source = (REFS / 'meshopt_decoder.test.js').read_text()
    section = source.split('decodeVertexBuffer: function () {', 1)[1].split('decodeVertexBuffer_More:', 1)[0]
    for name, path, length in [('encoded', 'golden-compressed.bin', 85), ('expected', 'golden-expected.bin', 48)]:
        literal = re.search(r'var ' + name + r' = new Uint8Array\(\[([\s\S]*?)\]\);', section).group(1)
        raw = bytes(int(token.strip(), 0) for token in literal.split(',') if token.strip())
        assert len(raw) == length and raw == (REFS / path).read_bytes()
        pins.append(dict(path=str((REFS / path).relative_to(REPO)), bytes=len(raw), sha256=sha(raw),
                         literalSource='meshopt_decoder.test.js decodeVertexBuffer ' + name))
    for path in [PRIMARY / 'receipt.json', PRIMARY / 'probe.py', PRIMARY / 'adjudication.json',
                 REPO / 'docs/architecture/c1-payload-integrity-contract.md',
                 AUDIT / 'prereq_review/2026-10-10-whole-slice-implementation-ready.md']:
        raw = path.read_bytes()
        pins.append(dict(path=str(path.relative_to(REPO)), bytes=len(raw), sha256=sha(raw)))
    return pins


def generate(directory):
    directory.mkdir(parents=True, exist_ok=False)
    cases = []

    def emit(name, text=BASE, binary=b'\0' * 12, kind='admitted', fmt='glb',
             fact='', extras=None, accessors=1, primitives=1, vertices=1, components=3,
             exclusions=(), manifest=None, payload=None, scalar=None, expected_payloads=None):
        case = directory / name
        case.mkdir()
        uri = 'tile.' + fmt
        root = tileset(uri) if manifest is None else manifest
        data = (glb(text, binary) if fmt == 'glb' else text.encode() if isinstance(text, str) else text)
        if fmt == 'b3dm':
            data = b3dm(glb(text, binary))
        if payload is not None:
            data = payload
        members = [('tileset.json', packed(root)), (uri, data)]
        members += list((extras or {}).items())
        for member, raw in members:
            target = case / member
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(raw)
        literal = text.encode() if isinstance(text, str) else text
        (case / 'literal-payload.json').write_bytes(literal)
        raw = archive(members)
        (case / 'input.3tz').write_bytes(raw)
        if expected_payloads is None:
            expected_payloads = [dict(uri=uri, accessorsChecked=accessors,
                                      primitivesChecked=primitives, vertices=vertices)]
        report = dict(ok=True, archive='$INPUT', tiles=1 + len(root['root'].get('children', [])),
                      contentReferences=1 + len(root['root'].get('children', [])), entries=len(members) + 1,
                      checks=CHECKS, notInspected=sorted(set(EXCLUSIONS) | set(exclusions)),
                      limits=LIMITS, payloads=expected_payloads)
        cases.append(dict(name=name, path=name + '/input.3tz', expectedCategory=kind,
                          expectedExitCode=CODES[kind], rationale=fact,
                          expectedReport=report if kind == 'admitted' else None,
                          independentPayloadComponents=components if kind == 'admitted' else None,
                          scalarOracle=scalar, sha256=sha(raw), bytes=len(raw),
                          literalSha256=sha(literal), memberSha256={n: sha(v) for n, v in members}))

    # These reproduce all original fourteen semantic controls. Literal payload
    # hashes are compared below to immutable original receipts; new tile name
    # and archive identity are intentionally distinct from historical evidence.
    old = [
        ('stored-zero-control', BASE, 'glb', 'admitted'),
        ('count-decimal', BASE.replace('"count":1', '"count":1.0'), 'glb', 'admitted'),
        ('component-exponent', BASE.replace('"componentType":5126', '"componentType":5.126e3'), 'glb', 'admitted'),
        ('position-reference-exponent', BASE.replace('"POSITION":0', '"POSITION":0e0'), 'glb', 'admitted'),
        ('buffer-length-decimal', BASE.replace('"byteLength":12}', '"byteLength":12.0}', 1), 'glb', 'admitted'),
        ('local-gltf-integer-representations', BASE.replace('"byteLength":12', '"byteLength":12e0').replace('"buffer":0', '"buffer":0.0').replace('"bufferView":0', '"bufferView":0e0').replace('"componentType":5126', '"componentType":5126.0').replace('"count":1', '"count":1e0').replace('"POSITION":0', '"POSITION":0.0').replace('"mesh":0', '"mesh":0e0').replace('"scene":0', '"scene":0.0').replace('"nodes":[0]', '"nodes":[0.0]').replace('"mode":0', '"mode":0e0').replace('"byteLength":12e0}', '"byteLength":12e0,"uri":"zero.bin"}', 1), 'gltf', 'admitted'),
        ('count-fractional', BASE.replace('"count":1', '"count":1.5'), 'glb', 'invalid_input'),
        ('position-reference-fractional', BASE.replace('"POSITION":0', '"POSITION":0.5'), 'glb', 'invalid_input'),
        ('component-fractional', BASE.replace('"componentType":5126', '"componentType":5126.5'), 'glb', 'invalid_input'),
        ('count-fraction-rounds-to-integer', BASE.replace('"count":1', '"count":1.0000000000000001'), 'glb', 'invalid_input'),
        ('count-fraction-exponent-rounds-to-integer', BASE.replace('"count":1', '"count":10000000000000001e-16'), 'glb', 'invalid_input'),
        ('viewless-zero-control', VIEWLESS, 'glb', 'admitted'),
        ('viewless-arbitrary-bounds', VIEWLESS.replace('"min":[0,0,0],"max":[0,0,0]', '"min":[17,-4,8],"max":[17,-4,8]'), 'glb', 'admitted'),
        ('stored-zero-arbitrary-bounds', BASE.replace('"min":[0,0,0],"max":[0,0,0]', '"min":[17,-4,8],"max":[17,-4,8]'), 'glb', 'invalid_input')]
    original = {row['label']: row for row in json.loads((PRIMARY / 'receipt.json').read_text())['receipts']}
    for name, text, fmt, kind in old:
        assert sha(text.encode()) == original[name]['literalSourceSha256'], name
        emit('primary-' + name, text, None if name.startswith('viewless') else b'\0' * 12,
             kind, fmt, fact='Selected glTF 2.0.1 JSON integer / viewless exemption primary rerun.',
             extras={'zero.bin': b'\0' * 12} if fmt == 'gltf' else None,
             scalar=scalar_oracle(b'\0' * 12, 5126, 'VEC3', 1))

    for token in ['1e0', '10e-1', '1000.000e-3', '1.0000000000000000',
                  '1.0000000000000001', '10000000000000001e-16', '-1', '0.1',
                  '18446744073709551615', '18446744073709551616', '1e400', '1e-400']:
        value = numeric_oracle(token)
        kind = 'admitted' if value == 1 else 'invalid_input' if value == 'invalid_input' else 'resource_limit'
        emit('exact-count-' + token.replace('-', 'm').replace('.', '_'),
             VIEWLESS.replace('"count":1', '"count":' + token), None, kind,
             fact='Independent exact count oracle: ' + str(value) + '; native/domain work range precedes resolution.')
    for token in ['-0', '-0.0', '0e999999999999999999999999999999', '-0e-999999999999999999999999999999']:
        assert numeric_oracle(token) == 0
        emit('reference-zero-' + token.replace('-', 'm').replace('.', '_'),
             BASE.replace('"POSITION":0', '"POSITION":' + token), fact='Mathematical zero before exponent magnitude/sign.')
    for field, old_token in [('POSITION', '0'), ('componentType', '5126'), ('mode', '0'), ('bufferView', '0'), ('buffer', '0')]:
        for token in ['18446744073709551615', '18446744073709551616', '1e400']:
            emit('domain-' + field + '-' + token, BASE.replace('"' + field + '":' + old_token, '"' + field + '":' + token),
                 kind='invalid_input', fact='Huge valid unsigned declaration outside actual reference/enum domain remains InvalidInput.')
    emit('declared-length-overflow', BASE.replace('"byteLength":12}', '"byteLength":1e400}', 1),
         kind='resource_limit', fact='Mathematical declared buffer length exceeds member/work allowance before missing-resource resolution.')
    emit('view-length-overflow', BASE.replace('"buffer":0,"byteLength":12', '"buffer":0,"byteLength":1e400'),
         kind='invalid_input', fact='Huge numeric view range exceeds its declared12-byte buffer domain; invalid actual range is not a work request.')
    emit('stride-overflow', BASE.replace('"buffer":0,"byteLength":12', '"buffer":0,"byteLength":12,"byteStride":1e400'),
         kind='invalid_input', fact='Stride exceeds glTF uint32/4..252 schema/layout domain.')

    # Presence-aware actual fields: absent defaults, correct type, null and wrong
    # type. Each correct-type control changes exactly the selected field.
    locations = [('accessor-offset', lambda d: d['accessors'][0], 'byteOffset', 0),
                 ('view-offset', lambda d: d['bufferViews'][0], 'byteOffset', 0),
                 ('view-stride', lambda d: d['bufferViews'][0], 'byteStride', 12),
                 ('view-target', lambda d: d['bufferViews'][0], 'target', 34962),
                 ('accessor-normalized', lambda d: d['accessors'][0], 'normalized', False),
                 ('primitive-mode', lambda d: d['meshes'][0]['primitives'][0], 'mode', 0),
                 ('root-extensions', lambda d: d, 'extensions', {}),
                 ('root-used', lambda d: d, 'extensionsUsed', []),
                 ('root-required', lambda d: d, 'extensionsRequired', [])]
    for name, locate, key, good in locations:
        for suffix, value, kind in [('absent', 'ABSENT', 'admitted'), ('typed', good, 'admitted'),
                                    ('null', None, 'invalid_input'), ('wrong', 'wrong', 'invalid_input')]:
            doc = json.loads(BASE)
            locate(doc).pop(key, None)
            if value != 'ABSENT':
                locate(doc)[key] = value
            # With mode absent, the default is TRIANGLES, so independently
            # provide three viewless positions for this one presence quartet.
            if name == 'primitive-mode':
                doc = json.loads(VIEWLESS)
                doc['accessors'][0]['count'] = 3
                doc['meshes'][0]['primitives'][0].pop('mode', None)
                if value != 'ABSENT':
                    doc['meshes'][0]['primitives'][0]['mode'] = value
                emit('presence-' + name + '-' + suffix, packed(doc), None, kind,
                     fact='Absent/default versus present consumed type.', vertices=3, components=9)
            else:
                emit('presence-' + name + '-' + suffix, packed(doc), kind=kind,
                     fact='Absent/default versus present consumed type.')
    for key in ['uri', 'extensions']:
        for suffix, value in [('null', None), ('wrong', 7)]:
            doc = json.loads(BASE)
            doc['buffers'][0][key] = value
            emit('presence-buffer-' + key + '-' + suffix, packed(doc), kind='invalid_input',
                 fact='Present buffer URI/extension field retains type meaning.')
    for key in ['indices', 'material']:
        for suffix, value in [('null', None), ('wrong', '0')]:
            doc = json.loads(BASE)
            doc['meshes'][0]['primitives'][0][key] = value
            emit('presence-primitive-' + key + '-' + suffix, packed(doc), kind='invalid_input',
                 fact='Present optional reference retains type meaning.')

    for suffix, fragment, kind in [('huge-number', '1e400', 'admitted'),
                                   ('numeric-marker', '{"$serde_json::private::Number":"1e400"}', 'admitted'),
                                   ('nested-marker', '{"extensions":7,"deep":[1e400]}', 'admitted'),
                                   ('valid-surrogate', '"\\ud83d\\udc08"', 'admitted'),
                                   ('lone-high', '"\\ud800"', 'invalid_input'),
                                   ('lone-low', '"\\udc00"', 'invalid_input'),
                                   ('wrong-pair', '"\\ud800x"', 'invalid_input'),
                                   ('duplicate-decoded', '{"a":0,"\\u0061":1}', 'invalid_input')]:
        emit('opaque-' + suffix, BASE[:-1] + ',"extras":' + fragment + '}', kind=kind,
             fact='Opaque semantic values still receive shared grammar/Unicode/unique-key admission; no owned numeric materialization.')
    for suffix, literal in [('duplicate-root', BASE[:-1] + ',"asset":{}}'),
                            ('trailing', BASE + ' 0'), ('malformed-exponent', BASE.replace('"count":1', '"count":1e')),
                            ('nan-token', BASE.replace('"count":1', '"count":NaN'))]:
        emit('json-' + suffix, literal, kind='invalid_input', fact='Single-cause JSON grammar/duplicate admission.')
    for suffix, value, kind in [('huge-number', '1e400', 'unsupported'),
                               ('marker', '{"$serde_json::private::Number":"1e400"}', 'admitted'),
                               ('surrogate', '"\\ud800"', 'invalid_input'),
                               ('duplicate', '{"a":0,"\\u0061":1}', 'invalid_input')]:
        doc = tileset()
        raw = packed(doc)[:-1] + b',"extras":' + value.encode() + b'}'
        emit('owned-root-' + suffix, fact='Ordinary owned adapter after shared admission; unrepresentable valid number Unsupported.')
        # Replace authored root bytes and archive; expected report unchanged.
        case = directory / cases[-1]['name']
        (case / 'tileset.json').write_bytes(raw)
        data = (case / 'tile.glb').read_bytes()
        archived = archive([('tileset.json', raw), ('tile.glb', data)])
        (case / 'input.3tz').write_bytes(archived)
        cases[-1].update(expectedCategory=kind, expectedExitCode=CODES[kind],
                         expectedReport=cases[-1]['expectedReport'] if kind == 'admitted' else None,
                         sha256=sha(archived), bytes=len(archived),
                         memberSha256={'tileset.json': sha(raw), 'tile.glb': sha(data)})
    for suffix, raw, kind in [('huge-number', b'{"extras":1e400}', 'unsupported'),
                             ('marker', b'{"extras":{"$serde_json::private::Number":"1e400"}}', 'admitted'),
                             ('unicode', b'{"extras":"\\ud800"}', 'invalid_input'),
                             ('duplicate', b'{"extras":0,"extras":1}', 'invalid_input')]:
        emit('owned-conversion-' + suffix, kind=kind, extras={'conversion.json': raw},
             fact='Actual conversion report consumer retains owned numeric/marker behavior after shared admission.')

    for suffix, schema_uri, kind in [('absent', 'ABSENT', 'admitted'), ('null', None, 'invalid_input'),
                                     ('wrong', 7, 'invalid_input'), ('local', 'schema.json', 'admitted'),
                                     ('remote', 'https://example.invalid/schema.json', 'unsupported'),
                                     ('escape', '../schema.json', 'invalid_input'), ('missing', 'missing.json', 'invalid_input')]:
        doc = json.loads(BASE)
        doc.update(extensionsUsed=['EXT_structural_metadata'], extensions={'EXT_structural_metadata': {}})
        if schema_uri != 'ABSENT':
            doc['extensions']['EXT_structural_metadata']['schemaUri'] = schema_uri
        emit('schema-uri-' + suffix, packed(doc), kind=kind,
             extras={'schema.json': b'{"id":"independent","classes":{}}'} if suffix == 'local' else None,
             exclusions=['EXT_structural_metadata'],
             fact='Actual borrowed payload schemaUri consumer, caller URI authority and owned schema JSON admission.')
    doc = json.loads(BASE)
    doc.update(extensionsUsed=['EXT_structural_metadata'], extensions={'EXT_structural_metadata': {'schemaUri': 'schema.json'}})
    for suffix, raw, kind in [('huge', b'{"extras":1e400}', 'unsupported'),
                             ('unicode', b'{"extras":"\\ud800"}', 'invalid_input'),
                             ('duplicate', b'{"a":0,"\\u0061":1}', 'invalid_input')]:
        emit('schema-json-' + suffix, packed(doc), kind=kind, extras={'schema.json': raw},
             fact='schemaUri documents share admission and owned representation refusal.')

    emit('viewless-inverted-arbitrary-bounds', VIEWLESS.replace('"min":[0,0,0],"max":[0,0,0]',
         '"min":[17,4,8],"max":[-3,-4,-8]'), None,
         fact='Published viewless arbitrary-values exemption has no invented min<=max cross constraint.')
    emit('viewless-f64-finite-f32-infinity', VIEWLESS.replace('"min":[0,0,0]', '"min":[1e39,0,0]'), None,
         kind='invalid_input', fact='Finite f64 1e39 cannot be a finite FLOAT component after binary32 interpretation; viewless exemption preserves component meaning.')
    rounded = struct.unpack('<f', struct.pack('<f', 0.1))[0]
    assert rounded != 0.1 and math.isfinite(rounded)
    emit('stored-rounded-decimal-bound', BASE.replace('"min":[0,0,0],"max":[0,0,0]',
         '"min":[0.1,0,0],"max":[0.1,0,0]'), struct.pack('<3f', 0.1, 0, 0),
         scalar=scalar_oracle(struct.pack('<3f', 0.1, 0, 0), 5126, 'VEC3', 1),
         fact='Published FLOAT bounds permit decimal rounding to binary32; exact lexical representability is not required.')
    emit('viewless-rounded-decimal-bound', VIEWLESS.replace('"min":[0,0,0]', '"min":[0.1,0,0]'), None,
         fact='Finite decimal0.1 rounds to finite binary32; arbitrary viewless bound remains admitted.')
    for suffix, change in [('shape', '"min":[0,0]'), ('null', '"min":null'), ('wrong-type', '"min":["0",0,0]')]:
        emit('viewless-bounds-' + suffix, VIEWLESS.replace('"min":[0,0,0]', change), None, 'invalid_input',
             fact='Viewless exemption preserves bound shape/finite/component type.')
    emit('missing-bin', binary=None, kind='invalid_input', fact='Referenced buffer differs from viewless zero semantics.')
    for suffix, bad in [('nan', struct.pack('<I', 0x7fc00000)), ('infinity', struct.pack('<I', 0x7f800000))]:
        emit('binary-' + suffix, binary=bad + b'\0' * 8, kind='invalid_input', fact='Actual stored f32 must be finite.')
    good = glb(BASE, b'\0' * 12)
    emit('unknown-glb-chunk', payload=glb(BASE, b'\0' * 12, struct.pack('<I4s', 4, b'TEST') + b'abcd'),
         fact='Complete unknown chunks are ignored under selected GLB authority.')
    emit('duplicate-bin', payload=good + struct.pack('<I4s', 0, b'BIN\0'), kind='invalid_input',
         fact='Envelope length/framing corruption; not claimed to isolate duplicate chunk precedence.')

    for mode, count in [(0, 3), (1, 2), (3, 3), (4, 3)]:
        doc = json.loads(VIEWLESS)
        doc['accessors'][0]['count'] = count
        doc['meshes'][0]['primitives'][0]['mode'] = mode
        emit('core-mode-' + str(mode), packed(doc), None, vertices=count, components=3 * count,
             fact='Full admitted core POINTS/LINES/LINE_STRIP/TRIANGLES cardinality and viewless zeros.')
    emit('core-lines-odd', VIEWLESS.replace('"mode":0', '"mode":1').replace('"count":1', '"count":3'),
         None, 'invalid_input', fact='Ordinary LINES cardinality requires even count.')
    for mode in ['2', '5', '6', '7', '18446744073709551615', '1e400']:
        kind = 'unsupported' if mode in ['2', '5', '6'] else 'invalid_input'
        emit('core-mode-domain-' + mode, VIEWLESS.replace('"mode":0', '"mode":' + mode).replace('"count":1', '"count":3'),
             None, kind, fact='Defined glTF modes2/5/6 are outside finite profile; undefined7/huge mode values are invalid enum domain.')
    triangle = json.loads(BASE)
    triangle['buffers'][0]['byteLength'] = 42
    triangle['bufferViews'] = [dict(buffer=0, byteLength=36), dict(buffer=0, byteOffset=36, byteLength=6)]
    triangle['accessors'][0].update(count=3, min=[-2, -3, 0], max=[1, 2, 4])
    triangle['accessors'].append(dict(bufferView=1, componentType=5123, count=3, type='SCALAR'))
    triangle['meshes'][0]['primitives'][0].update(indices=1, mode=4)
    position_bytes = struct.pack('<9f', -2, -3, 0, 1, 0, 4, 0, 2, 0)
    triangle_bytes = position_bytes + struct.pack('<3H', 0, 1, 2)
    oracle = scalar_oracle(position_bytes, 5126, 'VEC3', 3)
    emit('stored-indexed-triangle', packed(triangle), triangle_bytes, accessors=2, vertices=3, components=12,
         scalar=oracle, fact='Independent little-endian actual finite POSITION extrema and ordinary indices0,1,2.')
    mismatch = copy.deepcopy(triangle)
    mismatch['accessors'][0]['max'][0] = 2
    emit('stored-raw-bound-mismatch', packed(mismatch), triangle_bytes, kind='invalid_input',
         fact='One declared raw bound disagrees with independent packed extrema.')
    emit('ordinary-index-oob', packed(triangle), position_bytes + struct.pack('<3H', 0, 1, 3), kind='invalid_input',
         fact='Ordinary index3 exceeds three POSITION vertices.')
    emit('ordinary-index-reserved', packed(triangle), position_bytes + struct.pack('<3H', 0, 1, 65535), kind='invalid_input',
         fact='Core indices prohibit unsigned component maximum absent admitted restart mode.')
    agreement = copy.deepcopy(triangle)
    agreement['accessors'].append(dict(componentType=5126, count=2, type='VEC3'))
    agreement['meshes'][0]['primitives'][0]['attributes']['NORMAL'] = 2
    emit('attribute-count-mismatch', packed(agreement), triangle_bytes, kind='invalid_input',
         fact='NORMAL count2 disagrees with POSITION count3; viewless values remain semantic zeros.')
    for key, value in [('indices', None), ('material', None)]:
        broken = copy.deepcopy(triangle)
        broken['meshes'][0]['primitives'][0][key] = value
        emit('indexed-reference-null-' + key, packed(broken), triangle_bytes, kind='invalid_input',
             fact='Present optional reference null cannot collapse to absent/default.')
    for name, present in [('extensions-view', 'bufferViews'), ('extensions-accessor', 'accessors')]:
        broken = copy.deepcopy(triangle)
        broken[present][0]['extensions'] = None
        emit('presence-' + name + '-null', packed(broken), triangle_bytes, kind='invalid_input',
             fact='Present consumed extension container remains a type error.')
    emit('sparse-outside-profile', VIEWLESS.replace('"componentType":5126', '"sparse":{},"componentType":5126'),
         None, 'unsupported', fact='Sparse accessor profile boundary; no sparse-conformance claim.')
    for media in ['application/octet-stream', 'application/gltf-buffer']:
        doc = json.loads(BASE)
        doc['buffers'][0]['uri'] = 'data:' + media + ';base64,' + base64.b64encode(b'\0' * 12).decode()
        emit('base64-' + media.split('/')[1], packed(doc), fmt='gltf', fact='Both selected standard-base64 buffer media types.')
    for suffix, encoded in [('bad-alphabet', '*AAA'), ('bad-pad', 'AA=A'), ('trailing-bits', 'AB==')]:
        doc = {'asset': {'version': '2.0'}, 'buffers': [{'byteLength': 1, 'uri': 'data:application/octet-stream;base64,' + encoded}]}
        emit('base64-' + suffix, packed(doc), fmt='gltf', kind='invalid_input',
             fact='Canonical standard base64 alphabet/padding/trailing bits.', accessors=0, primitives=0, vertices=0, components=0)
    for encoded, data in [('AA==', b'\0'), ('AAA=', b'\0\0')]:
        doc = {'asset': {'version': '2.0'}, 'buffers': [{'byteLength': len(data), 'uri': 'data:application/octet-stream;base64,' + encoded}]}
        emit('base64-padded-' + str(len(data)), packed(doc), fmt='gltf', fact='Canonical padded small buffer positive; tiny remaining-allowance proof needs private probe.',
             accessors=0, primitives=0, vertices=0, components=0)

    # Integer raw extrema and normalization at the signed minimum endpoint.
    quant = json.loads(BASE)
    quant.update(extensionsUsed=['KHR_mesh_quantization'], extensionsRequired=['KHR_mesh_quantization'])
    raw = struct.pack('<3bB', -128, 0, 127, 0)
    quant['buffers'][0]['byteLength'] = 4
    quant['bufferViews'][0].update(byteLength=4, byteStride=4)
    quant['accessors'][0].update(componentType=5120, normalized=True, min=[-128, 0, 127], max=[-128, 0, 127])
    oracle = scalar_oracle(raw, 5120, 'VEC3', 1, stride=4, normalized=True)
    assert oracle['normalizedValues'] == [[-1, 0, 1]]
    emit('quantized-signed-endpoints', packed(quant), raw, fact='Raw integer min/max remain raw; normalized signed minimum clamps to -1.', scalar=oracle)
    wrong = copy.deepcopy(quant)
    wrong['accessors'][0]['min'] = [-1, 0, 1]
    emit('quantized-normalized-bound-mismatch', packed(wrong), raw, kind='invalid_input', fact='Normalized float bounds cannot replace actual raw integer extrema.')
    # Unused matrix accessors prove all-accessor work and final column padding.
    for shape, ctype, values, occupied in [('MAT2', 5121, [1, 2, 3, 4], 6),
                                           ('MAT3', 5121, list(range(1, 10)), 11),
                                           ('MAT3', 5123, list(range(1, 10)), 22)]:
        rows = 2 if shape == 'MAT2' else 3
        size = 1 if ctype == 5121 else 2
        column = ((rows * size + 3) // 4) * 4
        raw_matrix = bytearray(column * rows)
        for col in range(rows):
            for row in range(rows):
                struct.pack_into('<B' if size == 1 else '<H', raw_matrix, col * column + row * size, values[col * rows + row])
        raw_matrix = bytes(raw_matrix[:occupied])
        doc = json.loads(BASE)
        doc['buffers'][0]['byteLength'] = 12 + occupied
        doc['bufferViews'].append(dict(buffer=0, byteOffset=12, byteLength=occupied))
        doc['accessors'].append(dict(bufferView=1, componentType=ctype, count=1, type=shape, min=values, max=values))
        oracle = scalar_oracle(raw_matrix, ctype, shape, 1)
        emit('matrix-final-padding-' + shape + '-' + str(ctype), packed(doc), b'\0' * 12 + raw_matrix,
             accessors=2, components=3 + len(values), fact='Column starts aligned; final padding omitted; unused accessor counted.', scalar=oracle)
        doc['buffers'][0]['byteLength'] -= 1
        doc['bufferViews'][1]['byteLength'] -= 1
        emit('matrix-one-byte-short-' + shape + '-' + str(ctype), packed(doc), b'\0' * 12 + raw_matrix[:-1],
             kind='invalid_input', fact='One occupied matrix byte missing at exact final component boundary.')

    # Pinned upstream meshoptimizer bytes are reference truth, not a producer.
    compressed = (REFS / 'golden-compressed.bin').read_bytes()
    decoded = (REFS / 'golden-expected.bin').read_bytes()
    oracle = scalar_oracle(decoded, 5126, 'VEC3', 4)
    mesh = json.loads(BASE)
    mesh.update(extensionsUsed=['EXT_meshopt_compression'], extensionsRequired=['EXT_meshopt_compression'])
    mesh['buffers'] = [dict(byteLength=48), dict(byteLength=85, uri='compressed.bin')]
    mesh['bufferViews'] = [dict(buffer=0, byteLength=48, byteStride=12,
        extensions={'EXT_meshopt_compression': dict(buffer=1, byteOffset=0, byteLength=85, byteStride=12,
                                                    count=4, mode='ATTRIBUTES', filter='NONE')})]
    mesh['accessors'][0].update(count=4, min=oracle['rawMin'], max=oracle['rawMax'])
    emit('meshopt-none-placeholder-glb', packed(mesh), None, extras={'compressed.bin': compressed},
         vertices=4, components=12, scalar=oracle, fact='Upstream 85->48 golden; required URI-less GLB fallback buffer0 without BIN.')
    emit('meshopt-none-placeholder-gltf', packed(mesh), None, fmt='gltf', extras={'compressed.bin': compressed},
         vertices=4, components=12, scalar=oracle, fact='Required URI-less local glTF fallback placeholder.')
    for suffix, marker in [('true', True), ('false', False), ('null', None), ('wrong', 'true')]:
        marked = copy.deepcopy(mesh)
        marked['buffers'][0]['extensions'] = {'EXT_meshopt_compression': {'fallback': marker}}
        emit('meshopt-fallback-marker-' + suffix, packed(marked), None,
             kind='admitted' if isinstance(marker, bool) else 'invalid_input',
             extras={'compressed.bin': compressed}, vertices=4, components=12,
             fact='Fallback marker optional boolean presence/type; placeholder has only compressed-view role.')
    actual = copy.deepcopy(mesh)
    actual['buffers'][0].update(uri='fallback.bin', extensions={'EXT_meshopt_compression': {'fallback': True}})
    emit('meshopt-actual-fallback', packed(actual), None, fmt='gltf', extras={'compressed.bin': compressed, 'fallback.bin': decoded},
         vertices=4, components=12, scalar=oracle, fact='Actual fallback resource preserves allowed compressed-view role.')
    ordinary = copy.deepcopy(mesh)
    ordinary['bufferViews'].append(dict(buffer=0, byteLength=4))
    emit('meshopt-uncovered-placeholder', packed(ordinary), None, kind='invalid_input', extras={'compressed.bin': compressed},
         fact='Placeholder cannot supply ordinary uncompressed view.')
    optional = copy.deepcopy(mesh)
    optional.pop('extensionsRequired')
    emit('meshopt-optional-placeholder', packed(optional), None, kind='invalid_input', extras={'compressed.bin': compressed},
         fact='URI-less compressed fallback requires meshopt required declaration.')
    unused = copy.deepcopy(mesh)
    unused['buffers'].append(dict(byteLength=32))
    emit('meshopt-unused-placeholder', packed(unused), None, extras={'compressed.bin': compressed}, vertices=4, components=12,
         fact='Required meshopt allows unused URI-less placeholder without invented use obligation.')

    restart = json.loads(VIEWLESS)
    restart['accessors'][0]['count'] = 4
    restart['accessors'].append(dict(bufferView=0, componentType=5123, count=5, type='SCALAR'))
    restart['buffers'] = [dict(byteLength=10)]
    restart['bufferViews'] = [dict(buffer=0, byteLength=10)]
    restart['meshes'][0]['primitives'][0].update(mode=3, indices=1)
    restart.update(extensionsUsed=['KHR_mesh_primitive_restart'], extensionsRequired=['KHR_mesh_primitive_restart'])
    for suffix, indices, kind in [('two-segments', [0, 1, 65535, 2, 3], 'admitted'),
                                  ('ordinary-oob', [0, 1, 65535, 2, 4], 'invalid_input'),
                                  ('leading-empty', [65535, 0, 1, 2, 3], 'unsupported'),
                                  ('singleton', [0, 65535, 1, 2, 3], 'unsupported')]:
        emit('restart-' + suffix, packed(restart), struct.pack('<5H', *indices), kind,
             accessors=2, vertices=4, components=17,
             fact='Pinned draft U16 sentinel segments; bounded profile excludes empty/singleton strips.')
    missing = copy.deepcopy(restart)
    missing.pop('extensionsRequired')
    emit('restart-not-required', packed(missing), struct.pack('<5H', 0, 1, 65535, 2, 3), 'invalid_input',
         fact='Draft restart has no fallback and must be required.')

    polygon = json.loads(BASE)
    polygon['extensionsUsed'] = ['EXT_mesh_polygon']
    polygon['buffers'][0]['byteLength'] = 32
    polygon['bufferViews'].append(dict(buffer=0, byteOffset=12, byteLength=20))
    for offset, count in [(0, 1), (4, 3), (16, 1)]:
        polygon['accessors'].append(dict(bufferView=1, byteOffset=offset, componentType=5125, count=count, type='SCALAR'))
    polygon['meshes'][0]['primitives'][0]['extensions'] = {'EXT_mesh_polygon': dict(count=1, indicesOffsets=1, loopIndices=2, loopIndicesOffsets=3)}
    polygon_bytes = b'\0' * 12 + struct.pack('<5I', 0, 0, 0, 0, 0)
    emit('polygon-optional-reference-profile', packed(polygon), polygon_bytes, accessors=4, components=8,
         exclusions=['EXT_mesh_polygon'],
         fact='Bounded optional polygon reference/unsigned scalar checks; no polygon topology certification.')
    bad = copy.deepcopy(polygon)
    bad['meshes'][0]['primitives'][0]['extensions']['EXT_mesh_polygon']['indicesOffsets'] = 999
    emit('polygon-reference-oob', packed(bad), polygon_bytes, kind='invalid_input', fact='Optional polygon actual accessor reference outside array.')

    for suffix, feature, batch, kind in [('zero', b'{"BATCH_LENGTH":0}', b'', 'admitted'),
                                       ('decimal', b'{"BATCH_LENGTH":0.0}', b'', 'admitted'),
                                       ('exponent', b'{"BATCH_LENGTH":0e0}', b'', 'admitted'),
                                       ('nonzero', b'{"BATCH_LENGTH":2}', b'{"opaque":[1e400]}', 'admitted'),
                                       ('fraction', b'{"BATCH_LENGTH":0.00000000000000001}', b'', 'invalid_input'),
                                       ('u32-overflow', b'{"BATCH_LENGTH":4294967296}', b'', 'invalid_input'),
                                       ('huge', b'{"BATCH_LENGTH":1e400}', b'', 'invalid_input'),
                                       ('null', b'{"BATCH_LENGTH":null}', b'', 'invalid_input'),
                                       ('missing', b'{}', b'', 'invalid_input'),
                                       ('duplicate', b'{"BATCH_LENGTH":0,"BATCH_LENGTH":0}', b'', 'invalid_input'),
                                       ('feature-unicode', b'{"BATCH_LENGTH":0,"extras":"\\ud800"}', b'', 'invalid_input'),
                                       ('batch-unicode', b'{"BATCH_LENGTH":0}', b'{"extras":"\\ud800"}', 'invalid_input')]:
        wrapper_payload = good
        wrapper_accessors, wrapper_components = 1, 3
        if suffix == 'nonzero':
            # Pinned b3dm requires _BATCHID for nonzero BATCH_LENGTH/present
            # Batch Table. Supply an independently valid viewless unsigned
            # scalar ID instead of using an otherwise invalid positive.
            batched = json.loads(BASE)
            batched['accessors'].append(dict(componentType=5121, count=1, type='SCALAR'))
            batched['meshes'][0]['primitives'][0]['attributes']['_BATCHID'] = 1
            wrapper_payload = glb(packed(batched), b'\0' * 12)
            wrapper_accessors, wrapper_components = 2, 4
        emit('b3dm-' + suffix, fmt='b3dm', payload=b3dm(wrapper_payload, feature, batch), kind=kind,
             accessors=wrapper_accessors, components=wrapper_components,
             exclusions=['b3dm feature and batch table semantics'],
             fact='Modern 28-byte b3dm tables, pinned integer schema uint32 range, shared borrowed table admission; semantics excluded.')

    shared = json.loads(BASE)
    shared['meshes'][0]['primitives'] *= 2
    emit('shared-position-two-primitives', packed(shared), primitives=2, vertices=2, components=3,
         fact='One accessor charged once; full POSITION count repeated for each primitive.')
    shared['accessors'].append(dict(componentType=5126, count=2, type='VEC4'))
    emit('unused-viewless-accessor', packed(shared), accessors=2, primitives=2, vertices=2, components=11,
         fact='Unused/viewless accessor components included despite no scene or primitive use.')
    root = tileset()
    root['root']['children'] = [tile('tile.glb')]
    emit('same-uri-cache', manifest=root,
         fact='Two tile references to same resolved member: one payload report and one successful component charge.')
    root = tileset()
    root['root']['children'] = [tile('./tile.glb')]
    emit('resolved-alias-cache', manifest=root,
         fact='Dot-path URI aliases resolve to same member and one payload report.')
    root = tileset()
    root['root']['children'] = [tile('alias.glb')]
    emit('distinct-member-identical-bytes', manifest=root, extras={'alias.glb': good}, components=6,
         expected_payloads=[dict(uri='alias.glb', accessorsChecked=1, primitivesChecked=1, vertices=1),
                            dict(uri='tile.glb', accessorsChecked=1, primitivesChecked=1, vertices=1)],
         fact='Distinct member names retain two reports/charges despite byte identity.')
    repeated = json.loads(BASE)
    repeated['buffers'] = [dict(byteLength=12, uri='zero.bin'), dict(byteLength=12, uri='zero.bin')]
    emit('repeated-resource-declarations', packed(repeated), None, fmt='gltf', extras={'zero.bin': b'\0' * 12},
         fact='Two declarations retain separate resource visits/actual/declared charges; one POSITION accessor.')

    manifest = dict(schemaVersion=1, scope='Finite independently authored format/category/report controls; no full C1/A2/release claim.',
                    status='Expectations and fixture generation only; no target execution asserted.',
                    referencePins=reference_pins(), cases=cases,
                    driverSha256=sha(Path(__file__).read_bytes()))
    (directory / 'manifest.json').write_text(json.dumps(manifest, indent=2, sort_keys=True) + '\n')
    return manifest


def snapshot(root):
    return {str(path.relative_to(root)): sha(path.read_bytes())
            for path in sorted(root.rglob('*')) if path.is_file()}


def source_snapshot():
    paths = sorted((REPO / 'src').rglob('*')) + sorted((REPO / 'docs/schema').rglob('*'))
    paths += [REPO / name for name in ['Cargo.toml', 'Cargo.lock', 'build.rs', 'pyproject.toml']]
    return {str(path.relative_to(REPO)): sha(path.read_bytes()) for path in paths if path.is_file()}


def controlled_launch():
    os.setpriority(os.PRIO_PROCESS, 0, 10)
    os.sched_setaffinity(0, set(sorted(os.sched_getaffinity(0))[:2]))


def run(binary, binary_hash, pin, pin_hash, directory, run_dir):
    assert sha(binary.read_bytes()) == binary_hash, 'Binary identity mismatch'
    assert sha(pin.read_bytes()) == pin_hash, 'Coordinator source pin identity mismatch'
    manifest = json.loads((directory / 'manifest.json').read_text())
    assert manifest['driverSha256'] == sha(Path(__file__).read_bytes()), 'Driver identity mismatch'
    assert manifest['referencePins'] == reference_pins(), 'Independent reference identity mismatch'
    run_dir.mkdir(parents=True, exist_ok=False)
    before = snapshot(directory)
    production_before = source_snapshot()
    records = []
    env = dict(os.environ, RAYON_NUM_THREADS='2', RUST_TEST_THREADS='2', CARGO_BUILD_JOBS='2')
    for case in manifest['cases']:
        path = (directory / case['path']).resolve()
        assert sha(path.read_bytes()) == case['sha256']
        command = [str(binary.resolve()), '--json', 'validate', str(path)]
        started = time.monotonic()
        result = subprocess.run(command, env=env, preexec_fn=controlled_launch,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20, check=False)
        (run_dir / (case['name'] + '.stdout')).write_bytes(result.stdout)
        (run_dir / (case['name'] + '.stderr')).write_bytes(result.stderr)
        try:
            response = json.loads(result.stdout)
        except (ValueError, UnicodeError):
            response = None
        observed = ('admitted' if result.returncode == 0 else
                    (response or {}).get('error', {}).get('code', 'unknown'))
        expected_report = copy.deepcopy(case['expectedReport'])
        if expected_report:
            expected_report['archive'] = str(path)
        passed = (result.returncode == case['expectedExitCode'] and observed == case['expectedCategory']
                  and (expected_report is None or response == expected_report))
        record = dict(name=case['name'], passed=passed, observedCategory=observed,
                      expectedCategory=case['expectedCategory'], exitCode=result.returncode,
                      command=command, elapsedSeconds=time.monotonic() - started,
                      response=response, expectedReport=expected_report,
                      stdoutSha256=sha(result.stdout), stderrSha256=sha(result.stderr),
                      archiveSha256=case['sha256'])
        records.append(record)
        print(json.dumps(dict(name=case['name'], passed=passed, observed=observed,
                              expected=case['expectedCategory'])), flush=True)
    immutable = before == snapshot(directory)
    unchanged = production_before == source_snapshot() and sha(binary.read_bytes()) == binary_hash
    receipt = dict(schemaVersion=1, binarySha256=binary_hash, sourcePinSha256=pin_hash,
                   sourcePin=json.loads(pin.read_text()), sourceSnapshot=production_before,
                   manifestSha256=sha((directory / 'manifest.json').read_bytes()),
                   driverSha256=sha(Path(__file__).read_bytes()), fixturesBefore=before,
                   fixturesUnchanged=immutable, sourceAndBinaryUnchanged=unchanged,
                   executionPolicy=dict(nice=10, affinity=sorted(os.sched_getaffinity(0))[:2],
                                        RAYON_NUM_THREADS=2, RUST_TEST_THREADS=2, CARGO_BUILD_JOBS=2),
                   cases=records, passes=sum(row['passed'] for row in records),
                   failures=sum(not row['passed'] for row in records),
                   scope=manifest['scope'], limits='Public CLI fixed profile only. No causal resolver injection/private small-limit or total RAM claim.')
    (run_dir / 'receipt.json').write_text(json.dumps(receipt, indent=2, sort_keys=True) + '\n')
    return 0 if immutable and unchanged and all(row['passed'] for row in records) else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--generate', type=Path)
    parser.add_argument('--fixtures', type=Path)
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--binary-sha256')
    parser.add_argument('--source-pin', type=Path)
    parser.add_argument('--source-pin-sha256')
    parser.add_argument('--run-dir', type=Path)
    args = parser.parse_args()
    if args.generate:
        result = generate(args.generate)
        print(json.dumps(dict(cases=len(result['cases']), manifest=str(args.generate / 'manifest.json'))))
    if args.binary:
        for name in ['binary_sha256', 'source_pin', 'source_pin_sha256', 'run_dir']:
            if not getattr(args, name):
                parser.error('--' + name.replace('_', '-') + ' required with --binary')
        directory = args.fixtures or args.generate
        if directory is None:
            parser.error('--fixtures or --generate required with --binary')
        return run(args.binary, args.binary_sha256, args.source_pin, args.source_pin_sha256, directory, args.run_dir)
    if not args.generate:
        parser.error('--generate or --binary required')
    return 0


if __name__ == '__main__':
    sys.exit(main())
