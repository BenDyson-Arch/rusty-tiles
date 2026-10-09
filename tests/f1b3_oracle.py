#!/usr/bin/env python3
"""Independent full-detail core-PBR/UV1/tangent/color acceptance.

No production imports. Reuses independently established GLB/image/archive and
local-file binding mechanics, with separately authored extended corner logic.
"""
import argparse
import copy
import json
import math
from pathlib import Path, PurePosixPath
import struct
import subprocess
import tempfile
import zipfile

import f1a_oracle as geometry
import f1b_oracle as texture
import f1b2_oracle as binding

require = geometry.require
OracleError = geometry.OracleError
PROFILE = 'f1b-core-pbr-gltf-v1'
SLOTS = ('baseColorTexture', 'metallicRoughnessTexture', 'normalTexture', 'occlusionTexture', 'emissiveTexture')
ATTRIBUTES = ('POSITION', 'NORMAL', 'TANGENT', 'TEXCOORD_0', 'TEXCOORD_1', 'COLOR_0')
UV0 = ((0.125, 0.25), (0.875, 0.375), (0.25, 0.875))
UV1 = ((0.875, 0.75), (0.125, 0.625), (0.75, 0.125))
COLORS = ((0.25, 0.5, 0.75, 0.25), (0.75, 0.25, 0.5, 0.75), (0.5, 0.75, 0.25, 1.0))
IMAGE_PIXELS = (
    ((22, 44, 66, 255),) * 6,
    ((32, 80, 192, 255), (200, 32, 144, 128), (64, 192, 48, 64), (160, 96, 224, 192), (96, 224, 80, 255), (224, 144, 32, 0)),
    ((16, 48, 208, 32), (208, 160, 32, 224), (80, 224, 96, 64), (240, 80, 160, 128), (112, 192, 48, 192), (48, 112, 240, 255)),
    ((192, 96, 224, 255), (80, 176, 224, 64), (144, 208, 224, 128), (208, 144, 224, 192), (112, 64, 224, 0), (176, 192, 224, 255)),
    ((32, 224, 160, 128), (224, 32, 80, 255), (80, 192, 240, 64), (160, 96, 32, 192), (112, 240, 96, 0), (192, 64, 224, 255)),
    ((64, 128, 192, 255), (192, 64, 128, 0), (128, 192, 64, 128), (96, 160, 224, 64), (224, 96, 160, 192), (160, 224, 96, 255)),
)


def slot(material, name):
    return material.get('pbrMetallicRoughness', {}).get(name) if name in SLOTS[:2] else material.get(name)


def unit(vector):
    length = math.sqrt(sum(x * x for x in vector))
    require(length > 0 and math.isfinite(length), 'nonzero finite vector')
    return tuple(x / length for x in vector)


def direction(matrix, vector):
    return tuple(sum(matrix[r][c] * vector[c] for c in range(3)) for r in range(3))


def conformal(matrix):
    # Independent column Gram matrix, avoiding the production normal/tangent path.
    gram = [[sum(matrix[r][i] * matrix[r][j] for r in range(3)) for j in range(3)] for i in range(3)]
    scale = sum(gram[i][i] for i in range(3)) / 3
    return scale > 0 and all(abs(gram[i][j] - (scale if i == j else 0)) <= 1e-10 * scale for i in range(3) for j in range(3))


def scene_triangles(data, resolver=None, output=False):
    doc, binary = geometry.decode_glb(data)
    materials, images, raw_images = texture.resources(doc, binary, resolver)
    # An equal-pixel/equal-byte image must not hide a logical identity swap.
    for original, canonical in zip(doc.get('materials', []), materials):
        for name in SLOTS:
            info = slot(original, name)
            if info is not None:
                image_id = doc['textures'][info['index']]['source']
                if output:
                    image_id = int(PurePosixPath(doc['images'][image_id]['uri']).stem)
                slot(canonical['fields'], name)['texture']['source_image_id'] = image_id
    triangles, selected_primitives, active = [], [], set()

    def walk(index, parent):
        require(index not in active, 'node cycle')
        active.add(index)
        node = doc['nodes'][index]
        matrix = geometry.product(parent, geometry.node_matrix(node))
        determinant = geometry.determinant(matrix)
        require(determinant != 0, 'singular accumulated transform')
        if 'mesh' in node:
            for primitive in doc['meshes'][node['mesh']]['primitives']:
                selected_primitives.append(primitive)
                require(primitive.get('mode', 4) == 4, 'triangle mode')
                attributes = primitive['attributes']
                require(set(attributes) <= set(ATTRIBUTES), 'unadmitted corner attribute')
                decoded = {name: texture.accessor(doc, binary, accessor) for name, accessor in attributes.items()}
                count = len(decoded['POSITION'])
                require(all(len(values) == count for values in decoded.values()), 'corner accessor counts')
                for name, index_ in attributes.items():
                    item = doc['accessors'][index_]
                    shape = 'VEC4' if name == 'TANGENT' else 'VEC2' if name.startswith('TEXCOORD_') else 'VEC3'
                    require(item['type'] in ('VEC3', 'VEC4') if name == 'COLOR_0' else item['type'] == shape, 'attribute shape ' + name)
                    if output or name in ('POSITION', 'NORMAL', 'TANGENT'):
                        require(item['componentType'] == 5126 and not item.get('normalized', False), 'f32 attribute encoding ' + name)
                    else:
                        require(item['componentType'] == 5126 and not item.get('normalized', False) or item['componentType'] in (5121, 5123) and item.get('normalized') is True, 'source UV/color component encoding')
                    if output and name == 'COLOR_0':
                        require(item['type'] == 'VEC4', 'canonical output RGBA')
                for name in ('NORMAL', 'TANGENT'):
                    for value in decoded.get(name, []):
                        require(abs(math.sqrt(sum(x*x for x in value[:3])) - 1) <= 1e-4, 'unit XYZ ' + name)
                if 'TANGENT' in decoded:
                    require('NORMAL' in decoded and conformal(matrix), 'authored tangent companion/conformal profile')
                    require(all(value[3] in (-1.0, 1.0) for value in decoded['TANGENT']), 'tangent sign')
                for value in decoded.get('COLOR_0', []):
                    require(all(0 <= x <= 1 for x in value), 'unit color range')
                for name in ('TEXCOORD_0', 'TEXCOORD_1'):
                    require(all(abs(x) <= 1000000 for value in decoded.get(name, []) for x in value), 'UV domain')
                require('TEXCOORD_1' not in decoded or 'TEXCOORD_0' in decoded, 'consecutive UV sets')
                material = materials[primitive['material']] if 'material' in primitive else {'present': False, 'fields': {}}
                for info in texture.texture_infos(material['fields']):
                    uv_set = info.get('texCoord', 0)
                    require(type(uv_set) is int and uv_set in (0, 1) and 'TEXCOORD_' + str(uv_set) in decoded, 'texture coordinate association')
                if 'normalTexture' in material['fields']:
                    require('NORMAL' in decoded and 'TANGENT' in decoded, 'normal-texture authored companions')
                indices = texture.accessor(doc, binary, primitive['indices']) if 'indices' in primitive else list(range(count))
                require(len(indices) % 3 == 0 and all(type(i) is int and 0 <= i < count for i in indices), 'triangle indices')
                for start in range(0, len(indices), 3):
                    ids = list(indices[start:start + 3])
                    if 'TANGENT' in decoded:
                        require(len({decoded['TANGENT'][i][3] for i in ids}) == 1, 'uniform triangle handedness')
                    if determinant < 0:
                        ids[1], ids[2] = ids[2], ids[1]
                    corners = []
                    for i in ids:
                        corner = {'POSITION': geometry.z_up(geometry.point(matrix, decoded['POSITION'][i]))}
                        if 'NORMAL' in decoded:
                            corner['NORMAL'] = geometry.z_up(geometry.normal(matrix, decoded['NORMAL'][i]))
                        if 'TANGENT' in decoded:
                            t = decoded['TANGENT'][i]
                            corner['TANGENT'] = (*geometry.z_up(unit(direction(matrix, t[:3]))), t[3] * (-1 if determinant < 0 else 1))
                        for name in ('TEXCOORD_0', 'TEXCOORD_1', 'COLOR_0'):
                            if name in decoded:
                                value = tuple(decoded[name][i])
                                corner[name] = (*value, 1.0) if name == 'COLOR_0' and len(value) == 3 else value
                        corners.append(corner)
                    triangles.append({'corners': tuple(corners), 'material': copy.deepcopy(material)})
        for child in node.get('children', []):
            walk(child, matrix)
        active.remove(index)

    selected = doc.get('scene', 0)
    require(doc.get('scenes') and type(selected) is int and 0 <= selected < len(doc['scenes']), 'selected scene')
    require('scene' in doc or len(doc['scenes']) == 1, 'unambiguous selected scene')
    for root in doc['scenes'][selected]['nodes']:
        walk(root, geometry.identity())
    used = texture.used_resource_ids(doc, selected_primitives)
    if output:
        for ids, key in zip(used, ('materials', 'textures', 'images', 'samplers')):
            require(ids == set(range(len(doc.get(key, [])))), 'exact leaf core-PBR ' + key + ' closure')
    return triangles, used[2], images, raw_images


def match_triangles(expected, actual):
    require(len(expected) == len(actual), 'extended triangle multiplicity')
    pending = list(actual)
    def corner_matches(a, b):
        if a.keys() != b.keys():
            return False
        for name in a:
            if len(a[name]) != len(b[name]):
                return False
            for component, (x, y) in enumerate(zip(a[name], b[name])):
                tolerance = max(1e-6, abs(x)*2**-24) if name == 'POSITION' else 2e-7 if name in ('NORMAL', 'TANGENT') else max(1, abs(x))*2**-24
                if name == 'TANGENT' and component == 3:
                    tolerance = 0
                if abs(x-y) > tolerance + (0 if tolerance == 0 else 1e-12):
                    return False
        return True
    for wanted in expected:
        found = None
        for i, candidate in enumerate(pending):
            if wanted['material'] != candidate['material']:
                continue
            corners = candidate['corners']
            if any(all(corner_matches(a, b) for a, b in zip(wanted['corners'], corners[rotation:] + corners[:rotation])) for rotation in range(3)):
                found = i
                break
        require(found is not None, 'oriented position/N/T/UV0/UV1/RGBA/core-PBR/source-ID fidelity')
        pending.pop(found)


def inspect(source, archive, leaf_limit):
    model = binding.bind_source(source, scene_reader=scene_triangles)
    result = texture.inspect(source, archive, leaf_limit, source_model=model, scene_reader=scene_triangles,
                             triangle_matcher=match_triangles, triangle_positions=lambda t: [c['POSITION'] for c in t['corners']])
    result.update(external_files=model['external_files'], external_bytes=model['external_bytes'], source_dependencies=model['physical'])
    return result


VARIANTS = ('mr-only', 'occlusion-defaults', 'occlusion-strength', 'emissive-only', 'all-slots',
            'shared-orm', 'shared-colorspace', 'uv1-slots', 'uv1-outside', 'uv1-u8', 'uv1-u16',
            'color-f32-rgb', 'color-f32-rgba-mask', 'color-u8-rgb', 'color-u8-rgba', 'color-u16',
            'unused-attributes', 'normal-signs', 'normal-scales', 'uniform-rotation', 'reflected',
            'interleaved', 'nonuniform-nonnormal', 'selected-equal-images', 'nested-conformal', 'nonorthogonal-tangent')
EXTERNAL_VARIANTS = ('emissive-only', 'all-slots', 'uv1-slots', 'color-f32-rgb', 'color-u8-rgb', 'normal-signs', 'interleaved', 'selected-equal-images')


def fixture(variant='all-slots', triangles=8):
    require(variant in VARIANTS, 'known core-PBR fixture')
    doc = {'asset': {'version': '2.0'}, 'scene': 0, 'scenes': [{'nodes': [0]}], 'nodes': [{'mesh': 0}],
           'meshes': [{'primitives': []}], 'bufferViews': [], 'accessors': [], 'images': [],
           'samplers': [{'magFilter': 9728, 'minFilter': 9728, 'wrapS': 33071, 'wrapT': 33071}, {}],
           'textures': [{'source': i, 'sampler': (i % 2)} for i in (3, 1, 5, 2, 4)], 'materials': []}
    binary = bytearray()
    def view(raw, stride=None):
        binary.extend(b'\0' * (-len(binary) % 4))
        index = len(doc['bufferViews'])
        doc['bufferViews'].append({'buffer': 0, 'byteOffset': len(binary), 'byteLength': len(raw)})
        if stride is not None:
            doc['bufferViews'][index]['byteStride'] = stride
        binary.extend(raw)
        return index
    def add(values, shape, component=5126, normalized=False):
        code, size = {5126: ('f', 4), 5121: ('B', 1), 5123: ('H', 2)}[component]
        width = {'VEC2': 2, 'VEC3': 3, 'VEC4': 4, 'SCALAR': 1}[shape]
        values = [(v,) for v in values] if width == 1 else values
        packed = [struct.pack('<' + code*width, *v) for v in values]
        stride = (size*width + 3) // 4 * 4 if width > 1 else size
        raw = b''.join(row + b'\0'*(stride-len(row)) for row in packed)
        accessor = {'bufferView': view(raw, stride if stride != size*width else None), 'componentType': component, 'type': shape, 'count': len(values)}
        if normalized:
            accessor['normalized'] = True
        if width > 1:
            accessor.update(min=[min(v[i] for v in values) for i in range(width)], max=[max(v[i] for v in values) for i in range(width)])
        index = len(doc['accessors']); doc['accessors'].append(accessor)
        return index
    normal = variant in ('all-slots', 'shared-colorspace', 'uv1-slots', 'unused-attributes', 'normal-signs', 'normal-scales', 'uniform-rotation', 'reflected', 'interleaved', 'selected-equal-images', 'nested-conformal', 'nonorthogonal-tangent')
    for i in range(triangles):
        positions = [(4*i, 0, 0), (4*i+1, 0, 0), (4*i, 1, 0)]
        attributes = {'POSITION': add(positions, 'VEC3'), 'NORMAL': add([(0, 0, 1)]*3, 'VEC3'),
                      'TEXCOORD_0': add(UV0, 'VEC2'), 'TEXCOORD_1': add(UV1, 'VEC2')}
        if normal:
            attributes['TANGENT'] = add([((0.6, 0, 0.8) if variant == 'nonorthogonal-tangent' else (0.6, 0.8, 0)) + ((-1.0 if i % 2 else 1.0),)]*3, 'VEC4')
        if variant == 'uv1-outside':
            attributes['TEXCOORD_1'] = add([(-0.25, 1.25), (1.5, -0.75), (0.5, 0.5)], 'VEC2')
        elif variant in ('uv1-u8', 'uv1-u16'):
            maximum = 255 if variant.endswith('u8') else 65535
            attributes['TEXCOORD_1'] = add([(1, maximum-1), (maximum//3, maximum//2), (maximum, 0)], 'VEC2', 5121 if maximum == 255 else 5123, True)
        color_shape = 'VEC3' if variant in ('color-f32-rgb', 'color-u8-rgb') or variant == 'color-u16' and i % 2 == 0 else 'VEC4'
        values = [v[:3] if color_shape == 'VEC3' else v for v in COLORS]
        if variant in ('color-u8-rgb', 'color-u8-rgba', 'color-u16'):
            maximum = 65535 if variant == 'color-u16' else 255
            values = [(1, maximum//3, maximum-1) + ((maximum//2,) if color_shape == 'VEC4' else ())]*3
            attributes['COLOR_0'] = add(values, color_shape, 5123 if maximum == 65535 else 5121, True)
        else:
            attributes['COLOR_0'] = add(values, color_shape)
        material = {'pbrMetallicRoughness': {'baseColorFactor': [0.8, 0.6, 0.4, 0.75], 'metallicFactor': 0.375, 'roughnessFactor': 0.625}, 'doubleSided': bool(i%2)}
        active_slots = SLOTS if normal else ('baseColorTexture', 'metallicRoughnessTexture', 'occlusionTexture', 'emissiveTexture')
        if variant == 'mr-only': active_slots = ('metallicRoughnessTexture',)
        if variant.startswith('occlusion-'): active_slots = ('occlusionTexture',)
        if variant == 'emissive-only': active_slots = ('emissiveTexture',)
        if variant.startswith('color-') or variant == 'unused-attributes': active_slots = ()
        if variant == 'shared-orm': active_slots = ('metallicRoughnessTexture', 'occlusionTexture')
        if variant.startswith('normal-') or variant in ('uniform-rotation', 'reflected', 'nested-conformal', 'nonorthogonal-tangent'): active_slots = ('normalTexture',)
        for name in active_slots:
            # Scrambled texture order exercises per-slot remapping.
            texture_index = {'baseColorTexture': 1, 'metallicRoughnessTexture': 3, 'normalTexture': 0, 'occlusionTexture': 4, 'emissiveTexture': 2}[name]
            if variant == 'shared-orm':
                doc['textures'][4]['source'] = 2
            if variant == 'shared-colorspace':
                for t in doc['textures']: t['source'] = 1
            info = {'index': texture_index}
            if i % 2 or variant in ('uv1-slots', 'uv1-outside', 'uv1-u8', 'uv1-u16'):
                info['texCoord'] = 1 if name != 'normalTexture' or i%2 else 0
            if name == 'normalTexture' and variant == 'normal-scales': info['scale'] = (0, 0.5, 2, -1)[i%4]
            if name == 'occlusionTexture' and (i%2 or variant == 'occlusion-strength'): info['strength'] = (0, 0.375, 1)[i%3]
            (material['pbrMetallicRoughness'] if name in SLOTS[:2] else material)[name] = info
        if 'emissiveTexture' in active_slots and i % 3:
            material['emissiveFactor'] = [0.2, 0.4, 0.6] if i%3 == 1 else [0, 0, 0]
        if variant == 'color-f32-rgba-mask':
            material.update(alphaMode='MASK', alphaCutoff=0.3)
        doc['materials'].append(material)
        primitive = {'attributes': attributes, 'material': i}
        if i%2 == 0: primitive['indices'] = add([0, 1, 2], 'SCALAR', 5121)
        doc['meshes'][0]['primitives'].append(primitive)
        if variant == 'interleaved':
            fields = ('POSITION', 'NORMAL', 'TANGENT', 'TEXCOORD_0', 'TEXCOORD_1', 'COLOR_0')
            values_ = {name: texture.accessor(doc, binary, attributes[name]) for name in fields}
            raw = b''.join(struct.pack('<18f', *(x for name in fields for x in values_[name][j])) for j in range(3))
            shared = view(raw, 72)
            for name, offset in zip(fields, (0, 12, 24, 40, 48, 56)):
                doc['accessors'][attributes[name]].update(bufferView=shared, byteOffset=offset)
    if variant == 'uniform-rotation': doc['nodes'][0]['matrix'] = [0, 2, 0, 0, -2, 0, 0, 0, 0, 0, 2, 0, 3, -5, 7, 1]
    if variant == 'reflected': doc['nodes'][0]['scale'] = [-2, 2, 2]
    if variant == 'nonuniform-nonnormal': doc['nodes'][0]['scale'] = [-2, 3, 0.5]
    if variant == 'nested-conformal':
        doc['nodes'][0]['scale'] = [0.5, 1/3, 0.25]
        doc['nodes'].append({'scale': [2, 3, 4], 'children': [0]})
        doc['scenes'][0]['nodes'] = [1]
    if variant == 'selected-equal-images':
        doc['meshes'].append(copy.deepcopy(doc['meshes'][0]))
        doc['nodes'].append({'mesh': 1, 'translation': [1, 2, 3]})
        doc['scenes'].append({'nodes': [1]}); doc['scene'] = 1
    for i, pixels in enumerate(IMAGE_PIXELS):
        if variant == 'selected-equal-images' and i == 5: pixels = IMAGE_PIXELS[1]
        doc['images'].append({'bufferView': view(texture.encode_png(pixels=pixels)), 'mimeType': 'image/png'})
    doc['buffers'] = [{'byteLength': len(binary)}]
    return texture.encode_glb(doc, bytes(binary))


def source_bundle(variant='all-slots', *, external=False):
    data = fixture(variant)
    doc, binary = geometry.decode_glb(data)
    if not external:
        return {'document': doc, 'binary': binary, 'files': {}, 'hardlinks': {}, 'container': 'glb'}
    files = {'geometry/data.bin': binary}
    doc['buffers'][0]['uri'] = 'geometry/data.bin'
    for i, image in enumerate(doc['images']):
        view = doc['bufferViews'][image.pop('bufferView')]
        start = view.get('byteOffset', 0)
        name = 'images/' + str(i) + '.png'
        files[name] = binary[start:start+view['byteLength']]
        image['uri'] = name
    if variant == 'selected-equal-images':
        doc['images'][5]['uri'] = 'images/1.png'
        del files['images/5.png']
    return {'document': doc, 'binary': None, 'files': files, 'hardlinks': {}, 'container': 'gltf'}


def write_fixture(root, variant='all-slots', *, external=False):
    return binding.write_bundle(root, source_bundle(variant, external=external))


def viewer_fixture():
    """Six independently authored nearest/clamp panels for a real lit renderer."""
    doc={'asset':{'version':'2.0'},'scene':0,'scenes':[{'nodes':[0]}],'nodes':[{'mesh':0,'scale':[1.25,1.25,1.25]}],
         'meshes':[{'primitives':[]}],'accessors':[],'bufferViews':[],'images':[],
         'textures':[{'source':i,'sampler':0} for i in range(5)],
         'samplers':[{'magFilter':9728,'minFilter':9728,'wrapS':33071,'wrapT':33071}],'materials':[]}
    binary=bytearray()
    def view(raw):
        binary.extend(b'\0'*(-len(binary)%4)); i=len(doc['bufferViews'])
        doc['bufferViews'].append({'buffer':0,'byteOffset':len(binary),'byteLength':len(raw)})
        binary.extend(raw); return i
    def add(values,width):
        a={'bufferView':view(struct.pack('<'+'f'*(width*len(values)),*(x for v in values for x in v))),
           'componentType':5126,'type':'VEC'+str(width),'count':len(values)}
        a.update(min=[min(v[i] for v in values) for i in range(width)],max=[max(v[i] for v in values) for i in range(width)])
        i=len(doc['accessors']);doc['accessors'].append(a);return i
    for i in range(6):
        left=-3.4+2.4*(i%3); bottom=-2.2+2.4*(i//3)
        positions=[(left,bottom,0),(left+2,bottom,0),(left+2,bottom+2,0),(left,bottom,0),(left+2,bottom+2,0),(left,bottom+2,0)]
        uv=[(0,1),(1,1),(1,0),(0,1),(1,0),(0,0)]
        material={'pbrMetallicRoughness':{'baseColorFactor':[0.6,0.45,0.3,1],'metallicFactor':0,'roughnessFactor':0.7},'doubleSided':True}
        attrs={'POSITION':add(positions,3),'NORMAL':add([(0,0,1)]*6,3),'TEXCOORD_0':add(uv,2),'TEXCOORD_1':add([(1-u,v) for u,v in uv],2)}
        if i in (0,5):
            material['pbrMetallicRoughness'].update(metallicFactor=0.8,roughnessFactor=0.8,metallicRoughnessTexture={'index':1,'texCoord':1})
        if i in (1,5):
            material['normalTexture']={'index':2,'texCoord':1,'scale':1.5}
            attrs['TANGENT']=add([(1,0,0,-1)]*6,4)
        if i in (2,5):material['occlusionTexture']={'index':3,'texCoord':1,'strength':1}
        if i in (3,5):
            material['emissiveTexture']={'index':4,'texCoord':1}
            material['emissiveFactor']=[0.5,0.4,0.3]
            if i==3:material['pbrMetallicRoughness']['baseColorFactor']=[0,0,0,1]
        if i in (4,5):
            material['pbrMetallicRoughness']['baseColorTexture']={'index':0,'texCoord':1}
            attrs['COLOR_0']=add([(0.2,0.6,0.9,1)]*6,4)
        doc['materials'].append(material)
        doc['meshes'][0]['primitives'].append({'attributes':attrs,'material':i})
    for pixels in IMAGE_PIXELS[1:]:
        doc['images'].append({'bufferView':view(texture.encode_png(pixels=pixels)),'mimeType':'image/png'})
    doc['buffers']=[{'byteLength':len(binary)}]
    return texture.encode_glb(doc,bytes(binary))


def synthetic_archive(root):
    """Independent archive author, exercising the complete inspection predicate."""
    source=root/'source.glb';source.write_bytes(fixture('all-slots',2))
    doc,binary=geometry.decode_glb(source.read_bytes())
    expected,used,images,raw=scene_triangles(source.read_bytes())
    require(used=={1,2,3,4,5},'literal all-slot selected closure')
    doc['images']=[{'uri':'../textures/'+str(i)+'.png','mimeType':'image/png'} for i in range(1,6)]
    for t in doc['textures']:t['source']-=1
    positions=[c['POSITION'] for t in expected for c in t['corners']]
    low=[min(p[i] for p in positions) for i in range(3)];high=[max(p[i] for p in positions) for i in range(3)]
    box=[(a+b)/2 for a,b in zip(low,high)]+[(high[0]-low[0])/2,0,0,0,(high[1]-low[1])/2,0,0,0,(high[2]-low[2])/2]
    error=max(1.0,2*math.sqrt(sum(box[i]**2 for i in (3,7,11))))
    tileset={'asset':{'version':'1.1'},'geometricError':error,'root':{'boundingVolume':{'box':box},'geometricError':error,'refine':'REPLACE','children':[{'boundingVolume':{'box':box},'geometricError':0,'content':{'uri':'t/0.glb'}}]}}
    report={'schema_version':3,'profile':PROFILE,'coordinates':'local-gltf','source_bytes':source.stat().st_size,'external_files':0,'external_bytes':0,'triangles':2,'leaf_tiles':1,'leaf_triangles':2,'routing_geometric_error_metres':error,'images':5,'image_bytes':sum(len(raw[i]) for i in used),'image_pixels':30}
    members={'tileset.json':json.dumps(tileset).encode(),'conversion.json':json.dumps(report).encode(),'t/0.glb':texture.encode_glb(doc,binary)}
    members.update({'textures/'+str(i)+'.png':raw[i] for i in used})
    archive=root/'control.3tz';geometry.write_control_archive(archive,members);inspect(source,archive,2)
    return source,archive,members


def refusal_bundles():
    cases = []
    def add(name, mutate, kind='invalid_input', variant='all-slots'):
        bundle = source_bundle(variant)
        mutable = bytearray(bundle['binary']); mutate(bundle['document'], mutable); bundle['binary'] = bytes(mutable)
        cases.append((name, bundle, kind))
    def attr(doc, name): return doc['accessors'][doc['meshes'][0]['primitives'][0]['attributes'][name]]
    def component(doc, binary, name, index, value):
        a = attr(doc, name); v = doc['bufferViews'][a['bufferView']]
        struct.pack_into('<f', binary, v.get('byteOffset', 0) + a.get('byteOffset', 0) + index*4, value)
        a.pop('min', None); a.pop('max', None)
    for name in SLOTS:
        add('bad-index-' + name, lambda d,b,n=name: slot(d['materials'][0],n).__setitem__('index', 99))
        add('missing-uv1-' + name, lambda d,b,n=name: (slot(d['materials'][0],n).__setitem__('texCoord',1), d['meshes'][0]['primitives'][0]['attributes'].pop('TEXCOORD_1')))
        add('uv2-' + name, lambda d,b,n=name: slot(d['materials'][0],n).__setitem__('texCoord',2), 'unsupported')
        add('slot-extension-' + name, lambda d,b,n=name: slot(d['materials'][0],n).__setitem__('extensions',{'KHR_texture_transform':{}}), 'unsupported')
    for name in ('TANGENT','TEXCOORD_1','COLOR_0'):
        add('count-' + name, lambda d,b,n=name: attr(d,n).__setitem__('count',2))
        add('shape-' + name, lambda d,b,n=name: attr(d,n).__setitem__('type','VEC2' if n != 'TEXCOORD_1' else 'VEC3'))
        add('nan-' + name, lambda d,b,n=name: component(d,b,n,0,float('nan')))
        add('range-' + name, lambda d,b,n=name: attr(d,n).__setitem__('byteOffset',100000))
    add('tangent-no-normal',lambda d,b: d['meshes'][0]['primitives'][0]['attributes'].pop('NORMAL'),'unsupported')
    add('normal-no-tangent',lambda d,b: d['meshes'][0]['primitives'][0]['attributes'].pop('TANGENT'),'unsupported')
    add('tangent-nonconformal',lambda d,b: d['nodes'][0].__setitem__('scale',[2,3,4]),'unsupported')
    add('tangent-nonconformal-unused-material',lambda d,b: d['nodes'][0].__setitem__('scale',[2,3,4]),'unsupported','unused-attributes')
    add('tangent-mixed-w',lambda d,b: component(d,b,'TANGENT',7,-1),'unsupported')
    add('tangent-sign-zero',lambda d,b: component(d,b,'TANGENT',3,0))
    add('tangent-zero',lambda d,b: [component(d,b,'TANGENT',i,0) for i in range(3)])
    add('tangent-nonunit',lambda d,b: component(d,b,'TANGENT',0,10))
    add('color-above-one',lambda d,b: component(d,b,'COLOR_0',0,1.5),'unsupported')
    add('color-negative',lambda d,b: component(d,b,'COLOR_0',3,-0.25),'unsupported')
    add('color-integer-unnormalized',lambda d,b: attr(d,'COLOR_0').__setitem__('componentType',5121))
    add('color-short-unnormalized',lambda d,b: attr(d,'COLOR_0').__setitem__('componentType',5123))
    add('color-float-normalized',lambda d,b: attr(d,'COLOR_0').__setitem__('normalized',True))
    add('uv1-integer-unnormalized',lambda d,b: attr(d,'TEXCOORD_1').__setitem__('componentType',5121))
    add('tangent-float-normalized',lambda d,b: attr(d,'TANGENT').__setitem__('normalized',True))
    add('uv1-not-consecutive',lambda d,b: d['meshes'][0]['primitives'][0]['attributes'].pop('TEXCOORD_0'))
    add('color-one',lambda d,b: d['meshes'][0]['primitives'][0]['attributes'].__setitem__('COLOR_1',0),'unsupported')
    add('uv1-domain',lambda d,b: component(d,b,'TEXCOORD_1',0,1000001),'unsupported')
    add('occlusion-strength-range',lambda d,b: slot(d['materials'][0],'occlusionTexture').__setitem__('strength',1.5))
    add('emissive-factor-range',lambda d,b: d['materials'][0].__setitem__('emissiveFactor',[0,1.5,0]))
    def unused(doc,binary,component,shape,normalized):
        width={'VEC3':3,'VEC4':4,'SCALAR':1,'MAT4':16}[shape]
        size={5120:1,5122:2,5125:4,5126:4}[component]
        stride=(width*size+3)//4*4
        binary.extend(b'\0'*(-len(binary)%4));start=len(binary);binary.extend(b'\0'*(stride*3))
        index=len(doc['bufferViews']);view={'buffer':0,'byteOffset':start,'byteLength':stride*3}
        if stride!=width*size:view['byteStride']=stride
        doc['bufferViews'].append(view)
        a={'bufferView':index,'componentType':component,'type':shape,'count':3}
        if normalized:a['normalized']=True
        doc['accessors'].append(a);doc['buffers'][0]['byteLength']=len(binary)
    for component,shape,normalized in ((5120,'VEC3',True),(5122,'VEC3',False),(5126,'SCALAR',False),(5126,'MAT4',False),(5125,'VEC4',False)):
        add('unused-storage-'+str(component)+'-'+shape,lambda d,b,c=component,s=shape,n=normalized:unused(d,b,c,s,n),'unsupported')
    # Whole-document new semantics must beat earlier dependency opening.
    for name, bundle, kind in list(cases):
        if name in ('bad-index-emissiveTexture','slot-extension-occlusionTexture','shape-COLOR_0',
                    'color-integer-unnormalized','color-short-unnormalized','color-float-normalized',
                    'uv1-integer-unnormalized','tangent-float-normalized') or name.startswith('unused-storage-'):
            external = source_bundle('all-slots',external=True)
            external['document'] = copy.deepcopy(bundle['document'])
            external['document']['buffers'][0]['uri'] = 'missing.bin'
            cases.append((name+'-before-missing-file',external,kind))
    return cases


def self_test():
    values = {}
    for variant in VARIANTS:
        model = scene_triangles(fixture(variant))
        require(len(model[0]) == 8, 'authored variant multiplicity')
        match_triangles(model[0], model[0]); values[variant] = sorted(model[1])
    base = scene_triangles(fixture('all-slots'))[0]
    require(base[0]['corners'][0]['TEXCOORD_1'] == (0.875,0.75), 'literal independent UV1')
    require(base[0]['corners'][0]['COLOR_0'] == (0.25,0.5,0.75,0.25), 'literal independent RGBA')
    for variant, maximum in (('color-u8-rgb',255),('color-u16',65535)):
        color = scene_triangles(fixture(variant))[0][0]['corners'][0]['COLOR_0']
        require(color == (1/maximum,(maximum//3)/maximum,(maximum-1)/maximum,1.0), 'literal normalized RGB fractions')
    reflected = scene_triangles(fixture('reflected'))[0][0]['corners']
    require(geometry.close_value(reflected[0]['TANGENT'],(-0.6,0,0.8,-1),2e-7), 'literal reflected tangent/W/up-axis')
    require([c['TEXCOORD_1'] for c in reflected] == [UV1[0],UV1[2],UV1[1]], 'literal reflected all-corner association')
    rotated=scene_triangles(fixture('uniform-rotation'))[0][0]['corners'][0]
    require(geometry.close_value(rotated['POSITION'],(3,-7,-5),1e-12) and geometry.close_value(rotated['TANGENT'],(-.8,0,.6,1),2e-7),'literal proper rotation and translation')
    nonorthogonal=scene_triangles(fixture('nonorthogonal-tangent'))[0][0]['corners'][0]
    require(abs(sum(x*y for x,y in zip(nonorthogonal['NORMAL'],nonorthogonal['TANGENT'][:3]))-.8)<2e-7,'literal authored nonorthogonality survives')
    for variant,maximum in (('uv1-u8',255),('uv1-u16',65535)):
        require(scene_triangles(fixture(variant))[0][0]['corners'][0]['TEXCOORD_1']==(1/maximum,(maximum-1)/maximum),'literal normalized UV1 fractions')
    require(conformal(geometry.node_matrix({'scale':[-2,2,2]})) and not conformal(geometry.node_matrix({'scale':[2,3,4]})), 'independent conformality')
    controls = {}
    def rejected(name, callback):
        try: callback()
        except (OracleError,KeyError,IndexError,ValueError): controls[name]='rejected'
        else: raise OracleError('undetected extended semantic corruption '+name)
    mutations = [('omit-'+name,lambda a,n=name:a[0]['corners'][0].pop(n)) for name in ('TANGENT','COLOR_0','TEXCOORD_1')]
    mutations += [('omit-'+name,lambda a,n=name:(a[0]['material']['fields']['pbrMetallicRoughness'] if n in SLOTS[:2] else a[0]['material']['fields']).pop(n)) for name in SLOTS]
    mutations += [
        ('wrong-uv1',lambda a:a[0]['corners'][0].__setitem__('TEXCOORD_1',UV0[0])),
        ('wrong-color-alpha',lambda a:a[0]['corners'][0].__setitem__('COLOR_0',(0.25,0.5,0.75,1))),
        ('wrong-tangent-w',lambda a:a[0]['corners'][0].__setitem__('TANGENT',(0.6,0,0.8,-1))),
        ('wrong-tangent-xyz',lambda a:a[0]['corners'][0].__setitem__('TANGENT',(1,0,0,1))),
        ('wrong-winding',lambda a:a[0].__setitem__('corners',(a[0]['corners'][0],a[0]['corners'][2],a[0]['corners'][1]))),
        ('wrong-slot',lambda a:slot(a[0]['material']['fields'],'emissiveTexture').__setitem__('texture',copy.deepcopy(slot(a[0]['material']['fields'],'occlusionTexture')['texture']))),
        ('wrong-normal-scale',lambda a:slot(a[0]['material']['fields'],'normalTexture').__setitem__('scale',0.5)),
        ('wrong-occlusion-strength',lambda a:slot(a[0]['material']['fields'],'occlusionTexture').__setitem__('strength',0.5)),
        ('missing-texcoord-field',lambda a:slot(a[1]['material']['fields'],'emissiveTexture').pop('texCoord')),
        ('wrong-logical-image-id',lambda a:slot(a[0]['material']['fields'],'baseColorTexture')['texture'].__setitem__('source_image_id',5)),
    ]
    for name, mutation in mutations:
        changed=copy.deepcopy(base); mutation(changed)
        rejected(name,lambda a=changed:match_triangles(base,a))
    equal=scene_triangles(fixture('selected-equal-images'))[0]
    changed=copy.deepcopy(equal); slot(changed[0]['material']['fields'],'baseColorTexture')['texture']['source_image_id']=5
    rejected('equal-byte-logical-image-swap',lambda:match_triangles(equal,changed))
    cyclic=copy.deepcopy(base)
    for t in cyclic: t['corners']=t['corners'][1:]+t['corners'][:1]
    match_triangles(base,cyclic)
    with tempfile.TemporaryDirectory(prefix='f1b3-semantic-controls-') as temporary:
        source,archive,members=synthetic_archive(Path(temporary))
        for name,change in (
            ('missing-nonbase-image',lambda m:m.pop('textures/3.png')),
            ('orphan-nonbase-image',lambda m:m.__setitem__('textures/0.png',texture.encode_png())),
            ('changed-nonbase-image-bytes',lambda m:m.__setitem__('textures/2.png',texture.encode_png(pixels=IMAGE_PIXELS[2],comment='same-pixels-different-bytes'))),
        ):
            changed=copy.deepcopy(members);change(changed);geometry.write_control_archive(archive,changed)
            rejected(name,lambda:inspect(source,archive,2))
        for name,change in (
            ('leaf-wrong-slot',lambda d,b:d['materials'][0]['emissiveTexture'].__setitem__('index',4)),
            ('leaf-wrong-uv-selection',lambda d,b:d['materials'][0]['normalTexture'].__setitem__('texCoord',1)),
            ('leaf-omitted-normal-slot',lambda d,b:d['materials'][0].pop('normalTexture')),
        ):
            changed=copy.deepcopy(members);changed['t/0.glb']=texture.mutated_source(changed['t/0.glb'],change)
            geometry.write_control_archive(archive,changed);rejected(name,lambda:inspect(source,archive,2))
    return {'literal_UV1_RGBA_normalized_RGB_reflected_tangent': 'passed','variants':values,'sensitive_semantic_controls':controls,
            'scope':'bounded conformal authored tangents; exact core-PBR closure and corner semantics; no universal renderer claim'}


def run_binary(binary):
    binary=Path(binary).resolve(); cases=[]; refusals=[]
    with tempfile.TemporaryDirectory(prefix='f1b3-candidate-') as temporary:
        root=Path(temporary)
        for external, variants, limits in ((False,VARIANTS,(1,3,1000)),(True,EXTERNAL_VARIANTS,(1,1000))):
            for variant in variants:
                source=write_fixture(root/'sources'/('external-' if external else 'embedded-')/variant,variant,external=external)
                for limit in limits:
                    output=root/'outputs'/str(external)/(variant+'-'+str(limit)+'.3tz')
                    completed=subprocess.run([str(binary),'--json','mesh-local-to-3tz','-i',str(source),'-o',str(output),'--leaf-triangles',str(limit)],text=True,capture_output=True,timeout=60)
                    require(completed.returncode==0,'candidate '+variant+': '+completed.stdout+completed.stderr)
                    evidence=inspect(source,output,limit); summary=json.loads(completed.stdout)
                    require(summary.get('ok') is True and summary['meshReport']==evidence['report'],'CLI/core-PBR report parity')
                    cases.append({'variant':variant,'external':external,'leaf_limit':limit,'status':'passed',**evidence})
        for name,bundle,kind in refusal_bundles():
            source=binding.write_bundle(root/'refusal-sources'/name,bundle)
            for limit in (1,1000):
                output=root/'absent-output'/name/(str(limit)+'.3tz')
                completed=subprocess.run([str(binary),'--json','mesh-local-to-3tz','-i',str(source),'-o',str(output),'--leaf-triangles',str(limit)],text=True,capture_output=True,timeout=60)
                summary=json.loads(completed.stdout)
                require(completed.returncode=={'unsupported':2,'invalid_input':3,'io':1}[kind] and summary.get('ok') is False and summary['error']['kind']==kind,'refusal '+name+': '+completed.stdout+completed.stderr)
                require(not output.exists() and not output.parent.exists(),'refusal created workspace '+name)
                refusals.append({'case':name,'kind':kind,'leaf_limit':limit,'output_parent_exists':False})
    return {'binary':str(binary),'binary_sha256':texture.digest(binary.read_bytes()),'positive_cases':cases,'refusal_cases':refusals}


def write_static(root):
    root=Path(root); records=[]
    for variant in VARIANTS:
        source=write_fixture(root/variant,variant,external=variant in EXTERNAL_VARIANTS)
        model=binding.bind_source(source,scene_reader=scene_triangles)
        assets=[{'path':str(path.relative_to(root)),'bytes':path.stat().st_size,'sha256':texture.digest(path.read_bytes())} for path in sorted(source.parent.rglob('*')) if path.is_file()]
        records.append({'variant':variant,'source':str(source.relative_to(root)),'source_sha256':texture.digest(source.read_bytes()),'selected_image_ids':sorted(model['used_images']),'external_files':model['external_files'],'external_bytes':model['external_bytes'],'assets':assets})
    (root/'manifest.json').write_text(json.dumps({'author':'Independent Python struct/JSON/PNG literals, no production helpers','profile':PROFILE,'fixtures':records},indent=2)+'\n')
    return records


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--self-test',action='store_true'); parser.add_argument('--binary',type=Path)
    parser.add_argument('--fixture',type=Path); parser.add_argument('--variant',choices=VARIANTS,default='all-slots')
    parser.add_argument('--external',action='store_true'); parser.add_argument('--static-fixtures',type=Path)
    parser.add_argument('--source',type=Path); parser.add_argument('--archive',type=Path); parser.add_argument('--leaf-limit',type=int)
    parser.add_argument('--json-output',type=Path); args=parser.parse_args(); result={}
    if args.self_test or args.binary: result['self_test']=self_test()
    if args.fixture: result['fixture']=str(write_fixture(args.fixture,args.variant,external=args.external))
    if args.static_fixtures: result['static_fixtures']=write_static(args.static_fixtures)
    if args.binary: result['candidate']=run_binary(args.binary)
    if args.archive:
        require(args.source and args.leaf_limit,'source and limit required'); result['inspection']=inspect(args.source,args.archive,args.leaf_limit)
    encoded=json.dumps(result,indent=2,allow_nan=False)+'\n'
    if args.json_output: args.json_output.parent.mkdir(parents=True,exist_ok=True); args.json_output.write_text(encoded)
    else: print(encoded,end='')


if __name__=='__main__': main()
