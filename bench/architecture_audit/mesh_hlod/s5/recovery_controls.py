#!/usr/bin/env python3
"""Literal root-recovery archives and named controls; no producer imports."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import subprocess

import controls
import native_controls
import omission
from omission_controls import tile


def key(first, end, depth):
    return dict(first=first, end=end, depth=depth, leafBegin=first, leafEnd=end)


def base():
    leaves = [tile(f'leaf{i}.glb', 0) for i in range(4)]
    for i, leaf in enumerate(leaves):
        leaf['extras'] = dict(first=str(i), end=str(i+1), depth=2)
    left = tile('left.glb', 2, leaves[:2])
    left['extras'] = dict(first='0', depth=1)
    root = tile('root.glb', 4, [left, *leaves[2:]])
    root['extras'] = dict(first='0', end='4', depth=0, recovery=dict(
        version=1, root=key(0, 4, 0), encodedDepth=0,
        frontier=[dict(key=key(0, 2, 1), triangles=2),
                  dict(key=key(2, 3, 2), triangles=1),
                  dict(key=key(3, 4, 2), triangles=1)], omitted=[key(2, 4, 1)]))
    files = {f'leaf{i}.glb': controls.glb([i]) for i in range(4)}
    files['left.glb'] = controls.glb([0, 1], leaf=False)
    doc, binary = native_controls.unpack(controls.glb([0, 1, 2, 3], leaf=False))
    doc['asset']['extras'] = {'rustyTiles': {'descendantRecovery': 1}}
    files['root.glb'] = native_controls.pack(doc, binary)
    return dict(asset={'version': '1.1'}, geometricError=4, root=root), files


def generate(root):
    root = Path(root)
    root.mkdir(parents=True, exist_ok=True)
    source = {'triangles': {
        f'0,0,{i},0': dict(xyz=[controls.transformed(p) for p in face],
            normals=[[0, -1, 0], [-1, 0, 0], [0, 0, 1]], uvs=controls.UVS,
            vertices=[0, 1, 2], material=controls.MATERIALS[i % 2],
            expanded_ordinal=i, material_index=i % 2)
        for i, face in enumerate(controls.POINTS)}}
    (root / 'source.json').write_text(json.dumps(source, sort_keys=True, indent=2) + '\n')
    cases = []

    def case(name, edit=lambda doc, files: None, check=None, native=None):
        doc, files = base()
        edit(doc, files)
        folder = root / name
        folder.mkdir(exist_ok=True)
        for uri, data in files.items():
            (folder / uri).write_bytes(data)
        (folder / 'tileset.json').write_text(json.dumps(doc, sort_keys=True) + '\n')
        cases.append(dict(name=name, check=check, native=native or check))

    case('complete-frontier')

    case('noncanonical-root-rank', lambda doc, files: doc['root']['extras'].update(first='00'), 'recoveryRelation')
    case('signed-root-rank', lambda doc, files: doc['root']['extras'].update(first='+0'), 'recoveryRelation')
    case('long-root-rank', lambda doc, files: doc['root']['extras'].update(first='0'*65536), 'recoveryRelation')
    case('noncanonical-frontier-rank', lambda doc, files: doc['root']['children'][0]['extras'].update(first='00'), 'recoveryOwnership')
    case('oversized-declaration', lambda doc, files: doc['root']['extras']['recovery'].update(omitted=['x'*65536]), 'recoveryRelation', 'recoveryBounds')

    def relation(doc):
        return doc['root']['extras']['recovery']

    def gap(doc, files):
        relation(doc)['frontier'][1]['key']['first'] = 3
    case('coverage-gap', gap, 'recoveryCoverage')

    def overlap(doc, files):
        relation(doc)['frontier'][1]['key']['first'] = 1
        relation(doc)['frontier'][1]['key']['leafBegin'] = 1
    case('coverage-overlap', overlap, 'recoveryCoverage')

    def undeclared(doc, files):
        del doc['root']['extras']['recovery']
    case('undeclared-relation', undeclared, 'recoveryRelation')

    def low(doc, files):
        doc['root']['geometricError'] = 1
    case('g-below-frontier', low, 'recoveryError', 'geometricError increases')

    def many(doc, files):
        r = relation(doc)
        r['frontier'] = r['frontier'] * 3
    case('input-count-bound', many, 'recoveryBounds')

    def oversized(doc, files):
        relation(doc)['frontier'][0]['triangles'] = 8193
    case('input-triangle-bound', oversized, 'recoveryBounds')

    def count(doc, files):
        relation(doc)['frontier'][0]['triangles'] = 1
    case('input-count-mismatch', count, 'recoveryBounds')

    def depth(doc, files):
        relation(doc)['frontier'][0]['key']['depth'] = 2
    case('ownership-depth', depth, 'recoveryOwnership')

    def boolean_depth(doc, files):
        doc['root']['children'][0]['extras']['depth'] = True
    case('noninteger-ownership-depth', boolean_depth, 'recoveryOwnership')

    def scaffold(doc, files):
        relation(doc)['omitted'][0]['depth'] = 2
    case('ownership-scaffold', scaffold, 'recoveryOwnership')

    def marker(doc, files):
        payload, binary = native_controls.unpack(files['root.glb'])
        del payload['asset']['extras']
        files['root.glb'] = native_controls.pack(payload, binary)
    case('missing-payload-declaration', marker, 'recoveryRelation')

    def null_marker(doc, files):
        payload, binary = native_controls.unpack(files['root.glb'])
        payload['asset']['extras']['rustyTiles']['descendantRecovery'] = None
        files['root.glb'] = native_controls.pack(payload, binary)
    case('null-payload-declaration', null_marker, 'recoveryRelation')

    def nullable(doc, files):
        doc['root']['children'][0]['children'][0]['extras'] = None
        payload, binary = native_controls.unpack(files['leaf0.glb'])
        payload['asset']['extras'] = None
        files['leaf0.glb'] = native_controls.pack(payload, binary)
    case('unrelated-null-extras', nullable)

    def linked(doc, files):
        left = doc['root']['children'][0]
        files['left.json'] = json.dumps(dict(asset={'version': '1.1'}, geometricError=2, root=left)).encode()
        link = tile('left.json', 2)
        doc['root']['children'][0] = link
    case('linked-frontier', linked)

    def collapsed(doc, files, shift=2):
        r = relation(doc)
        r['encodedDepth'] = shift
        for child in r['frontier']:
            child['key']['depth'] += shift
        r['omitted'][0]['depth'] += shift
        pending = list(doc['root']['children'])
        while pending:
            child = pending.pop()
            child['extras']['depth'] += shift
            pending.extend(child.get('children', []))
    case('collapsed-root-chain', collapsed)
    case('deepest-frontier-leaves', lambda doc, files: collapsed(doc, files, 29))
    (root / 'cases.json').write_text(json.dumps(cases, indent=2) + '\n')
    return cases


def run(root, binary=None):
    from oracle import audit, Failure
    root = Path(root)
    cases = generate(root)
    if binary:
        binary = Path(binary).resolve(strict=True)
        digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    results = []
    for case in cases:
        folder = root / case['name']
        try:
            result = omission.audit(folder)
            assert case['check'] is None, (case, result)
            assert result['recoveredRoots'] == 1 and result['bypassedCells'] == 1
            assert result['omittedCells'] == 0
        except omission.Failure as error:
            assert error.check == case['check'], (case, error.check, error.detail)
        try:
            result = audit(folder, root / 'source.json', scratch=root, work_limit=2_000_000)
            assert case['check'] is None, (case, result)
        except Failure as error:
            expected = 'geometricErrorOrder' if case['name'] == 'g-below-frontier' else case['check']
            assert error.check == expected, (case, error.check, error.detail)
        native = 'not run'
        if binary:
            archive = folder.with_suffix('.3tz')
            packed = subprocess.run([str(binary), 'convert', '-i', str(folder), '-o', str(archive),
                '--force', '--json'], capture_output=True, timeout=30)
            assert packed.returncode == 0, (case, packed.stdout[-2000:], packed.stderr[-2000:])
            checked = subprocess.run([str(binary), 'validate', str(archive), '--expect-profile', 'hlod',
                '--json'], capture_output=True, timeout=30)
            report = json.loads(checked.stdout)
            if case['check']:
                assert checked.returncode == 3 and case['native'] in report['error']['message'], (case, report)
                native = case['native']
            else:
                assert checked.returncode == 0, (case, report)
                native = 'pass'
        results.append(dict(name=case['name'], oracle=case['check'] or 'pass', native=native))
    if binary:
        assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest
    return results + borders(root / 'borders', binary)


def borders(root, binary):
    """Contracted mixed-depth replacements retain their original D11 owners."""
    import border_controls as bc
    from oracle import audit, Failure
    results = []
    for name, variant, expected in [('mixed-depth-frontier', 'mixed-within', None),
                                    ('same-depth-frontier', 'contracted', None),
                                    ('same-depth-gap', 'same-depth-gap', 'sameDepthBorderGap')]:
        folder = root / name
        folder.mkdir(parents=True, exist_ok=True)
        doc, files, source, records = bc.build(bc.Case(variant))
        r = doc['root']
        if variant == 'mixed-within':
            r['children'][1] = r['children'][1]['children'][0]
            files.pop('B.glb')
            skipped = dict(first=32, end=64, depth=1, leafBegin=2, leafEnd=4)
            depths = [1, 2]
            records = [x for x in records if x['kind'] == 'tile' and x['tile']['uri'] != 'B.glb']
        else:
            skipped = dict(first=0, end=64, depth=1, leafBegin=0, leafEnd=4)
            depths = [2, 2]
            records = [x for x in records if x['kind'] == 'tile']
            for record in records:
                record['tile']['depth'] = record['tile']['encodedDepth'] = 2
                record['contract']['depth'] = 2
                record['contract']['scale'] = bc.scale(2)
        faces = [t['xyz'] for t in source['triangles'].values()]
        payload, data = native_controls.unpack(bc.glb(faces, bc.identities(faces), leaf=False))
        payload['asset']['extras'] = {'rustyTiles': {'descendantRecovery': 1}}
        files['root.glb'] = native_controls.pack(payload, data)
        r['content'] = {'uri': 'root.glb'}
        r['geometricError'] = doc['geometricError'] = 1.
        frontier = []
        for i, (child, depth) in enumerate(zip(r['children'], depths)):
            child['extras'] = dict(first=str(32 * i), depth=depth)
            payload, _ = native_controls.unpack(files[child['content']['uri']])
            primitive = payload['meshes'][0]['primitives'][0]
            count = payload['accessors'][primitive['indices']]['count'] // 3
            frontier.append(dict(key=dict(first=32*i, end=32*(i+1), depth=depth,
                                          leafBegin=2*i, leafEnd=2*(i+1)), triangles=count))
        r['extras'] = dict(first='0', end='64', depth=0, recovery=dict(version=1,
            root=dict(first=0, end=64, depth=0, leafBegin=0, leafEnd=4), encodedDepth=0,
            frontier=frontier, omitted=[skipped]))
        records.append(dict(schemaVersion=1, kind='omitted', key=skipped,
            reason='contraction_stalled', source=True, attemptedTopology={},
            frontier=[dict(leafBegin=c['key']['leafBegin'], leafEnd=c['key']['leafEnd'])
                      for c in frontier if skipped['first'] <= c['key']['first']]))
        root_record = copy.deepcopy(records[0])
        root_record['tile'].update(uri='root.glb', sha256=hashlib.sha256(files['root.glb']).hexdigest(),
            bytes=len(files['root.glb']), leafBegin=0, leafEnd=4, depth=0, encodedDepth=0,
            geometricError=1., topology=dict.fromkeys(bc.TOPOLOGY, 0),
            attemptedTopology=dict.fromkeys(bc.TOPOLOGY, 0))
        root_record['contract'] = dict(depth=0, scale=bc.scale(0), interface=[], certificates=[], terminalVertices=[])
        if variant == 'mixed-within':
            # The two finer border chords enclose two small planar gaps against A's
            # single chord. The exact source root fills both with zero source loss.
            for receipt in ('topology', 'attemptedTopology'):
                root_record['tile'][receipt]['closedHoles'] = 2
                root_record['tile'][receipt]['closedHoleFinalG'] = 1.
        records.append(root_record)
        for uri, data in files.items():
            (folder / uri).write_bytes(data)
        (folder / 'tileset.json').write_text(json.dumps(doc, sort_keys=True) + '\n')
        source_path, audit_path = root / f'{name}.source.json', root / f'{name}.borders.jsonl'
        source_path.write_text(json.dumps(source, sort_keys=True) + '\n')
        audit_path.write_text(''.join(json.dumps(x, sort_keys=True) + '\n' for x in records))
        try:
            result = audit(folder, source_path, border_audit=audit_path)
            assert expected is None, (name, result)
        except Failure as error:
            assert error.check == expected, (name, expected, error.check, error.detail)
        if variant == 'mixed-within':
            import recovery
            restore = recovery.ownership_view
            recovery.ownership_view = lambda tiles: tiles
            try:
                audit(folder, source_path, border_audit=audit_path)
                raise AssertionError('ownership-restoration control was insensitive')
            except Failure as error:
                assert error.check == 'borderDepth', (error.check, error.detail)
            finally:
                recovery.ownership_view = restore
        verdict = 'not run'
        if binary:
            code, report = bc.native(binary, folder, folder.with_suffix('.3tz'))
            assert (code == 0 if expected is None else code == 3 and expected in report['error']['message']), (name, report)
            verdict = expected or 'pass'
        results.append(dict(name=name, oracle=expected or 'pass', native=verdict))
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('outputdir', type=Path)
    parser.add_argument('--binary', type=Path)
    args = parser.parse_args()
    print(json.dumps(run(args.outputdir, args.binary), indent=2))
