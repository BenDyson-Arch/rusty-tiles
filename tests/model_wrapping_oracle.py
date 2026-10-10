#!/usr/bin/env python3
"""Independent W1 source/member/Decimal bounds reader with sensitive controls.

Uses only Python's standard library. No rusty-tiles parser, source/model IR,
coordinate helper, generated bounds or historical tool digest is an oracle.
"""
import argparse
from decimal import Decimal, localcontext
import hashlib
import json
import math
import os
from pathlib import Path
import posixpath
import struct
import subprocess
import tempfile
import urllib.parse
import zipfile
import zlib


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def dec(value):
    return Decimal.from_float(float(value))


def identity():
    return [[Decimal(int(r == c)) for c in range(4)] for r in range(4)]


def multiply(a, b):
    return [[sum((a[r][k] * b[k][c] for k in range(4)), Decimal(0))
             for c in range(4)] for r in range(4)]


def rotation(q):
    x, y, z, w = q
    return [[1-2*(y*y+z*z), 2*(x*y-z*w), 2*(x*z+y*w)],
            [2*(x*y+z*w), 1-2*(x*x+z*z), 2*(y*z-x*w)],
            [2*(x*z-y*w), 2*(y*z+x*w), 1-2*(x*x+y*y)]]


def local_matrix(node, normalized):
    if 'matrix' in node:
        return [[dec(node['matrix'][c*4+r]) for c in range(4)] for r in range(4)]
    q = list(map(dec, node.get('rotation', [0, 0, 0, 1])))
    if normalized:
        norm = sum((v*v for v in q), Decimal(0)).sqrt()
        q = [v/norm for v in q]
    linear = rotation(q)
    scale = list(map(dec, node.get('scale', [1, 1, 1])))
    translation = list(map(dec, node.get('translation', [0, 0, 0])))
    result = identity()
    for r in range(3):
        for c in range(3):
            result[r][c] = linear[r][c] * scale[c]
        result[r][3] = translation[r]
    return result


def envelope(document, binary):
    text = json.dumps(document, separators=(',', ':')).encode()
    text += b' ' * ((-len(text)) % 4)
    if binary is None:
        return struct.pack('<4sIIII', b'glTF', 2, 20+len(text), len(text), 0x4e4f534a)+text
    binary += b'\0' * ((-len(binary)) % 4)
    return (struct.pack('<4sIIII', b'glTF', 2, 28+len(text)+len(binary), len(text), 0x4e4f534a)
            + text + struct.pack('<II', len(binary), 0x004e4942) + binary)


def parse_document(data):
    if not data.startswith(b'glTF'):
        return json.loads(data), None
    magic, version, length, size, kind = struct.unpack_from('<4sIIII', data)
    require(magic == b'glTF' and version == 2 and length == len(data) and kind == 0x4e4f534a,
            'independent GLB envelope')
    end = 20+size
    if end == len(data):
        return json.loads(data[20:end]), None
    binary_size, binary_kind = struct.unpack_from('<II', data, end)
    require(binary_kind == 0x004e4942 and end+8+binary_size == len(data), 'independent BIN envelope')
    return json.loads(data[20:end]), data[end+8:]


def uri_member(base, uri):
    require(not urllib.parse.urlsplit(uri).scheme and not uri.startswith('/')
            and '?' not in uri and '#' not in uri, 'local URI authority')
    parts = []
    for segment in uri.split('/'):
        decoded = urllib.parse.unquote(segment, encoding='utf-8', errors='strict')
        require('/' not in decoded and '\\' not in decoded, 'encoded separator control')
        if decoded in ('', '.'):
            continue
        if decoded == '..':
            require(parts, 'resource escape control')
            parts.pop()
        else:
            parts.append(decoded)
    return posixpath.join(base, *parts)


def source_points(document, buffers, normalized):
    nodes = document['nodes']
    selected = document.get('scene', 0)
    roots = document['scenes'][selected]['nodes']
    output = []

    def values(index):
        accessor = document['accessors'][index]
        view = document['bufferViews'][accessor['bufferView']]
        require(accessor['componentType'] == 5126 and accessor['type'] == 'VEC3', 'fixture POSITION domain')
        start = view.get('byteOffset', 0)+accessor.get('byteOffset', 0)
        stride = view.get('byteStride', 12)
        return [list(map(dec, struct.unpack_from('<3f', buffers[view['buffer']], start+i*stride)))
                for i in range(accessor['count'])]

    def visit(index, parent):
        node = nodes[index]
        world = multiply(parent, local_matrix(node, normalized))
        if 'mesh' in node:
            for primitive in document['meshes'][node['mesh']]['primitives']:
                positions = values(primitive['attributes']['POSITION'])
                require('indices' not in primitive, 'fixture unindexed TRIANGLES domain')
                for position in positions:
                    p = position+[Decimal(1)]
                    yup = [sum((world[r][k]*p[k] for k in range(4)), Decimal(0)) for r in range(3)]
                    output.append([yup[0], -yup[2], yup[1]])
        for child in node.get('children', []):
            visit(child, world)

    for node in roots:
        visit(node, identity())
    return output


def expected_placement(anchor, quaternion, offset):
    if anchor is None:
        return identity()
    longitude, latitude, height = anchor
    require(latitude == 0 and longitude in (0, 90), 'independent cardinal placement reference')
    basis = [[0, 0, 1], [1, 0, 0], [0, 1, 0]] if longitude == 0 else [[-1, 0, 0], [0, 0, 1], [0, 1, 0]]
    q = list(map(dec, quaternion))
    norm = sum((v*v for v in q), Decimal(0)).sqrt()
    q = [v/norm for v in q]
    r = rotation(q)
    linear = [[sum((Decimal(basis[row][k])*r[k][column] for k in range(3)), Decimal(0))
               for column in range(3)] for row in range(3)]
    translated = list(map(dec, [offset[0], -offset[2], offset[1]]))
    origin = [Decimal(6378137)+dec(height), Decimal(0), Decimal(0)] if longitude == 0 else [Decimal(0), Decimal(6378137)+dec(height), Decimal(0)]
    result = identity()
    for row in range(3):
        result[row][:3] = linear[row]
        result[row][3] = origin[row]+sum((linear[row][k]*translated[k] for k in range(3)), Decimal(0))
    return result


def inspect_manifest_errors(manifest, report):
    require(manifest['root']['geometricError'] == report['root_geometric_error_metres'] == 0,
            'full-detail root error contract')
    require(manifest['root']['refine'] == 'REPLACE', 'explicit root replacement refinement')
    require('children' not in manifest['root'], 'one content root')
    box = list(map(dec, manifest['root']['boundingVolume']['box']))
    require(len(box) == 12 and all(v.is_finite() for v in box)
            and all(box[i] == 0 for i in (4, 5, 6, 8, 9, 10))
            and all(box[i] >= 0 for i in (3, 7, 11)), 'finite axis-aligned conservative box')
    require(box == list(map(dec, report['bounding_box'])), 'report box identity')
    require(manifest['geometricError'] == report['tileset_geometric_error_metres'],
            'report omission error identity')
    omission = dec(manifest['geometricError'])
    # Independent high-precision mathematical diagonal, not the producer's
    # sequence of outward f64 operations. The admitted box is axis aligned.
    diagonal = 2*sum((box[i]*box[i] for i in (3, 7, 11)), Decimal(0)).sqrt()
    required = max(Decimal(1), diagonal)
    require(omission.is_finite() and omission >= required,
            'positive omission error encloses box diagonal')
    if diagonal <= 1:
        require(omission == 1, 'degenerate/sub-metre omission floor')
    else:
        require(omission-required <= 16*dec(math.ulp(float(required))),
                'omission error follows conservative diagonal policy')
    return box


def inspect_members(members, source, dependencies, expected_root):
    root_name = 'model/source.glb' if source.startswith(b'glTF') else 'model/source.gltf'
    require(members[root_name] == source, 'exact source document bytes')
    document, embedded = parse_document(source)
    buffers = []
    expected_names = {'tileset.json', 'conversion.json', '@3dtilesIndex1@', root_name}
    for declaration in document['buffers']:
        if 'uri' in declaration:
            name = uri_member('model', declaration['uri'])
            require(name in members and members[name] == dependencies[name[6:]], 'buffer member association')
            expected_names.add(name)
            buffers.append(members[name][:declaration['byteLength']])
        else:
            require(embedded is not None, 'embedded BIN ownership')
            buffers.append(embedded[:declaration['byteLength']])
    for image in document.get('images', []):
        name = uri_member('model', image['uri'])
        require(name in members and members[name] == dependencies[name[6:]], 'image member association')
        expected_names.add(name)
    require(set(members) == expected_names, 'exact archive member closure')
    manifest = json.loads(members['tileset.json'])
    report = json.loads(members['conversion.json'])
    require(manifest['root']['content']['uri'] == root_name, 'synthetic content authority')
    box = inspect_manifest_errors(manifest, report)
    require(manifest['root']['transform'] == report['root_transform'], 'report matrix identity')
    matrix = [[dec(manifest['root']['transform'][column*4+row]) for column in range(4)] for row in range(4)]
    maximum = Decimal(0)
    for normalized in (False, True):
        for point in source_points(document, buffers, normalized):
            for axis, half in enumerate((box[3], box[7], box[11])):
                require(box[axis]-half <= point[axis] <= box[axis]+half, 'original node geometry outside box')
            p = point+[Decimal(1)]
            actual = [sum((matrix[row][k]*p[k] for k in range(4)), Decimal(0)) for row in range(3)]
            expected = [sum((expected_root[row][k]*p[k] for k in range(4)), Decimal(0)) for row in range(3)]
            maximum = max(maximum, *(abs(a-b) for a, b in zip(actual, expected)))
    require(maximum <= Decimal('0.000001'), 'independent placed world coordinate allowance')
    payload = {name for name in expected_names if name.startswith('model/')}
    require(report['model_payload_files'] == len(payload), 'emitted logical file count')
    require(report['model_payload_bytes'] == sum(len(members[name]) for name in payload), 'emitted alias byte count')
    require(report['source_bytes'] == len(source), 'root byte count')
    require(report['triangles'] == len(source_points(document, buffers, False))//3, 'selected triangle multiplicity')
    return {'members': len(members), 'max_world_difference_metres': str(maximum), 'report': report}


def png():
    def chunk(kind, data):
        return struct.pack('>I', len(data))+kind+data+struct.pack('>I', zlib.crc32(kind+data))
    return (b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR', struct.pack('>IIBBBBB', 1, 1, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(bytes([0, 64, 128, 255, 128])))+chunk(b'IEND', b''))


def fixture():
    binary = struct.pack('<9f', 0, 0, 0, 1, 0, 0, 0, 1, 0)
    document = {'asset': {'version': '2.0'}, 'scene': 0, 'scenes': [{'nodes': [0]}],
                'nodes': [{'mesh': 0}], 'meshes': [{'primitives': [{'attributes': {'POSITION': 0}}]}],
                'buffers': [{'byteLength': 36}], 'bufferViews': [{'buffer': 0, 'byteLength': 36}],
                'accessors': [{'bufferView': 0, 'componentType': 5126, 'count': 3,
                               'type': 'VEC3', 'min': [0, 0, 0], 'max': [1, 1, 0]}]}
    return document, binary


def call(binary, arguments):
    result = subprocess.run([str(binary), '--json', *map(str, arguments)], capture_output=True,
                            env={**os.environ, 'PATH': ''})
    value = json.loads(result.stdout)
    return result, value


def refused(binary, arguments, category):
    result, value = call(binary, arguments)
    require(result.returncode != 0 and value['error']['code'] == category, f'typed refusal {category}: {value}')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--json-output', type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    binary_hash = hashlib.sha256(binary.read_bytes()).hexdigest()
    records, controls = [], []
    with localcontext() as ctx, tempfile.TemporaryDirectory(prefix='w1-independent-') as temporary:
        ctx.prec = 80
        root = Path(temporary)
        for variant in ('identity', 'sub-metre', 'degenerate', 'translation-f32', 'near-unit-quaternion', 'nested-order', 'deep-128'):
            document, payload = fixture()
            if variant == 'sub-metre':
                document['nodes'][0]['scale'] = [0.25, 0.25, 0.25]
            elif variant == 'degenerate':
                payload = bytes(36)
                document['accessors'][0]['max'] = [0, 0, 0]
            elif variant == 'translation-f32':
                document['nodes'][0]['translation'] = [0.1, -0.3, 0.2]
            elif variant == 'near-unit-quaternion':
                document['nodes'][0]['rotation'] = [0, 0, 0.7071069, 0.7071069]
                document['nodes'][0]['translation'] = [12345.125, -30000.5, 55.5]
            elif variant == 'nested-order':
                document['nodes'] = [{'children': [1], 'translation': [99999.125, -20000.5, 0],
                                      'rotation': [0, 0, math.sqrt(0.5), math.sqrt(0.5)]},
                                     {'mesh': 0, 'translation': [0.1, 0.3, -7.25], 'scale': [-2, 3, 4]}]
            elif variant == 'deep-128':
                document['nodes'] = [{'children': [i+1], 'translation': [0.1, -0.3, 0.2]} for i in range(127)]
                document['nodes'].append({'mesh': 0, 'translation': [-12.7, 38.1, -25.4]})
            source = envelope(document, payload)
            input_path = root/f'{variant}.glb'
            input_path.write_bytes(source)
            for anchor in (None, [0, 0, 10], [90, 0, 25]):
                label = f'{variant}-{anchor}'
                output = root/f'{len(records)}.3tz'
                quaternion = [0, 0, math.sqrt(0.5), math.sqrt(0.5)]
                offset = [10, 2, -5]
                placement = [] if anchor is None else ['--anchor', *anchor, '--orientation-xyzw', *quaternion, '--scene-offset', *offset]
                result, frontend = call(binary, ['glb-to-3tz', '-i', input_path, '-o', output, *placement])
                require(result.returncode == 0, f'{label}: {result.stderr.decode()} {frontend}')
                with zipfile.ZipFile(output) as archive:
                    members = {name: archive.read(name) for name in archive.namelist()}
                inspected = inspect_members(members, source, {}, expected_placement(anchor, quaternion, offset))
                require(frontend['modelReport'] == inspected['report'], 'CLI/archive report equality')
                records.append({'case': label, 'source_sha256': hashlib.sha256(source).hexdigest(),
                                'artifact_sha256': hashlib.sha256(output.read_bytes()).hexdigest(), **inspected})
                if variant == 'identity' and anchor is None:
                    for label, omission, remove_refine in (
                            ('zero-tileset-omission-error', 0, False),
                            ('understated-tileset-omission-error', 1, False),
                            ('missing-root-refinement', None, True)):
                        corrupted = dict(members)
                        wrong = json.loads(corrupted['tileset.json'])
                        report = json.loads(corrupted['conversion.json'])
                        if omission is not None:
                            wrong['geometricError'] = omission
                            report['tileset_geometric_error_metres'] = omission
                        if remove_refine:
                            del wrong['root']['refine']
                        corrupted['tileset.json'] = json.dumps(wrong).encode()
                        corrupted['conversion.json'] = json.dumps(report).encode()
                        try:
                            inspect_members(corrupted, source, {}, identity())
                        except (AssertionError, KeyError):
                            controls.append(label)
                        else:
                            raise AssertionError(label+' control was insensitive')
                if variant == 'translation-f32' and anchor is None:
                    corrupted = dict(members)
                    wrong = json.loads(corrupted['tileset.json'])
                    wrong['root']['boundingVolume']['box'][3] = 0.49
                    report = json.loads(corrupted['conversion.json'])
                    report['bounding_box'] = wrong['root']['boundingVolume']['box']
                    # Keep the omission policy consistent with the altered box,
                    # so this control specifically exercises geometry enclosure.
                    b = list(map(dec, report['bounding_box']))
                    diagonal = 2*sum((b[i]*b[i] for i in (3, 7, 11)), Decimal(0)).sqrt()
                    omission = max(1, math.nextafter(float(diagonal), math.inf))
                    wrong['geometricError'] = report['tileset_geometric_error_metres'] = omission
                    corrupted['tileset.json'] = json.dumps(wrong).encode()
                    corrupted['conversion.json'] = json.dumps(report).encode()
                    try:
                        inspect_members(corrupted, source, {}, identity())
                    except AssertionError:
                        controls.append('shrunken-original-box')
                    else:
                        raise AssertionError('shrunken bounds control was insensitive')
                if variant == 'nested-order' and anchor is not None:
                    corrupted = dict(members)
                    wrong = json.loads(corrupted['tileset.json'])
                    wrong['root']['transform'][12] += 1
                    corrupted['tileset.json'] = json.dumps(wrong).encode()
                    report = json.loads(corrupted['conversion.json'])
                    report['root_transform'] = wrong['root']['transform']
                    corrupted['conversion.json'] = json.dumps(report).encode()
                    try:
                        inspect_members(corrupted, source, {}, expected_placement(anchor, quaternion, offset))
                    except AssertionError:
                        controls.append('wrong-world-transform')
                    else:
                        raise AssertionError('world transform control was insensitive')

        document, payload = fixture()
        document['buffers'] = [{'byteLength': 36, 'uri': uri} for uri in
                               ('geometry%20%CE%B2%20%25.bin', 'other.bin', './folder/../geometry%20%CE%B2%20%25.bin')]
        document['images'] = [{'uri': 'tex%20x.png'}, {'uri': './folder/../tex%20x.png'}]
        source = json.dumps(document, indent=2).encode()
        input_path = root/'external #+%.gltf'
        input_path.write_bytes(source)
        (root/'geometry β %.bin').write_bytes(payload)
        os.link(root/'geometry β %.bin', root/'other.bin')
        image = png()
        (root/'tex x.png').write_bytes(image)
        dependencies = {'geometry β %.bin': payload, 'other.bin': payload, 'tex x.png': image}
        output = root/'external.3tz'
        result, frontend = call(binary, ['glb-to-3tz', '-i', input_path, '-o', output])
        require(result.returncode == 0, result.stderr.decode())
        with zipfile.ZipFile(output) as archive:
            members = {name: archive.read(name) for name in archive.namelist()}
        inspected = inspect_members(members, source, dependencies, identity())
        require(inspected['report']['external_files'] == 2, 'unique hard-link/image capture count')
        require(inspected['report']['external_bytes'] == len(payload)+len(image), 'unique capture bytes')
        require(frontend['modelReport'] == inspected['report'], 'external CLI/archive report equality')
        records.append({'case': 'percent-dot-hardlink-image-aliases', **inspected})
        corrupted = dict(members)
        del corrupted['model/other.bin']
        try:
            inspect_members(corrupted, source, dependencies, identity())
        except (AssertionError, KeyError):
            controls.append('omitted-hardlink-alias')
        else:
            raise AssertionError('alias closure control was insensitive')

        result, frontend = call(binary, ['createTilesetJson', '-i', input_path])
        require(result.returncode == 0, result.stderr.decode())
        standalone = json.loads((root/'tileset.json').read_bytes())
        inspect_manifest_errors(standalone, frontend['modelReport'])
        require(uri_member('', standalone['root']['content']['uri']) == input_path.name, 'sibling reference base and escaping')
        require(frontend['modelReport']['product'] == 'manifest' and frontend['modelReport']['model_payload_files'] == 0,
                'reference artifact reports no copied payload')
        require(input_path.read_bytes() == source and (root/'geometry β %.bin').read_bytes() == payload,
                'standalone leaves source/resource bytes intact')

        for uri in ('source.gltf', 'data:application/octet-stream;base64,AAAA', '../outside.bin', 'a%2fb.bin'):
            doc, _ = fixture()
            doc['buffers'][0]['uri'] = uri
            input_path = root/'refuse.gltf'
            input_path.write_text(json.dumps(doc))
            if uri == 'source.gltf':
                (root/uri).write_bytes(payload)
            destination = root/'prior.3tz'
            destination.write_bytes(b'KEEP')
            refused(binary, ['glb-to-3tz', '-i', input_path, '-o', destination, '--force'], 'unsupported')
            require(destination.read_bytes() == b'KEEP', 'unsupported operation preserves prior artifact')
            controls.append('refused-'+uri)
        for is_glb in (False, True):
            source_name = 'source.glb' if is_glb else 'source.gltf'
            for suffix in ('/data.bin', '/nested/data.bin'):
                with tempfile.TemporaryDirectory(dir=root, prefix='root-prefix-') as directory:
                    directory = Path(directory)
                    uri = source_name+suffix
                    doc, payload = fixture()
                    doc['buffers'][0]['uri'] = uri
                    input_path = directory/('original.glb' if is_glb else 'original.gltf')
                    input_path.write_bytes(envelope(doc, None) if is_glb else json.dumps(doc).encode())
                    dependency = directory/uri
                    dependency.parent.mkdir(parents=True)
                    dependency.write_bytes(payload)
                    absent = directory/'absent'
                    refused(binary, ['glb-to-3tz', '-i', input_path, '-o', absent/'out.3tz'], 'unsupported')
                    require(not absent.exists(), 'prefix collision refuses before output-parent work')
                    require(not any(path.name.startswith('.') for path in directory.iterdir()),
                            'prefix collision leaves no workspace')
                    controls.append('refused-'+uri)
        for field, value in (('animations', [{}]), ('extensionsUsed', ['EXT_structural_metadata'])):
            doc, payload = fixture()
            doc[field] = value
            input_path = root/'unsupported.glb'
            input_path.write_bytes(envelope(doc, payload))
            refused(binary, ['glb-to-3tz', '-i', input_path, '-o', root/'unused.3tz'], 'unsupported')
            controls.append('excluded-'+field)
        # Syntax/product breaks remain visible, including old HPR and -o routes.
        for arguments in (['glb-to-3tz', '-i', root/'identity.glb', '-o', root/'unused.3tz', '--rotationDegrees', 1, 2, 3],
                          ['createTilesetJson', '-i', root/'identity.glb', '-o', root/'elsewhere.json']):
            result = subprocess.run([str(binary), *map(str, arguments)], capture_output=True)
            require(result.returncode == 2, 'retired syntax is a visible CLI refusal')
        controls.extend(['retired-HPR', 'retired-arbitrary-manifest-output'])

    require(hashlib.sha256(binary.read_bytes()).hexdigest() == binary_hash, 'binary identity stable throughout execution')
    result = {'schema_version': 1, 'profile': 'w1-static-model-v1', 'binary': {'path': str(binary), 'sha256': binary_hash},
              'oracle_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              'decimal_precision': 80, 'cases': records, 'controls_detected': controls,
              'proof_limits': ['standalone source tree must remain stable throughout the entire call, including callbacks and alias retargets, and throughout subsequent use',
                               'no animation/imported metadata/unknown extension admission',
                               'browser appearance/picking and platform/wheel/Blender checks are separate']}
    text = json.dumps(result, indent=2)+'\n'
    if args.json_output:
        args.json_output.parent.mkdir(parents=True, exist_ok=True)
        args.json_output.write_text(text)
    else:
        print(text, end='')


if __name__ == '__main__':
    main()
