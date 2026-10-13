#!/usr/bin/env python3
"""Generate KHR_texture_basisu / KTX2 controls with the standard library only.

Never invokes project code or the target. Each case declares the rule it is built to
break (or none) and the category the pinned texts give; `ktx2_oracle` independently
recomputes the verdict from the bytes and the generator refuses any disagreement.
KTX2 sources are pinned upstream KTX-Software reference files, one real in-repo encoder
output (H8 quality leaf) and containers packed here from the KTX 2.0 layout; negatives
are byte patches of those. The archives are generated on demand (`generate`) and
frozen by hash in fixtures/manifest.json (`python3 -I generator.py --freeze`).
"""
import hashlib, io, json, struct, sys, zipfile, zlib
from pathlib import Path

HERE = Path(__file__).resolve().parent
PRIMARY = HERE / 'primary'
sys.path.insert(0, str(HERE))
import ktx2_oracle as oracle  # noqa: E402

EXT = oracle.EXT


def sha(raw): return hashlib.sha256(raw).hexdigest()
def packed(obj): return json.dumps(obj, separators=(',', ':'), allow_nan=False).encode()
def primary(name): return (PRIMARY / name).read_bytes()


def put32(raw, at, value): raw = bytearray(raw); struct.pack_into('<I', raw, at, value); return bytes(raw)
def put64(raw, at, value): raw = bytearray(raw); struct.pack_into('<Q', raw, at, value); return bytes(raw)
def get32(raw, at): return struct.unpack_from('<I', raw, at)[0]
def get64(raw, at): return struct.unpack_from('<Q', raw, at)[0]
def level_entry(p): return 80 + 24 * p


def pack_uastc(width, height, levels, orientation=b'rd'):
    """An unsupercompressed UASTC RGB container laid out per KTX 2.0 "Basic Structure":
    header, level index, one basic DFD block, one KTXorientation pair, then levels
    smallest first, each 16-aligned. Block bytes are a fixed pattern: C1 does not decode
    them."""
    dfd = struct.pack('<IIIBBBB4B8BIIII', 44, 0, 2 | (40 << 16), 166, 1, 2, 0, 3, 3, 0, 0,
                      16, 0, 0, 0, 0, 0, 0, 0, (0 << 24) | (127 << 16), 0, 0, 0xFFFFFFFF)
    pair = b'KTXorientation\0' + orientation + b'\0'
    kvd = struct.pack('<I', len(pair)) + pair + b'\0' * (-len(pair) % 4)
    dfd_offset = 80 + 24 * levels
    kvd_offset = dfd_offset + len(dfd)
    cursor = kvd_offset + len(kvd)
    body, index = b'', {}
    for p in reversed(range(levels)):
        start = -(-cursor // 16) * 16
        blocks = -(-max(1, width >> p) // 4) * -(-max(1, height >> p) // 4)
        data = bytes((p * 7 + i) % 251 for i in range(16 * blocks))
        body += b'\0' * (start - cursor) + data
        index[p] = (start, len(data), len(data))
        cursor = start + len(data)
    header = oracle.IDENTIFIER + struct.pack('<9I', 0, 1, width, height, 0, 0, 1, levels, 0)
    header += struct.pack('<4IQQ', dfd_offset, len(dfd), kvd_offset, len(kvd), 0, 0)
    header += b''.join(struct.pack('<QQQ', *index[p]) for p in range(levels))
    return header + dfd + kvd + body


def png_rgba_4x4():
    """A 4x4 RGBA8 PNG for the fallback `source` (core image; C1 does not decode it)."""
    def chunk(kind, data): return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    rows = b''.join(b'\0' + bytes([200, 100, 50, 255]) * 4 for _ in range(4))
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 4, 4, 8, 6, 0, 0, 0)) + \
        chunk(b'IDAT', zlib.compress(rows, 9)) + chunk(b'IEND', b'')


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


TILESET = dict(asset=dict(version='1.1'), geometricError=0,
               root=dict(boundingVolume=dict(box=[0, 0, 0, 10, 0, 0, 0, 10, 0, 0, 0, 10]), geometricError=0,
                         content=dict(uri='tile.glb')))


def tile(images, textures, material_texture=True, used=(EXT,), required=(EXT,)):
    """One-triangle GLB. `images` are (bytes, image fields); fields without `uri` are stored
    in a bufferView. Returns (glb bytes, extra archive members, document, image bytes)."""
    binary = struct.pack('<9f', 0, 0, 0, 1, 0, 0, 0, 1, 0)
    views = [dict(buffer=0, byteOffset=0, byteLength=36)]
    doc_images, extra = [], []
    for data, fields in images:
        image = dict(fields)
        if 'uri' in image:
            extra.append((image['uri'], data))
        else:
            binary += b'\0' * (-len(binary) % 4)
            views.append(dict(buffer=0, byteOffset=len(binary), byteLength=len(data)))
            image['bufferView'] = len(views) - 1
            binary += data
        doc_images.append(image)
    material = dict(pbrMetallicRoughness=dict(baseColorTexture=dict(index=0))) if material_texture else dict()
    doc = dict(asset=dict(version='2.0'), buffers=[dict(byteLength=len(binary))], bufferViews=views,
               accessors=[dict(bufferView=0, componentType=5126, count=3, type='VEC3', min=[0, 0, 0], max=[1, 1, 0])],
               meshes=[dict(primitives=[dict(attributes=dict(POSITION=0), material=0, mode=4)])],
               materials=[material], textures=textures, images=doc_images,
               samplers=[dict(magFilter=9729, minFilter=9987)],
               nodes=[dict(mesh=0)], scenes=[dict(nodes=[0])], scene=0)
    if used: doc['extensionsUsed'] = list(used)
    if required: doc['extensionsRequired'] = list(required)
    return glb(doc, binary), extra, doc, [data for data, _ in images]


KTX2 = {'mimeType': 'image/ktx2'}
BASISU = [{'sampler': 0, 'extensions': {EXT: {'source': 0}}}]


def single(ktx2, material_texture=True):
    return tile([(ktx2, KTX2)], BASISU, material_texture)


def cases():
    """(name, source, intended rule or None, intended category, tile tuple)."""
    alpha = primary('alpha_simple_basis.ktx2')          # ETC1S RGB+AAA 8x8, BasisLZ
    mips = primary('rgba-mipmap-reference-basis.ktx2')  # ETC1S RGB 64x64, 7 levels
    rg = primary('rg_reference_basis.ktx2')             # ETC1S RRR+GGG, KTXswizzle ra01
    cyan = primary('cyan_rgb_reference_uastc.ktx2')     # UASTC RGB 64x64, scheme 0
    document = primary('ktx_document_uastc_rdo4_zstd5.ktx2')  # UASTC RGBA 1024^2, Zstd, 11 levels
    leaf = primary('h8-quality-leaf4-uastc-zstd.ktx2')  # in-repo encoder, UASTC RGB 128x256, Zstd, 9 levels
    synthetic = pack_uastc(12, 8, 4)
    png = png_rgba_4x4()
    A, I, U = 'admitted', oracle.INVALID, oracle.UNSUPPORTED
    dfd = lambda raw: get32(raw, 48)
    rows = [
        ('etc1s-rgba-upstream', 'KTX-Software alpha_simple_basis', None, A, single(alpha)),
        ('etc1s-mips-upstream', 'KTX-Software rgba-mipmap-reference-basis', None, A, single(mips)),
        ('uastc-none-upstream', 'KTX-Software cyan_rgb_reference_uastc', None, A, single(cyan)),
        ('uastc-zstd-mips-upstream', 'KTX-Software ktx_document_uastc_rdo4_zstd5', None, A, single(document)),
        ('uastc-zstd-h8-encoder', 'H8 quality leaf (in-repo encoder)', None, A, single(leaf)),
        ('uastc-none-packed-mips', 'packed 12x8, 4 levels', None, A, single(synthetic)),
        ('fallback-png-not-required', 'alpha_simple + PNG fallback', None, A,
         tile([(alpha, KTX2), (png, {'mimeType': 'image/png'})],
              [{'source': 1, 'extensions': {EXT: {'source': 0}}}], required=())),
        ('external-ktx2-uri', 'cyan as tex.ktx2 member, no mimeType', None, A,
         tile([(cyan, {'uri': 'tex.ktx2'})], BASISU)),
        ('non-material-2d', 'cyan, texture unused by the material', None, A, single(cyan, False)),
        # Container integrity.
        ('truncated-header', 'alpha_simple[:60]', 'header-truncated', I, single(alpha[:60])),
        ('truncated-base-level', 'cyan minus its last byte', 'level-range', I, single(cyan[:-1])),
        ('truncated-encoder-output', 'H8 leaf minus 1000 bytes', 'level-range', I, single(leaf[:-1000])),
        ('bad-identifier', 'alpha_simple with "KTX 10"', 'identifier', I,
         single(alpha[:5] + b'1' + alpha[6:])),
        ('level-past-eof', 'cyan level 0 byteLength + 16', 'level-range', I,
         single(put64(cyan, level_entry(0) + 8, len(cyan)))),
        ('hostile-level-offset', 'cyan level 0 byteOffset 2^63', 'level-range', I,
         single(put64(cyan, level_entry(0), 1 << 63))),
        ('hostile-level-count', 'cyan levelCount 0xffffffff', 'level-count', I,
         single(put32(cyan, 40, 0xFFFFFFFF))),
        ('overlapping-levels', 'ktx_document level 1 at level 2 offset', 'level-offset', I,
         single(put64(document, level_entry(1), get64(document, level_entry(2))))),
        ('level-count-zero', 'cyan levelCount 0', 'level-count', I, single(put32(cyan, 40, 0))),
        ('level-count-over-dimensions', 'rgba-mipmap at 32x32 with 7 levels', 'level-count', I,
         single(put32(put32(mips, 20, 32), 24, 32))),
        ('not-multiple-of-4', 'packed 6x8, 1 level', 'multiple-of-4', I, single(pack_uastc(6, 8, 1))),
        ('uastc-zlib-scheme', 'H8 leaf scheme 3', 'scheme', I, single(put32(leaf, 44, 3))),
        ('uastc-vendor-scheme', 'H8 leaf scheme 0x10000', 'scheme', I, single(put32(leaf, 44, 0x10000))),
        ('etc1s-zstd-scheme', 'alpha_simple scheme 2', 'model-scheme', I, single(put32(alpha, 44, 2))),
        ('uastc-basislz-scheme', 'H8 leaf scheme 1', 'model-scheme', I, single(put32(leaf, 44, 1))),
        ('not-basis-color-model', 'cyan DFD colorModel RGBSDA', 'color-model', I,
         single(cyan[:dfd(cyan) + 12] + b'\x01' + cyan[dfd(cyan) + 13:])),
        ('texel-block-6x4', 'cyan DFD texelBlockDimension0 5', 'texel-block', I,
         single(cyan[:dfd(cyan) + 16] + b'\x05' + cyan[dfd(cyan) + 17:])),
        ('vk-format-defined', 'cyan vkFormat 43', 'vkformat', I, single(put32(cyan, 12, 43))),
        ('uastc-rrrg-channel', 'cyan channel UASTC_RRRG', 'channels', I,
         single(cyan[:dfd(cyan) + 31] + b'\x05' + cyan[dfd(cyan) + 32:])),
        ('etc1s-swizzle-upstream', 'KTX-Software rg_reference_basis (KTXswizzle ra01)', 'kvd-metadata', I,
         single(rg)),
        ('orientation-ru', 'alpha_simple KTXorientation ru', 'kvd-metadata', I,
         single(alpha.replace(b'KTXorientation\0rd', b'KTXorientation\0ru'))),
        ('kvd-pair-length', 'cyan first keyAndValueByteLength 1', 'kvd-pairs', I,
         single(put32(cyan, get32(cyan, 56), 1))),
        ('dfd-total-size', 'cyan dfdTotalSize + 4', 'dfd-size', I,
         single(put32(cyan, dfd(cyan), get32(cyan, dfd(cyan)) + 4))),
        ('dfd-misplaced', 'cyan dfdByteOffset + 4', 'dfd-position', I, single(put32(cyan, 48, dfd(cyan) + 4))),
        ('uncompressed-length', 'H8 leaf level 0 uncompressedByteLength + 16', 'level-lengths', I,
         single(put64(leaf, level_entry(0) + 16, get64(leaf, level_entry(0) + 16) + 16))),
        ('trailing-bytes', 'cyan + 16 zero bytes', 'trailing-bytes', I, single(cyan + b'\0' * 16)),
        ('basislz-slice-past-level', 'alpha_simple rgbSliceByteLength over its level', 'image-descriptor', I,
         single(put32(alpha, get64(alpha, 64) + 20 + 8, get64(alpha, level_entry(0) + 8) + 1))),
        ('basislz-global-length', 'alpha_simple tablesByteLength + 1', 'basislz-length', I,
         single(put32(alpha, get64(alpha, 64) + 12, get32(alpha, get64(alpha, 64) + 12) + 1))),
        ('global-data-for-uastc', 'cyan sgdByteOffset 8 without global data', 'global-data', I,
         single(put64(cyan, 64, 8))),
        ('non-2d-material', 'cyan layerCount 1, material map', 'not-2d', I, single(put32(cyan, 32, 1))),
        ('non-2d-no-material', 'cyan layerCount 1, no material use', 'not-2d', U,
         single(put32(cyan, 32, 1), False)),
        # Texture and image rules.
        ('mime-png', 'cyan declared image/png', 'mime-type', I,
         tile([(cyan, {'mimeType': 'image/png'})], BASISU)),
        ('mime-missing-buffer-view', 'cyan in a bufferView without mimeType', 'mime-type', I,
         tile([(cyan, {})], BASISU)),
        ('uri-png-suffix', 'cyan as tex.png member, no mimeType', 'mime-type', I,
         tile([(cyan, {'uri': 'tex.png'})], BASISU)),
        ('fallback-is-ktx2', 'core source and extension source both the KTX2 image', 'fallback-is-ktx2', I,
         tile([(cyan, KTX2)], [{'source': 0, 'extensions': {EXT: {'source': 0}}}], required=())),
        ('no-fallback-not-required', 'no fallback, extension only in extensionsUsed', 'fallback-required', I,
         tile([(cyan, KTX2)], BASISU, required=())),
        ('undeclared-extension', 'extension object, PNG fallback, no extensionsUsed', 'undeclared-extension', I,
         tile([(cyan, KTX2), (png, {'mimeType': 'image/png'})],
              [{'source': 1, 'extensions': {EXT: {'source': 0}}}], used=(), required=())),
    ]
    return rows


CHECKS = ['archiveIndex', 'archiveCrc', 'archiveStoredRecordLayout', 'contentHashes', 'tilesetSchema',
          'payloadEnvelope', 'bufferRanges', 'accessorValues', 'primitiveIndices', 'hierarchyBounds',
          'geometricErrorOrder', 'resourceReferences', 'recordedBudgets', 'decodedContentContainment']
NOT_INSPECTED = sorted(['geometricErrorAccuracy', 'materialAndImageSemantics', 'metadata and feature semantics',
                        'metadataSemantics', 'skins, animations, morph targets and rendered extent',
                        'KTX2 level data and transcoding', 'image decoding'])
LIMITS = dict(jsonBytes=8388608, jsonDepth=64, memberBytes=134217728, archiveStoredBytes=1073741824,
              archiveEntries=524288, accessorElements=4000000, documentDecodedBytes=134217728,
              hierarchyVisits=262144, hierarchyDepth=128, references=1048576, totalPayloadElements=16000000,
              totalBytesRead=2147483648, sourceArchiveBytes=1073741824, centralDirectoryBytes=33554432,
              documentItems=131072, containmentSpanBits=4096, containmentWorkUnits=536870912,
              containmentBytesRead=2147483648, containmentDecodedBytes=2147483648, containmentNodeDepth=64)


def generate(directory, check_oracle=oracle):
    """Write every case archive under `directory`; return the manifest rows."""
    directory.mkdir(parents=True, exist_ok=False)
    rows = []
    for name, source, rule, expected, (payload, extra, doc, image_bytes) in cases():
        violations = check_oracle.gltf_violations(doc, image_bytes)
        verdict = check_oracle.category(violations)
        members = [('tileset.json', packed(TILESET)), ('tile.glb', payload)] + extra
        raw = archive(members)
        (directory / name).mkdir(); (directory / name / 'input.3tz').write_bytes(raw)
        report = dict(ok=True, archive='$INPUT', tiles=1, contentReferences=1, entries=len(members) + 1,
                      checks=CHECKS, notInspected=NOT_INSPECTED, limits=LIMITS,
                      payloads=[dict(uri='tile.glb', accessorsChecked=1, primitivesChecked=1, vertices=3)])
        rows.append(dict(
            name=name, path=name + '/input.3tz', source=source, intendedRule=rule,
            oracleViolations=sorted({r for _, r in violations}), oracleCategory=verdict,
            expectedCategory=expected, expectedExitCode={'admitted': 0, 'unsupported': 2}.get(expected, 3),
            expectedReport=report if expected == 'admitted' else None,
            ktx2Sha256=[sha(b) for b in image_bytes], sha256=sha(raw), bytes=len(raw),
            memberSha256={k: sha(v) for k, v in members}))
    return rows


def disagreements(rows):
    """Cases whose oracle verdict misses the intended category or rule."""
    return [r['name'] for r in rows if r['oracleCategory'] != r['expectedCategory']
            or (r['intendedRule'] is not None and r['intendedRule'] not in r['oracleViolations'])]


def freeze():
    import shutil, tempfile
    work = Path(tempfile.mkdtemp(dir=Path.home() / '.cache'))
    try:
        rows = generate(work / 'fixtures')
    finally:
        shutil.rmtree(work)
    assert not disagreements(rows), disagreements(rows)
    pins = json.loads((PRIMARY / 'pins.json').read_text())
    for ref in pins['files']:
        raw = (PRIMARY / ref['file']).read_bytes(); assert sha(raw) == ref['sha256'] and len(raw) == ref['bytes']
    manifest = dict(schemaVersion=1, status='Predictions frozen before target execution; not execution evidence',
                    rule='C1 KHR_texture_basisu / KTX2 container amendment (2026-10-13)',
                    generatorSha256=sha(Path(__file__).read_bytes()),
                    oracleSha256=sha((HERE / 'ktx2_oracle.py').read_bytes()),
                    runnerSha256=sha((HERE / 'runner.py').read_bytes()), referencePins=pins, cases=rows)
    (HERE / 'fixtures').mkdir(exist_ok=True)
    (HERE / 'fixtures' / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    files = sorted(p for p in HERE.rglob('*') if p.is_file() and p.name not in ('integrity.json', 'README.md')
                   and '__pycache__' not in p.parts and p.parent.name != 'mutants')
    integrity = dict(status='C1 KHR_texture_basisu / KTX2 lane; archives are regenerated by runner.py and '
                            'checked against fixtures/manifest.json',
                     files={str(p.relative_to(HERE)): dict(bytes=p.stat().st_size, sha256=sha(p.read_bytes()))
                            for p in files})
    (HERE / 'integrity.json').write_text(json.dumps(integrity, indent=2) + '\n')


if __name__ == '__main__':
    assert sys.argv[1:] == ['--freeze'], 'usage: python3 -I generator.py --freeze'
    freeze()
