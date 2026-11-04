#!/usr/bin/env python3
"""D5 amendment A3 controls: texture-aware published G (owner decision 2026-11-04).

Literal archives; no producer imports. Four 8 m right triangles, base-colour PNG pages:
leaves at 256 x 256 texels per unit UV (spacing 0.03125 m). `flat` is a coarse root over
two leaves (root page 32: spacing 0.25 m, implied G at t = 2 of 2 m). `deep` is a root
(page 16, 0.5 m) over two middle tiles (page 64, 0.125 m, implied 1 m: refinable at
depth 2) over four leaves; the root's implied G is 4 m. `copy` repeats the flat root's
payload on a single-child tile between root and leaves (a single-child chain).

The oracle and, with --binary, native validation must accept honest G and refuse it
understated (at the root, at depth 2, on a copy, under a stricter t, with UVs shrunk or
on TEXCOORD_1), below a child it must stay above, and under every malformed or missing
declaration. The fast form (pxPerTexel null) binds nothing.
"""
import argparse
import json
from pathlib import Path
import struct
import subprocess
import zlib

import controls

TEXTURED = {'pbrMetallicRoughness': {'baseColorTexture': {'index': 0}, 'metallicFactor': 0.0}}
FACES = [[[16 * i, 0, 0], [16 * i + 8, 0, 0], [16 * i, 8, 0]] for i in range(4)]
BALANCED = dict(version=1, pxPerTexel=2.0, refinableRatio=1.25)


def stored_zlib(data):
    """A zlib stream of stored (uncompressed) deflate blocks: byte-identical under every
    zlib build (zlib.compress differs between stock zlib and zlib-ng)."""
    out = bytearray(b'\x78\x01')
    for at in range(0, max(len(data), 1), 65535):
        block = data[at:at + 65535]
        out += bytes([at + 65535 >= len(data)]) + struct.pack('<HH', len(block), 0xFFFF ^ len(block)) + block
    return bytes(out) + struct.pack('>I', zlib.adler32(data))


def png(edge):
    row = b'\0' + b'\x80\x40\x20\xff' * edge
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', edge, edge, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', stored_zlib(row * edge)) + chunk(b'IEND', b''))


KTX2 = (Path(__file__).resolve().parents[2] / 'c1_payload_integrity/production_controls/'
        'ktx2_basisu_amendment/2026-10-13/primary/rgba-mipmap-reference-basis.ktx2')


def textured(data, edge, texcoord=None):
    """The payload with one base-colour PNG page of `edge` x `edge` on its material (`edge`
    'ktx2': the committed 64 x 64 ETC1S KTX2 page). With `texcoord` (a UV scale), the page
    binds TEXCOORD_1 = TEXCOORD_0 x scale in its own view."""
    length = int.from_bytes(data[12:16], 'little')
    doc, binary = json.loads(data[20:20 + length]), bytearray(data[28 + length:])

    def view(blob):
        binary.extend(bytes(-len(binary) % 8))
        doc['bufferViews'].append(dict(buffer=0, byteOffset=len(binary), byteLength=len(blob)))
        binary.extend(blob)
        return len(doc['bufferViews']) - 1
    material = json.loads(json.dumps(TEXTURED))
    if texcoord is not None:
        prim = doc['meshes'][0]['primitives'][0]
        src = doc['accessors'][prim['attributes']['TEXCOORD_0']]
        sv = doc['bufferViews'][src['bufferView']]
        uv = struct.unpack(f'<{2 * src["count"]}f', binary[sv['byteOffset']:sv['byteOffset'] + 8 * src['count']])
        doc['accessors'].append(dict(src, bufferView=view(struct.pack(f'<{len(uv)}f', *(v * texcoord for v in uv))),
                                     byteOffset=0))
        prim['attributes']['TEXCOORD_1'] = len(doc['accessors']) - 1
        material['pbrMetallicRoughness']['baseColorTexture']['texCoord'] = 1
    if edge == 'ktx2':
        doc['images'] = [dict(bufferView=view(KTX2.read_bytes()), mimeType='image/ktx2')]
        doc['textures'] = [dict(sampler=0, extensions={'KHR_texture_basisu': {'source': 0}})]
        for key in ('extensionsUsed', 'extensionsRequired'):
            doc[key] = sorted(set(doc.get(key, [])) | {'KHR_texture_basisu'})
    else:
        doc['images'] = [dict(bufferView=view(png(edge)), mimeType='image/png')]
        doc['textures'] = [dict(source=0, sampler=0)]
    binary.extend(bytes(-len(binary) % 4))
    doc['buffers'][0]['byteLength'] = len(binary)
    doc['materials'] = [material]
    doc['samplers'] = [dict(magFilter=9729, minFilter=9729, wrapS=33071, wrapT=33071)]
    text = json.dumps(doc, separators=(',', ':')).encode()
    text += b' ' * (-len(text) % 4)
    return (b'glTF' + struct.pack('<III', 2, 28 + len(text) + len(binary), len(text)) + b'JSON' + text
            + struct.pack('<I', len(binary)) + b'BIN\0' + bytes(binary))


ABSENT = object()


def generate(root):
    from exact_controls import payload, tile
    root = Path(root)
    root.mkdir(parents=True, exist_ok=True)
    cases = []

    def case(name, g, *, shape='flat', edge=32, mid=(1.001, 1.001), mid_edge=(64, 64), copy_g=None,
             declaration=BALANCED, uv_scale=1.0, texcoord=None, check=None, native=None, bound=None,
             extras=True, report=None, expect=True):
        folder = root / name
        folder.mkdir(exist_ok=True)
        files = {}

        def coarse(uri, faces, page, scale=1.0):
            uvs = controls.UVS
            controls.UVS = [(u * scale, v * scale) for u, v in uvs]
            try:
                files[uri] = textured(payload(faces, leaf=False), page, texcoord)
            finally:
                controls.UVS = uvs

        leaves = []
        for i, face in enumerate(FACES if shape == 'deep' else [FACES[:2], FACES[2:]]):
            faces = [face] if shape == 'deep' else face
            first = i if shape == 'deep' else 2 * i
            files[f'leaf{i}.glb'] = textured(payload(faces, leaf=True, first=first), 256,
                                             None if texcoord is None else 1.0)
            leaves.append(tile(f'leaf{i}.glb'))
        coarse('coarse.glb', FACES, edge, uv_scale)
        if shape == 'deep':
            kids = []
            for k in range(2):
                coarse(f'mid{k}.glb', FACES[2 * k:2 * k + 2], mid_edge[k])
                m = tile(f'mid{k}.glb', leaves[2 * k:2 * k + 2])
                m['geometricError'] = mid[k]
                kids.append(m)
        elif shape == 'copy':
            m = tile('coarse.glb', leaves)
            m['geometricError'] = g if copy_g is None else copy_g
            kids = [m]
        else:
            kids = leaves
        top = tile('coarse.glb', kids)
        top['geometricError'] = g
        rusty = {'lod': 'hlod'}
        if declaration is not ABSENT:
            rusty['textureError'] = declaration
        if report is not None:
            files['conversion.json'] = json.dumps(report).encode()
        for uri, data in files.items():
            (folder / uri).write_bytes(data)
        asset = {'version': '1.1', 'extras': {'rustyTiles': rusty}} if extras else {'version': '1.1'}
        (folder / 'tileset.json').write_text(json.dumps(dict(asset=asset, geometricError=g, root=top)))
        source = root / (name + '-source.json')
        source.write_text(json.dumps({'triangles': {f'0,0,0,{i}': dict(
            xyz=t, vertices=[i * 3 + k for k in range(3)], expanded_ordinal=0, material_index=0,
            material=json.loads(files['leaf0.glb'][20:20 + int.from_bytes(files['leaf0.glb'][12:16], 'little')])['materials'][0],
            normals=controls.NORMALS, uvs=controls.UVS)
            for i, t in enumerate(FACES)}}) + '\n')
        cases.append(dict(name=name, check=check, native=native or check, bound=bound, source=source.name,
                          expect=expect))

    # One level.
    case('texture-honest', 2.001, bound=1)
    case('texture-understated', 1.9, check='textureError')
    case('texture-stricter-t', 2.001, declaration=dict(BALANCED, pxPerTexel=1.25), check='textureError')
    case('texture-quality-honest', 3.201, declaration=dict(BALANCED, pxPerTexel=1.25), bound=1)
    case('texture-source-limited', 0.5, edge=256, bound=0)
    case('texture-uv-shrunk', 2.001, uv_scale=0.5, check='textureError')
    # The 1e-4 relative slack, both sides of the edge (implied G 2).
    case('texture-tolerance-inside', 2 * (1 - 0.9e-4), bound=1)
    case('texture-tolerance-outside', 2 * (1 - 1.1e-4), check='textureError')
    # A KTX2 root page (64 x 64: spacing 0.125 m, implied G 1).
    case('texture-ktx2-honest', 1.001, edge='ktx2', bound=1)
    case('texture-ktx2-understated', 0.9, edge='ktx2', check='textureError')
    # Textured surface with no UV area is infinitely coarse, never untextured.
    case('texture-uv-zero', 1000.0, uv_scale=0.0, check='textureError')
    # Base colour on TEXCOORD_1: honest, and that set (not TEXCOORD_0) shrunk on the root.
    case('texture-texcoord1-honest', 2.001, texcoord=1.0, bound=1)
    case('texture-texcoord1-understated', 2.001, texcoord=0.5, check='textureError')
    # Declarations: the fast form binds nothing; anything else malformed or missing refuses.
    case('texture-fast', 0.5, declaration=dict(BALANCED, pxPerTexel=None), bound=None)
    for name, declaration in (
            ('missing', ABSENT), ('null', None), ('ratio-1e6', dict(BALANCED, refinableRatio=1e6)),
            ('px-1e9', dict(BALANCED, pxPerTexel=1e9)), ('extra-key', dict(BALANCED, extra=1)),
            ('version-true', dict(BALANCED, version=True)), ('version-1.0', dict(BALANCED, version=1.0)),
            ('version-2', dict(BALANCED, version=2)), ('px-string', dict(BALANCED, pxPerTexel='2')),
            ('no-ratio', dict(version=1, pxPerTexel=2.0))):
        case(f'texture-declaration-{name}', 2.001, declaration=declaration, check='textureError')
    # Review r2: stripping the declaration never switches the check off. Native holds the
    # archive to HLOD by the caller, by conversion.json or by its payload identity.
    case('texture-stripped-expected', 0.5, extras=False, check='textureError')
    case('texture-stripped-report', 0.5, extras=False, report={'lod': 'hlod'}, expect=False, check='textureError')
    case('texture-stripped-identity', 0.5, extras=False, expect=False, check='textureError')
    # The producer records its profile in conversion.json: fast must declare null.
    mesh = dict(operation='mesh_tiler')
    case('texture-profile-balanced', 2.001, report=dict(mesh, profile='balanced'), bound=1)
    case('texture-profile-relabelled-fast', 0.5, declaration=dict(BALANCED, pxPerTexel=None),
         report=dict(mesh, profile='balanced'), check='textureError')
    case('texture-profile-fast-declares-t', 2.001, report=dict(mesh, profile='fast'), check='textureError')
    # Two levels: refinable at depth 2, understated there, and parent propagation.
    case('texture-deep-honest', 4.001, shape='deep', edge=16, bound=3)
    case('texture-deep-understated-depth2', 4.001, shape='deep', edge=16, mid=(0.9, 1.001), check='textureError')
    case('texture-deep-propagated', 8.003, shape='deep', edge=16, mid=(8.002, 1.001), mid_edge=(8, 64), bound=2)
    case('texture-deep-parent-below-child', 4.001, shape='deep', edge=16, mid=(8.002, 1.001), mid_edge=(8, 64),
         check='geometricErrorOrder', native='geometricError increases')
    # A single-child copy chain: the copy and the tile above it both look through to leaves.
    case('texture-copy-honest', 2.001, shape='copy', bound=2)
    case('texture-copy-low', 2.001, shape='copy', copy_g=1.0, check='textureError')
    return cases


def run(root, binary=None):
    from oracle import audit, Failure
    root = Path(root)
    results = []
    for c in generate(root):
        folder = root / c['name']
        try:
            result = audit(folder, root / c['source'], scratch=root, work_limit=20_000_000)
            assert c['check'] is None, (c['name'], result.get('textureError'))
            got = result['textureError'].get('textureBoundTiles')
            assert got == c['bound'], (c['name'], result['textureError'])
            oracle = 'pass'
        except Failure as e:
            assert e.check == c['check'], (c['name'], e.check, e.detail)
            oracle = e.check
        native = 'not run'
        if binary:
            archive = folder.with_suffix('.3tz')
            packed = subprocess.run([str(binary), 'convert', '-i', str(folder), '-o', str(archive), '--force', '--json'],
                                    capture_output=True, timeout=30)
            assert packed.returncode == 0, (c['name'], packed.stderr[-2000:])
            flags = ['--expect-profile', 'hlod'] if c['expect'] else []
            checked = subprocess.run([str(binary), 'validate', str(archive), '--json', *flags],
                                     capture_output=True, timeout=30)
            report = json.loads(checked.stdout)
            if c['check']:
                message = report['error']['message']
                assert checked.returncode == 3 and c['native'] in message, (c['name'], report)
                native = c['native']
            else:
                assert checked.returncode == 0, (c['name'], report)
                assert report['interfaceCensus']['textureBoundTiles'] == (c['bound'] or 0), (c['name'], report['interfaceCensus'])
                native = 'pass'
        results.append(dict(name=c['name'], oracle=oracle, native=native))
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('outputdir', type=Path)
    parser.add_argument('--binary', type=Path)
    args = parser.parse_args()
    print(json.dumps(run(args.outputdir, args.binary), indent=2))
