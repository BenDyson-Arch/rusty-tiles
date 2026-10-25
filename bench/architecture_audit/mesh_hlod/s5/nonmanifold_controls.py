#!/usr/bin/env python3
"""Locally balanced edges must survive every globally incident ownership cut.

Literal source geometry includes two entirely closed leaves with no open edges.
No producer incidence, geometry or report supplies the independent expectations.
"""
import argparse
import hashlib
import json
import struct
from pathlib import Path
import subprocess

import controls
import native_controls
from oracle import audit, Failure


A, B = [0, 0, 0], [2, 0, 0]
C, D, E, F = [0, 2, 0], [2, -2, 0], [0, 0, 2], [0, 0, -2]


def payload(ids, points, leaf):
    doc, binary = native_controls.unpack(controls.glb(ids, points=points, leaf=leaf))
    doc['nodes'] = [{'mesh': 0}]
    return native_controls.pack(doc, binary)


def generate(root):
    root = Path(root)
    root.mkdir(parents=True, exist_ok=True)
    cases = []
    for kind, left, right in (
        ('three', [[A, B, C], [B, A, D]], [[A, B, E]]),
        ('both-balanced', [[A, B, C], [B, A, D]], [[A, B, E], [B, A, F]]),
        ('both-closed', [[A, B, C], [B, A, E], [A, C, E], [B, E, C]],
                        [[A, B, D], [B, A, F], [A, D, F], [B, F, D]]),
    ):
        original = left + right
        source = {'triangles': {
            f'0,0,{i},0': dict(xyz=xyz, vertices=[0, 1, 2], expanded_ordinal=i,
                material_index=i % 2, material=controls.MATERIALS[i % 2],
                normals=controls.NORMALS, uvs=controls.UVS)
            for i, xyz in enumerate(original)
        }}
        source_path = root / (kind + '-source.json')
        source_path.write_text(json.dumps(source))
        variants = [(False, False), (True, False)]
        if kind == 'three':
            variants.append((False, True))
        for lost, one_incidence in variants:
            name = kind + ('-one-incidence' if one_incidence else
                           '-lost' if lost else '-retained')
            folder = root / name
            folder.mkdir(exist_ok=True)
            coarse = original.copy()
            if lost:
                # Exchange AB for the other quadrilateral diagonal. A and B
                # and every outside boundary edge remain present.
                a, b, c = left[0]
                _, _, d = left[1]
                coarse[:2] = [[a, d, c], [b, c, d]]
            split = len(left)
            # A locked unordered edge need not retain every incident source
            # face. One correctly oriented witness remains valid when the
            # source offers both directions along a nonmanifold interface.
            coarse_ids = [0] if one_incidence else list(range(split))
            files = {'coarse.glb': payload(coarse_ids, coarse, False),
                     'left.glb': payload(list(range(split)), original, True),
                     'right.glb': payload(list(range(split, len(original))), original, True)}
            tile = native_controls.tile
            doc = dict(asset={'version': '1.1'}, geometricError=0,
                root=tile(children=[tile('coarse.glb', [tile('left.glb')]), tile('right.glb')]))
            files['tileset.json'] = json.dumps(controls.refinable(doc)).encode()
            for member, data in files.items():
                (folder / member).write_bytes(data)
            cases.append((name, folder, source_path, lost))
    # Original identity remains authoritative when no geometric boundary
    # survives reduction, including when the separated boxes do not overlap.
    vertices = [A, B, C, D, E, F]
    faces = [(0, 1, 2), (1, 0, 4), (0, 2, 4), (1, 4, 2),
             (0, 1, 3), (1, 0, 5), (0, 3, 5), (1, 5, 3)]
    xyz = [[vertices[v] for v in face] for face in faces]
    source = {'triangles': {f'0,0,0,{i}': dict(xyz=points, vertices=faces[i],
        expanded_ordinal=0, material_index=0, material=controls.MATERIALS[0],
        normals=controls.NORMALS, uvs=controls.UVS) for i, points in enumerate(xyz)}}
    source_path = root / 'shared-closed-source.json'
    source_path.write_text(json.dumps(source))
    def shared(ids, leaf, gap=0):
        doc, data = native_controls.unpack(payload(ids, xyz, leaf))
        doc['nodes'][0]['translation'] = [0, 0, gap]
        for primitive in doc['meshes'][0]['primitives']:
            primitive['material'] = 0
        metadata = doc['extensions']['EXT_structural_metadata']
        for table in metadata['propertyTables']:
            schema = metadata['schema']['classes'][table['class']]['properties']
            for key, column in table['properties'].items():
                view = doc['bufferViews'][column['values']]
                wide = schema[key]['componentType'] == 'UINT64'
                for row, face in enumerate(ids):
                    value = face if key == 'triangle_index' else (
                        faces[face][int(key[-1])] if key.startswith('vertex_') else 0)
                    struct.pack_into('<Q' if wide else '<I', data,
                        view.get('byteOffset', 0) + row * (8 if wide else 4), value)
        return native_controls.pack(doc, data)
    for name, gap in [('shared-closed-retained', 0),
                      ('shared-closed-gap', -.125), ('shared-closed-disjoint', -10)]:
        folder = root / name
        folder.mkdir(exist_ok=True)
        files = {'coarse.glb': shared(list(range(4)), False),
                 'left.glb': shared(list(range(4)), True),
                 'right.glb': shared(list(range(4, 8)), True, gap)}
        tile = native_controls.tile
        files['tileset.json'] = json.dumps(controls.refinable(dict(asset={'version': '1.1'},
            geometricError=0, root=tile(children=[
                tile('coarse.glb', [tile('left.glb')]), tile('right.glb')])))).encode()
        for member, data in files.items():
            (folder / member).write_bytes(data)
        cases.append((name, folder, source_path, bool(gap)))
    return cases


def run(root, binary=None):
    results = []
    if binary:
        binary = Path(binary).resolve(strict=True)
        digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    for name, folder, source, lost in generate(root):
        identity = name.startswith('shared-closed-')
        oracle_check = 'interfacePoints' if identity else 'interfaceSegments'
        archive_check = 'sourceIdentity' if identity else 'interfaceSegments'
        try:
            result = audit(folder, source, scratch=root, work_limit=100_000)
            assert not lost, (name, result)
            assert result['checkedSegments'] >= 3, (name, result)
            verdict = 'pass'
        except Failure as error:
            assert lost and error.check == oracle_check, (name, error)
            verdict = error.check
        if binary:
            archive = folder.with_suffix('.3tz')
            packed = subprocess.run([str(binary), 'convert', '-i', str(folder),
                '-o', str(archive), '--force', '--json'], capture_output=True, timeout=20)
            assert packed.returncode == 0, (name, packed.stdout[-2000:])
            checked = subprocess.run([str(binary), 'validate', str(archive), '--json'],
                capture_output=True, timeout=20)
            report = json.loads(checked.stdout)
            if lost:
                assert checked.returncode == 3, (name, report)
                assert archive_check in report['error']['message'], (name, report)
            else:
                assert checked.returncode == 0, (name, report)
                assert {'hlodFeatureIds', 'replaceContentContinuity', 'interfaceSegments',
                        'interfacePoints', 'interfaceWinding'} <= set(report['checks']), name
        results.append(dict(name=name, oracle=verdict,
            native=(archive_check if lost else 'pass') if binary else 'not run'))
    if binary:
        assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest, 'binary changed'
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('outputdir', type=Path)
    parser.add_argument('--binary', type=Path)
    args = parser.parse_args()
    print(json.dumps(run(args.outputdir, args.binary), indent=2))
