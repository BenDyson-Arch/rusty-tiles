#!/usr/bin/env python3
"""D1/D11 amendment A2 (constructive last resort) controls.

Literal archives; no producer imports.

- Folds: a coarse root over the book-reversed-face leaves (`exact_controls.py`) keeps
  every source face and a second copy of [A,B,C]: A -> B three times against the
  source's two, and a coincident pair. Refused as a fold unless its payload declares
  `lastResort` {stage 2, foldFaces 3}; undeclared, stage 1, miscounted, and stage 0
  without a fold stage below are refused.
- N3 drops (A2: border-free components drop at any size, measured into G): a coarse tile
  over drop_controls' leaves declares stage 3 and drops the small component without cap
  pressure, or the larger one beside a kept smaller one under pressure. Both are admitted;
  the same drops at stage 2, or at a stage-0 root over a stage-2 tile, are refused
  (cap pressure, smallest first), as is a stage-3 drop beyond G, one undeclared, and a
  stage-3 claim whose fold count does not verify.
- PR #313 review: the sampling-gap triangle, leaf declarations, disagreeing payload
  stages/fold counts, four relative-stage cases and all 120 seeded combinations.
  Their GLB bytes and parsed tilesets match the independent review's literal archives.
- Seam owner: a planar quad split by a shared chain A-M-B between a left (least) and a
  right (greatest) leaf. A coarse tile re-triangulates its own leaf's quad with a face on
  the three shared points and a new edge A-B. The chain at x=0 hashes to A's least
  holder, at x=2 to its greatest: there the highest-indexed side owns the seam. Over the
  owner it is admitted; mirrored over the other leaf, refused.

Both the oracle and, with --binary, native validation must agree.
"""
import argparse
import json
from pathlib import Path
import struct
import subprocess

import controls
from exact_controls import A, B, C, D, E, payload, tile


def declare(data, stage, folds, dropped=None):
    length = int.from_bytes(data[12:16], 'little')
    doc, binary = json.loads(data[20:20 + length]), data[28 + length:]
    extras = {} if stage is None else {'lastResort': {'stage': stage, 'foldFaces': folds}}
    if dropped is not None:
        extras['droppedComponents'] = dropped
    doc['asset'].setdefault('extras', {})['rustyTiles'] = extras
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
    # N3 drops, on drop_controls' geometry: large separated kept faces (diagonal about
    # 8.06) and a small one (0.71), 8.6 above the first; G covers what each drop loses.
    wide = [[[4 * i, 0, 0], [4 * i + 1, 0, 0], [4 * i, 8, 0]] for i in range(8192)]
    small = [[0, 9, 0], [0.5, 9, 0], [0, 9.5, 0]]

    def drop(name, left, right, coarse, stage, folds, dropped, g, check=None, reason=None, middle=None):
        folder = root / name
        folder.mkdir(exist_ok=True)
        (folder / 'coarse.glb').write_bytes(declare(payload(coarse, leaf=False), stage, folds, dropped))
        (folder / 'left.glb').write_bytes(payload(left, leaf=True))
        (folder / 'right.glb').write_bytes(payload(right, leaf=True, first=len(left)))
        if middle is None:
            top = tile('coarse.glb', [tile('left.glb'), tile('right.glb')])
        else:
            # A stage-2 tile over the left leaf, holding it unchanged, below the root.
            (folder / 'middle.glb').write_bytes(declare(payload(left, leaf=False), middle, 0, 0))
            top = tile('coarse.glb', [tile('middle.glb', [tile('left.glb')]), tile('right.glb')])
            top['children'][0]['geometricError'] = 0
        top['geometricError'] = g
        (folder / 'tileset.json').write_text(json.dumps(dict(asset={'version': '1.1'}, geometricError=g, root=top)))
        source(root / (name + '-source.json'), left + right)
        cases.append(dict(name=name, check=check, reason=reason, source=name + '-source.json'))
    drop('n3-drop-without-pressure', wide[:4], [small], wide[:4], 3, 0, 1, 2.0)
    drop('n3-drop-not-smallest', wide, [small], wide[1:] + [small], 3, 0, 1, 16.0)
    drop('stage-2-drop-without-pressure', wide[:4], [small], wide[:4], 2, 0, 1, 2.0, 'coarseDrops', 'cap')
    drop('stage-2-drop-not-smallest', wide, [small], wide[1:] + [small], 2, 0, 1, 16.0, 'coarseDrops',
         'smallest first')
    drop('stage-0-drop-without-pressure', wide[:4], [small], wide[:4], 0, 0, 1, 2.0, 'coarseDrops', 'cap',
         middle=2)
    drop('n3-drop-beyond-g', wide[:4], [small], wide[:4], 3, 0, 1, 0.5, 'coarseDrops', 'farther than')
    drop('n3-drop-undeclared', wide[:4], [small], wide[:4], 3, 0, 0, 2.0, 'coarseDrops', 'declares 0')
    drop('n3-claim-miscounted', wide[:4], [small], wide[:4], 3, 1, 1, 2.0, 'lastResort', 'declared 1 fold faces')
    # Independent PR #313 review: a 12-unit face hidden between fixed samples.
    from drops import samples
    triangle = [[0, 0, 0], [12, 0, 0], [0, 12, 0]]
    patches = [[[x + .03125, y, 0], [x + .0625, y, 0], [x + .03125, y + .03125, 0]]
               for x, y, _ in samples(triangle)]
    drop('sampling-gap', patches, [triangle], patches, 3, 0, 1, .125, 'coarseDrops')
    left, right = wide[:1], [small]
    for relative in (None, 1, 2, 3):
        drop(f'self-claimed3-relative-{relative}', left, right, left, 3, 0, 1, 2, middle=relative)
    for name in ('leaf-stage3', 'leaf-fold99', 'mixed-payload-stages', 'multi-fold-count'):
        rr = right + [[[3, 9, 0], [3.5, 9, 0], [3, 9.5, 0]]] if name == 'mixed-payload-stages' else right
        drop(name, left, rr, left + right, 3, 0, int(name == 'mixed-payload-stages'), 16,
             'lastResort', 'leaf tile' if name.startswith('leaf') else 'payloads disagree')
        folder = root / name
        if name.startswith('leaf'):
            (folder / 'left.glb').write_bytes(declare(payload(left, leaf=True), 3, 99 if name == 'leaf-fold99' else 0))
        else:
            (folder / 'coarse.glb').write_bytes(declare(payload(left, leaf=False), 3, 0,
                                                       int(name == 'mixed-payload-stages')))
            (folder / 'extra.glb').write_bytes(declare(payload(right, leaf=False),
                2 if name == 'mixed-payload-stages' else 3, 0 if name == 'mixed-payload-stages' else 1, 0))
            doc = json.loads((folder / 'tileset.json').read_text())
            doc['root'].pop('content')
            doc['root']['contents'] = [{'uri': 'coarse.glb'}, {'uri': 'extra.glb'}]
            (folder / 'tileset.json').write_text(json.dumps(doc))
    # Review r2: skinny finite-f32 face for which a rounded closest point
    # underestimates the true distance. Preserve literal coordinates, not a search.
    numeric_face = [[-1.4724074926764578e+23, 2.9648180383498556e+22, -7.774404609302878e+22], [5.001019053693828e+22, 1.1372596964707108e+23, -1.23504860697962e+23], [-3.7499307865693355e+22, 7.642523899974737e+22, -1.032032911033214e+23]]
    numeric_drop = [[2.9580527309896196e+22, 1.0501786345078302e+23, -1.1876531748611357e+23], [2.9580525058096382e+22, 1.0501786345078302e+23, -1.1876531748611357e+23], [2.9580527309896196e+22, 1.0501787245798227e+23, -1.1876531748611357e+23]]
    numeric_g = 8277542449007818.0
    for name, scale, sign, margin, check in (
            ('numeric-bound', 1., 1., 1., 'coarseDrops'),
            ('numeric-bound-small', 2.**-40, 1., 1., 'coarseDrops'),
            ('numeric-bound-reflected', 1., -1., 1., 'coarseDrops'),
            ('numeric-bound-margin', 1., 1., 2., None)):
        kept = [[v * scale * sign for v in p] for p in numeric_face]
        lost = [[v * scale * sign for v in p] for p in numeric_drop]
        g = numeric_g * scale * margin
        drop(name, [kept], [lost], [kept], 3, 0, 1, g, check)
        # The original review has bounds sized for the large literal coordinates.
        folder = root / name
        extent = max(abs(v) for t in (kept, lost) for p in t for v in p) * 4 + 10
        top = tile('coarse.glb', [tile('left.glb', box=(0, extent)),
                                tile('right.glb', box=(0, extent))], box=(0, extent))
        top['geometricError'] = g
        (folder / 'tileset.json').write_text(json.dumps(dict(
            asset={'version': '1.1'}, geometricError=g, root=top)))
    # The review's complete seeded 120-case corpus, with pinned expected checks.
    import random
    rng = random.Random(313)
    expected = 'LLLDDDDDDPDDLDDLDDDDLPPPDDDDPDLDLDLDDPDLDLDLDDDDDDDPPLLLDPDDDDDLLPDLPDDLDLDDDLDDLDDPDLLLDDLLPDDPDLDDDLDDLDDLDPDLDDDLDDLD'
    for i, outcome in enumerate(expected):
        stage = rng.choice([None, 0, 1, 2, 3])
        middle = rng.choice([None, 1, 2, 3])
        omit = rng.choice([True, False])
        folds, count, g = rng.choice([0, 0, 0, 1]), rng.choice([0, 1, 2]), rng.choice([.5, 2, 16])
        drop(f'random-{i:03d}', left, right, left if omit else left + right, stage, folds, count, g,
             {'P': None, 'D': 'coarseDrops', 'L': 'lastResort'}[outcome], middle=middle)
    # Seam owner: shared chain A-M-B between a left quad (above) and a right quad (below).
    for name, x, mine, check in (('seam-owner-least-admitted', 0, 'left', None),
                                 ('seam-owner-least-refused', 0, 'right', 'seamOwner'),
                                 ('seam-owner-greatest-admitted', 2, 'right', None),
                                 ('seam-owner-greatest-refused', 2, 'left', 'seamOwner')):
        P, Q = [x, 0, 0], [x + 2, 0, 0]
        M, U, W = [x + 1, -.5, 0], [x, 2, 0], [x, -2, 0]
        left, right = [[P, M, U], [M, Q, U]], [[M, P, W], [Q, M, W]]
        folder = root / name
        folder.mkdir(exist_ok=True)
        (folder / 'left.glb').write_bytes(payload(left, leaf=True))
        (folder / 'right.glb').write_bytes(payload(right, leaf=True, first=2))
        if mine == 'left':
            coarse = [[P, M, Q], [P, Q, U]]
            children = [tile('coarse.glb', [tile('left.glb')]), tile('right.glb')]
        else:
            coarse = [[Q, M, P], [Q, P, W]]
            children = [tile('left.glb'), tile('coarse.glb', [tile('right.glb')])]
        (folder / 'coarse.glb').write_bytes(payload(coarse, leaf=False, first=0 if mine == 'left' else 2))
        document = dict(asset={'version': '1.1'}, geometricError=0, root=tile(children=children))
        (folder / 'tileset.json').write_text(json.dumps(controls.refinable(document), separators=(',', ':')))
        source(root / (name + '-source.json'), left + right)
        cases.append(dict(name=name, check=check, reason=None, source=name + '-source.json'))
    return cases


def run(root, binary=None):
    from oracle import audit, Failure, distance2
    from drops import covers
    from spatial import Work, WorkLimit
    from fractions import Fraction
    # A non-dyadic centroid must not be silently converted back to binary64.
    assert distance2((Fraction(1, 3), 0, 0),
                     ((0, 0, 0), (0, 1, 0), (0, 0, 1))) == Fraction(1, 9)
    # One subdivision is necessary: no individual retained triangle covers the
    # original triangle. Equality and exhausted work must not become false passes.
    triangle = ((0, 0, 0), (2, 0, 0), (0, 2, 0))
    faces = [{'xyz': f} for f in (
        ((0, 0, 0), (1, 0, 0), (0, 1, 0)), ((1, 0, 0), (2, 0, 0), (1, 1, 0)),
        ((0, 1, 0), (1, 1, 0), (0, 2, 0)), ((1, 0, 0), (1, 1, 0), (0, 1, 0)))]
    assert covers(triangle, faces, 1e-6, distance2, Work())
    try:
        covers(triangle, faces, 1e-6, distance2, Work(1))
        raise AssertionError('certificate ignored work limit')
    except WorkLimit:
        pass
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
