#!/usr/bin/env python3
"""D11 coarse-drop controls (owner decision 2026-10-28).

Literal archives from the base analytic payloads; no producer imports. middle.glb sits
over leaf2 (face 2, which shares a key with the left leaves) and leaf3 (face 3, its
own component, at most 4 from face 2). An honest drop of face 3 is declared and
covered by G; the oracle and, with --binary, native validation must count it, and
both must refuse it undeclared, with G below its loss, and the border-touching drop.
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
    root = Path(root)
    root.mkdir(parents=True, exist_ok=True)
    expected = {'triangles': {
        f'0,0,{i},0': dict(xyz=[controls.transformed(p) for p in face], normals=[[0, -1, 0], [-1, 0, 0], [0, 0, 1]],
                           uvs=controls.UVS, vertices=[0, 1, 2], material=controls.MATERIALS[i % 2],
                           expanded_ordinal=i, material_index=i % 2)
        for i, face in enumerate(controls.POINTS)}}
    (root / 'source.json').write_text(json.dumps(expected, sort_keys=True, indent=2) + '\n')
    cases = []

    def case(name, kept, declared, g, check=None, dropped=None):
        out = root / name
        out.mkdir(exist_ok=True)
        doc = base()
        right = doc['root']['children'][1]
        # G stays ordered: the middle cell's error bounds its parent's and the root's.
        right['children'][0]['geometricError'] = g
        right['geometricError'] = doc['root']['geometricError'] = doc['geometricError'] = max(g, 4)
        files = {uri: controls.glb(ids, leaf=uri.startswith('leaf')) for uri, ids in GROUPS.items()}
        files['middle.glb'] = declare(controls.glb(kept, leaf=False), declared)
        for uri, data in files.items():
            (out / uri).write_bytes(data)
        (out / 'tileset.json').write_text(json.dumps(doc))
        cases.append(dict(name=name, check=check, dropped=dropped))

    case('coarse-drop', [2], 1, 4, dropped=1)
    case('coarse-drop-undeclared', [2], 0, 4, check='coarseDrops')
    case('coarse-drop-understated', [2], 1, 2, check='coarseDrops')
    case('coarse-drop-border', [3], 1, 2 * FLOOR, check='interfacePoints')
    return cases


def run(root, binary=None):
    from oracle import audit, Failure
    root = Path(root)
    results = []
    for c in generate(root):
        folder = root / c['name']
        try:
            result = audit(folder, root / 'source.json', scratch=root, work_limit=2_000_000)
            assert c['check'] is None, (c['name'], result.get('coarseDrops'))
            assert result['coarseDrops']['droppedComponents'] == c['dropped'], (c['name'], result['coarseDrops'])
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
                assert checked.returncode == 3, (c['name'], report)
                assert c['check'] in message, (c['name'], message)
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
