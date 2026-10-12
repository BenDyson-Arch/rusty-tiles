#!/usr/bin/env python3
"""Generate meshopt stream-header controls (C1 rule H-1) from the retained pinned primary
texts with the standard library only; never invokes project code or the target.

Verdicts come from the EXT_meshopt_compression text alone: Appendix A gives exactly one
header byte per mode, so a case is admitted iff its stream starts with that byte. Stream
bytes are the pinned upstream meshoptimizer test literals; the 0xd0 twin and the
unknown-version bytes are header substitutions of those literals.
"""
import hashlib, io, json, re, struct, zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
PRIMARY = HERE / 'primary'
EXT = 'EXT_meshopt_compression'


def sha(raw): return hashlib.sha256(raw).hexdigest()
def packed(obj): return json.dumps(obj, separators=(',', ':'), allow_nan=False).encode()


def spec_headers(text):
    """{'ATTRIBUTES': 0xa0, ...} from Appendix A, '## Mode N: name' sections."""
    appendix = text.split('# Appendix A: Bitstream', 1)[1]
    headers = {}
    for number, name, body in re.findall(r'^## Mode (\d): (\w+)\n(.*?)(?=^## |\Z)', appendix, re.M | re.S):
        found = re.findall(r'Header byte, which must be equal to `(0x[0-9a-f]{2})`', body)
        assert len(found) == 1, (number, name, found)
        headers[name.upper()] = int(found[0], 16)
    assert sorted(headers) == ['ATTRIBUTES', 'INDICES', 'TRIANGLES'], headers
    return headers


def upstream(source, case):
    """The encoded and expected literals of one pinned upstream test case."""
    body = source.split('\t' + case + ': function () {', 1)[1].split('\n\t},', 1)[0]
    arrays = {}
    for name, kind, text in re.findall(r'var (encoded|expected) = new (Uint8|Uint16|Uint32)Array\(\[([\s\S]*?)\]\);', body):
        values = [int(token.strip(), 0) for token in text.split(',') if token.strip()]
        arrays[name] = bytes(values) if kind == 'Uint8' else struct.pack('<%dI' % len(values), *values)
    return arrays['encoded'], arrays['expected']


def glb(doc, binary):
    raw = packed(doc); raw += b' ' * (-len(raw) % 4)
    padded = binary + b'\0' * (-len(binary) % 4)
    chunks = struct.pack('<I4s', len(raw), b'JSON') + raw + struct.pack('<I4s', len(padded), b'BIN\0') + padded
    return struct.pack('<4sII', b'glTF', 2, 12 + len(chunks)) + chunks


def archive(members):
    stream = io.BytesIO(); records = []
    with zipfile.ZipFile(stream, 'w', compression=zipfile.ZIP_STORED) as target:
        for name, raw in members:
            records.append((hashlib.md5(name.encode()).digest(), stream.tell()))
            target.writestr(zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0)), raw)
        records.sort(key=lambda row: struct.unpack('<QQ', row[0]))
        target.writestr(zipfile.ZipInfo('@3dtilesIndex1@', (1980, 1, 1, 0, 0, 0)),
                        b''.join(d + struct.pack('<Q', o) for d, o in records))
    raw = stream.getvalue()
    with zipfile.ZipFile(io.BytesIO(raw)) as target:
        assert target.testzip() is None
        for name, data in members: assert target.read(name) == data
    return raw


def bounds(points):
    return [min(p[i] for p in points) for i in range(3)], [max(p[i] for p in points) for i in range(3)]


def document(mode, stream, count, stride, expected):
    """One-tile glTF whose only compressed view is `stream`; the decoded view is the
    fallback buffer 1 (no URI, so the extension is required)."""
    view = dict(buffer=1, byteOffset=0, byteLength=count * stride,
                extensions={EXT: dict(buffer=0, byteOffset=0, byteLength=len(stream), byteStride=stride,
                                      count=count, mode=mode, filter='NONE')})
    doc = dict(asset=dict(version='2.0'), extensionsUsed=[EXT], extensionsRequired=[EXT],
               buffers=[None, dict(byteLength=count * stride, extensions={EXT: dict(fallback=True)})],
               bufferViews=[view], accessors=[], meshes=[], nodes=[dict(mesh=0)], scenes=[dict(nodes=[0])], scene=0)
    binary = stream + b'\0' * (-len(stream) % 4)
    if mode == 'ATTRIBUTES':
        # Decoded bytes are four little-endian float32 VEC3 POSITIONs (all finite).
        points = [struct.unpack_from('<3f', expected, 12 * i) for i in range(4)]
        low, high = bounds(points)
        doc['accessors'].append(dict(bufferView=0, componentType=5126, count=4, type='VEC3', min=low, max=high))
        doc['meshes'].append(dict(primitives=[dict(attributes=dict(POSITION=0), mode=0)]))
        vertices = 4
    else:
        # Plain POSITIONs after the stream: enough vertices for every decoded index.
        indices = struct.unpack('<%dI' % count, expected)
        vertices = (max(indices) + 1) if mode == 'TRIANGLES' else 3
        points = [((i % 4) * 0.5, (i // 4) * 0.5, 0.0) for i in range(vertices)]
        low, high = bounds(points)
        offset = len(binary)
        binary += b''.join(struct.pack('<3f', *p) for p in points)
        doc['bufferViews'].append(dict(buffer=0, byteOffset=offset, byteLength=12 * vertices))
        position = dict(bufferView=1, componentType=5126, count=vertices, type='VEC3', min=low, max=high)
        if mode == 'TRIANGLES':
            # Decoded view is the triangle list's UNSIGNED_INT indices.
            doc['accessors'] += [dict(bufferView=0, componentType=5125, count=count, type='SCALAR'), position]
            doc['meshes'].append(dict(primitives=[dict(attributes=dict(POSITION=1), indices=0, mode=4)]))
        else:
            # INDICES data that is not mesh topology (the text allows this): an unused
            # SCALAR float32 window over the decoded view; every value is finite.
            assert all(v == v and abs(v) < 1 for v in struct.unpack('<%df' % count, expected))
            doc['accessors'] += [position, dict(bufferView=0, componentType=5126, count=count, type='SCALAR')]
            doc['meshes'].append(dict(primitives=[dict(attributes=dict(POSITION=0), mode=0)]))
    doc['buffers'][0] = dict(byteLength=len(binary))
    return doc, binary, vertices


CHECKS = ['archiveIndex', 'archiveCrc', 'archiveStoredRecordLayout', 'contentHashes', 'tilesetSchema',
          'payloadEnvelope', 'bufferRanges', 'accessorValues', 'primitiveIndices', 'hierarchyBounds',
          'geometricErrorOrder', 'resourceReferences', 'recordedBudgets', 'decodedContentContainment']
NOT_INSPECTED = ['geometricErrorAccuracy', 'materialAndImageSemantics', 'metadata and feature semantics',
                 'metadataSemantics', 'skins, animations, morph targets and rendered extent']
LIMITS = dict(jsonBytes=8388608, jsonDepth=64, memberBytes=134217728, archiveStoredBytes=1073741824,
              archiveEntries=524288, accessorElements=4000000, documentDecodedBytes=134217728,
              hierarchyVisits=262144, hierarchyDepth=128, references=1048576, totalPayloadElements=16000000,
              totalBytesRead=2147483648, sourceArchiveBytes=1073741824, centralDirectoryBytes=33554432,
              documentItems=131072, containmentSpanBits=4096, containmentWorkUnits=536870912,
              containmentBytesRead=2147483648, containmentDecodedBytes=2147483648, containmentNodeDepth=64)


def generate():
    directory = HERE / 'fixtures'; directory.mkdir(exist_ok=False)
    refs = json.loads((PRIMARY / 'pins.json').read_text())
    for ref in refs:
        raw = (PRIMARY / ref['file']).read_bytes(); assert sha(raw) == ref['sha256'] and len(raw) == ref['bytes']
    headers = spec_headers((PRIMARY / 'EXT_meshopt_compression.md').read_text())
    source = (PRIMARY / 'meshopt_decoder.test.js').read_text()
    # (mode, upstream case, count, stride): one allowed-header and one other-version literal
    # per mode where upstream has both; INDICES has only the allowed literal.
    literals = [('ATTRIBUTES', 'decodeVertexBuffer', 4, 12), ('ATTRIBUTES', 'decodeVertexBufferV1', 4, 12),
                ('TRIANGLES', 'decodeIndexBufferV1_More', 12, 4), ('TRIANGLES', 'decodeIndexBuffer32', 12, 4),
                ('INDICES', 'decodeIndexSequence', 6, 4)]
    streams = []
    for mode, case, count, stride in literals:
        encoded, expected = upstream(source, case)
        assert len(expected) == count * stride
        streams.append((mode, case, encoded, count, stride, expected))
    allowed = {mode: (case, encoded, count, stride, expected)
               for mode, case, encoded, count, stride, expected in streams if encoded[0] == headers[mode]}
    assert sorted(allowed) == sorted(headers)
    # Header substitutions of the allowed literal: the other codec version in the same high
    # nibble (0xd0 has no upstream literal) and the next, unknown version.
    for mode, (case, encoded, count, stride, expected) in allowed.items():
        present = {s[2][0] for s in streams if s[0] == mode}
        for byte in sorted({headers[mode] ^ 1, (headers[mode] | 1) + 1} - present):
            streams.append((mode, case + '+header0x%02x' % byte, bytes([byte]) + encoded[1:], count, stride, expected))
    tileset = dict(asset=dict(version='1.1'), geometricError=0,
                   root=dict(boundingVolume=dict(box=[0, 0, 0, 10, 0, 0, 0, 10, 0, 0, 0, 10]), geometricError=0,
                             content=dict(uri='tile.glb')))
    rows = []
    for mode, case, encoded, count, stride, expected in streams:
        name = '%s-0x%02x' % (mode.lower(), encoded[0])
        admitted = encoded[0] == headers[mode]
        doc, binary, vertices = document(mode, encoded, count, stride, expected)
        members = [('tileset.json', packed(tileset)), ('tile.glb', glb(doc, binary))]
        raw = archive(members)
        (directory / name).mkdir(); (directory / name / 'input.3tz').write_bytes(raw)
        report = dict(ok=True, archive='$INPUT', tiles=1, contentReferences=1, entries=3, checks=CHECKS,
                      notInspected=NOT_INSPECTED, limits=LIMITS,
                      payloads=[dict(uri='tile.glb', accessorsChecked=len(doc['accessors']), primitivesChecked=1,
                                     vertices=vertices)])
        rows.append(dict(
            name=name, path=name + '/input.3tz', mode=mode, header='0x%02x' % encoded[0],
            specHeader='0x%02x' % headers[mode], upstreamCase=case,
            expectedCategory='admitted' if admitted else 'invalid_input', expectedExitCode=0 if admitted else 3,
            expectedReport=report if admitted else None,
            rationale='Appendix A requires header byte 0x%02x for %s; this stream starts with 0x%02x.'
                      % (headers[mode], mode, encoded[0]),
            streamSha256=sha(encoded), decodedSha256=sha(expected), sha256=sha(raw), bytes=len(raw),
            memberSha256={k: sha(v) for k, v in members}))
    manifest = dict(schemaVersion=1, status='Predictions frozen before target execution; not execution evidence',
                    rule='C1 H-1: first compressed byte must equal the EXT_meshopt_compression Appendix A header',
                    specHeaders={k: '0x%02x' % v for k, v in sorted(headers.items())},
                    generatorSha256=sha(Path(__file__).read_bytes()), runnerSha256=sha((HERE / 'runner.py').read_bytes()),
                    referencePins=refs, cases=rows)
    (directory / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    files = sorted(p for p in HERE.rglob('*') if p.is_file() and p.name not in ('integrity.json', 'README.md'))
    integrity = dict(status='Additive C1 meshopt stream-header lane; primary/ are relative symlinks to '
                            'validator_caps_amendment/2026-10-11/stride_controls/primary',
                     files={str(p.relative_to(HERE)): dict(bytes=p.stat().st_size, sha256=sha(p.read_bytes()))
                            for p in files})
    (HERE / 'integrity.json').write_text(json.dumps(integrity, indent=2) + '\n')


if __name__ == '__main__':
    generate()
