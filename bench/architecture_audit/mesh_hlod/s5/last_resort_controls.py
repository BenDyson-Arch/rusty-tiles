#!/usr/bin/env python3
"""D1/D11 amendment A2 (constructive last resort) controls.

Literal archives; no producer imports.

- Folds: a coarse root over the book-reversed-face leaves (`exact_controls.py`) keeps
  every source face and a second copy of [A,B,C]: A -> B three times against the
  source's two, and a coincident pair. Refused as a fold unless its payload declares
  `lastResort` {stage 2, foldFaces 3}; undeclared, stage 1, miscounted, and stage 0
  without a fold stage below are refused.
- Least owner: a planar quad split by a shared chain A-M-B. A coarse tile re-triangulates
  its own leaf's quad with a face on the three shared points and a new edge A-B. Over the
  leaf holding their least leaf it is admitted; mirrored over the other leaf, refused.

Both the oracle and, with --binary, native validation must agree.
"""
import argparse
import json
from pathlib import Path
import struct
import subprocess

import controls
from exact_controls import A, B, C, D, E, payload, tile


def declare(data, stage, folds):
    length = int.from_bytes(data[12:16], 'little')
    doc, binary = json.loads(data[20:20 + length]), data[28 + length:]
    doc['asset'].setdefault('extras', {})['rustyTiles'] = {'lastResort': {'stage': stage, 'foldFaces': folds}}
    text = json.dumps(doc).encode()
    text += b' ' * (-len(text) % 4)
    return (b'glTF' + struct.pack('<III', 2, 28 + len(text) + len(binary), len(text)) + b'JSON' + text
            + struct.pack('<I', len(binary)) + b'BIN\0' + binary)


def source(path, faces):
    path.write_text(json.dumps({'triangles': {f'0,0,0,{i}': dict(
        xyz=t, vertices=[i * 3 + k for k in range(3)], expanded_ordinal=0, material_index=0,
        material=controls.MATERIALS[0], normals=controls.NORMALS, uvs=controls.UVS)
        for i, t in enumerate(faces)}}, separators=(',', ':')) + '\n')


def generate(root):
    root = Path(root)
    root.mkdir(parents=True, exist_ok=True)
    cases = []
    # Folds: the duplicated [A,B,C] makes A -> B three times (source: twice) and a
    # coincident pair; three fold faces, unsupported unless admitted.
    left, right = [[A, B, C], [A, B, D]], [[B, A, E]]
    for name, declaration, check, reason in (
            ('last-resort-fold-declared', (2, 3), None, None),
            ('last-resort-fold-undeclared', None, 'interfaceWinding', None),
            ('last-resort-fold-stage-1', (1, 3), 'interfaceWinding', None),
            ('last-resort-fold-miscounted', (2, 4), 'lastResort', 'declared 4 fold faces'),
            ('last-resort-stage-0-alone', (0, 3), 'lastResort', 'stage 0')):
        folder = root / name
        folder.mkdir(exist_ok=True)
        coarse = payload([[A, B, C], [A, B, C], [A, B, D], [B, A, E]], leaf=False)
        (folder / 'coarse.glb').write_bytes(declare(coarse, *declaration) if declaration else coarse)
        (folder / 'left.glb').write_bytes(payload(left, leaf=True))
        (folder / 'right.glb').write_bytes(payload(right, leaf=True, first=2))
        (folder / 'tileset.json').write_text(json.dumps(dict(asset={'version': '1.1'}, geometricError=0,
            root=tile('coarse.glb', [tile('left.glb'), tile('right.glb')])), separators=(',', ':')))
        source(root / (name + '-source.json'), left + right)
        cases.append(dict(name=name, check=check, reason=reason, source=name + '-source.json'))
    # Least owner: shared chain A-M-B between a left quad (above) and a right quad (below).
    M, U, W = [1, -.5, 0], [0, 2, 0], [0, -2, 0]
    left, right = [[A, M, U], [M, [2, 0, 0], U]], [[M, A, W], [[2, 0, 0], M, W]]
    for name, mine, check in (('least-owner-admitted', 'left', None),
                              ('least-owner-refused', 'right', 'leastOwner')):
        folder = root / name
        folder.mkdir(exist_ok=True)
        (folder / 'left.glb').write_bytes(payload(left, leaf=True))
        (folder / 'right.glb').write_bytes(payload(right, leaf=True, first=2))
        if mine == 'left':
            coarse = [[A, M, [2, 0, 0]], [A, [2, 0, 0], U]]
            children = [tile('coarse.glb', [tile('left.glb')]), tile('right.glb')]
        else:
            coarse = [[[2, 0, 0], M, A], [[2, 0, 0], A, W]]
            children = [tile('left.glb'), tile('coarse.glb', [tile('right.glb')])]
        (folder / 'coarse.glb').write_bytes(payload(coarse, leaf=False, first=0 if mine == 'left' else 2))
        document = dict(asset={'version': '1.1'}, geometricError=0, root=tile(children=children))
        (folder / 'tileset.json').write_text(json.dumps(controls.refinable(document), separators=(',', ':')))
        source(root / (name + '-source.json'), left + right)
        cases.append(dict(name=name, check=check, reason=None, source=name + '-source.json'))
    return cases


def run(root, binary=None):
    from oracle import audit, Failure
    root = Path(root)
    results = []
    for c in generate(root):
        folder = root / c['name']
        try:
            audit(folder, root / c['source'], scratch=root, work_limit=20_000_000)
            assert c['check'] is None, (c['name'], 'passed')
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
                native = 'pass'
        results.append(dict(name=c['name'], oracle=oracle, native=native))
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('outputdir', type=Path)
    parser.add_argument('--binary', type=Path)
    args = parser.parse_args()
    print(json.dumps(run(args.outputdir, args.binary), indent=2))
