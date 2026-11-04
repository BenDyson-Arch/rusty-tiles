#!/usr/bin/env python3
"""D5 amendment A3 controls: texture-aware published G (owner decision 2026-11-04).

Literal archives; no producer imports. A coarse root holds the faces of two textured
leaves (8 m right triangles, base-colour PNG pages): leaves at 256 x 256 texels per unit
UV, the root at 32 x 32, so the root is refinable (spacing 0.25 m against 0.03125 m) and
its texture-implied error at t = 2 is 16 * 0.25 / 2 = 2 m. The oracle and, with --binary,
native validation must accept the honest G, and refuse it understated, under a stricter
declared t, with the root's UVs shrunk (coarser texels) and with a malformed declaration.
An undeclared archive and a source-limited root (page as fine as its leaves) bind nothing.
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


def png(edge):
    row = b'\0' + b'\x80\x40\x20\xff' * edge
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', edge, edge, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(row * edge)) + chunk(b'IEND', b''))


def textured(data, edge):
    """The payload with one base-colour PNG page of `edge` x `edge` on its material."""
    length = int.from_bytes(data[12:16], 'little')
    doc, binary = json.loads(data[20:20 + length]), bytearray(data[28 + length:])
    binary.extend(bytes(-len(binary) % 8))
    image = png(edge)
    doc['bufferViews'].append(dict(buffer=0, byteOffset=len(binary), byteLength=len(image)))
    binary.extend(image)
    binary.extend(bytes(-len(binary) % 4))
    doc['buffers'][0]['byteLength'] = len(binary)
    doc['materials'] = [TEXTURED]
    doc['images'] = [dict(bufferView=len(doc['bufferViews']) - 1, mimeType='image/png')]
    doc['textures'] = [dict(source=0, sampler=0)]
    doc['samplers'] = [dict(magFilter=9729, minFilter=9729, wrapS=33071, wrapT=33071)]
    text = json.dumps(doc, separators=(',', ':')).encode()
    text += b' ' * (-len(text) % 4)
    return (b'glTF' + struct.pack('<III', 2, 28 + len(text) + len(binary), len(text)) + b'JSON' + text
            + struct.pack('<I', len(binary)) + b'BIN\0' + bytes(binary))


def generate(root):
    from exact_controls import payload, tile
    root = Path(root)
    root.mkdir(parents=True, exist_ok=True)
    cases = []
    left, right = FACES[:2], FACES[2:]

    def case(name, g, coarse_edge=32, declaration=dict(version=1, pxPerTexel=2.0, refinableRatio=1.25),
             uv_scale=1.0, check=None, bound=None):
        folder = root / name
        folder.mkdir(exist_ok=True)
        uvs = controls.UVS
        controls.UVS = [(u * uv_scale, v * uv_scale) for u, v in uvs]
        try:
            (folder / 'coarse.glb').write_bytes(textured(payload(FACES, leaf=False), coarse_edge))
        finally:
            controls.UVS = uvs
        (folder / 'left.glb').write_bytes(textured(payload(left, leaf=True), 256))
        (folder / 'right.glb').write_bytes(textured(payload(right, leaf=True, first=len(left)), 256))
        top = tile('coarse.glb', [tile('left.glb'), tile('right.glb')])
        top['geometricError'] = g
        rusty = {'lod': 'hlod'}
        if declaration is not None:
            rusty['textureError'] = declaration
        (folder / 'tileset.json').write_text(json.dumps(dict(
            asset={'version': '1.1', 'extras': {'rustyTiles': rusty}}, geometricError=g, root=top)))
        source = root / (name + '-source.json')
        source.write_text(json.dumps({'triangles': {f'0,0,0,{i}': dict(
            xyz=t, vertices=[i * 3 + k for k in range(3)], expanded_ordinal=0, material_index=0,
            material=TEXTURED, normals=controls.NORMALS, uvs=controls.UVS)
            for i, t in enumerate(FACES)}}) + '\n')
        cases.append(dict(name=name, check=check, bound=bound, source=source.name))

    case('texture-honest', 2.001, bound=1)
    case('texture-understated', 1.9, check='textureError')
    case('texture-undeclared', 0.5, declaration=None, bound=None)
    case('texture-stricter-t', 2.001, declaration=dict(version=1, pxPerTexel=1.25, refinableRatio=1.25),
         check='textureError')
    case('texture-source-limited', 0.5, coarse_edge=256, bound=0)
    case('texture-uv-shrunk', 2.001, uv_scale=0.5, check='textureError')
    case('texture-bad-declaration', 2.001, declaration=dict(version=2, pxPerTexel=2.0, refinableRatio=1.25),
         check='textureError')
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
            checked = subprocess.run([str(binary), 'validate', str(archive), '--json', '--expect-profile', 'hlod'],
                                     capture_output=True, timeout=30)
            report = json.loads(checked.stdout)
            if c['check']:
                message = report['error']['message']
                assert checked.returncode == 3 and c['check'] in message, (c['name'], report)
                native = c['check']
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
