#!/usr/bin/env python3
"""Upward-omission, cover, refinement and producer-report controls.

Literal archives built from the base analytic payloads; no producer imports.
Every case runs the source-aware oracle and, with --binary, native validation;
both must reach the same named verdict and the same derived omission counts.
The R9 negatives reproduce S1 756283e2: an omitted cell carrying its children's
error is never refined by CesiumJS (maximum SSE 16), so nothing below it draws.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import struct
import subprocess

import controls

BOX = {'box': [0, 0, 0, 32, 0, 0, 0, 32, 0, 0, 0, 32]}
# 16 x the box diagonal 2|half| = 16 * 2 * sqrt(3) * 32.
FLOOR = 16 * 2 * (3 * 32 * 32) ** .5


def tile(uri, g, children=None):
    d = dict(boundingVolume=copy.deepcopy(BOX), geometricError=g, refine='REPLACE')
    if uri:
        d['content'] = {'uri': uri}
    if children:
        d['children'] = children
    return d


def base():
    left = tile('left.glb', 2, [tile('leaf0.glb', 0), tile('leaf1.glb', 0)])
    right = tile('right.glb', 2, [tile('middle.glb', 1, [tile('leaf2.glb', 0), tile('leaf3.glb', 0)])])
    return dict(asset={'version': '1.1'}, geometricError=4, root=tile('root.glb', 4, [left, right]))


GROUPS = {'root.glb': [0, 1, 2, 3], 'left.glb': [0, 1], 'right.glb': [2, 3], 'middle.glb': [2, 3],
          **{f'leaf{i}.glb': [i] for i in range(4)}}


def omitted(node, g):
    node.pop('content', None)
    node['geometricError'] = g


def generate(root):
    root = Path(root)
    root.mkdir(parents=True, exist_ok=True)
    expected = json.loads(json.dumps({'triangles': {
        f'0,0,{i},0': dict(xyz=[controls.transformed(p) for p in face], normals=[[0, -1, 0], [-1, 0, 0], [0, 0, 1]],
                           uvs=controls.UVS, vertices=[0, 1, 2], material=controls.MATERIALS[i % 2],
                           expanded_ordinal=i, material_index=i % 2)
        for i, face in enumerate(controls.POINTS)}}))
    (root / 'source.json').write_text(json.dumps(expected, sort_keys=True, indent=2) + '\n')
    half = {'triangles': {k: v for k, v in expected['triangles'].items() if k in ('0,0,0,0', '0,0,1,0')}}
    (root / 'source-left.json').write_text(json.dumps(half, sort_keys=True, indent=2) + '\n')
    big = 2 * FLOOR
    cases = []

    def case(name, edit, check=None, counts=None, source='source.json', groups=GROUPS, extra=None):
        out = root / name
        out.mkdir(exist_ok=True)
        doc = base()
        files = {uri: controls.glb(ids, leaf=uri.startswith('leaf')) for uri, ids in groups.items()}
        edit(doc, files)
        for uri, data in files.items():
            (out / uri).write_bytes(data)
        (out / 'tileset.json').write_text(json.dumps(doc))
        cases.append(dict(name=name, check=check, counts=counts, source=source, **(extra or {})))

    def omitted_root(doc, files):
        omitted(doc['root'], big)
        doc['geometricError'] = big
        files.pop('root.glb')
    case('omitted-root', omitted_root,
         counts=dict(omittedCells=1, omissionSources=1, omittedByDepth=[1], frontierTiles=2, frontierLeaves=4),
         extra=dict(producer={'hlod': {'omittedProxies': 1, 'omissionSources': 1, 'omittedByDepth': [1, 0]}}))

    def chain(doc, files):
        omitted_root(doc, files)
        omitted(doc['root']['children'][1], FLOOR * 1.5)
        files.pop('right.glb')
    case('omitted-chain', chain,
         counts=dict(omittedCells=2, omissionSources=1, omittedByDepth=[1, 1], frontierTiles=2, frontierLeaves=4),
         extra=dict(producer={'hlod': {'omittedProxies': 2, 'omissionSources': 1, 'omittedByDepth': [1, 1]}},
                    mismatch={'hlod': {'omittedProxies': 2, 'omissionSources': 2, 'omittedByDepth': [2]}}))

    def external(doc, files):
        chain(doc, files)
        right = doc['root']['children'][1]
        files['right.json'] = json.dumps(dict(asset={'version': '1.1'}, geometricError=right['geometricError'],
                                             root=right)).encode()
        doc['root']['children'][1] = dict(boundingVolume=copy.deepcopy(BOX), geometricError=right['geometricError'],
                                          content={'uri': 'right.json'})
    # The link placeholder adds no cell depth; the omitted document root is a cell.
    case('omitted-external-root', external,
         counts=dict(omittedCells=2, omissionSources=1, omittedByDepth=[1, 1], frontierTiles=2, frontierLeaves=4))

    def children_error(doc, files):
        omitted_root(doc, files)
        doc['root']['geometricError'] = 2
    case('omitted-children-error', children_error, check='emptyCellRefinement')

    def below_floor(doc, files):
        chain(doc, files)
        doc['root']['children'][1]['geometricError'] = FLOOR * (1 - 1e-9)
    case('omitted-one-below-floor', below_floor, check='emptyCellRefinement')

    def external_children_error(doc, files):
        external(doc, files)
        linked = json.loads(files['right.json'])
        linked['root']['geometricError'] = 1
        files['right.json'] = json.dumps(linked).encode()
    case('omitted-external-children-error', external_children_error, check='emptyCellRefinement')

    def content_above_branch_end(doc, files):
        right = doc['root']['children'][1]
        right.pop('children')
        omitted(right, 2)
        for uri in ('right.glb', 'middle.glb', 'leaf2.glb', 'leaf3.glb'):
            files.pop(uri)
        files['root.glb'] = controls.glb([0, 1], leaf=False)
    # No renderable descendant, so the REPLACE barrier is silent: only omission sees it.
    case('content-above-empty-branch', content_above_branch_end, check='omissionPropagation', source='source-left.json')

    def terminal(doc, files):
        content_above_branch_end(doc, files)
        omitted(doc['root'], big)
        doc['geometricError'] = big
        files.pop('root.glb')
    case('terminal-empty-cell', terminal, check='frontierCoverage', source='source-left.json')

    def add(doc, files):
        doc['root']['children'][0]['refine'] = 'ADD'
    case('add-over-leaves', add, check='frontierCoverage')

    def cloned(doc, files):
        # Withdrawn 8074b7db: an omitted cell repeats its children's payloads
        # under new member names, rendering the leaves' cover twice.
        doc['root'].pop('content')
        doc['root']['contents'] = [{'uri': 'f0-d0-0.glb'}, {'uri': 'f0-d0-1.glb'}]
        files['f0-d0-0.glb'] = files['left.glb']
        files['f0-d0-1.glb'] = files['right.glb']
        files.pop('root.glb')
    case('cloned-frontier', cloned, check='frontierCoverage')

    def implicit(g):
        def edit(doc, files):
            files.clear()
            files['2-0-0.glb'] = controls.glb([0, 1, 2, 3], leaf=True)
            # Cell (2,0,0) holds the leaf's Z-up bounds x [0,16], y -2, z [-4,4].
            r = tile('{level}-{x}-{y}.glb', g)
            r['boundingVolume'] = {'box': [48, 40, 2, 64, 0, 0, 0, 64, 0, 0, 0, 64]}
            r['implicitTiling'] = dict(subdivisionScheme='QUADTREE', subtreeLevels=3, availableLevels=3,
                                       subtrees={'uri': '{level}-{x}-{y}.subtree'})
            doc['root'], doc['geometricError'] = r, g
            subtree = dict(buffers=[{'byteLength': 11}],
                           bufferViews=[dict(buffer=0, byteOffset=0, byteLength=3), dict(buffer=0, byteOffset=8, byteLength=3)],
                           tileAvailability={'bitstream': 0}, contentAvailability=[{'bitstream': 1}],
                           childSubtreeAvailability={'constant': 0})
            data = json.dumps(subtree, separators=(',', ':')).encode()
            data += b' ' * (-len(data) % 8)
            # Tiles 0, 1 and 5 (root, level 1 Morton 0, level 2 Morton 0); content only at 5.
            binary = bytes([0x23]) + bytes(7) + bytes([0x20]) + bytes(7)
            files['0-0-0.subtree'] = b'subt' + struct.pack('<IQQ', 1, len(data), len(binary)) + data + binary
        return edit
    case('implicit-omitted', implicit(8192),
         counts=dict(omittedCells=2, omissionSources=1, omittedByDepth=[1, 1], frontierTiles=1, frontierLeaves=1))
    case('implicit-omitted-children-error', implicit(8), check='emptyCellRefinement')
    (root / 'cases.json').write_text(json.dumps(cases, indent=2) + '\n')
    return cases


def run(root, binary=None):
    from oracle import audit, Failure
    import omission
    root = Path(root)
    cases = generate(root)
    results = []
    if binary:
        binary = Path(binary).resolve(strict=True)
        digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    for c in cases:
        folder = root / c['name']
        try:
            result = audit(folder, root / c['source'], scratch=root, work_limit=2_000_000)
            assert c['check'] is None, (c, result)
            derived = {k: result['omission'][k] for k in c['counts']}
            assert derived == c['counts'], (c['name'], derived)
            oracle = 'pass'
        except Failure as e:
            assert e.check == c['check'], (c['name'], e.check, e.detail)
            oracle = e.check
        # The archive-only owner reaches the same verdict without the source.
        try:
            alone = omission.audit(folder)
            assert c['check'] is None, (c['name'], alone)
        except omission.Failure as e:
            assert e.check == c['check'], (c['name'], e.check, e.detail)
        for key, ok in (('producer', True), ('mismatch', False)):
            if key in c:
                manifest = root / f'{c["name"]}-{key}.json'
                manifest.write_text(json.dumps(c[key], sort_keys=True) + '\n')
                compared = omission.audit(folder, manifest)
                assert compared['ok'] is ok and compared['producerReport']['ok'] is ok, (c['name'], compared)
                try:
                    audit(folder, root / c['source'], scratch=root, work_limit=2_000_000, producer_manifest=manifest)
                    assert ok, c['name']
                except Failure as e:
                    assert not ok and e.check == 'omissionReport', (c['name'], e.check)
        native = 'not run'
        if binary:
            archive = folder.with_suffix('.3tz')
            packed = subprocess.run([str(binary), 'convert', '-i', str(folder), '-o', str(archive), '--force', '--json'],
                                    capture_output=True, timeout=30)
            assert packed.returncode == 0, (c['name'], packed.stdout[-2000:], packed.stderr[-2000:])
            checked = subprocess.run([str(binary), 'validate', str(archive), '--json'], capture_output=True, timeout=30)
            report = json.loads(checked.stdout)
            if c['check']:
                assert checked.returncode == 3 and c['check'] in report['error']['message'], (c['name'], report)
                native = c['check']
            else:
                assert checked.returncode == 0, (c['name'], report)
                census = report['interfaceCensus']
                for key, value in c['counts'].items():
                    assert census[key] == value, (c['name'], key, census[key], value)
                native = 'pass'
        results.append(dict(name=c['name'], oracle=oracle, native=native))
    if binary:
        assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest, 'binary changed'
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('outputdir', type=Path)
    parser.add_argument('--binary', type=Path)
    args = parser.parse_args()
    print(json.dumps(run(args.outputdir, args.binary), indent=2))
