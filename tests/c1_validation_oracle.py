"""Independent C1 byte corpus. Uses only Python's standard library, no producer codecs.

--generate DIR writes fixtures/manifest; --binary BIN additionally exercises CLI.
Fixtures contain an ordinary ZIP written by zipfile and independently computed
3TZ records: MD5(name), little-endian local-header offset, sorted as two u64s.
"""
import argparse
import base64
import copy
import hashlib
import json
import pathlib
import struct
import subprocess
import zipfile
import zlib

CODES = {'invalid_input': 3, 'unsupported': 2, 'resource_limit': 3, 'io': 1}


def glb(document, binary, extra=b''):
    text = json.dumps(document, separators=(',', ':'), allow_nan=False).encode()
    text += b' ' * (-len(text) % 4)
    binary += b'\0' * (-len(binary) % 4)
    chunks = struct.pack('<I4s', len(text), b'JSON') + text + struct.pack('<I4s', len(binary), b'BIN\0') + binary + extra
    return struct.pack('<4sII', b'glTF', 2, 12 + len(chunks)) + chunks


def archive(path, files, index_offset=None):
    records = []
    with zipfile.ZipFile(path, 'w', compression=zipfile.ZIP_STORED) as target:
        for name, data in files.items():
            info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0)); info.external_attr = 0o100644 << 16
            target.writestr(info, data)
            records.append((hashlib.md5(name.encode()).digest(), target.getinfo(name).header_offset))
        records.sort(key=lambda pair: struct.unpack('<QQ', pair[0]))
        info = zipfile.ZipInfo('@3dtilesIndex1@', (1980, 1, 1, 0, 0, 0)); info.external_attr = 0o100644 << 16
        target.writestr(info, b''.join(h + struct.pack('<Q', offset if index_offset is None else index_offset) for h, offset in records))
    # Independent container check: record offsets really identify local headers.
    raw = path.read_bytes()
    for _, offset in records:
        assert raw[offset:offset + 4] == b'PK\x03\x04'



def framed_archive(files, zip64=False, descriptor=None, corrupt=None):
    """Literal stored ZIP layout; first member varies local/descriptor framing.

    PKWARE APPNOTE 6.3.10 §§4.3.7,4.3.9,4.4.7–9,4.5.3. Descriptor
    selector is None, 'signed' or 'unsigned'; ZIP64 uses sentinel size fields
    plus 0x0001 extra fields, with zero local extra sizes for streamed data.
    No project writer or reader establishes these expectations.
    """
    body = bytearray(); entries = []; records = []

    def member(name, data, first=False):
        offset = len(body); encoded = name.encode(); size = len(data)
        crc = zlib.crc32(data); wide = first and zip64
        streamed = first and descriptor is not None; flags = 8 if streamed else 0
        local_crc = 0 if streamed else crc
        local_sizes = [0, 0] if streamed else [size, size]
        if first and corrupt == 'local_crc': local_crc ^= 1
        if first and corrupt == 'local_compressed_size': local_sizes[1] += 1
        if first and corrupt == 'local_uncompressed_size': local_sizes[0] += 1
        local_extra = struct.pack('<HHQQ', 1, 16, *local_sizes) if wide else b''
        local_csize, local_usize = (0xffffffff, 0xffffffff) if wide else local_sizes[::-1]
        body.extend(struct.pack('<4s5H3I2H', b'PK\x03\x04', 45 if wide else 20,
                    flags, 0, 0, 33, local_crc, local_csize, local_usize, len(encoded), len(local_extra)))
        body.extend(encoded + local_extra + data)
        if streamed:
            values = [crc, size, size]
            if corrupt == 'descriptor_crc': values[0] ^= 1
            if corrupt == 'descriptor_compressed_size': values[1] += 1
            if corrupt == 'descriptor_uncompressed_size': values[2] += 1
            if descriptor == 'signed': body.extend(b'PK\x07\x08')
            body.extend(struct.pack('<IQQ' if wide else '<III', *values))
        central_extra = struct.pack('<HHQQ', 1, 16, size, size) if wide else b''
        declared_size = 0xffffffff if wide else size
        central = struct.pack('<4s6H3I5H2I', b'PK\x01\x02', 45 if wide else 20,
                    45 if wide else 20, flags, 0, 0, 33, crc, declared_size, declared_size,
                    len(encoded), len(central_extra), 0, 0, 0, 0o100644 << 16, offset)
        entries.append(central + encoded + central_extra)
        return offset

    for index, (name, data) in enumerate(files.items()):
        offset = member(name, data, index == 0)
        records.append((hashlib.md5(name.encode()).digest(), offset))
    records.sort(key=lambda pair: struct.unpack('<QQ', pair[0]))
    member('@3dtilesIndex1@', b''.join(h + struct.pack('<Q', off) for h, off in records))
    start = len(body); central = b''.join(entries); body.extend(central)
    body.extend(struct.pack('<4s4H2IH', b'PK\x05\x06', 0, 0, len(entries), len(entries), len(central), start, 0))
    return bytes(body)


def generate(directory):
    directory = pathlib.Path(directory); directory.mkdir(parents=True, exist_ok=True)
    document = {'asset': {'version': '2.0'}, 'scene': 0, 'scenes': [{'nodes': [0]}], 'nodes': [{'mesh': 0}],
                'buffers': [{'byteLength': 42}], 'bufferViews': [{'buffer': 0, 'byteLength': 36}, {'buffer': 0, 'byteOffset': 36, 'byteLength': 6}],
                'accessors': [{'bufferView': 0, 'componentType': 5126, 'count': 3, 'type': 'VEC3', 'min': [0, 0, 0], 'max': [1, 1, 0]},
                              {'bufferView': 1, 'componentType': 5123, 'count': 3, 'type': 'SCALAR'}],
                'meshes': [{'primitives': [{'attributes': {'POSITION': 0}, 'indices': 1, 'mode': 4}]}]}
    payload = struct.pack('<9f3H', 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, 2)
    tileset = {'asset': {'version': '1.1'}, 'geometricError': 0, 'root': {'boundingVolume': {'box': [0, 0, 0, 2000, 0, 0, 0, 2000, 0, 0, 0, 2000]},
               'geometricError': 0, 'content': {'uri': 'tile.glb'}}}
    cases = []

    def emit(name, data, kind, fact, extras=None, manifest=None):
        path = directory / (name + '.3tz')
        selected = manifest or tileset
        files = {'tileset.json': json.dumps(selected).encode(), selected['root']['content']['uri']: data}
        files.update(extras or {})
        archive(path, files)
        cases.append(dict(name=name, path=path.name, expectedKind=kind, exitCode=CODES.get(kind, 0),
                          analyticFact=fact, sha256=hashlib.sha256(path.read_bytes()).hexdigest()))

    emit('triangle', glb(document, payload), None, 'Three finite positions, indices 0,1,2; position bytes 36 and index bytes 6.')
    for name,value,kind in [('min_version_2_0','2.0',None),('min_version_2_1','2.1','unsupported'),
                            ('min_version_3_0','3.0','unsupported'),('min_version_bad_type',2,'invalid_input')]:
        minimum=copy.deepcopy(document);minimum['asset']['minVersion']=value
        emit(name,glb(minimum,payload),kind,'Independent asset.minVersion client compatibility gate: supported2.0, future2.1/3.0 Unsupported, nonstring malformed InvalidInput.')
    for name, mode, count in [('points', 0, 3), ('lines', 1, 2)]:
        doc = copy.deepcopy(document); doc['meshes'][0]['primitives'][0]['mode'] = mode; doc['accessors'][1]['count'] = count
        emit(name, glb(doc, payload), None, 'Core POINTS or LINES topology with valid finite positions and indices.')
    restart = copy.deepcopy(document)
    restart.update(extensionsUsed=['KHR_mesh_primitive_restart'], extensionsRequired=['KHR_mesh_primitive_restart'])
    restart['buffers'][0]['byteLength'] = 58
    restart['bufferViews'][0]['byteLength'] = 48
    restart['bufferViews'][1].update(byteOffset=48, byteLength=10)
    restart['accessors'][0].update(count=4, max=[1,1,0])
    restart['accessors'][1]['count'] = 5
    restart['meshes'][0]['primitives'][0]['mode'] = 3
    restart_positions = struct.pack('<12f',0,0,0,1,0,0,0,1,0,1,1,0)
    emit('primitive_restart_lines',glb(restart,restart_positions+struct.pack('<5H',0,1,65535,2,3)),None,
         'Pinned draft restart LINE_STRIP: U16 maximum splits two independently analytic segments 0-1 and 2-3; root declares used and required.')
    missing = copy.deepcopy(restart); missing.pop('extensionsRequired')
    emit('primitive_restart_not_required',glb(missing,restart_positions+struct.pack('<5H',0,1,65535,2,3)),'invalid_input',
         'Pinned draft explicitly requires extensionsRequired; extensionsUsed alone supplies no fallback.')
    emit('primitive_restart_ordinary_index_oob',glb(restart,restart_positions+struct.pack('<5H',0,1,65535,2,4)),'invalid_input',
         'Restart sentinel is allowed but ordinary vertex index four exceeds four positions.')
    for name, values in [('primitive_restart_empty_segment',[65535,0,1,2,3]),
                         ('primitive_restart_singleton_segment',[0,65535,1,2,3])]:
        emit(name,glb(restart,restart_positions+struct.pack('<5H',*values)),'unsupported',
             'Valid draft restart indices outside the admitted bounded LINE_STRIP profile: each segment must contain at least two vertices; no core-spec invalidity claimed.')
    doc = copy.deepcopy(document); doc['accessors'][0].update(min=[0, -999, -1], max=[1, -999, 0])
    emit('terrain', glb(doc, struct.pack('<9f3H', 0, -999, 0, 1, -999, 0, 0, -999, -1, 0, 1, 2)), None,
         'Finite negative-height triangle positions; no terrain producer or horizon proxy involved.')
    emit('unknown_chunk', glb(document, payload, struct.pack('<I4s', 4, b'TEST') + b'abcd'), None,
         'A complete aligned unknown GLB chunk follows BIN; glTF clients must ignore unknown chunk types.')
    doc = copy.deepcopy(document); doc['buffers'][0].update(uri='positions.bin')
    text = json.dumps(doc).encode(); local = copy.deepcopy(tileset); local['root']['content']['uri'] = 'tile.gltf'
    path = directory / 'external_bin.3tz'
    archive(path, {'tileset.json': json.dumps(local).encode(), 'tile.gltf': text, 'positions.bin': payload})
    cases.append(dict(name='external_bin', path=path.name, expectedKind=None, exitCode=0,
                      analyticFact='Local glTF and separately stored 42-byte binary resource.', sha256=hashlib.sha256(path.read_bytes()).hexdigest()))
    doc = copy.deepcopy(document)
    interleaved = b''.join(struct.pack('<3f4B', *position, 255, 100, 20, 255) for position in [(0, 0, 0), (1, 0, 0), (0, 1, 0)])
    doc['buffers'][0]['byteLength'] = 54; doc['bufferViews'][0].update(byteLength=48, byteStride=16); doc['bufferViews'][1]['byteOffset'] = 48
    doc['accessors'].append({'bufferView': 0, 'byteOffset': 12, 'componentType': 5121, 'normalized': True, 'count': 3, 'type': 'VEC4'})
    doc['meshes'][0]['primitives'][0]['attributes']['COLOR_0'] = 2
    emit('interleaved_normalized', glb(doc, interleaved + struct.pack('<3H', 0, 1, 2)), None,
         '16-byte records contain 12-byte FLOAT POSITION and 4-byte normalized UBYTE COLOR_0.')
    wrong = copy.deepcopy(doc); wrong['meshes'][0]['primitives'][0]['attributes']['COLOR_foo'] = wrong['meshes'][0]['primitives'][0]['attributes'].pop('COLOR_0')
    emit('color_set_noninteger',glb(wrong,interleaved+struct.pack('<3H',0,1,2)),'invalid_input',
         'COLOR_foo has a valid color accessor but its set suffix is not a nonnegative integer.')
    for suffix in ['01','1']:
        wrong = copy.deepcopy(doc); wrong['meshes'][0]['primitives'][0]['attributes']['COLOR_'+suffix] = wrong['meshes'][0]['primitives'][0]['attributes'].pop('COLOR_0')
        emit('color_set_'+suffix,glb(wrong,interleaved+struct.pack('<3H',0,1,2)),'invalid_input',
             'Published core indexed semantics prohibit leading zeroes and require sets to start at zero without gaps.')
    wrong = copy.deepcopy(document); wrong['buffers'][0]['byteLength']=68
    wrong['bufferViews'].append({'buffer':0,'byteOffset':44,'byteLength':24})
    wrong['accessors'].append({'bufferView':2,'componentType':5126,'count':3,'type':'VEC2'})
    wrong['meshes'][0]['primitives'][0]['attributes']['TEXCOORD_foo']=2
    emit('texcoord_set_noninteger',glb(wrong,payload+b'\0\0'+struct.pack('<6f',0,0,1,0,0,1)),'invalid_input',
         'TEXCOORD_foo has valid VEC2 FLOAT values but an invalid set suffix.')
    wrong=copy.deepcopy(document);wrong['buffers'][0]['byteLength']=40
    wrong['bufferViews']=[{'buffer':0,'byteLength':40}]
    wrong['accessors'][0]['byteOffset']=4
    wrong['accessors'][1].update(bufferView=0,componentType=5121)
    emit('mixed_index_vertex_view',glb(wrong,bytes([0,1,2,0])+payload[:36]),'invalid_input',
         'One 40-byte view contains UBYTE indices at offset zero and FLOAT vertex positions at offset four; view has no target or stride to reveal the illegal mixed roles.')
    wrong=copy.deepcopy(document);wrong['buffers'][0]['byteLength']=56
    wrong['bufferViews'].append({'buffer':0,'byteOffset':44,'byteLength':12})
    wrong['accessors'].append({'bufferView':2,'componentType':5125,'count':3,'type':'SCALAR'})
    wrong['meshes'][0]['primitives'][0]['attributes']['_CUSTOM']=2
    emit('custom_attribute_u32',glb(wrong,payload+b'\0\0'+struct.pack('<3I',1,2,3)),'invalid_input',
         'Published glTF core prohibits unsigned-int component types for application-specific vertex attributes.')
    doc = copy.deepcopy(document); doc['buffers'][0]['byteLength'] = 56
    doc['bufferViews'].append({'buffer': 0, 'byteOffset': 44, 'byteLength': 12})
    doc['accessors'].append({'bufferView': 2, 'componentType': 5121, 'count': 1, 'type': 'MAT3'})
    emit('matrix_padding', glb(doc, payload + b'\0\0' + bytes([1,2,3,0,4,5,6,0,7,8,9,0])), None,
         'UBYTE MAT3 columns start every four bytes; storage is 12 bytes, not 9.')
    trailing = copy.deepcopy(doc); trailing['buffers'][0]['byteLength']=55; trailing['bufferViews'][2]['byteLength']=11
    emit('matrix_final_padding_omitted',glb(trailing,payload+b'\0\0'+bytes([1,2,3,0,4,5,6,0,7,8,9])),None,
         'Published Khronos glTF 2.0 Data Alignment permits omitted trailing matrix padding when no further data follows; all nine UBYTE components fit in eleven bytes.')
    doc = copy.deepcopy(document); doc.update(extensionsUsed=['KHR_mesh_quantization'], extensionsRequired=['KHR_mesh_quantization'])
    doc['buffers'][0]['byteLength'] = 30; doc['bufferViews'][0].update(byteLength=24, byteStride=8); doc['bufferViews'][1]['byteOffset'] = 24
    doc['accessors'][0].update(componentType=5123, normalized=True, max=[65534,65534,0])
    emit('quantized_positions', glb(doc, struct.pack('<12H',0,0,0,0,65534,0,0,0,0,65534,0,0)+struct.pack('<3H',0,1,2)), None,
         'KHR_mesh_quantization normalized USHORT positions with separately aligned unsigned indices.')
    legacy = copy.deepcopy(tileset); legacy['root']['content']['uri'] = 'tile.b3dm'
    def b3dm(embedded, feature=b'{"BATCH_LENGTH":0}', feature_binary=b'', batch=b'', batch_binary=b''):
        feature += b' ' * (-(28+len(feature)) % 8)
        body = feature + feature_binary + batch + batch_binary + embedded
        body += b'\0' * (-(28+len(body)) % 8)
        return struct.pack('<4s6I',b'b3dm',1,28+len(body),len(feature),len(feature_binary),len(batch),len(batch_binary))+body
    wrapped=b3dm(glb(document,payload))
    emit('b3dm',wrapped,None,
         'Modern 28-byte b3dm v1 header; uint32 BATCH_LENGTH zero; tables and embedded GLB start eight-byte aligned; external tile padding zero.',manifest=legacy)
    emit('b3dm_short_header',b'b3dm','invalid_input','Four-byte magic without modern 28-byte header.',manifest=legacy)
    wrong=bytearray(wrapped);struct.pack_into('<I',wrong,8,len(wrong)+8)
    emit('b3dm_total_length',bytes(wrong),'invalid_input','Declared entire tile length exceeds actual bytes.',manifest=legacy)
    wrong=bytearray(wrapped);struct.pack_into('<I',wrong,12,0xfffffffc)
    emit('b3dm_table_overclaim',bytes(wrong),'invalid_input','Feature JSON claims an out-of-resource table range.',manifest=legacy)
    wrong=bytearray(wrapped);struct.pack_into('<I',wrong,12,struct.unpack_from('<I',wrong,12)[0]-1)
    emit('b3dm_table_alignment',bytes(wrong),'invalid_input','Feature JSON boundary moves the embedded GLB start off its eight-byte alignment.',manifest=legacy)
    wrong=bytearray(wrapped);struct.pack_into('<I',wrong,4,2)
    emit('b3dm_future_version',bytes(wrong),'unsupported','Future b3dm version is not the admitted modern version one.',manifest=legacy)
    emit('b3dm_missing_batch_length',b3dm(glb(document,payload),feature=b'{}'),'invalid_input',
         'Feature table is a JSON object but required BATCH_LENGTH is absent.',manifest=legacy)
    emit('b3dm_duplicate_feature_key',b3dm(glb(document,payload),feature=b'{"BATCH_LENGTH":0,"BATCH_LENGTH":0}'),'invalid_input',
         'Duplicate feature-table JSON keys must not silently overwrite.',manifest=legacy)
    emit('b3dm_batch_binary_without_json',b3dm(glb(document,payload),batch_binary=bytes(8)),'invalid_input',
         'Header declares batch binary bytes with zero batch JSON length.',manifest=legacy)
    wrong=bytearray(wrapped);embedded_offset=28+struct.unpack_from('<I',wrong,12)[0]
    struct.pack_into('<I',wrong,embedded_offset+8,0xfffffffc)
    emit('b3dm_embedded_glb_overclaim',bytes(wrong),'invalid_input','Embedded GLB length claims bytes outside its tile.',manifest=legacy)
    # Even if this particular GLB already ends on an eight-byte boundary,
    # append a full aligned block: trailing tile bytes must remain zero.
    wrong=bytearray(wrapped)+bytearray(8);wrong[-1]=1;struct.pack_into('<I',wrong,8,len(wrong))
    emit('b3dm_nonzero_tile_padding',bytes(wrong),'invalid_input','Tile padding outside the embedded GLB length contains a nonzero byte.',manifest=legacy)
    # MIT-licensed upstream golden stream and independently published decoded
    # bytes, not generated by rusty-tiles or its meshopt encoder.
    # https://github.com/zeux/meshoptimizer/blob/3d8e9b8a2a2b9a5becfc6fbc512307b04207ab5d/js/meshopt_decoder.test.js
    encoded = bytes.fromhex('a0013f0000005857580126000000010c00000058010800000000000000013f0000001718170126000000010c0000001701080000000000000000000000000000000000000000000000000000000000000000000000')
    decoded = bytes.fromhex('0000000000000000000000002c01000000000000f401000000002c01000000000000f4012c012c0100000000f401f401')
    points = list(struct.iter_unpack('<3f', decoded))
    meshopt = {'asset': {'version':'2.0'}, 'scene':0, 'scenes':[{'nodes':[0]}], 'nodes':[{'mesh':0}],
      'extensionsUsed':['EXT_meshopt_compression'], 'extensionsRequired':['EXT_meshopt_compression'],
      'buffers':[{'byteLength':len(encoded)}],
      'bufferViews':[{'buffer':0,'byteOffset':0,'byteLength':48,'byteStride':12,
        'extensions':{'EXT_meshopt_compression':{'buffer':0,'byteOffset':0,'byteLength':len(encoded),
          'byteStride':12,'count':4,'mode':'ATTRIBUTES','filter':'NONE'}}}],
      'accessors':[{'bufferView':0,'componentType':5126,'count':4,'type':'VEC3',
        'min':[min(p[i]for p in points)for i in range(3)],'max':[max(p[i]for p in points)for i in range(3)]}],
      'meshes':[{'primitives':[{'attributes':{'POSITION':0},'mode':0}]}]}
    emit('meshopt_none_golden', glb(meshopt,encoded), None,
         'Upstream pinned meshoptimizer golden ATTRIBUTES/NONE stream decodes to independently published 48 bytes (four finite FLOAT VEC3 points).')
    for name, extras in [('extras_nested_extensions', {'extensions': {'my_application_key': 'anything'}}),
                         ('extras_extensions_scalar', {'extensions': 42})]:
        doc = copy.deepcopy(document); doc['extras'] = extras
        emit(name, glb(doc, payload), None, 'Application-defined extras keys called extensions have no glTF extension semantics.')
    uv = copy.deepcopy(document); uv['extensionsUsed'] = ['KHR_mesh_quantization']
    uv['buffers'][0]['byteLength'] = 56
    uv['bufferViews'].append({'buffer': 0, 'byteOffset': 44, 'byteLength': 12})
    uv['accessors'].append({'bufferView': 2, 'componentType': 5122, 'type': 'VEC2', 'count': 3})
    uv['meshes'][0]['primitives'][0]['attributes']['TEXCOORD_0'] = 2
    for required in [True, False]:
        doc = copy.deepcopy(uv)
        if required: doc['extensionsRequired'] = ['KHR_mesh_quantization']
        emit('quantized_uv_required_' + str(required), glb(doc, payload + b'\0\0' + bytes(12)),
             None if required else 'invalid_input',
             'Signed SHORT VEC2 texture coordinates need KHR_mesh_quantization required even alongside ordinary FLOAT POSITION.')
    external_meshopt = copy.deepcopy(meshopt); external_meshopt['buffers'][0]['uri'] = 'data.bin'
    external_manifest = copy.deepcopy(tileset); external_manifest['root']['content']['uri'] = 'tile.gltf'
    def meshopt_case(name, doc, kind, fact):
        emit(name, json.dumps(doc).encode(), kind, fact, extras={'data.bin': encoded}, manifest=external_manifest)
    meshopt_case('meshopt_external_control', external_meshopt, None,
                 'Pinned compressed golden bytes supplied as an archive-local resource with exact declared size.')
    doc = copy.deepcopy(external_meshopt); doc['buffers'][0]['byteLength'] = 48
    meshopt_case('meshopt_compressed_over_declared', doc, 'invalid_input',
                 '85 compressed bytes exist physically but range exceeds declared buffer byteLength48.')
    for tagged in [True, False]:
        doc = copy.deepcopy(external_meshopt); placeholder = {'byteLength': 48}
        if tagged: placeholder['extensions'] = {'EXT_meshopt_compression': {'fallback': True}}
        doc['buffers'].append(placeholder); doc['bufferViews'][0]['buffer'] = 1
        meshopt_case('meshopt_fallback_one_' + str(tagged), doc, None,
                     'URI-less placeholder buffer1 is covered by a required compressed view; fallback tag is optional.')
    doc = copy.deepcopy(external_meshopt)
    doc['buffers'].insert(0, {'byteLength': 48, 'extensions': {'EXT_meshopt_compression': {'fallback': True}}})
    doc['bufferViews'][0]['extensions']['EXT_meshopt_compression']['buffer'] = 1
    meshopt_case('meshopt_fallback_zero', doc, None,
                 'Local glTF placeholder buffer0 is allowed; compressed stream resides in archive-local buffer1.')
    text = json.dumps(doc, separators=(',', ':')).encode(); text += b' ' * (-len(text) % 4)
    envelope = struct.pack('<4sII', b'glTF', 2, 20 + len(text)) + struct.pack('<I4s', len(text), b'JSON') + text
    emit('meshopt_glb_fallback_zero', envelope, None,
         'GLB has JSON-only framing and placeholder buffer0; required compressed stream is external buffer1.', extras={'data.bin': encoded})
    for name in ['uncovered', 'not_required']:
        broken = copy.deepcopy(doc)
        if name == 'uncovered': broken['bufferViews'].append({'buffer': 0, 'byteOffset': 0, 'byteLength': 4})
        else: broken.pop('extensionsRequired')
        meshopt_case('meshopt_placeholder_' + name, broken, 'invalid_input',
                     'Unavailable placeholder bytes cannot serve ordinary views or optional-only compressed fallback.')
    doc = copy.deepcopy(document)
    doc.update(extensionsUsed=['EXT_meshopt_compression'], extensionsRequired=['EXT_meshopt_compression'])
    doc['buffers'][0]['extensions'] = {'EXT_meshopt_compression': {'fallback': True}}
    emit('fallback_marker_uncompressed', glb(doc, payload), 'invalid_input',
         'Fallback-marked actual BIN buffer is referenced by ordinary uncompressed views, violating the marker role constraint.')
    doc = copy.deepcopy(external_meshopt)
    doc['buffers'][0]['extensions'] = {'EXT_meshopt_compression': {'fallback': True}}
    meshopt_case('fallback_marker_compressed_source', doc, 'invalid_input',
                 'Fallback-marked actual resource is referenced as a compressed source, which the fallback marker forbids.')
    doc = copy.deepcopy(external_meshopt)
    doc['buffers'][0]['extensions'] = {'EXT_meshopt_compression': {'fallback': 'true'}}
    meshopt_case('fallback_marker_bad_type', doc, 'invalid_input',
                 'EXT_meshopt_compression buffer fallback marker is a string rather than its defined boolean.')
    doc = copy.deepcopy(external_meshopt)
    doc['buffers'].append({'byteLength': len(decoded), 'uri': 'fallback.bin',
                           'extensions': {'EXT_meshopt_compression': {'fallback': True}}})
    doc['bufferViews'][0]['buffer'] = 1
    emit('fallback_marker_actual_resource', json.dumps(doc).encode(), None,
         'Fallback-marked actual decoded resource serves only a compressed view and is never used as a compressed source.',
         extras={'data.bin': encoded, 'fallback.bin': decoded}, manifest=external_manifest)
    doc = copy.deepcopy(external_meshopt); doc['buffers'].append({'byteLength': 48})
    meshopt_case('meshopt_unused_placeholder', doc, None,
                 'Required meshopt document has a URI-less unused placeholder; no references violate fallback coverage roles.')
    changes = [
        ('empty_bin', 'invalid_input', 'Declared 42-byte buffer and required POSITION payload absent.'),
        ('stride_one', 'invalid_input', 'POSITION stride 1 cannot hold a 12-byte element.'),
        ('offset_overflow', 'invalid_input', 'Accessor offset u64MAX cannot identify stored bytes.'),
        ('index_range', 'invalid_input', 'Index 65535 is outside three vertices.'),
        ('buffer_short', 'invalid_input', '36-byte position view exceeds declared one-byte buffer.'),
        ('nan_position', 'invalid_input', 'First position component is IEEE754 NaN.'),
        ('infinite_position', 'invalid_input', 'First position component is IEEE754 infinity.'),
        ('sparse', 'unsupported', 'Sparse accessors are deliberately outside C1 admission.'),
        ('data_uri', 'unsupported', 'Non-base64 data URI is outside bounded buffer admission.'),
        ('required_unknown', 'unsupported', 'Unknown required extension cannot be inspected.'),
        ('count_limit', 'resource_limit', 'POSITION count u64MAX exceeds four-million-element ceiling.'),
        ('count_sum_overflow', 'resource_limit', 'Two u64MAX counts must return a category instead of panic.'),
    ]
    for name, kind, fact in changes:
        doc = copy.deepcopy(document); data = payload
        if name == 'empty_bin': data = b''
        if name == 'stride_one': doc['bufferViews'][0]['byteStride'] = 1
        if name == 'offset_overflow': doc['accessors'][0]['byteOffset'] = 2**64 - 1
        if name == 'index_range': data = payload[:-2] + struct.pack('<H', 65535)
        if name == 'buffer_short': doc['buffers'][0]['byteLength'] = 1
        if name in ('nan_position','infinite_position'): data = struct.pack('<f', float('nan' if name == 'nan_position' else 'inf')) + payload[4:]
        if name == 'sparse': doc['accessors'][0]['sparse'] = {'count': 1, 'indices': {'bufferView': 1, 'componentType': 5123}, 'values': {'bufferView': 0}}
        if name == 'data_uri': doc['buffers'][0]['uri'] = 'data:application/octet-stream,raw'
        if name == 'required_unknown': doc.update(extensionsUsed=['UNKNOWN_required'], extensionsRequired=['UNKNOWN_required'])
        if name.startswith('count_'): doc['accessors'][0]['count'] = 2**64 - 1
        if name == 'count_sum_overflow': doc['meshes'][0]['primitives'] *= 2
        emit(name, glb(doc, data), kind, fact)
    for mime in ['application/octet-stream','application/gltf-buffer']:
        embedded=copy.deepcopy(document);embedded['buffers'][0]['uri']='data:'+mime+';base64,'+base64.b64encode(payload).decode()
        local=copy.deepcopy(tileset);local['root']['content']['uri']='tile.gltf'
        emit('buffer_base64_'+mime.split('/')[-1],json.dumps(embedded).encode(),None,
             'Independent inline base64 contains exactly the analytic 42-byte triangle buffer.',manifest=local)
    for name,encoded,declared in [('bad_base64','!!!',42),('missing_bytes','',42),
                                 ('length_mismatch',base64.b64encode(payload).decode(),43)]:
        embedded=copy.deepcopy(document);embedded['buffers'][0].update(uri='data:application/octet-stream;base64,'+encoded,byteLength=declared)
        emit('buffer_base64_'+name,glb(embedded,b''),'invalid_input','Inline buffer base64 syntax or decoded byte length fails independently known 42-byte buffer requirements.')
    good = glb(document, payload)
    for name,uri in [('remote','https://example.invalid/positions.bin'),('absolute','/positions.bin'),
                     ('percent','positions%2ebin'),('query','positions.bin?x=1'),('fragment','positions.bin#part')]:
        doc=copy.deepcopy(document);doc['buffers'][0]['uri']=uri
        emit('buffer_uri_'+name,glb(doc,payload),'unsupported','Syntactically meaningful non-local/non-plain URI is outside the archive-local resource profile.')
    for name,uri in [('empty',''),('backslash','a\\positions.bin'),('escape','../positions.bin')]:
        doc=copy.deepcopy(document);doc['buffers'][0]['uri']=uri
        emit('buffer_uri_'+name,glb(doc,payload),'invalid_input','Malformed or archive-escaping URI must fail as invalid input.')
    duplicated=json.dumps(tileset).replace('"geometricError": 0,','"geometricError":0,"geometricError":0,',1).encode()
    emit('tileset_duplicate_key',good,'invalid_input','Root tileset JSON repeats geometricError; bounded unique parser must reject it.',extras={'tileset.json':duplicated})
    emit('truncated_header', good[:10], 'invalid_input', 'GLB header contains only ten bytes.')
    damaged = bytearray(good); offset = 20 + struct.unpack_from('<I', damaged, 12)[0]
    struct.pack_into('<I', damaged, offset, 4294967292)
    emit('bin_overclaim', bytes(damaged), 'invalid_input', 'BIN chunk claims ~4GiB but contains 44 bytes.')
    duplicate = glb(document, payload, struct.pack('<I4s', 0, b'BIN\0'))
    emit('duplicate_bin', duplicate, 'invalid_input', 'Two BIN chunks are not a permitted GLB layout.')
    padding = bytearray(good); padding[-1] = 1
    emit('bin_nonzero_padding', bytes(padding), 'invalid_input', 'The two BIN padding bytes must be zero; final byte is one.')
    doc = copy.deepcopy(document); doc['buffers'][0]['byteLength'] = 53
    doc['bufferViews'].append({'buffer':0,'byteOffset':44,'byteLength':9})
    doc['accessors'].append({'bufferView':2,'componentType':5121,'count':1,'type':'MAT3'})
    emit('matrix_missing_padding', glb(doc,payload+b'\0\0'+bytes(range(9))), 'invalid_input',
         'UBYTE MAT3 requires column padding and twelve-byte storage, but only nine bytes are provided.')
    doc = copy.deepcopy(document); doc['bufferViews'][0]['extensions'] = {'EXT_meshopt_compression':
         {'buffer': 0, 'byteOffset': 0, 'byteLength': 1, 'byteStride': 12, 'count': 2**64-1, 'mode': 'ATTRIBUTES', 'filter': 'NONE'}}
    doc.update(extensionsUsed=['EXT_meshopt_compression'], extensionsRequired=['EXT_meshopt_compression'])
    emit('meshopt_count_limit', glb(doc, payload), 'resource_limit', 'Meshopt decoded count exceeds cap before decode allocation.')
    implicit = {'asset':{'version':'1.1'},'geometricError':0,'root':{'boundingVolume':{'box':[0,0,0,1,0,0,0,1,0,0,0,1]},'geometricError':0,
                'implicitTiling':{'subdivisionScheme':'QUADTREE','subtreeLevels':1,'availableLevels':1,'subtrees':{'uri':'{level}-{x}-{y}.subtree'}}}}
    subtree_text = b'{"tileAvailability":{"constant":1},"contentAvailability":[],"childSubtreeAvailability":{"constant":0}}'
    for name,text,kind in [('implicit_no_content',subtree_text,None),
                           ('implicit_duplicate_subtree_key',subtree_text.replace(b'"constant":1',b'"constant":1,"constant":1'),'invalid_input')]:
        padded=text+b' '*(-len(text)%8)
        data=struct.pack('<4sIQQ',b'subt',1,len(padded),0)+padded
        path=directory/(name+'.3tz');archive(path,{'tileset.json':json.dumps(implicit).encode(),'0-0-0.subtree':data})
        cases.append(dict(name=name,path=path.name,expectedKind=kind,exitCode=CODES.get(kind,0),
                          analyticFact='Independent binary subtree header, one constant-available tile, zero content slots and unavailable child subtrees; duplicate-key variant invalid.',sha256=hashlib.sha256(path.read_bytes()).hexdigest()))
    polygon = copy.deepcopy(document);polygon['extensionsUsed']=['EXT_mesh_polygon']
    polygon['buffers'][0]['byteLength']=64
    polygon['bufferViews'].append({'buffer':0,'byteOffset':44,'byteLength':20})
    for offset,count in [(0,1),(4,3),(16,1)]:
        polygon['accessors'].append({'bufferView':2,'byteOffset':offset,'componentType':5125,'count':count,'type':'SCALAR'})
    polygon['meshes'][0]['primitives'][0]['extensions']={'EXT_mesh_polygon':{'count':1,'indicesOffsets':2,'loopIndices':3,'loopIndicesOffsets':4}}
    polygon_data=payload+b'\0\0'+struct.pack('<5I',0,0,1,2,0)
    emit('polygon_optional_tables',glb(polygon,polygon_data),None,'Optional polygon metadata references three bounded unsigned SCALAR accessors; full polygon topology is deliberately not inspected.')
    broken=copy.deepcopy(polygon);broken['meshes'][0]['primitives'][0]['extensions']['EXT_mesh_polygon']['indicesOffsets']=99
    emit('polygon_accessor_reference_oob',glb(broken,polygon_data),'invalid_input','Optional polygon accessor reference99 exceeds five accessors.')
    broken=copy.deepcopy(polygon);broken['meshes'][0]['primitives'][0]['extensions']['EXT_mesh_polygon']['indicesOffsets']=0
    emit('polygon_accessor_wrong_scalar',glb(broken,polygon_data),'invalid_input','Polygon offset reference targets FLOAT VEC3 POSITION rather than unsigned SCALAR.')
    def container_case(name, data, kind, fact):
        path=directory/(name+'.3tz');path.write_bytes(data)
        cases.append(dict(name=name,path=path.name,expectedKind=kind,exitCode=CODES.get(kind,0),analyticFact=fact,
                          sha256=hashlib.sha256(data).hexdigest()))
    path=directory/'index_offset_max.3tz'
    archive(path,{'tileset.json':json.dumps(tileset).encode(),'tile.glb':good},index_offset=2**64-1)
    container_case('index_offset_max',path.read_bytes(),'invalid_input','Valid stored ZIP and CRC, but every 3TZ index offset is u64MAX outside the file.')
    raw=bytearray((directory/'triangle.3tz').read_bytes()); central=raw.index(b'PK\x01\x02')
    struct.pack_into('<I',raw,central+42,0xffffffff)
    container_case('zip_local_offset_sentinel',bytes(raw),'invalid_input','Central directory local offset is ZIP64 sentinel without required ZIP64 extra field.')
    raw=(directory/'triangle.3tz').read_bytes(); end=raw.rindex(b'PK\x05\x06')
    size,offset=struct.unpack_from('<II',raw,end+12)
    record=struct.pack('<4sQ2H2I4Q',b'PK\x06\x06',44,45,45,0,0,2**64-1,2**64-1,size,offset)
    locator=struct.pack('<4sIQI',b'PK\x06\x07',0,end,1)
    ending=struct.pack('<4s4H2IH',b'PK\x05\x06',0,0,65535,65535,0xffffffff,0xffffffff,0)
    container_case('zip64_count_max',raw[:end]+record+locator+ending,'resource_limit',
                   'ZIP64 claims u64MAX entries; admission must reject before directory allocation.')
    zip_files = {'tileset.json': json.dumps(tileset).encode(), 'tile.glb': good}
    for field in ['crc', 'compressed_size', 'uncompressed_size']:
        container_case('zip_local_' + field + '_mismatch',
                       framed_archive(zip_files, corrupt='local_' + field), 'invalid_input',
                       'No descriptor: local ' + field + ' differs from central directory while member bytes and index remain correct.')
    container_case('zip_local_zip64', framed_archive(zip_files, zip64=True), None,
                   'Small stored member elects ZIP64 local and central size fields; local 0x0001 original/compressed sizes match actual bytes.')
    for field in ['compressed_size', 'uncompressed_size']:
        container_case('zip_local_zip64_' + field + '_mismatch',
                       framed_archive(zip_files, zip64=True, corrupt='local_' + field), 'invalid_input',
                       'Local ZIP64 ' + field + ' differs from central size; no descriptor or actual payload corruption.')
    for wide in [False, True]:
        for signature in ['signed', 'unsigned']:
            name = 'zip_descriptor_' + ('64_' if wide else '32_') + signature
            container_case(name, framed_archive(zip_files, zip64=wide, descriptor=signature), None,
                           'Stored bit3 member: zero local CRC/sizes, correct immediate ' + ('ZIP64' if wide else '32-bit') +
                           ' data descriptor ' + ('with' if signature == 'signed' else 'without') + ' optional signature; central/index unchanged in meaning.')
            for field in ['crc', 'compressed_size', 'uncompressed_size']:
                container_case(name + '_' + field + '_mismatch',
                               framed_archive(zip_files, zip64=wide, descriptor=signature, corrupt='descriptor_' + field),
                               'invalid_input', 'Immediate descriptor ' + field + ' contradicts actual data and central directory.')
    for field in ['crc', 'compressed_size', 'uncompressed_size']:
        container_case('zip_descriptor_32_local_' + field + '_nonzero',
                       framed_archive(zip_files, descriptor='signed', corrupt='local_' + field), 'invalid_input',
                       'Bit3 ordinary local ' + field + ' is nonzero instead of its defined zero placeholder, despite a correct descriptor.')
    container_case('zip_descriptor_64_local_crc_nonzero',
                   framed_archive(zip_files, zip64=True, descriptor='signed', corrupt='local_crc'), 'invalid_input',
                   'Bit3 local CRC must be zero even when size fields use ZIP64 sentinels and a correct descriptor follows.')
    known_sizes = bytearray(framed_archive(zip_files, zip64=True, descriptor='signed'))
    local_extra_values = 30 + len('tileset.json') + 4
    struct.pack_into('<QQ', known_sizes, local_extra_values, len(zip_files['tileset.json']), len(zip_files['tileset.json']))
    container_case('zip_descriptor_64_known_local_sizes', bytes(known_sizes), None,
                   'ZIP64 local extra sizes are known and agree with descriptor/central; ordinary fields retain sentinels and CRC remains zero.')
    for field in ['compressed_size', 'uncompressed_size']:
        container_case('zip_descriptor_64_local_' + field + '_mismatch',
                       framed_archive(zip_files, zip64=True, descriptor='signed', corrupt='local_' + field), 'invalid_input',
                       'ZIP64 local extra ' + field + ' is neither zero nor the correct descriptor/central size.')
    # Four literal bytes analytically selected so CRC equals descriptor signature.
    # Unsigned descriptor must not lose its CRC to an assumed signature prefix.
    signature_crc_bytes = bytes.fromhex('ac0a7ad5')
    assert zlib.crc32(signature_crc_bytes) == 0x08074b50
    signature_doc = {'asset': {'version': '2.0'}, 'buffers': [{'uri': 'data.bin', 'byteLength': 4}]}
    signature_tileset = copy.deepcopy(tileset); signature_tileset['root']['content']['uri'] = 'tile.gltf'
    signature_files = {'data.bin': signature_crc_bytes, 'tileset.json': json.dumps(signature_tileset).encode(),
                       'tile.gltf': json.dumps(signature_doc).encode()}
    for wide in [False, True]:
        container_case('zip_descriptor_' + ('64' if wide else '32') + '_crc_equals_signature',
                       framed_archive(signature_files, zip64=wide, descriptor='unsigned'), None,
                       'Signatureless descriptor CRC equals 0x08074b50; four stored bytes ac0a7ad5 have that independently checked CRC.')
    manifest = {'schemaVersion': 1, 'generator': 'Independent standard-library glTF/GLB/ZIP/3TZ byte fixtures', 'cases': cases}
    (directory / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    return manifest


def snapshot(directory):
    return {str(p.relative_to(directory)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in pathlib.Path(directory).rglob('*') if p.is_file()}


def check_cli(binary, directory, manifest):
    before = snapshot(directory); results = []
    for case in manifest['cases']:
        result = subprocess.run([str(binary), 'validate', str(directory / case['path']), '--json'],
                                capture_output=True, text=True, timeout=20)
        report = json.loads(result.stdout)
        assert result.returncode == case['exitCode'], (case['name'], result.returncode, report, result.stderr)
        if case['expectedKind']:
            assert report['error']['code'] == case['expectedKind'], (case['name'], report)
        else: assert report['ok'] is True
        results.append({'name': case['name'], 'exitCode': result.returncode, 'report': report})
    assert snapshot(directory) == before, 'Validation modified fixture inputs'
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__); parser.add_argument('--generate', type=pathlib.Path, required=True)
    parser.add_argument('--binary', type=pathlib.Path); parser.add_argument('--output', type=pathlib.Path)
    args = parser.parse_args(); manifest = generate(args.generate)
    if args.binary:
        result = {'binarySha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(),
                  'cases': check_cli(args.binary, args.generate, manifest), 'readOnly': True}
        if args.output: args.output.write_text(json.dumps(result, indent=2)+'\n')
        else: print(json.dumps(result, indent=2))
