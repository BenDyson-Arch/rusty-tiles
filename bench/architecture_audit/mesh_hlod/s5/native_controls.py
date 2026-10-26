#!/usr/bin/env python3
"""R7 native scene, frame and decoded-gap controls."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import controls
IDENTITY = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]
LEFT = [(0, 0, 0), (0, 100, 0), (-100, 0, 0)]
RIGHT = [(0, 100, 0), (0, 0, 0), (100, 0, 0)]

def unpack(raw):
    n = struct.unpack_from('<I', raw, 12)[0]
    return json.loads(raw[20:20 + n]), bytearray(raw[28 + n:])

def pack(doc, binary):
    doc['buffers'][0]['byteLength'] = len(binary)
    text = json.dumps(doc, separators=(',', ':')).encode()
    text += b' ' * (-len(text) % 4)
    binary += b'\x00' * (-len(binary) % 4)
    return b'glTF' + struct.pack('<III', 2, 28 + len(text) + len(binary), len(text)) + b'JSON' + text + struct.pack('<I', len(binary)) + b'BIN\x00' + binary

def triangle_column(doc, binary, key, value):
    column = doc['extensions']['EXT_structural_metadata']['propertyTables'][1]['properties'][key]
    view = doc['bufferViews'][column['values']]
    struct.pack_into('<Q', binary, view.get('byteOffset', 0), value)

def payload(points, ident=0, *, leaf=True, quantized=False, right_ids=False):
    rows = [points] * (ident + 1)
    doc, binary = unpack(controls.glb([ident], points=rows, leaf=leaf, quantized=quantized))
    doc['nodes'] = [{'mesh': 0}]
    if right_ids:
        for key, value in zip(('triangle_index', 'vertex_0', 'vertex_1', 'vertex_2'), (1, 1, 0, 3)):
            triangle_column(doc, binary, key, value)
    return pack(doc, binary)

def translated(x=0, y=0, z=0):
    result = IDENTITY.copy()
    result[12:15] = [x, y, z]
    return result

def tile(uri=None, children=None, transform=None):
    size = 1000 if uri else 10000
    result = dict(boundingVolume={'box': [0, 0, 0, size, 0, 0, 0, size, 0, 0, 0, size]}, geometricError=0, refine='REPLACE')
    if uri:
        result['content'] = {'uri': uri}
    if children:
        result['children'] = children
    if transform:
        result['transform'] = transform
    return result

def basic(*, quantized=False, gap=0, rebase=0, placement=None):
    right = [(x - rebase + gap, y, z) for x, y, z in RIGHT]
    files = {'left.glb': payload(LEFT, quantized=quantized), 'right.glb': payload(right, quantized=quantized, right_ids=True)}
    root = tile(children=[tile('left.glb'), tile('right.glb', transform=placement)])
    return dict(asset={'version': '1.1'}, geometricError=0, root=root), files

def axis(*, missing=False, common=False):
    left = [(0, 0, 0), (100, 0, 0), (0, 10, 0)]
    y = 0 if common else 10
    right = [(100, y, 0), (0, y, 0), (100, y + 10, 0)]
    proxy = [(x, y, z + 0.25) for x, y, z in left] if missing else left
    files = {'left.glb': payload(proxy, leaf=False), 'leafl.glb': payload(left), 'right.glb': payload(right, 1, leaf=False), 'leafr.glb': payload(right, 1)}
    root = tile(children=[tile('left.glb', [tile('leafl.glb')]), tile('right.glb', [tile('leafr.glb')], None if common else translated(z=-10))])
    return dict(asset={'version': '1.1'}, geometricError=0, root=root), files

def scene(kind):
    tileset, files = basic()
    doc, binary = unpack(files['left.glb'])
    primitive = doc['meshes'][0]['primitives'][0]
    if kind == 'animation':

        def accessor(values, shape, count):
            binary.extend(b'\x00' * (-len(binary) % 4))
            view = len(doc['bufferViews'])
            doc['bufferViews'].append(dict(buffer=0, byteOffset=len(binary), byteLength=4 * len(values)))
            binary.extend(struct.pack('<' + 'f' * len(values), *values))
            index = len(doc['accessors'])
            doc['accessors'].append(dict(bufferView=view, componentType=5126, type=shape, count=count))
            return index
        times = accessor([0, 1, 2], 'SCALAR', 3)
        poses = accessor([0, 0, 0, 1, 0, 0, 2, 0, 0], 'VEC3', 3)
        doc['animations'] = [dict(samplers=[dict(input=times, output=poses, interpolation='LINEAR')], channels=[dict(sampler=0, target=dict(node=0, path='translation'))])]
    elif kind == 'ambiguous-scene':
        doc.pop('scene')
        doc['scenes'].append({'nodes': []})
    elif kind == 'rtc-extension':
        doc['extensions']['CESIUM_RTC'] = {'center': [1, 0, 0]}
        doc['extensionsUsed'].append('CESIUM_RTC')
    elif kind == 'material-neutral-ior':
        doc['materials'][0].setdefault('extensions', {})['KHR_materials_ior'] = {'ior': 1.4}
        doc['extensionsUsed'].append('KHR_materials_ior')
    elif kind == 'active-morph':
        doc['meshes'][0]['weights'] = [1]
        primitive['targets'] = [{'POSITION': primitive['attributes']['POSITION']}]
    elif kind == 'gpu-instancing':
        doc['nodes'][0]['extensions'] = {'EXT_mesh_gpu_instancing': {'attributes': {'TRANSLATION': primitive['attributes']['POSITION']}}}
        doc['extensionsUsed'].append('EXT_mesh_gpu_instancing')
    else:
        raise ValueError(kind)
    files['left.glb'] = pack(doc, binary)
    return tileset, files

def generate():
    cases = [('quantized-crack-one-step', basic(quantized=True, gap=1), 3, 'sourceIdentity'), ('rebase-control', basic(), 0, None), ('rebase-identical-world', basic(rebase=10, placement=translated(x=10)), 0, None), ('axis-control', axis(), 0, None), ('axis-missing-interface', axis(missing=True), 3, 'interfacePoints'), ('axis-commonframe-missing-interface', axis(missing=True, common=True), 3, 'interfacePoints')]
    for kind, code, label in [('animation', 2, 'animation'), ('ambiguous-scene', 2, 'selected glTF scene'), ('rtc-extension', 2, 'CESIUM_RTC'), ('material-neutral-ior', 0, None), ('active-morph', 2, 'morph'), ('gpu-instancing', 2, 'EXT_mesh_gpu_instancing')]:
        cases.append(('scene-' + kind, scene(kind), code, label))
    cases.append(('one-ulp-frame-gap', basic(rebase=1, placement=translated(x=float.fromhex('0x1.0000000000001p+0'))), 3, 'sourceIdentity'))
    for axis_name, code, label in [('Y', 0, None), ('Z', 2, 'up-axis override')]:
        doc, files = basic()
        doc['asset']['gltfUpAxis'] = axis_name
        cases.append(('up-axis-' + axis_name, (doc, files), code, label))
    return cases

def run(root, binary):
    root, binary = Path(root), Path(binary).resolve(strict=True)
    root.mkdir(parents=True, exist_ok=True)
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    results = []
    for name, (doc, files), code, label in generate():
        files = {**files, 'tileset.json': json.dumps(controls.refinable(doc)).encode()}
        folder = root / name
        folder.mkdir(exist_ok=True)
        for member, data in files.items():
            (folder / member).write_bytes(data)
        archive = root / (name + '.3tz')
        packed = subprocess.run([str(binary), 'convert', '-i', str(folder), '-o', str(archive), '--force', '--json'], capture_output=True, timeout=20)
        assert packed.returncode == 0, (name, packed.stdout[-2000:])
        checked = subprocess.run([str(binary), 'validate', str(archive), '--json', '--expect-profile', 'hlod'], capture_output=True, timeout=20)
        report = json.loads(checked.stdout)
        assert checked.returncode == code, (name, code, checked.returncode, report)
        if code:
            expected = 'unsupported' if code == 2 else 'invalid_input'
            assert report.get('ok') is False and report['error']['code'] == expected, (name, report)
            assert label in report['error']['message'], (name, label, report)
        else:
            assert {'hlodFeatureIds', 'interfaceSegments', 'interfaceWinding'} <= set(report['checks']), name
            census = report['interfaceCensus']
            assert census['basis'] == 'decoded leaf canonical f32 keys', (name, census)
            assert census['sourceOpenEdgesWithSharedEndpoints'] <= census['sourceOpenEdges'], (name, census)
            if name in ('rebase-control', 'rebase-identical-world', 'axis-control',
                        'scene-material-neutral-ior', 'up-axis-Y'):
                assert (census['sharedPoints'], census['sharedSegments'],
                        census['sourceOpenEdges'], census['sourceOpenEdgesWithSharedEndpoints']) == (2, 1, 4, 0), (name, census)
        results.append(dict(name=name, exit=code, check=label))
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest, 'binary changed'
    return dict(binarySha256=digest, results=results)
if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('outputdir', type=Path)
    parser.add_argument('--binary', required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps(run(args.outputdir, args.binary), indent=2))
