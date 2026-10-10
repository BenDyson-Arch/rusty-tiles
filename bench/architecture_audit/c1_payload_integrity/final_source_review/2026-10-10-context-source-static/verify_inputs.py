"""Nonauthor static fixture review; no rusty-tiles imports or target execution."""
import hashlib
import io
import json
import pathlib
import posixpath
import struct
import zipfile


REPO = pathlib.Path(__file__).resolve().parents[5]
BASE = REPO / 'bench/architecture_audit/c1_payload_integrity/production_controls/context_controls/2026-10-10'


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def metric(raw):
    pending = [(json.loads(raw), 0)]
    nodes = depth = 0
    while pending:
        value, level = pending.pop()
        nodes += 1
        depth = max(depth, level)
        if isinstance(value, dict):
            pending.extend((child, level + 1) for child in value.values())
        elif isinstance(value, list):
            pending.extend((child, level + 1) for child in value)
    return dict(bytes=len(raw), nodes=nodes, depth=depth, sha256=sha(raw))


def glb_json(raw):
    assert raw[:4] == b'glTF'
    version, total, length = struct.unpack_from('<III', raw, 4)
    assert version == 2 and total == len(raw) and raw[16:20] == b'JSON'
    return raw[20:20 + length]


manifest = json.loads((BASE / 'fixtures/manifest.json').read_bytes())
preparation = json.loads((BASE / 'preparation-manifest.json').read_bytes())
for path, pin in preparation['files'].items():
    raw = (BASE / path).read_bytes()
    assert len(raw) == pin['bytes'] and sha(raw) == pin['sha256'], path
for pin in json.loads((BASE / 'references/receipt.json').read_bytes())['references']:
    raw = (BASE / pin['path']).read_bytes()
    assert len(raw) == pin['bytes'] and sha(raw) == pin['sha256']

records = []
shapes = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4, 'MAT2': 4, 'MAT3': 9, 'MAT4': 16}
checks = ['archiveIndex', 'archiveCrc', 'archiveStoredRecordLayout', 'contentHashes', 'tilesetSchema', 'payloadEnvelope', 'bufferRanges', 'accessorValues', 'primitiveIndices', 'hierarchyBounds', 'geometricErrorOrder', 'resourceReferences', 'recordedBudgets']
base_exclusions = {'decodedContentBounds', 'metadataSemantics', 'materialAndImageSemantics', 'geometricErrorAccuracy', 'scene transforms, skins, animations, morph targets and rendered bounds', 'metadata and feature semantics', 'EXT_structural_metadata'}
for case in manifest['cases']:
    archive_path = BASE / case['path']
    raw_archive = archive_path.read_bytes()
    assert sha(raw_archive) == case['sha256'] and len(raw_archive) == case['bytes']
    with zipfile.ZipFile(io.BytesIO(raw_archive)) as archive:
        supplied = {item.filename: archive.read(item) for item in archive.infolist()}
        assert archive.testzip() is None
        rows = [(hashlib.md5(item.filename.encode()).digest(), item.header_offset)
                for item in archive.infolist() if item.filename != '@3dtilesIndex1@']
        rows.sort(key=lambda row: struct.unpack('<QQ', row[0]))
        assert supplied.pop('@3dtilesIndex1@') == b''.join(digest + struct.pack('<Q', offset) for digest, offset in rows)
    assert set(supplied) == set(case['memberPins'])
    for name, pin in case['memberPins'].items():
        raw = supplied[name]
        assert (archive_path.parent / name).read_bytes() == raw
        assert len(raw) == pin['bytes'] and sha(raw) == pin['sha256']

    actual = {}
    payload_docs = {}
    for name, raw in supplied.items():
        if name == 'tileset.json':
            actual['root-tileset'] = raw
        elif name == 'external.json':
            actual['external-tileset'] = raw
        elif name == 'schema.json':
            actual[case['target'] if str(case['target']).startswith('metadata-schema-') else 'schema'] = raw
        elif name == 'conversion.json':
            actual['conversion-report'] = raw
        elif name.endswith('.gltf'):
            actual['local-gltf'] = raw
            payload_docs[name] = json.loads(raw)
        elif name.endswith('.glb'):
            actual['glb-json'] = glb_json(raw)
            payload_docs[name] = json.loads(actual['glb-json'])
        elif name.endswith('.b3dm'):
            assert raw[:4] == b'b3dm'
            version, total, feature, feature_bin, batch, batch_bin = struct.unpack_from('<6I', raw, 4)
            assert version == 1 and total == len(raw) and feature_bin == batch_bin == 0
            actual['b3dm-feature-table'] = raw[28:28 + feature]
            if batch:
                actual['b3dm-batch-table'] = raw[28 + feature:28 + feature + batch]
            inner = raw[28 + feature + batch:]
            inner_total = struct.unpack_from('<I', inner, 8)[0]
            inner = inner[:inner_total]
            key = 'b3dm-embedded-glb' if case['target'] == 'b3dm-embedded-glb' else 'glb-json'
            actual[key] = glb_json(inner)
            payload_docs[name] = json.loads(actual[key])
        elif name.endswith('.subtree'):
            assert raw[:8] == b'subt\x01\0\0\0'
            length, binary_length = struct.unpack_from('<QQ', raw, 8)
            assert length % 8 == binary_length % 8 == 0 and 24 + length + binary_length == len(raw)
            actual['subtree-json'] = raw[24:24 + length]
            doc = json.loads(actual['subtree-json'])
            if 'tileMetadata' in doc:
                binary = raw[24 + length:]
                props = doc['propertyTables'][doc['tileMetadata']]['properties']['extras']
                strings = doc['bufferViews'][props['values']]
                offsets = doc['bufferViews'][props['stringOffsets']]
                start, end = struct.unpack_from('<II', binary, offsets.get('byteOffset', 0))
                assert start == 0 and end == strings['byteLength']
                offset = strings.get('byteOffset', 0)
                actual['nested-metadata-extras'] = binary[offset + start:offset + end]
    if not case['target']:
        actual = {'root': actual['root-tileset'], 'glb': actual['glb-json'], 'schema': actual['schema']}
    assert set(actual) == set(case['metrics'])
    for context, raw in actual.items():
        assert metric(raw) == case['metrics'][context], (case['name'], context)
        assert (archive_path.parent / ('interpreted-' + context + '.json')).read_bytes() == raw
    target = case['target']
    if target:
        for other in actual:
            if other != target:
                assert all(case['metrics'][other][fact] < case['metrics'][target][fact] for fact in ['bytes', 'nodes', 'depth'])

    root = json.loads(supplied['tileset.json'])
    implicit = 'implicitTiling' in root['root']
    refs = []
    tiles = 0
    content_references = 0
    pending = [('tileset.json', root['root'])]
    while pending:
        base, node = pending.pop()
        tiles += 1
        if 'content' in node:
            content_references += 1
            uri = node['content']['uri']
            if implicit:
                uri = uri.format(level=0, x=0, y=0)
            resolved = posixpath.normpath(posixpath.join(posixpath.dirname(base), uri))
            if resolved.endswith('.json'):
                pending.append((resolved, json.loads(supplied[resolved])['root']))
            else:
                refs.append(resolved)
        pending.extend((base, child) for child in node.get('children', []))
    unique = sorted(set(refs))
    counters = []
    components = 0
    for name in unique:
        doc = payload_docs[name]
        count = sum(accessor['count'] * shapes[accessor['type']] for accessor in doc['accessors'])
        components += count
        primitives = [primitive for mesh in doc['meshes'] for primitive in mesh['primitives']]
        vertices = sum(doc['accessors'][primitive['attributes']['POSITION']]['count'] for primitive in primitives)
        counters.append(dict(uri=name, accessorsChecked=len(doc['accessors']), primitivesChecked=len(primitives), vertices=vertices))
    expected = case['expectedReport']
    assert expected['ok'] is True and expected['archive'] == '$INPUT' and expected['limits'] == '$SELECTED'
    assert expected['checks'] == checks
    assert expected['payloads'] == counters
    assert expected['tiles'] == tiles and expected['contentReferences'] == content_references
    assert expected['entries'] == len(supplied) + 1
    assert case['successfulUniqueComponents'] == components
    if case['aggregateEquality'] is not None:
        assert case['aggregateEquality'] == components
    exclusions = set(base_exclusions)
    if any(name.endswith('.b3dm') for name in unique):
        exclusions.add('b3dm feature and batch table semantics')
    if implicit:
        exclusions.add('implicitAddressingAndAvailability')
    assert expected['notInspected'] == sorted(exclusions)
    records.append(dict(case=case['name'], suppliedSlices=len(actual), targetDominanceVerified=bool(target), components=components, fullReportTruthVerified=True))

print(json.dumps(dict(preparedFilesVerified=len(preparation['files']), contexts=sum(bool(case['target']) for case in manifest['cases']), aggregateCases=sum(case['aggregateEquality'] is not None for case in manifest['cases']), cases=records, scope='Independent static supplied-slice/metric/archive/report review; no target execution'), indent=2))
