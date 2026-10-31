#!/usr/bin/env python3
"""Independent literal C1 context fixtures. Python stdlib; no target execution.

Every context has one authored stress JSON document that strictly dominates all
other interpreted documents in supplied bytes, logical nodes, and root-zero
depth. Its exact metric is the selected admission ceiling; one less must fail.
"""
import argparse
import copy
import hashlib
import io
import json
from pathlib import Path
import struct
import zipfile

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[5]
BOX = [0, 0, 0, 10, 0, 0, 0, 10, 0, 0, 0, 10]
CHECKS = ['archiveIndex', 'archiveCrc', 'archiveStoredRecordLayout', 'contentHashes',
          'tilesetSchema', 'payloadEnvelope', 'bufferRanges', 'accessorValues',
          'primitiveIndices', 'hierarchyBounds', 'geometricErrorOrder',
          'resourceReferences', 'recordedBudgets']
EXCLUSIONS = ['EXT_structural_metadata', 'decodedContentBounds', 'geometricErrorAccuracy',
              'materialAndImageSemantics', 'metadata and feature semantics', 'metadataSemantics',
              'scene transforms, skins, animations, morph targets and rendered bounds']


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def packed(value):
    return json.dumps(value, separators=(',', ':'), ensure_ascii=True, allow_nan=False).encode()


def metrics(raw):
    value = json.loads(raw)
    def visit(item, depth):
        values = item.values() if isinstance(item, dict) else item if isinstance(item, list) else ()
        nodes, max_depth = 1, depth
        for child in values:
            count, deepest = visit(child, depth + 1)
            nodes += count
            max_depth = max(max_depth, deepest)
        return nodes, max_depth
    nodes, depth = visit(value, 0)
    return dict(bytes=len(raw), nodes=nodes, depth=depth, sha256=sha(raw))


def stress():
    nested = 0
    for _ in range(18):
        nested = [nested]
    return dict(padding=[0] * 1536, depth=nested)


def tile(uri):
    return dict(boundingVolume=dict(box=BOX), geometricError=0, content=dict(uri=uri))


def tileset(uri='point.glb'):
    return dict(asset=dict(version='1.1'), geometricError=0, schemaUri='schema.json', root=tile(uri))


def gltf(batch=False, primitives=1):
    doc = dict(asset=dict(version='2.0'), buffers=[dict(byteLength=12)],
               bufferViews=[dict(buffer=0, byteLength=12)],
               accessors=[dict(bufferView=0, componentType=5126, count=1, type='VEC3', min=[0, 0, 0], max=[0, 0, 0])],
               meshes=[dict(primitives=[dict(attributes=dict(POSITION=0), mode=0) for _ in range(primitives)])],
               extensionsUsed=['EXT_structural_metadata'],
               extensions=dict(EXT_structural_metadata=dict(schemaUri='schema.json')))
    if batch:
        doc['accessors'].append(dict(componentType=5121, count=1, type='SCALAR'))
        for primitive in doc['meshes'][0]['primitives']:
            primitive['attributes']['_BATCHID'] = 1
    return doc


def glb(raw):
    raw += b' ' * (-len(raw) % 4)
    chunks = struct.pack('<I4s', len(raw), b'JSON') + raw + struct.pack('<I4s', 12, b'BIN\0') + b'\0' * 12
    return struct.pack('<4sII', b'glTF', 2, len(chunks) + 12) + chunks, raw


def b3dm(inner, feature, batch=b''):
    feature += b' ' * (-(28 + len(feature)) % 8)
    if batch:
        batch += b' ' * (-len(batch) % 8)
    body = feature + batch + inner
    body += b'\0' * (-(28 + len(body)) % 8)
    return struct.pack('<4s6I', b'b3dm', 1, len(body) + 28, len(feature), 0, len(batch), 0) + body, feature, batch


def subtree(doc, binary=b''):
    raw = packed(doc)
    raw += b' ' * (-len(raw) % 8)
    binary += b'\0' * (-len(binary) % 8)
    return struct.pack('<4sIQQ', b'subt', 1, len(raw), len(binary)) + raw + binary, raw


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
        for name, supplied in members:
            assert check.read(name) == supplied
        for _, offset in records:
            assert raw[offset:offset + 4] == b'PK\x03\x04'
    return raw


def implicit_manifest(metadata=False):
    doc = tileset('point-{level}-{x}-{y}.glb')
    doc['root']['implicitTiling'] = dict(subdivisionScheme='QUADTREE', subtreeLevels=1,
                                       availableLevels=1, subtrees=dict(uri='{level}-{x}-{y}.subtree'))
    if metadata:
        doc.pop('schemaUri')
        doc['schema'] = dict(id='independentImplicit', classes=dict(rustyTile=dict(properties=dict(
            boundingBox=dict(type='SCALAR', componentType='FLOAT64', array=True, count=12, semantic='TILE_BOUNDING_BOX'),
            geometricError=dict(type='SCALAR', componentType='FLOAT64', semantic='TILE_GEOMETRIC_ERROR'),
            extras=dict(type='STRING')))))
    return doc


def reference_pins():
    receipt = json.loads((HERE / 'references/receipt.json').read_text())
    for record in receipt['references']:
        raw = (HERE / record['path']).read_bytes()
        assert len(raw) == record['bytes'] and sha(raw) == record['sha256']
    pins = list(receipt['references'])
    base = HERE.parents[1]
    for path in [base / 'fixture-bundle.json', base / 'driver.py', base / 'private_controls.rs',
                 base / '2026-10-10-component-bounds-decision.md',
                 REPO / 'docs/architecture/c1-payload-integrity-contract.md',
                 REPO / 'docs/schema/tileset.schema.json']:
        raw = path.read_bytes()
        pins.append(dict(path=str(path.relative_to(REPO)), bytes=len(raw), sha256=sha(raw),
                         role='Existing independent proof binding or selected bounded profile/schema, not a producer oracle'))
    # Parent frozen manifest already binds all exact glTF/b3dm/extension sources.
    pins.append(dict(publishedGltfRevision='8e798b02d254cea97659a333cfcb20875b62bdd4',
                     sourceSha256='10aebb6a8362a155b196448912bf6250bf046f150431fa73c6cd657c44185aff',
                     accessorSchemaSha256='86f9aacb0b616e1f6a1d5b8ff81e92294602e9e3c98c7f13d2cefdf987c87e35',
                     b3dmRevision='4d781014b52294759834018a931223b98ac1ce47',
                     b3dmSourceSha256='539762ac799aa8b685035647682b54a40b728904c51ee0685838e29a1ce04f3e'))
    return pins


def generate(directory):
    directory.mkdir(parents=True, exist_ok=False)
    cases = []
    schema = packed(dict(id='independentSchema', classes={}))
    ordinary_glb, ordinary_json = glb(packed(gltf()))

    def emit(name, members, documents, target=None, aggregate=None, payloads=None,
             tiles=1, content_references=1, components=3, extra_exclusions=()):
        case = directory / name
        case.mkdir()
        for member, raw in members:
            path = case / member
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(raw)
        for context, raw in documents.items():
            path = case / ('interpreted-' + context + '.json')
            path.write_bytes(raw)
        data = archive(members)
        (case / 'input.3tz').write_bytes(data)
        measured = {context: metrics(raw) for context, raw in documents.items()}
        if target:
            for context, value in measured.items():
                if context != target:
                    for fact in ['bytes', 'nodes', 'depth']:
                        assert measured[target][fact] > value[fact], (name, target, context, fact, measured)
        report = dict(ok=True, archive='$INPUT', limits='$SELECTED', tiles=tiles,
                      contentReferences=content_references, entries=len(members) + 1,
                      checks=CHECKS, notInspected=sorted(set(EXCLUSIONS) | set(extra_exclusions)),
                      payloads=payloads or [dict(uri='point.glb', accessorsChecked=1, primitivesChecked=1, vertices=1)])
        cases.append(dict(name=name, path='fixtures/' + name + '/input.3tz',
                          expectedReport=report, successfulUniqueComponents=components,
                          target=target, metrics=measured, aggregateEquality=aggregate,
                          sha256=sha(data), bytes=len(data),
                          memberPins={n: dict(bytes=len(v), sha256=sha(v)) for n, v in members},
                          expectation='Same valid archive admits at exact selected target fact, ResourceLimit at one less; positive full report and immutable archive bytes.'))

    for name in ['root-tileset', 'external-tileset', 'metadata-schema-root', 'metadata-schema-payload',
                 'conversion-report', 'local-gltf', 'glb-json', 'b3dm-feature-table',
                 'b3dm-batch-table', 'b3dm-embedded-glb', 'subtree-json', 'nested-metadata-extras']:
        root = tileset()
        doc = gltf()
        members_extra = []
        documents = {'schema': schema}
        target = name
        payloads = None
        tiles, contents, components, exclusions = 1, 1, 3, []
        if name == 'root-tileset':
            root['extras'] = stress()
        elif name == 'external-tileset':
            root['root']['content']['uri'] = 'external.json'
            external = tileset()
            external['extras'] = stress()
            raw = packed(external)
            members_extra.append(('external.json', raw))
            documents['external-tileset'] = raw
            tiles, contents = 2, 2
        elif name.startswith('metadata-schema-'):
            stressed_schema = packed(dict(id='independentSchema', classes={}, extras=stress()))
            documents.pop('schema')
            documents[name] = stressed_schema
            if name == 'metadata-schema-payload':
                # Isolate payload schemaUri: root has no separate schemaUri.
                root.pop('schemaUri')
            else:
                # Isolate the ordinary metadata_schema caller: payload omits
                # structural schemaUri while preserving the optional declaration.
                doc['extensions']['EXT_structural_metadata'] = {}
            schema_member = stressed_schema
        elif name == 'conversion-report':
            conversion = packed(dict(extras=stress()))
            members_extra.append(('conversion.json', conversion))
            documents[name] = conversion
        elif name in ['local-gltf', 'glb-json', 'b3dm-embedded-glb']:
            doc['extras'] = stress()
        elif name in ['b3dm-feature-table', 'b3dm-batch-table']:
            if name == 'b3dm-batch-table':
                doc = gltf(batch=True)
                components = 4
        elif name in ['subtree-json', 'nested-metadata-extras']:
            root = implicit_manifest(name == 'nested-metadata-extras')
            availability = dict(tileAvailability=dict(constant=1), contentAvailability=[dict(constant=1)],
                                childSubtreeAvailability=dict(constant=0))
            binary = b''
            if name == 'subtree-json':
                availability['extras'] = stress()
            else:
                # Independently packed single row: 12 FLOAT64 box lanes, one
                # FLOAT64 error, UTF8 JSON extras, UINT32 offsets0/end.
                extras = packed(stress())
                binary = struct.pack('<12dd', *BOX, 0) + extras
                binary += b'\0' * (-len(binary) % 8)
                offset_start = len(binary)
                binary += struct.pack('<2I', 0, len(extras))
                availability.update(buffers=[dict(byteLength=len(binary))],
                    bufferViews=[dict(buffer=0, byteOffset=0, byteLength=96),
                                 dict(buffer=0, byteOffset=96, byteLength=8),
                                 dict(buffer=0, byteOffset=104, byteLength=len(extras)),
                                 dict(buffer=0, byteOffset=offset_start, byteLength=8)],
                    tileMetadata=0, propertyTables=[dict(class_='rustyTile', count=1, properties=dict(
                        boundingBox=dict(values=0), geometricError=dict(values=1),
                        extras=dict(values=2, stringOffsets=3, stringOffsetType='UINT32')))])
                availability['propertyTables'][0]['class'] = availability['propertyTables'][0].pop('class_')
                documents[name] = extras
                assert struct.unpack_from('<12d', binary) == tuple(BOX)
                assert struct.unpack_from('<d', binary, 96) == (0,)
                assert struct.unpack_from('<2I', binary, offset_start) == (0, len(extras))
                assert binary[104:104 + len(extras)] == extras
            framed, subtree_raw = subtree(availability, binary)
            members_extra.append(('0-0-0.subtree', framed))
            documents['subtree-json'] = subtree_raw
            payloads = [dict(uri='point-0-0-0.glb', accessorsChecked=1, primitivesChecked=1, vertices=1)]
            exclusions.append('implicitAddressingAndAvailability')

        if not name.startswith('metadata-schema-'):
            schema_member = schema
        root_raw = packed(root)
        documents['root-tileset'] = root_raw
        payload_name = 'point.glb'
        if name == 'local-gltf':
            payload_name = 'point.gltf'
            root['root']['content']['uri'] = payload_name
            root_raw = packed(root)
            documents['root-tileset'] = root_raw
            doc['buffers'][0]['uri'] = 'zero.bin'
            payload_raw = packed(doc)
            documents['local-gltf'] = payload_raw
            members_extra.append(('zero.bin', b'\0' * 12))
            payloads = [dict(uri=payload_name, accessorsChecked=1, primitivesChecked=1, vertices=1)]
        else:
            payload_raw, glb_raw = glb(packed(doc))
            documents['glb-json'] = glb_raw
            if name.startswith('b3dm-'):
                payload_name = 'point.b3dm'
                root['root']['content']['uri'] = payload_name
                root_raw = packed(root)
                documents['root-tileset'] = root_raw
                feature = dict(BATCH_LENGTH=1 if name == 'b3dm-batch-table' else 0)
                batch = b''
                if name == 'b3dm-feature-table':
                    feature['extras'] = stress()
                if name == 'b3dm-batch-table':
                    batch = packed(dict(extras=stress()))
                payload_raw, feature_raw, batch_raw = b3dm(payload_raw, packed(feature), batch)
                documents['b3dm-feature-table'] = feature_raw
                if batch_raw:
                    documents['b3dm-batch-table'] = batch_raw
                if name == 'b3dm-embedded-glb':
                    documents['b3dm-embedded-glb'] = documents.pop('glb-json')
                payloads = [dict(uri=payload_name, accessorsChecked=2 if name == 'b3dm-batch-table' else 1,
                                 primitivesChecked=1, vertices=1)]
                exclusions.append('b3dm feature and batch table semantics')
            elif name in ['subtree-json', 'nested-metadata-extras']:
                payload_name = 'point-0-0-0.glb'
        members = [('tileset.json', root_raw), (payload_name, payload_raw), ('schema.json', schema_member)] + members_extra
        emit(name, members, documents, target=target, payloads=payloads, tiles=tiles,
             content_references=contents, components=components, extra_exclusions=exclusions)

    for name in ['same-uri-cache', 'resolved-alias-cache', 'distinct-byte-identical-members',
                 'shared-position-primitives', 'unused-viewless-accessor']:
        root = tileset()
        extra = []
        components, tiles, references = 3, 1, 1
        doc = gltf()
        payloads = [dict(uri='point.glb', accessorsChecked=1, primitivesChecked=1, vertices=1)]
        if name == 'shared-position-primitives':
            doc = gltf(primitives=2)
            payloads[0].update(primitivesChecked=2, vertices=2)
        elif name == 'unused-viewless-accessor':
            doc['accessors'].append(dict(componentType=5121, count=1, type='SCALAR'))
            payloads[0]['accessorsChecked'] = 2
            components = 4
        else:
            uri = {'same-uri-cache': 'point.glb', 'resolved-alias-cache': './point.glb',
                   'distinct-byte-identical-members': 'alias.glb'}[name]
            root['root']['children'] = [tile(uri)]
            tiles, references = 2, 2
            if name == 'distinct-byte-identical-members':
                extra = [('alias.glb', ordinary_glb)]
                payloads.insert(0, dict(uri='alias.glb', accessorsChecked=1, primitivesChecked=1, vertices=1))
                components = 6
        data, raw_json = glb(packed(doc))
        members = [('tileset.json', packed(root)), ('point.glb', data), ('schema.json', schema)] + extra
        emit(name, members, dict(root=packed(root), glb=raw_json, schema=schema), aggregate=components,
             components=components, tiles=tiles, content_references=references, payloads=payloads)

    manifest = dict(schemaVersion=1, date='2026-10-10',
                    status='Prepared expectations and fixture bytes; no target execution claim.',
                    scope='Actual bounded C1 selected-limit context and aggregate/cache proof inputs; no full A2/implicit/release acceptance.',
                    generatorSha256=sha(Path(__file__).read_bytes()), referencePins=reference_pins(), cases=cases,
                    logicalJsonRule='Root counts as node/depth0; scalar/container values count, object keys do not; chunk/table padding remains supplied bytes.')
    (directory / 'manifest.json').write_text(json.dumps(manifest, indent=2, sort_keys=True) + '\n')
    print(json.dumps(dict(cases=len(cases), contextCases=sum(bool(c['target']) for c in cases), aggregateCases=sum(c['aggregateEquality'] is not None for c in cases))))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--generate', type=Path, required=True)
    generate(parser.parse_args().generate)
