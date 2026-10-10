#!/usr/bin/env python3
"""Independent F1b GLB/PNG/UV/material/resource oracle (no production imports).

Geometry framing/matrix/index helpers come from the independently authored F1a
oracle. PNG writing/reading and textured scene decoding are implemented here.
"""
import argparse
import base64
import copy
import hashlib
import io
import json
import math
from pathlib import Path, PurePosixPath
import struct
import subprocess
import tempfile
import zipfile
import zlib

import f1a_oracle as geometry

OracleError = geometry.OracleError
require = geometry.require
PROFILE = 'f1d2-adaptive-root-proxy-gltf-v1'
PIXELS = ((255, 0, 0, 255), (0, 255, 0, 128), (0, 0, 255, 0),
          (255, 255, 0, 64), (255, 0, 255, 255), (0, 255, 255, 192))
# A prebuilt JPEG and its independently decoded Pillow RGB manifest are appended
# below by the fixture author. Runtime JPEG inspection requires no Pillow for
# this known fixture; arbitrary JPEG inspection requires the independent decoder.
JPEG_BASE64 = '/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQH/2wBDAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQH/wAARCAACAAMDAREAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwD7/n/ZR/Zb8UeI/iLqXib9mv4A+ItRtvjD8avDttqGu/Bz4d6ve2/h/wAIfFrxp4T8J6FBdah4cuJ4tH8L+FdE0bwz4d0yORbLRPD+kaZo2mwW2nWFrbRf6F/Qd8SfEXgz6Ev0M8o4P4+414UynGfRL+jXxZjMr4a4pz3IsuxXFPHvgtwRxzxzxLicFlePwuGr8QcacbcRcQcY8WZzVpTzHiLinPc54gzjE4zNs0xuLr/84X7bTxd8V/D39pZ485DwD4n+IfA+RY/hH6M3HGOyXhDjXiThrKcbxp4m/RR8D/EnxJ4vxeXZLmWCweJ4o8QfETizinj7jjP61GebcWcacS8QcU59i8fnmc5jj8T/AP/Z'
JPEG_MANIFEST = {'sha256': 'ec41b92388795ceda2260f6fe9a25d2f79cee10b539e2da678a43b813c5578b4', 'size': [3, 2], 'rgba': [[254, 0, 0, 255], [1, 255, 2, 255], [0, 0, 254, 255], [255, 255, 0, 255], [255, 0, 254, 255], [0, 255, 255, 255]], 'author': 'Pillow 12.3.0 JPEG quality=100, subsampling=0; RGB analytic six-color input'}


def digest(data):
    return hashlib.sha256(data).hexdigest()



def encode_glb(doc, binary):
    """Raw GLB author; retain document semantics, including negative URI controls."""
    doc = copy.deepcopy(doc)
    if not doc.get('buffers'):
        doc['buffers'] = [{'byteLength': len(binary)}]
    else:
        doc['buffers'][0]['byteLength'] = len(binary)
    raw = json.dumps(doc, separators=(',', ':'), allow_nan=False).encode()
    raw += b' ' * (-len(raw) % 4)
    binary += b'\0' * (-len(binary) % 4)
    return struct.pack('<4sII', b'glTF', 2, 28 + len(raw) + len(binary)) + struct.pack('<II', len(raw), 0x4e4f534a) + raw + struct.pack('<II', len(binary), 0x004e4942) + binary

def png_chunk(kind, data):
    return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xffffffff)


def paeth(a, b, c):
    p = a + b - c
    distances = (abs(p - a), abs(p - b), abs(p - c))
    return (a, b, c)[distances.index(min(distances))]


def encode_png(width=3, height=2, pixels=PIXELS, filters=(0, 0), comment=None):
    """Write a literal RGBA8 PNG with independently selected scanline filters."""
    require(width * height == len(pixels), 'PNG fixture pixel count')
    previous = bytes(width * 4)
    raw = bytearray()
    for y in range(height):
        row = bytes(channel for pixel in pixels[y * width:(y + 1) * width] for channel in pixel)
        kind = filters[y % len(filters)]
        raw.append(kind)
        for i, value in enumerate(row):
            left = row[i - 4] if i >= 4 else 0
            above = previous[i]
            upper_left = previous[i - 4] if i >= 4 else 0
            prediction = (0, left, above, (left + above) // 2, paeth(left, above, upper_left))[kind]
            raw.append((value - prediction) & 255)
        previous = row
    data = b'\x89PNG\r\n\x1a\n' + png_chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0))
    if comment is not None:
        data += png_chunk(b'tEXt', b'oracle\0' + comment.encode())
    return data + png_chunk(b'IDAT', zlib.compress(raw)) + png_chunk(b'IEND', b'')


def decode_png(data):
    require(data.startswith(b'\x89PNG\r\n\x1a\n'), 'PNG signature')
    offset = 8
    chunks = []
    while offset < len(data):
        require(offset + 12 <= len(data), 'PNG chunk header')
        size = struct.unpack_from('>I', data, offset)[0]
        kind = data[offset + 4:offset + 8]
        end = offset + 12 + size
        require(end <= len(data), 'PNG chunk range')
        payload = data[offset + 8:offset + 8 + size]
        require(struct.unpack_from('>I', data, end - 4)[0] == zlib.crc32(kind + payload) & 0xffffffff, 'PNG CRC')
        chunks.append((kind, payload))
        offset = end
    require(chunks and chunks[0][0] == b'IHDR' and chunks[-1] == (b'IEND', b''), 'PNG framing')
    require(sum(kind == b'IHDR' for kind, _ in chunks) == 1 and sum(kind == b'IEND' for kind, _ in chunks) == 1, 'PNG singleton chunks')
    require(len(chunks[0][1]) == 13, 'PNG IHDR length')
    width, height, depth, color, compression, filtering, interlace = struct.unpack('>IIBBBBB', chunks[0][1])
    require(width > 0 and height > 0 and depth == 8 and color in (2, 6) and (compression, filtering, interlace) == (0, 0, 0), 'oracle RGB/RGBA8 PNG profile')
    require(width * height <= 16 * 1024 * 1024, 'oracle PNG allocation ceiling')
    require(all(kind in (b'IHDR', b'IDAT', b'IEND') or kind[0] & 32 for kind, _ in chunks), 'unknown critical PNG chunk')
    compressed = b''.join(value for kind, value in chunks if kind == b'IDAT')
    channels = 4 if color == 6 else 3
    row_bytes = width * channels
    expected = (row_bytes + 1) * height
    try:
        stream = zlib.decompressobj()
        raw = stream.decompress(compressed, expected + 1)
    except zlib.error as error:
        raise OracleError('PNG IDAT zlib') from error
    require(len(raw) == expected and stream.eof and not stream.unused_data and not stream.unconsumed_tail, 'PNG decoded scanline size/stream closure')
    previous = bytearray(row_bytes)
    pixels = []
    for y in range(height):
        start = y * (row_bytes + 1)
        kind = raw[start]
        require(kind <= 4, 'PNG scanline filter')
        row = bytearray(raw[start + 1:start + 1 + row_bytes])
        for i in range(row_bytes):
            left = row[i - channels] if i >= channels else 0
            above = previous[i]
            upper_left = previous[i - channels] if i >= channels else 0
            prediction = (0, left, above, (left + above) // 2, paeth(left, above, upper_left))[kind]
            row[i] = (row[i] + prediction) & 255
        for x in range(width):
            pixel = tuple(row[x * channels:(x + 1) * channels])
            pixels.append(pixel if channels == 4 else (*pixel, 255))
        previous = row
    return width, height, tuple(pixels)


def decoded_image(data, mime):
    if mime == 'image/png':
        width, height, pixels = decode_png(data)
    elif mime == 'image/jpeg':
        require(data.startswith(b'\xff\xd8') and data.endswith(b'\xff\xd9'), 'JPEG envelope')
        try:
            from PIL import Image
        except ImportError:
            require(digest(data) == JPEG_MANIFEST['sha256'], 'Pillow required to independently decode arbitrary JPEG')
            width, height = JPEG_MANIFEST['size']
            pixels = tuple(tuple(pixel) for pixel in JPEG_MANIFEST['rgba'])
        else:
            try:
                with Image.open(io.BytesIO(data)) as image:
                    require(image.format == 'JPEG', 'JPEG MIME/content mismatch')
                    image.load()
                    width, height = image.size
                    rgba = image.convert('RGBA')
                    pixels = tuple(rgba.get_flattened_data() if hasattr(rgba, 'get_flattened_data') else rgba.getdata())
            except (OSError, ValueError) as error:
                raise OracleError('independent JPEG decode') from error
    else:
        raise OracleError('unsupported image MIME in oracle')
    return {'sha256': digest(data), 'mime': mime, 'width': width, 'height': height,
            'pixels': [list(pixel) for pixel in pixels]}


def accessor(doc, binary, index):
    require(type(index) is int and 0 <= index < len(doc.get('accessors', [])), 'accessor reference')
    item = doc['accessors'][index]
    require('sparse' not in item, 'oracle sparse exclusion')
    codes = {5121: ('B', 1), 5123: ('H', 2), 5125: ('I', 4), 5126: ('f', 4)}
    widths = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4}
    require(item['componentType'] in codes and item['type'] in widths, 'accessor encoding')
    code, size = codes[item['componentType']]
    width = widths[item['type']]
    view = doc['bufferViews'][item['bufferView']]
    start = item.get('byteOffset', 0)
    stride = view.get('byteStride', size * width)
    count = item['count']
    require(type(count) is int and count > 0 and start >= 0 and start % size == 0 and stride >= size * width and stride % size == 0, 'accessor count/alignment/stride')
    require(start + (count - 1) * stride + size * width <= view['byteLength'], 'accessor actual range')
    values = [struct.unpack_from('<' + code * width, binary, view.get('byteOffset', 0) + start + i * stride) for i in range(count)]
    require(all(geometry.finite(value) for value in values), 'nonfinite accessor')
    if item.get('normalized', False):
        require(item['type'] in ('VEC2', 'VEC3', 'VEC4') and item['componentType'] in (5121, 5123), 'normalized unsigned UV/color encoding')
        scale = 255 if item['componentType'] == 5121 else 65535
        values = [tuple(channel / scale for channel in value) for value in values]
    return [value[0] for value in values] if width == 1 else values


def texture_infos(material):
    """Independent core slot enumeration; never infer closure from base color."""
    for key in ('baseColorTexture', 'metallicRoughnessTexture'):
        if key in material.get('pbrMetallicRoughness', {}):
            yield material['pbrMetallicRoughness'][key]
    for key in ('normalTexture', 'occlusionTexture', 'emissiveTexture'):
        if key in material:
            yield material[key]


def resources(doc, binary, resolver=None):
    decoded = []
    raw_images = []
    for image in doc.get('images', []):
        mime = image.get('mimeType')
        if 'bufferView' in image:
            require('uri' not in image, 'image has URI and bufferView')
            view = doc['bufferViews'][image['bufferView']]
            require('byteStride' not in view and 'target' not in view, 'image view vertex layout')
            start = view.get('byteOffset', 0)
            data = binary[start:start + view['byteLength']]
        else:
            require(resolver is not None and isinstance(image.get('uri'), str), 'output image dependency resolver')
            data, resolved_mime = resolver(image['uri'])
            require(mime is None or mime == resolved_mime, 'external image MIME mismatch')
            mime = mime or resolved_mime
        decoded.append(decoded_image(data, mime))
        raw_images.append(data)
    textures = []
    for texture in doc.get('textures', []):
        index = texture['source']
        require(type(index) is int and 0 <= index < len(decoded), 'texture image reference')
        fields = {key: copy.deepcopy(value) for key, value in texture.items() if key not in ('name', 'source', 'sampler')}
        fields['image'] = decoded[index]
        fields['sampler_present'] = 'sampler' in texture
        if 'sampler' in texture:
            sampler_index = texture['sampler']
            require(type(sampler_index) is int and 0 <= sampler_index < len(doc.get('samplers', [])), 'sampler reference')
            fields['sampler'] = {key: copy.deepcopy(value) for key, value in doc['samplers'][sampler_index].items() if key != 'name'}
        textures.append(fields)
    materials = []
    for material in doc.get('materials', []):
        fields = {key: copy.deepcopy(value) for key, value in material.items() if key != 'name'}
        for info in texture_infos(fields):
            texture_index = info.pop('index')
            require(type(texture_index) is int and 0 <= texture_index < len(textures), 'material texture reference')
            info['texture'] = textures[texture_index]
        materials.append({'present': True, 'fields': fields})
    return materials, decoded, raw_images


def used_resource_ids(doc, primitives):
    materials = {p['material'] for p in primitives if 'material' in p}
    textures = {info['index'] for i in materials for info in texture_infos(doc['materials'][i])}
    images = {doc['textures'][i]['source'] for i in textures}
    samplers = {doc['textures'][i]['sampler'] for i in textures if 'sampler' in doc['textures'][i]}
    return materials, textures, images, samplers


def scene_triangles(data, resolver=None, output=False):
    doc, binary = geometry.decode_glb(data)
    materials, images, raw_images = resources(doc, binary, resolver)
    scenes = doc.get('scenes', [])
    selected = doc.get('scene', 0)
    require(scenes and type(selected) is int and 0 <= selected < len(scenes), 'selected scene')
    require('scene' in doc or len(scenes) == 1, 'ambiguous scene')
    triangles = []
    active = set()
    selected_primitives = []

    def walk(index, parent):
        require(index not in active, 'scene cycle')
        active.add(index)
        node = doc['nodes'][index]
        transform = geometry.product(parent, geometry.node_matrix(node))
        det = geometry.determinant(transform)
        require(det != 0, 'singular transform')
        if 'mesh' in node:
            for primitive in doc['meshes'][node['mesh']]['primitives']:
                selected_primitives.append(primitive)
                require(primitive.get('mode', 4) == 4, 'triangle primitive')
                attributes = geometry.geometry_attributes(doc, binary, primitive['attributes'])
                require(set(attributes) <= {'POSITION', 'NORMAL', 'TEXCOORD_0'}, 'unexpected primitive attribute')
                p = accessor(doc, binary, attributes['POSITION'])
                n = accessor(doc, binary, attributes['NORMAL']) if 'NORMAL' in attributes else None
                uv = accessor(doc, binary, attributes['TEXCOORD_0']) if 'TEXCOORD_0' in attributes else None
                require(n is None or len(n) == len(p), 'normal count')
                require(uv is None or len(uv) == len(p), 'UV count')
                for semantic, a in attributes.items():
                    item = doc['accessors'][a]
                    require(item['type'] == ('VEC2' if semantic == 'TEXCOORD_0' else 'VEC3'), 'attribute shape')
                    if semantic != 'TEXCOORD_0' or output:
                        require(item['componentType'] == 5126 and not item.get('normalized', False), 'output/source f32 encoding')
                indices = accessor(doc, binary, primitive['indices']) if 'indices' in primitive else list(range(len(p)))
                require(len(indices) % 3 == 0 and all(type(i) is int and 0 <= i < len(p) for i in indices), 'triangle indices')
                material = materials[primitive['material']] if 'material' in primitive else {'present': False, 'fields': {}}
                textured = 'baseColorTexture' in material['fields'].get('pbrMetallicRoughness', {})
                require(not textured or uv is not None, 'textured primitive missing UV')
                for start in range(0, len(indices), 3):
                    corners = list(indices[start:start + 3])
                    if det < 0:
                        corners[1], corners[2] = corners[2], corners[1]
                    positions = tuple(geometry.z_up(geometry.point(transform, p[i])) for i in corners)
                    normals = None if n is None else tuple(geometry.z_up(geometry.normal(transform, n[i])) for i in corners)
                    uvs = None if uv is None else tuple(tuple(uv[i]) for i in corners)
                    triangles.append((positions, normals, uvs, material))
        for child in node.get('children', []):
            walk(child, transform)
        active.remove(index)

    for root in scenes[selected]['nodes']:
        walk(root, geometry.identity())
    used = used_resource_ids(doc, selected_primitives)
    if output:
        for ids, key in zip(used, ('materials', 'textures', 'images', 'samplers')):
            require(ids == set(range(len(doc.get(key, [])))), 'exact leaf ' + key + ' closure')
    return triangles, used[2], images, raw_images


def match_triangles(expected, actual):
    require(len(expected) == len(actual), 'triangle multiplicity/coverage')
    pending = list(actual)
    for positions, normals, uvs, material in expected:
        found = None
        for index, (p, n, uv, m) in enumerate(pending):
            if material != m or (normals is None) != (n is None) or (uvs is None) != (uv is None):
                continue
            for rotation in range(3):
                q = p[rotation:] + p[:rotation]
                k = None if n is None else n[rotation:] + n[:rotation]
                t = None if uv is None else uv[rotation:] + uv[:rotation]
                position_match = all(abs(x - y) <= max(1e-6, abs(x) * 2 ** -24) + 1e-12 for a, b in zip(positions, q) for x, y in zip(a, b))
                normal_match = normals is None or all(abs(x - y) <= 2e-7 + 1e-12 for a, b in zip(normals, k) for x, y in zip(a, b))
                uv_match = uvs is None or all(abs(x - y) <= max(1, abs(x)) * 2 ** -24 + 1e-12 for a, b in zip(uvs, t) for x, y in zip(a, b))
                if position_match and normal_match and uv_match:
                    found = index
                    break
            if found is not None:
                break
        require(found is not None, 'oriented corner position/normal/UV/material/image/sampler fidelity')
        pending.pop(found)


def fixture(triangles=8, variant='standard', transformed=True):
    require(triangles > 0, 'positive fixture triangle count')
    if variant == 'reordered-spike':
        doc, binary = geometry.decode_glb(fixture(triangles, 'spike', transformed))
        doc['meshes'][0]['primitives'].reverse()
        return encode_glb(doc, binary)
    doc = {'asset': {'version': '2.0'}, 'scene': 0, 'scenes': [{'nodes': [0]}], 'nodes': [{'mesh': 0}],
           'meshes': [{'primitives': []}], 'buffers': [], 'bufferViews': [], 'accessors': [],
           'images': [], 'textures': [{'source': 0, 'sampler': 0}, {'source': 0}],
           'samplers': [{'magFilter': 9728, 'minFilter': 9984, 'wrapS': 33071, 'wrapT': 33648}],
           'materials': [
               {'pbrMetallicRoughness': {'baseColorTexture': {'index': 0, 'texCoord': 0}, 'baseColorFactor': [1, 0.5, 0.25, 0.5], 'metallicFactor': 0, 'roughnessFactor': 1}, 'alphaMode': 'MASK', 'alphaCutoff': 0.25, 'doubleSided': True},
               {'pbrMetallicRoughness': {'baseColorTexture': {'index': 1}}, 'alphaMode': 'OPAQUE'}]}
    binary = bytearray()

    def view(raw):
        binary.extend(b'\0' * (-len(binary) % 4))
        index = len(doc['bufferViews'])
        doc['bufferViews'].append({'buffer': 0, 'byteOffset': len(binary), 'byteLength': len(raw)})
        binary.extend(raw)
        return index

    def add(values, code, kind, component, normalized=False):
        width = {'VEC3': 3, 'VEC2': 2, 'SCALAR': 1}[kind]
        flattened = [c for value in values for c in value] if width > 1 else values
        raw = struct.pack('<' + code * len(flattened), *flattened)
        if normalized and component == 5121:
            raw = b''.join(raw[i:i + 2] + b'\0\0' for i in range(0, len(raw), 2))
        item = {'bufferView': view(raw), 'componentType': component, 'count': len(values), 'type': kind}
        if normalized:
            item['normalized'] = True
            if component == 5121:
                doc['bufferViews'][item['bufferView']]['byteStride'] = 4
        if kind == 'VEC3':
            item.update(min=[min(p[i] for p in values) for i in range(3)], max=[max(p[i] for p in values) for i in range(3)])
        index = len(doc['accessors'])
        doc['accessors'].append(item)
        return index

    for i in range(triangles):
        x = 0 if variant in ('coincident', 'degenerate') else 4 * i
        positions = [(x, 0, 0), (x + 1, 0, 0), (x, 1, 1)]
        if variant == 'degenerate':
            positions = [(0, 0, 0), (0, 0, 0), (1, 0, 0)]
        if variant in ('spike', 'reordered-spike') and i == (0 if variant == 'spike' else triangles - 1):
            positions = [(x, 0, 0), (x + 0.0001, 0, 0), (x, 100, 0)]
        if variant == 'thin':
            positions = [(x, 0, 0), (x + 0.0001, 0, 0), (x, 100, 0)]
        if variant == 'disconnected' and i == triangles - 1:
            positions = [(1000, 1000, 1000), (1000.01, 1000, 1000), (1000, 1000.01, 1000)]
        attributes = {'POSITION': add(positions, 'f', 'VEC3', 5126)}
        if variant != 'normal-absent':
            attributes['NORMAL'] = add([(0, -math.sqrt(0.5), math.sqrt(0.5))] * 3, 'f', 'VEC3', 5126)
        uv_values = [(0.125, 0.25), (0.875, 0.375), (0.25, 0.875)]
        if i % 2:
            uv_values = list(reversed(uv_values))
        if variant == 'wrap':
            uv_values = [(-0.25, 1.25), (1.5, -0.75), (0.5, 0.5)]
        if variant in ('uv-u8', 'uv-u16'):
            maximum = 255 if variant == 'uv-u8' else 65535
            values = [(1, maximum - 1), (maximum // 3, maximum // 2), (maximum, 0)]
            attributes['TEXCOORD_0'] = add(values, 'B' if maximum == 255 else 'H', 'VEC2', 5121 if maximum == 255 else 5123, True)
        else:
            attributes['TEXCOORD_0'] = add(uv_values, 'f', 'VEC2', 5126)
        primitive = {'attributes': attributes, 'material': i % 2}
        if variant != 'nonindexed':
            primitive['indices'] = add([0, 1, 2], 'B', 'SCALAR', 5121)
        if variant in ('mixed', 'uv-on-untextured') and i == triangles - 1:
            doc['materials'].append({'pbrMetallicRoughness': {'baseColorFactor': [0.3, 0.2, 0.1, 1]}})
            primitive['material'] = 2
            if variant == 'mixed':
                del attributes['TEXCOORD_0']
        doc['meshes'][0]['primitives'].append(primitive)
    if transformed:
        doc['nodes'][0]['matrix'] = [0, -2, 0, 0, -3, 0, 0, 0, 0, 0, 0.5, 0, 3, -5, 7, 1]
    if variant == 'nested':
        doc['nodes'].append({'translation': [5, 6, 7], 'children': [0]})
        doc['scenes'][0]['nodes'] = [1]
    if variant == 'instanced':
        doc['nodes'].append({'mesh': 0, 'translation': [100, 20, 30]})
        doc['scenes'][0]['nodes'].append(1)
    if variant in ('scenes', 'selected-image-one'):
        doc['nodes'].append({'mesh': 0, 'translation': [100, 20, 30]})
        doc['scenes'].append({'nodes': [1]})
        doc['scene'] = 1
        if variant == 'selected-image-one':
            doc['textures'].append({'source': 1, 'sampler': 0})
            doc['materials'].append({'pbrMetallicRoughness': {'baseColorTexture': {'index': 2}}, 'alphaMode': 'OPAQUE'})
            selected_mesh = copy.deepcopy(doc['meshes'][0])
            for primitive in selected_mesh['primitives']:
                primitive['material'] = 2
            doc['meshes'].append(selected_mesh)
            doc['nodes'][1]['mesh'] = 1
    if variant == 'two-images':
        doc['textures'][1]['source'] = 1
    if variant.startswith('sampler-'):
        filter_ = int(variant.split('-')[1])
        doc['samplers'][0] = {'magFilter': 9729 if filter_ % 2 else 9728, 'minFilter': filter_, 'wrapS': (33071, 33648, 10497)[filter_ % 3], 'wrapT': (10497, 33071, 33648)[filter_ % 3]}
    if variant == 'empty-sampler':
        doc['samplers'].append({})
        doc['textures'][1]['sampler'] = 1
    if variant == 'jpeg':
        data = base64.b64decode(JPEG_BASE64)
        mime = 'image/jpeg'
    else:
        data, mime = encode_png(), 'image/png'
    doc['images'].append({'bufferView': view(data), 'mimeType': mime})
    # Admission validates this deliberately unused image; selected output omits it.
    doc['images'].append({'bufferView': view(encode_png(comment='unused-source-image')), 'mimeType': 'image/png'})
    if variant == 'interleaved':
        for primitive in doc['meshes'][0]['primitives']:
            attributes = primitive['attributes']
            positions = accessor(doc, binary, attributes['POSITION'])
            normals = accessor(doc, binary, attributes['NORMAL'])
            uvs = accessor(doc, binary, attributes['TEXCOORD_0'])
            packed = b''.join(struct.pack('<8f', *p, *n, *uv) for p, n, uv in zip(positions, normals, uvs))
            shared = view(packed)
            doc['bufferViews'][shared]['byteStride'] = 32
            for semantic, offset in (('POSITION', 0), ('NORMAL', 12), ('TEXCOORD_0', 24)):
                doc['accessors'][attributes[semantic]].update(bufferView=shared, byteOffset=offset)
    doc['buffers'] = [{'byteLength': len(binary)}]
    return encode_glb(doc, bytes(binary))



def viewer_fixture():
    """Two analytically located face-on panels: OPAQUE and MASK, nearest/clamp."""
    doc = {'asset': {'version': '2.0'}, 'scene': 0, 'scenes': [{'nodes': [0]}], 'nodes': [{'mesh': 0}],
           'meshes': [{'primitives': []}], 'bufferViews': [], 'accessors': [],
           'textures': [{'source': 0, 'sampler': 0}],
           'samplers': [{'magFilter': 9728, 'minFilter': 9728, 'wrapS': 33071, 'wrapT': 33071}],
           'materials': [{'pbrMetallicRoughness': {'baseColorTexture': {'index': 0}, 'baseColorFactor': [1, 1, 1, 1], 'metallicFactor': 0, 'roughnessFactor': 1}, 'alphaMode': mode, 'alphaCutoff': 0.5, 'doubleSided': True} for mode in ('OPAQUE', 'MASK')]}
    binary = bytearray()
    def add(values, width):
        binary.extend(b'\0' * (-len(binary) % 4))
        view = len(doc['bufferViews'])
        raw = struct.pack('<' + 'f' * (len(values) * width), *(channel for value in values for channel in value))
        doc['bufferViews'].append({'buffer': 0, 'byteOffset': len(binary), 'byteLength': len(raw)})
        binary.extend(raw)
        index = len(doc['accessors'])
        item = {'bufferView': view, 'componentType': 5126, 'count': len(values), 'type': 'VEC' + str(width)}
        if width == 3:
            item.update(min=[min(p[i] for p in values) for i in range(3)], max=[max(p[i] for p in values) for i in range(3)])
        doc['accessors'].append(item)
        return index
    for material, left in enumerate((-2.2, 0.2)):
        positions = [(left, -1, 0), (left + 2, -1, 0), (left + 2, 1, 0), (left, -1, 0), (left + 2, 1, 0), (left, 1, 0)]
        uvs = [(0, 1), (1, 1), (1, 0), (0, 1), (1, 0), (0, 0)]
        doc['meshes'][0]['primitives'].append({'attributes': {'POSITION': add(positions, 3), 'NORMAL': add([(0, 0, 1)] * 6, 3), 'TEXCOORD_0': add(uvs, 2)}, 'material': material})
    binary.extend(b'\0' * (-len(binary) % 4))
    raw = encode_png()
    view = len(doc['bufferViews'])
    doc['bufferViews'].append({'buffer': 0, 'byteOffset': len(binary), 'byteLength': len(raw)})
    binary.extend(raw)
    doc['images'] = [{'bufferView': view, 'mimeType': 'image/png'}]
    doc['buffers'] = [{'byteLength': len(binary)}]
    return encode_glb(doc, bytes(binary))


def shared_uv_fixture(primitives=4096, vertices=300000):
    """Repeated indexed primitives share one large valid POSITION/UV accessor.

    Admission work should validate each accessor once. This is a resource probe,
    not a timing oracle or full quadratic triangle-matching benchmark.
    """
    require(3 <= vertices and 0 < primitives <= 4096, 'shared accessor fixture domain')
    doc, binary = geometry.decode_glb(fixture(1, transformed=False))
    binary = bytearray(binary)
    position = len(doc['accessors'])
    binary.extend(b'\0' * (-len(binary) % 4))
    offset = len(binary)
    raw = struct.pack('<3f', 0, 0, 0) * vertices
    doc['bufferViews'].append({'buffer': 0, 'byteOffset': offset, 'byteLength': len(raw)})
    doc['accessors'].append({'bufferView': len(doc['bufferViews']) - 1, 'componentType': 5126, 'count': vertices, 'type': 'VEC3', 'min': [0, 0, 0], 'max': [0, 0, 0]})
    binary.extend(raw)
    uv = len(doc['accessors'])
    raw = struct.pack('<2f', 0.25, 0.75) * vertices
    doc['bufferViews'].append({'buffer': 0, 'byteOffset': len(binary), 'byteLength': len(raw)})
    doc['accessors'].append({'bufferView': len(doc['bufferViews']) - 1, 'componentType': 5126, 'count': vertices, 'type': 'VEC2'})
    binary.extend(raw)
    prototype = doc['meshes'][0]['primitives'][0]
    prototype['attributes'] = {'POSITION': position, 'TEXCOORD_0': uv}
    doc['meshes'][0]['primitives'] = [copy.deepcopy(prototype) for _ in range(primitives)]
    doc['buffers'][0]['byteLength'] = len(binary)
    return encode_glb(doc, bytes(binary))

def inspect(source, archive, leaf_limit, *, source_model=None, scene_reader=None,
            triangle_matcher=None, triangle_positions=None, placement_expectation=None):
    reader = scene_triangles if scene_reader is None else scene_reader
    matcher = match_triangles if triangle_matcher is None else triangle_matcher
    positions_of = (lambda triangle: triangle[0]) if triangle_positions is None else triangle_positions
    source_data = Path(source).read_bytes()
    if source_model is None:
        expected, source_used, source_images, source_raw = reader(source_data)
        external_files, external_bytes = 0, 0
    else:
        expected = source_model['triangles']
        source_used = source_model['used_images']
        source_images = source_model['images']
        source_raw = source_model['raw_images']
        external_files = source_model['external_files']
        external_bytes = source_model['external_bytes']
    expected_members = {'textures/' + str(i) + ('.png' if source_images[i]['mime'] == 'image/png' else '.jpg') for i in source_used}
    with zipfile.ZipFile(archive) as z:
        require(z.testzip() is None, 'ZIP CRC')
        names = z.namelist()
        require(len(names) == len(set(names)), 'duplicate ZIP member')
        require(all(not n.startswith('/') and all(p not in ('', '.', '..') for p in n.split('/')) for n in names), 'unsafe archive member')
        tileset = json.loads(z.read('tileset.json'))
        root = tileset['root']
        children = root.get('children', [])
        require(tileset['asset']['version'] == '1.1' and root['refine'] == 'REPLACE' and children, 'tileset full detail hierarchy')
        require('content' not in root and 'contents' not in root and 'implicitTiling' not in root, 'flat empty root')
        box = root['boundingVolume']['box']
        error = max(1.0, 2 * math.sqrt(sum(box[i] ** 2 for i in (3, 7, 11))))
        require(abs(root['geometricError'] - error) <= 1e-10 * max(1, error), 'root routing metric')
        require(abs(tileset['geometricError'] - error) <= 1e-10 * max(1, error), 'tileset routing metric')
        actual = []
        leaf_uris = []
        used_names = set()
        for child in children:
            require(not child.get('children') and 'implicitTiling' not in child and 'transform' not in child and child['geometricError'] == 0, 'full detail explicit leaf')
            uri = child['content']['uri']
            require(uri not in leaf_uris and uri.startswith('t/') and uri.endswith('.glb'), 'leaf URI identity')
            leaf_uris.append(uri)

            def resolver(image_uri):
                require(image_uri.startswith('../textures/'), 'leaf relative shared image URI')
                path = PurePosixPath(uri).parent / image_uri
                parts = []
                for part in path.parts:
                    if part == '..':
                        require(parts, 'escaping image URI')
                        parts.pop()
                    elif part != '.':
                        parts.append(part)
                resolved = '/'.join(parts)
                require(resolved in expected_members and image_uri == '../' + resolved, 'image URI exact source identity')
                used_names.add(resolved)
                image_id = int(PurePosixPath(resolved).stem)
                bytes_ = z.read(resolved)
                require(bytes_ == source_raw[image_id], 'exact encoded source image forwarding')
                return bytes_, source_images[image_id]['mime']

            triangles, _, _, _ = reader(z.read(uri), resolver, output=True)
            require(0 < len(triangles) <= leaf_limit, 'leaf triangle ceiling')
            child_box = child['boundingVolume']['box']
            for bits in range(8):
                corner = tuple(child_box[i] + (-1 if bits & (1 << i) else 1) * child_box[3 + 4 * i] for i in range(3))
                require(geometry.box_contains(box, corner, 0), 'root encloses descendant box')
            for triangle in triangles:
                for point in positions_of(triangle):
                    require(geometry.box_contains(box, point, 0) and geometry.box_contains(child_box, point, 0), 'stored vertex bounds')
            actual.extend(triangles)
        require(used_names == expected_members, 'exact selected source image closure')
        require(set(names) == {'tileset.json', 'conversion.json', '@3dtilesIndex1@', *leaf_uris, *expected_members}, 'exact archive dependency closure')
        expected_report = {'schema_version': 7, 'profile': PROFILE, 'approximation': {'kind': 'full_detail'}, 'source_coordinates': 'local-gltf', **geometry.local_placement_expectation(), 'source_bytes': len(source_data), 'external_files': external_files, 'external_bytes': external_bytes,
                           'triangles': len(expected), 'leaf_tiles': len(leaf_uris), 'leaf_triangles': leaf_limit,
                           'routing_geometric_error_metres': error, 'images': len(source_used),
                           'image_bytes': sum(len(source_raw[i]) for i in source_used),
                           'image_pixels': sum(source_images[i]['width'] * source_images[i]['height'] for i in source_used)}
        report = json.loads(z.read('conversion.json'))
        for field in ('schema_version', 'source_bytes', 'triangles', 'leaf_tiles', 'leaf_triangles', 'images', 'image_bytes', 'image_pixels', 'external_files', 'external_bytes'):
            require(type(report.get(field)) is int and report[field] >= 0, 'typed nonnegative integer report field ' + field)
        resolved = geometry.check_placement(root, report, placement_expectation)
        expected_report.update(resolved)
        require(geometry.close_value(report, expected_report, 1e-10 * max(1, error)), 'typed report facts/fields')
        geometry.check_index(z, Path(archive).read_bytes())
        matcher(expected, actual)
        return {'source_triangles': len(expected), 'leaves': len(leaf_uris), 'published_images': sorted(expected_members),
                'members': sorted(names), 'report': report, 'source_sha256': digest(source_data),
                'archive_sha256': digest(Path(archive).read_bytes())}


def mutated_source(data, mutate):
    doc, binary = geometry.decode_glb(data)
    binary = bytearray(binary)
    mutate(doc, binary)
    doc['buffers'][0]['byteLength'] = len(binary)
    return encode_glb(doc, bytes(binary))



def solid_png(width, height):
    """Large dimension fixture without a per-channel Python authoring loop."""
    row = b'\0' + bytes((220, 80, 40, 255)) * width
    raw = row * height
    return b'\x89PNG\r\n\x1a\n' + png_chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0)) + png_chunk(b'IDAT', zlib.compress(raw)) + png_chunk(b'IEND', b'')


def replace_source_image(data, payload, image=0, copies=None):
    def mutate(doc, binary):
        binary.extend(b'\0' * (-len(binary) % 4))
        view = doc['bufferViews'][doc['images'][image]['bufferView']]
        view.update(byteOffset=len(binary), byteLength=len(payload))
        binary.extend(payload)
        if copies is not None:
            doc['images'] = [copy.deepcopy(doc['images'][image]) for _ in range(copies)]
    return mutated_source(data, mutate)


def resource_fixtures():
    """Admitted boundary probes. Callers measure resources; no timing pass claim."""
    return {
        'shared-uv-4096-primitives': shared_uv_fixture(),
        'png-4Mi-pixels': replace_source_image(fixture(8), solid_png(2048, 2048)),
        'png-edge4096': replace_source_image(fixture(8), solid_png(4096, 1)),
        'png-32-images': replace_source_image(fixture(8), encode_png(), copies=32),
        'png-16Mi-pixels-total': replace_source_image(fixture(8), solid_png(2048, 2048), copies=4),
    }

def rejection_fixtures():
    data = fixture(8)
    result = []
    unsupported = (
        ('image-uri-encoded-separator', lambda d, b: d['images'].__setitem__(0, {'uri': 'textures/a%2fb.png'})),
        ('image-uri-parent', lambda d, b: d['images'].__setitem__(0, {'uri': '../outside.png'})),
        ('image-uri-network', lambda d, b: d['images'].__setitem__(0, {'uri': 'https://example.invalid/source.png'})),
        ('image-uri-absolute', lambda d, b: d['images'].__setitem__(0, {'uri': '/tmp/outside.png'})),
        ('image-uri-data', lambda d, b: d['images'].__setitem__(0, {'uri': 'data:image/png;base64,AAAA'})),
        ('buffer-uri-scheme', lambda d, b: d['buffers'].append({'byteLength': 1, 'uri': 'https://example.invalid/data.bin'})),
        ('blend', lambda d, b: d['materials'][0].__setitem__('alphaMode', 'BLEND')),
        # F1b3 supersedes the old blanket exclusions with explicit remaining
        # profile boundaries. New admitted positives live in f1b3_oracle.py.
        ('normal-texture-without-tangent', lambda d, b: d['materials'][0].__setitem__('normalTexture', {'index': 0})),
        ('metallic-roughness-extension', lambda d, b: d['materials'][0]['pbrMetallicRoughness'].__setitem__('extensions', {'KHR_materials_specular': {}})),
        ('occlusion-texture-transform', lambda d, b: d['materials'][0].__setitem__('occlusionTexture', {'index': 0, 'extensions': {'KHR_texture_transform': {}}})),
        ('emissive-texture-transform', lambda d, b: d['materials'][0].__setitem__('emissiveTexture', {'index': 0, 'extensions': {'KHR_texture_transform': {}}})),
        ('texcoord-two', lambda d, b: d['materials'][0]['pbrMetallicRoughness']['baseColorTexture'].__setitem__('texCoord', 2)),
        ('color-one', lambda d, b: d['meshes'][0]['primitives'][0]['attributes'].__setitem__('COLOR_1', 0)),
        ('uv2', lambda d, b: d['meshes'][0]['primitives'][0]['attributes'].__setitem__('TEXCOORD_2', 0)),
        ('extensions-required', lambda d, b: d.__setitem__('extensionsRequired', ['KHR_texture_transform'])),
        ('extensions-used', lambda d, b: d.__setitem__('extensionsUsed', ['KHR_texture_transform'])),
        ('unused-image-extras', lambda d, b: d['images'][1].__setitem__('extras', {'ignored': True})),
        ('unused-image-uri', lambda d, b: d['images'].__setitem__(1, {'uri': 'https://example.invalid/unused.png'})),
        ('material-extras', lambda d, b: d['materials'][0].__setitem__('extras', {})),
        ('image-count', lambda d, b: d.__setitem__('images', [copy.deepcopy(d['images'][0]) for _ in range(33)])),
    )
    for name, mutate in unsupported:
        result.append((name, mutated_source(data, mutate), 'unsupported'))
    malformed = (
        ('textured-no-uv', lambda d, b: d['meshes'][0]['primitives'][0]['attributes'].pop('TEXCOORD_0')),
        ('uv-count', lambda d, b: d['accessors'][d['meshes'][0]['primitives'][0]['attributes']['TEXCOORD_0']].__setitem__('count', 2)),
        ('texture-source', lambda d, b: d['textures'][0].__setitem__('source', 99)),
        ('texture-sampler', lambda d, b: d['textures'][0].__setitem__('sampler', 99)),
        ('material-texture', lambda d, b: d['materials'][0]['pbrMetallicRoughness']['baseColorTexture'].__setitem__('index', 99)),
        ('invalid-mag-filter', lambda d, b: d['samplers'][0].__setitem__('magFilter', 123)),
        ('mime-mismatch', lambda d, b: d['images'][0].__setitem__('mimeType', 'image/jpeg')),
        ('image-view-target', lambda d, b: d['bufferViews'][d['images'][0]['bufferView']].__setitem__('target', 34962)),
        ('image-view-stride', lambda d, b: d['bufferViews'][d['images'][0]['bufferView']].__setitem__('byteStride', 4)),
        ('image-view-range', lambda d, b: d['bufferViews'][d['images'][0]['bufferView']].__setitem__('byteLength', len(b) + 100)),
    )
    for name, mutate in malformed:
        result.append((name, mutated_source(data, mutate), 'invalid_input'))

    def image_bytes(doc, binary, new_bytes, image=0):
        binary.extend(b'\0' * (-len(binary) % 4))
        view = doc['bufferViews'][doc['images'][image]['bufferView']]
        view.update(byteOffset=len(binary), byteLength=len(new_bytes))
        binary.extend(new_bytes)

    corrupt = bytearray(encode_png())
    corrupt[29] ^= 1
    jpeg_data = fixture(8, 'jpeg')
    _, _, _, jpeg_raw = scene_triangles(jpeg_data)
    result.append(('jpeg-missing-eoi', replace_source_image(jpeg_data, jpeg_raw[0][:-2]), 'invalid_input'))
    result.append(('png-crc', mutated_source(data, lambda d, b: image_bytes(d, b, corrupt)), 'invalid_input'))
    invalid_zlib = encode_png()[:33] + png_chunk(b'IDAT', b'invalid-zlib') + png_chunk(b'IEND', b'')
    result.append(('png-zlib', mutated_source(data, lambda d, b: image_bytes(d, b, invalid_zlib)), 'invalid_input'))
    result.append(('unused-invalid-png', mutated_source(data, lambda d, b: image_bytes(d, b, invalid_zlib, 1)), 'invalid_input'))
    for name, payload, kind in (
        ('png-missing-iend', encode_png()[:-12], 'invalid_input'),
        ('png-trailing-bytes', encode_png() + b'ignored', 'invalid_input'),
        ('png-iend-crc', encode_png()[:-1] + bytes([encode_png()[-1] ^ 1]), 'invalid_input'),
        ('png-apng', encode_png()[:33] + png_chunk(b'acTL', struct.pack('>II', 1, 0)) + encode_png()[33:], 'unsupported'),
    ):
        result.append((name, mutated_source(data, lambda d, b, payload=payload: image_bytes(d, b, payload)), kind))
    for color, channels in ((2, 3), (6, 4)):
        pixels16 = b''.join(struct.pack('>H', i * 8191) for i in range(channels))
        png16 = b'\x89PNG\r\n\x1a\n' + png_chunk(b'IHDR', struct.pack('>IIBBBBB', 1, 1, 16, color, 0, 0, 0)) + png_chunk(b'IDAT', zlib.compress(b'\0' + pixels16)) + png_chunk(b'IEND', b'')
        for image in (0, 1):
            result.append(('png16-' + str(color) + '-image' + str(image), mutated_source(data, lambda d, b, payload=png16, image=image: image_bytes(d, b, payload, image)), 'unsupported'))
    for name, width, height in (('image-edge', 4097, 1), ('image-pixel-ceiling', 4096, 1025)):
        oversized = b'\x89PNG\r\n\x1a\n' + png_chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0)) + png_chunk(b'IDAT', zlib.compress(b'\0')) + png_chunk(b'IEND', b'')
        result.append((name, mutated_source(data, lambda d, b, payload=oversized: image_bytes(d, b, payload)), 'unsupported'))
    result.append(('image-total-pixel-ceiling', replace_source_image(data, solid_png(2048, 2048), copies=5), 'unsupported'))
    return result


def synthetic_archive(root):
    source = root / 'source.glb'
    source.write_bytes(fixture(2, transformed=False))
    doc, binary = geometry.decode_glb(source.read_bytes())
    expected, used, images, raw = scene_triangles(source.read_bytes())
    require(used == {0}, 'synthetic selected closure')
    doc['images'] = [{'uri': '../textures/0.png', 'mimeType': 'image/png'}]
    leaf = encode_glb(doc, binary)
    positions = [p for triangle in expected for p in triangle[0]]
    low = [min(p[i] for p in positions) for i in range(3)]
    high = [max(p[i] for p in positions) for i in range(3)]
    box = [(low[i] + high[i]) / 2 for i in range(3)] + [(high[0] - low[0]) / 2, 0, 0, 0, (high[1] - low[1]) / 2, 0, 0, 0, (high[2] - low[2]) / 2]
    error = max(1.0, math.dist(low, high))
    manifest = {'asset': {'version': '1.1'}, 'geometricError': error, 'root': {'transform': list(geometry.IDENTITY_TRANSFORM), 'boundingVolume': {'box': box}, 'geometricError': error, 'refine': 'REPLACE', 'children': [{'boundingVolume': {'box': box}, 'geometricError': 0, 'content': {'uri': 't/0.glb'}}]}}
    report = {'schema_version': 7, 'profile': PROFILE, 'approximation': {'kind': 'full_detail'}, 'source_coordinates': 'local-gltf', **geometry.local_placement_expectation(), 'source_bytes': source.stat().st_size, 'external_files': 0, 'external_bytes': 0, 'triangles': 2, 'leaf_tiles': 1, 'leaf_triangles': 2, 'routing_geometric_error_metres': error, 'images': 1, 'image_bytes': len(raw[0]), 'image_pixels': 6}
    members = {'tileset.json': json.dumps(manifest).encode(), 'conversion.json': json.dumps(report).encode(), 't/0.glb': leaf, 'textures/0.png': raw[0]}
    output = root / 'synthetic.3tz'
    geometry.write_control_archive(output, members)
    inspect(source, output, 2)
    return source, output, members


def self_test():
    png_tests = {}
    for kind in range(5):
        data = encode_png(filters=(kind, kind))
        require(decode_png(data) == (3, 2, PIXELS), 'analytic PNG filter ' + str(kind))
        png_tests[str(kind)] = 'passed'
    require(decoded_image(base64.b64decode(JPEG_BASE64), 'image/jpeg')['pixels'] == JPEG_MANIFEST['rgba'], 'prebuilt independent JPEG manifest')
    expected, _, _, _ = scene_triangles(fixture(2))
    require(geometry.close_value(expected[0][0], ((3, -7, -5), (0, -7.5, -5), (3, -7, -7)), 1e-12), 'analytic reflected positions')
    require(expected[0][2] == ((0.125, 0.25), (0.25, 0.875), (0.875, 0.375)), 'analytic reflected UV corner association')
    wanted = (1 / math.sqrt(37), -6 / math.sqrt(37), 0)
    require(all(math.dist(n, wanted) < 1e-12 for n in expected[0][1]), 'analytic reflected normal')
    match_triangles(expected, expected)
    controls = {}

    def rejected(name, callback):
        try:
            callback()
        except (OracleError, KeyError, IndexError, struct.error, zipfile.BadZipFile):
            controls[name] = 'rejected'
        else:
            raise OracleError('undetected corruption: ' + name)

    for name, mutate in (
        ('missing-triangle', lambda a: a.pop()),
        ('wrong-winding', lambda a: a.__setitem__(0, ((a[0][0][0], a[0][0][2], a[0][0][1]), *a[0][1:]))),
        ('wrong-uv', lambda a: a.__setitem__(0, (*a[0][:2], ((0, 0), *a[0][2][1:]), a[0][3]))),
        ('uv-corner-order', lambda a: a.__setitem__(0, (*a[0][:2], tuple(reversed(a[0][2])), a[0][3]))),
        ('missing-normal', lambda a: a.__setitem__(0, (a[0][0], None, *a[0][2:]))),
        ('wrong-factor', lambda a: a[0][3]['fields']['pbrMetallicRoughness']['baseColorFactor'].__setitem__(0, 0.25)),
        ('alpha-cutoff', lambda a: a[0][3]['fields'].__setitem__('alphaCutoff', 0.75)),
        ('double-sided', lambda a: a[0][3]['fields'].__setitem__('doubleSided', False)),
        ('sampler-omission', lambda a: a[0][3]['fields']['pbrMetallicRoughness']['baseColorTexture']['texture']['sampler'].pop('magFilter')),
        ('alpha-only', lambda a: a[0][3]['fields']['pbrMetallicRoughness']['baseColorTexture']['texture']['image']['pixels'][0].__setitem__(3, 0)),
    ):
        changed = copy.deepcopy(expected)
        mutate(changed)
        rejected(name, lambda value=changed: match_triangles(expected, value))
    for name, value, remove in (
        ('explicit-default-alpha-omission', 'alphaMode', lambda m: m['fields'].pop('alphaMode')),
        ('explicit-default-sampler-omission', 'magFilter', lambda m: m['fields']['pbrMetallicRoughness']['baseColorTexture']['texture']['sampler'].pop('magFilter')),
    ):
        declared = copy.deepcopy(expected)
        if value == 'alphaMode':
            declared[0][3]['fields']['alphaMode'] = 'OPAQUE'
        else:
            declared[0][3]['fields']['pbrMetallicRoughness']['baseColorTexture']['texture']['sampler']['magFilter'] = 9729
        omitted = copy.deepcopy(declared)
        remove(omitted[0][3])
        rejected(name, lambda a=declared, b=omitted: match_triangles(a, b))
    # Winding-preserving cyclic reindexing must rotate all associated corner data.
    rotated = [(p[1:] + p[:1], n[1:] + n[:1] if n else n, uv[1:] + uv[:1] if uv else uv, m) for p, n, uv, m in expected]
    match_triangles(expected, rotated)
    variants = {}
    for variant in VARIANTS:
        triangles, _, _, _ = scene_triangles(fixture(8, variant))
        require(len(triangles) == (16 if variant == 'instanced' else 8), 'fixture variant count ' + variant)
        if variant == 'nested':
            require(math.dist(triangles[0][0][0], (8, -14, 1)) < 1e-12, 'analytic nested position')
        if variant in ('uv-u8', 'uv-u16'):
            maximum = 255 if variant == 'uv-u8' else 65535
            require(abs(triangles[0][2][0][0] - 1 / maximum) < 1e-12, 'analytic normalized UV')
        variants[variant] = 'passed'
    match_triangles(scene_triangles(fixture(8, 'spike'))[0], scene_triangles(fixture(8, 'reordered-spike'))[0])
    with tempfile.TemporaryDirectory(prefix='f1b-oracle-controls-') as temporary:
        root = Path(temporary)
        source, output, members = synthetic_archive(root)
        mutations = [
            ('orphan-image', lambda m: m.__setitem__('textures/unused.png', encode_png())),
            ('missing-image', lambda m: m.pop('textures/0.png')),
            ('wrong-image-rows', lambda m: m.__setitem__('textures/0.png', encode_png(pixels=PIXELS[3:] + PIXELS[:3]))),
            ('encoded-byte-change-same-pixels', lambda m: m.__setitem__('textures/0.png', encode_png(comment='different-source-bytes'))),
            ('image-alpha', lambda m: m.__setitem__('textures/0.png', encode_png(pixels=((255, 0, 0, 0), *PIXELS[1:])))),
        ]
        for name, mutation in mutations:
            changed = copy.deepcopy(members)
            mutation(changed)
            geometry.write_control_archive(output, changed)
            rejected(name, lambda: inspect(source, output, 2))
        for name, change in (
            ('leaf-wrong-texture-index', lambda d, b: d['materials'][0]['pbrMetallicRoughness']['baseColorTexture'].__setitem__('index', 99)),
            ('leaf-unused-image', lambda d, b: d['images'].append(copy.deepcopy(d['images'][0]))),
            ('leaf-wrong-image-uri', lambda d, b: d['images'][0].__setitem__('uri', '../textures/9.png')),
            ('leaf-uv-association', lambda d, b: struct.pack_into('<f', b, d['bufferViews'][d['accessors'][2]['bufferView']]['byteOffset'], 0.9)),
        ):
            changed = copy.deepcopy(members)
            changed['t/0.glb'] = mutated_source(changed['t/0.glb'], change)
            geometry.write_control_archive(output, changed)
            rejected(name, lambda: inspect(source, output, 2))
    return {'png_analytic_filters': png_tests, 'jpeg_manifest_sha256': JPEG_MANIFEST['sha256'],
            'analytic_transform_normal_UV_winding': 'passed', 'fixture_variants': variants,
            'sensitive_corruption_controls': controls,
            'scope': 'full-detail textured leaf fidelity; no coarse approximation/error-bound acceptance'}


VARIANTS = ('standard', 'nested', 'instanced', 'scenes', 'coincident', 'degenerate', 'normal-absent',
            'nonindexed', 'uv-u8', 'uv-u16', 'empty-sampler', 'wrap', 'mixed', 'jpeg',
            'spike', 'reordered-spike', 'thin', 'disconnected', 'interleaved', 'uv-on-untextured', 'two-images', 'selected-image-one',
            'sampler-9728', 'sampler-9729', 'sampler-9984', 'sampler-9985', 'sampler-9986', 'sampler-9987')


def run_binary(binary):
    binary = Path(binary).resolve()
    cases = []
    refusals = []
    with tempfile.TemporaryDirectory(prefix='f1b-candidate-') as temporary:
        root = Path(temporary)
        for variant in VARIANTS:
            source = root / (variant + '.glb')
            source.write_bytes(fixture(8, variant))
            for limit in (1, 3, 1000):
                output = root / (variant + '-' + str(limit) + '.3tz')
                command = [str(binary), '--json', 'mesh-local-to-3tz', '-i', str(source), '-o', str(output), '--leaf-triangles', str(limit)]
                completed = subprocess.run(command, text=True, capture_output=True, timeout=60)
                require(completed.returncode == 0, 'CLI ' + variant + ': ' + completed.stdout + completed.stderr)
                summary = json.loads(completed.stdout)
                require(summary.get('ok') is True, 'CLI success outcome')
                inspected = inspect(source, output, limit)
                require(summary['meshReport'] == inspected['report'], 'CLI/report/archive parity')
                require(limit >= inspected['source_triangles'] or inspected['leaves'] > 1, 'forced multileaf evidence')
                cases.append({'variant': variant, 'leaf_limit': limit, 'status': 'passed', **inspected})
        for name, data, kind in rejection_fixtures():
            source = root / (name + '.glb')
            source.write_bytes(data)
            for limit in (1, 1000):
                output = root / 'uncreated' / name / (str(limit) + '.3tz')
                command = [str(binary), '--json', 'mesh-local-to-3tz', '-i', str(source), '-o', str(output), '--leaf-triangles', str(limit)]
                completed = subprocess.run(command, text=True, capture_output=True, timeout=60)
                summary = json.loads(completed.stdout)
                require(completed.returncode == (2 if kind == 'unsupported' else 3) and summary.get('ok') is False, 'ineligible source outcome: ' + name + ': ' + completed.stdout + completed.stderr)
                require(summary['error'].get('kind') == kind, 'source classification ' + name + ': ' + completed.stdout)
                require(not output.exists() and not output.parent.exists(), 'preparation created output work: ' + name)
                refusals.append({'case': name, 'leaf_limit': limit, 'error_kind': kind, 'output_exists': False, 'output_parent_exists': False, 'source_sha256': digest(data)})
    return {'binary': str(binary), 'binary_sha256': digest(binary.read_bytes()), 'cases': cases, 'refusal_controls': refusals}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--self-test', action='store_true')
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--fixture', type=Path)
    parser.add_argument('--triangles', type=int, default=8)
    parser.add_argument('--variant', choices=VARIANTS, default='standard')
    parser.add_argument('--source', type=Path)
    parser.add_argument('--archive', type=Path)
    parser.add_argument('--leaf-limit', type=int)
    parser.add_argument('--json-output', type=Path)
    args = parser.parse_args()
    result = {}
    if args.self_test or args.binary:
        result['self_test'] = self_test()
    if args.binary:
        result['candidate'] = run_binary(args.binary)
    if args.fixture:
        args.fixture.write_bytes(fixture(args.triangles, args.variant))
        result['fixture'] = str(args.fixture)
    if args.archive:
        if not args.source or not args.leaf_limit:
            parser.error('--source and --leaf-limit required with --archive')
        result['inspection'] = inspect(args.source, args.archive, args.leaf_limit)
    encoded = json.dumps(result, indent=2, allow_nan=False) + '\n'
    if args.json_output:
        args.json_output.parent.mkdir(parents=True, exist_ok=True)
        args.json_output.write_text(encoded)
    else:
        print(encoded, end='')


if __name__ == '__main__':
    main()
