#!/usr/bin/env python3
"""Coarse self-intersection controls (#420): oracle and native counts on literal archives.

Literal archives; no producer imports. A coarse root over two leaves: a floor of unit
squares (two faces each) and a leaf supplying the keys of the coarse tile's extra faces.
Every coarse key is a source key and the coarse extent equals the leaves'. The census
reports and never refuses, so every archive validates; the oracle's census
(crossings.py) must equal the expected counts and, with --binary, native validation's
`interfaceCensus.coarseCrossings` field by field.

- crossing-none: the floor and a face beside it, copied: no pair.
- crossing-present: a child face standing in the floor, copied: one present pair.
- crossing-chord-floor (the #420 case): a new chord face standing on the floor, through
  three floor faces and touching a fourth on an edge: four new pairs.
- crossing-duplicate: the standing face twice in the child and the coarse tile: one face,
  one present pair.
- crossing-collinear: a collinear (zero-area) child face through the floor, copied: one
  present pair.
- crossing-shared-corner: a new face on the floor's plane over a floor corner it shares,
  overlapping the faces around it: counted alike by both validators.
- crossing-random: seeded random faces on a small integer grid (shared corners, shared
  edges, coplanar and collinear cases), copied: both validators count alike.
"""
import argparse
import json
from pathlib import Path
import subprocess

import controls
from exact_controls import payload, tile

G = 2.0


def floor():
    out = []
    for i in range(4):
        for j in range(4):
            out.append([[i, j, 0], [i + 1, j, 0], [i + 1, j + 1, 0]])
            out.append([[i, j, 0], [i + 1, j + 1, 0], [i, j + 1, 0]])
    return out


# Keys inside the floor's extent (x, y in [0, 4], z in [-1, 1]) so the coarse extent is
# the leaves'.
P1, P2, P3 = [0.25, 0.125, 1], [0.25, 0.125, -1], [2, 0.75, 0]
Q1, Q2, Q3 = [3.5, 3.5, 0.5], [3.5, 3.25, 0.5], [3.25, 3.5, 0.5]
STANDING = [[0.5, 0.3125, -1], [0.5, 0.3125, 1], [0.6875, 0.3125, 0]]
COLLINEAR = [[1.5, 0.375, -1], [1.5, 0.375, 0.5], [1.5, 0.375, 1]]
ASIDE = [[3.5, 3.5, 1], [3.75, 3.5, 1], [3.5, 3.75, 1]]


def random_faces(seed, n):
    """`n` faces of distinct corners on a 5 x 5 x 3 lattice, from a 64-bit xorshift (the
    same bytes on every platform and Python version)."""
    grid = [[x, y, z] for x in range(5) for y in range(5) for z in (-1, 0, 1)]
    state = seed
    out = []
    while len(out) < n:
        picks = []
        for _ in range(3):
            state ^= (state << 13) & 0xFFFFFFFFFFFFFFFF
            state ^= state >> 7
            state ^= (state << 17) & 0xFFFFFFFFFFFFFFFF
            picks.append(grid[state % len(grid)])
        if len({tuple(p) for p in picks}) == 3:
            out.append(picks)
    return out


def generate(root):
    root = Path(root)
    root.mkdir(parents=True, exist_ok=True)
    cases = []

    def case(name, left, right, coarse, expected=None):
        folder = root / name
        folder.mkdir(exist_ok=True)
        (folder / 'coarse.glb').write_bytes(payload(coarse, leaf=False))
        (folder / 'left.glb').write_bytes(payload(left, leaf=True))
        (folder / 'right.glb').write_bytes(payload(right, leaf=True, first=len(left)))
        top = tile('coarse.glb', [tile('left.glb'), tile('right.glb')])
        top['geometricError'] = G
        (folder / 'tileset.json').write_text(json.dumps(dict(asset={'version': '1.1'}, geometricError=G, root=top)))
        source = root / (name + '-source.json')
        source.write_text(json.dumps({'triangles': {f'0,0,0,{i}': dict(
            xyz=t, vertices=[i * 3 + k for k in range(3)], expanded_ordinal=0, material_index=0,
            material=controls.MATERIALS[0], normals=controls.NORMALS, uvs=controls.UVS)
            for i, t in enumerate(left + right)}}) + '\n')
        cases.append(dict(name=name, source=source.name, expected=expected))

    f = floor()
    case('crossing-none', f, [ASIDE], f + [ASIDE], dict(crossingTiles=0, presentPairs=0, newPairs=0))
    case('crossing-present', f, [STANDING], f + [STANDING], dict(crossingTiles=1, presentPairs=1, newPairs=0))
    case('crossing-chord-floor', f, [[P1, P2, Q1], [P3, Q2, Q3]], f + [[P1, P2, P3]],
         dict(crossingTiles=1, presentPairs=0, newPairs=4))
    # A duplicated child face (two entries, one face): counted once.
    case('crossing-duplicate', f, [STANDING, STANDING], f + [STANDING, STANDING],
         dict(crossingTiles=1, presentPairs=1, newPairs=0, faces=33))
    case('crossing-collinear', f, [COLLINEAR], f + [COLLINEAR], dict(crossingTiles=1, presentPairs=1, newPairs=0))
    case('crossing-shared-corner', f, [ASIDE], f + [ASIDE, [[0, 0, 0], [2, 1, 0], [1, 2, 0]]])
    faces = random_faces(0x420, 160)
    case('crossing-random', faces[:80], faces[80:], faces)
    return cases


def run(root, binary=None):
    from oracle import audit
    root = Path(root)
    results = []
    for c in generate(root):
        folder = root / c['name']
        result = audit(folder, root / c['source'], scratch=root, work_limit=200_000_000,
                       coarse_crossings=True)['coarseCrossings']
        assert result['tiles'] == 1 and result['incompleteTiles'] == 0, (c['name'], result)
        for k, v in (c['expected'] or {}).items():
            assert result[k] == v, (c['name'], k, result)
        native = 'not run'
        if binary:
            archive = folder.with_suffix('.3tz')
            packed = subprocess.run([str(binary), 'convert', '-i', str(folder), '-o', str(archive), '--force', '--json'],
                                    capture_output=True, timeout=30)
            assert packed.returncode == 0, (c['name'], packed.stderr[-2000:])
            checked = subprocess.run([str(binary), 'validate', str(archive), '--json', '--expect-profile', 'hlod',
                                      '--coarse-crossings'],
                                     capture_output=True, timeout=60)
            report = json.loads(checked.stdout)
            assert checked.returncode == 0 and report['ok'], (c['name'], report)
            assert report['interfaceCensus']['coarseCrossings'] == result, (
                c['name'], report['interfaceCensus']['coarseCrossings'], result)
            plain = subprocess.run([str(binary), 'validate', str(archive), '--json', '--expect-profile', 'hlod'],
                                   capture_output=True, timeout=60)
            assert plain.returncode == 0 and 'coarseCrossings' not in json.loads(plain.stdout)['interfaceCensus'], c['name']
            native = 'equal'
        results.append(dict(name=c['name'], newPairs=result['newPairs'], presentPairs=result['presentPairs'],
                            native=native))
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('outputdir', type=Path)
    parser.add_argument('--binary', type=Path)
    args = parser.parse_args()
    print(json.dumps(run(args.outputdir, args.binary), indent=2))
