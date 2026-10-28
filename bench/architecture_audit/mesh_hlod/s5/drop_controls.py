#!/usr/bin/env python3
"""D11 coarse-drop controls (owner decision 2026-10-28).

Literal archives; no producer imports. A coarse root over a leaf of 8,192 large
separated faces and a leaf holding one small face: an honest drop of the small one is
declared, under cap pressure, smallest first and covered by G. The oracle and, with
--binary, native validation must count it, and both must refuse it undeclared, with
G below its loss, without cap pressure, out of size order, with loss inside the face
beyond G, and (on the analytic hierarchy) where it touches the border.
"""
import argparse
import json
from pathlib import Path
import struct
import subprocess

import controls
from omission_controls import FLOOR, GROUPS, base


def declare(data, n):
    length = int.from_bytes(data[12:16], 'little')
    doc, binary = json.loads(data[20:20 + length]), data[28 + length:]
    doc['asset'].setdefault('extras', {})['rustyTiles'] = {'droppedComponents': n}
    text = json.dumps(doc).encode()
    text += b' ' * (-len(text) % 4)
    return (b'glTF' + struct.pack('<III', 2, 28 + len(text) + len(binary), len(text)) + b'JSON' + text
            + struct.pack('<I', len(binary)) + b'BIN\0' + binary)


def generate(root):
    from exact_controls import payload, tile
    root = Path(root)
    root.mkdir(parents=True, exist_ok=True)
    cases = []
    # Large separated kept faces (diagonal about 8.06) and a small one to drop (0.71).
    wide = [[[4 * i, 0, 0], [4 * i + 1, 0, 0], [4 * i, 8, 0]] for i in range(8192)]
    small = [[0, 9, 0], [0.5, 9, 0], [0, 9.5, 0]]
    # Corners within 0.36 of the first two kept faces; its long edge's points about 1.3 away.
    long = [[0.25, 0.25, 0.25], [4.25, 0.25, 0.25], [4.25, 0.5, 0.25]]

    def case(name, left, right, coarse, declared, g, check=None, dropped=None, reason=None):
        folder = root / name
        folder.mkdir(exist_ok=True)
        (folder / 'coarse.glb').write_bytes(declare(payload(coarse, leaf=False), declared))
        (folder / 'left.glb').write_bytes(payload(left, leaf=True))
        (folder / 'right.glb').write_bytes(payload(right, leaf=True, first=len(left)))
        top = tile('coarse.glb', [tile('left.glb'), tile('right.glb')])
        top['geometricError'] = g
        (folder / 'tileset.json').write_text(json.dumps(dict(asset={'version': '1.1'}, geometricError=g, root=top)))
        source = root / (name + '-source.json')
        source.write_text(json.dumps({'triangles': {f'0,0,0,{i}': dict(
            xyz=t, vertices=[i * 3 + k for k in range(3)], expanded_ordinal=0, material_index=0,
            material=controls.MATERIALS[0], normals=controls.NORMALS, uvs=controls.UVS)
            for i, t in enumerate(left + right)}}) + '\n')
        cases.append(dict(name=name, check=check, dropped=dropped, source=source.name, reason=reason))

    # Under cap pressure (8,193 child faces) the smallest component goes, within G.
    case('coarse-drop', wide, [small], wide, 1, 2.0, dropped=1)
    case('coarse-drop-undeclared', wide, [small], wide, 0, 2.0, check='coarseDrops', reason='declares 0')
    case('coarse-drop-understated', wide, [small], wide, 1, 0.5, check='coarseDrops', reason='farther than')
    # Review r1: children within the cap, a larger component dropped beside a kept
    # smaller one, and loss inside a dropped face beyond G though its corners are near.
    case('coarse-drop-without-pressure', wide[:4], [small], wide[:4], 1, 2.0, check='coarseDrops',
         reason='cap')
    case('coarse-drop-not-smallest', wide, [small], wide[1:] + [small], 1, 16.0, check='coarseDrops',
         reason='smallest first')
    case('coarse-drop-interior-loss', wide, [long], wide, 1, 0.5, check='coarseDrops', reason='farther than')
    # The analytic hierarchy: middle.glb drops face 2, which shares a key with the left
    # leaves, and is refused by the exact retention checks whatever it declares.
    folder = root / 'coarse-drop-border'
    folder.mkdir(exist_ok=True)
    doc = base()
    right = doc['root']['children'][1]
    right['children'][0]['geometricError'] = 2 * FLOOR
    right['geometricError'] = doc['root']['geometricError'] = doc['geometricError'] = 2 * FLOOR
    files = {uri: controls.glb(ids, leaf=uri.startswith('leaf')) for uri, ids in GROUPS.items()}
    files['middle.glb'] = declare(controls.glb([3], leaf=False), 1)
    for uri, data in files.items():
        (folder / uri).write_bytes(data)
    (folder / 'tileset.json').write_text(json.dumps(doc))
    expected = {'triangles': {
        f'0,0,{i},0': dict(xyz=[controls.transformed(p) for p in face], normals=[[0, -1, 0], [-1, 0, 0], [0, 0, 1]],
                           uvs=controls.UVS, vertices=[0, 1, 2], material=controls.MATERIALS[i % 2],
                           expanded_ordinal=i, material_index=i % 2)
        for i, face in enumerate(controls.POINTS)}}
    (root / 'coarse-drop-border-source.json').write_text(json.dumps(expected, sort_keys=True, indent=2) + '\n')
    cases.append(dict(name='coarse-drop-border', check='interfacePoints', dropped=None, reason=None,
                      source='coarse-drop-border-source.json'))
    return cases


def run(root, binary=None):
    from oracle import audit, Failure
    root = Path(root)
    results = []
    for c in generate(root):
        folder = root / c['name']
        try:
            result = audit(folder, root / c['source'], scratch=root, work_limit=20_000_000)
            assert c['check'] is None, (c['name'], result.get('coarseDrops'))
            assert result['coarseDrops']['droppedComponents'] == c['dropped'], (c['name'], result['coarseDrops'])
            oracle = 'pass'
        except Failure as e:
            assert e.check == c['check'] and (c.get('reason') or '') in e.detail, (c['name'], e.check, e.detail)
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
                assert checked.returncode == 3, (c['name'], report)
                assert c['check'] in message and (c.get('reason') or '') in message, (c['name'], message)
                native = message.split(':')[0]
            else:
                assert checked.returncode == 0, (c['name'], report)
                assert report['interfaceCensus']['droppedComponents'] == c['dropped'], (c['name'], report['interfaceCensus'])
                native = 'pass'
        results.append(dict(name=c['name'], oracle=oracle, native=native))
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('outputdir', type=Path)
    parser.add_argument('--binary', type=Path)
    args = parser.parse_args()
    print(json.dumps(run(args.outputdir, args.binary), indent=2))
