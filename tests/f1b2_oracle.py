#!/usr/bin/env python3
"""Independent local glTF dependency binding plus F1b geometry/resource oracle.

URI/file/buffer interpretation is implemented here without production helpers.
The established independent F1b oracle supplies PNG/material/math/index checks.
"""
import argparse
import base64
import copy
import json
import os
from pathlib import Path
import re
import stat
import struct
import subprocess
import tempfile

import f1b_oracle as texture

require = texture.require
OracleError = texture.OracleError
MIB = 1024 * 1024
PROFILE = 'f1d2-adaptive-root-proxy-gltf-v1'


class SourceError(OracleError):
    def __init__(self, kind, message):
        super().__init__(message)
        self.kind = kind


def reject(kind, message):
    raise SourceError(kind, message)


def relative_uri(uri):
    """Independent per-segment once-decoding; never form-URL '+' decoding."""
    if not isinstance(uri, str) or not uri:
        reject('invalid_input', 'URI must be a nonempty string')
    if len(uri.encode('utf-8')) > 4096 or len(uri.split('/')) > 128:
        reject('unsupported', 'finite URI ceiling')
    if uri.startswith('/') or any(character in uri for character in ':?#'):
        reject('unsupported', 'optional URI components outside relative file profile')
    if uri.endswith('/'):
        reject('invalid_input', 'URI trailing slash')
    parts = []
    allowed_ascii = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~'
    for raw in uri.split('/'):
        decoded = bytearray()
        i = 0
        while i < len(raw):
            character = raw[i]
            if character == '%':
                if i + 2 >= len(raw) or re.fullmatch('[0-9A-Fa-f]{2}', raw[i + 1:i + 3]) is None:
                    reject('invalid_input', 'malformed percent octet')
                octet = int(raw[i + 1:i + 3], 16)
                if octet in (47, 92):
                    reject('unsupported', 'encoded separator outside profile')
                decoded.append(octet)
                i += 3
            else:
                if ord(character) < 128 and character not in allowed_ascii:
                    reject('invalid_input', 'invalid literal URI character')
                decoded.extend(character.encode('utf-8'))
                i += 1
        try:
            part = decoded.decode('utf-8', 'strict')
        except UnicodeDecodeError:
            reject('invalid_input', 'decoded path is not UTF-8')
        if any(ord(character) < 32 or 127 <= ord(character) <= 159 for character in part):
            reject('invalid_input', 'decoded control octet')
        if part in ('.', '..') and raw == uri.split('/')[-1]:
            reject('invalid_input', 'terminal dot names a directory')
        if part in ('', '.'):
            continue
        if part == '..':
            if not parts:
                reject('unsupported', 'path escapes base')
            parts.pop()
            continue
        if any(character in part for character in '<>:"|?*') or part.endswith(('.', ' ')):
            reject('unsupported', 'filename outside portable subset')
        parts.append(part)
    if not parts:
        reject('invalid_input', 'no normalized resource filename')
    return tuple(parts)


def encode_container(document, binary=None):
    raw = json.dumps(document, ensure_ascii=False, separators=(',', ':'), allow_nan=False).encode()
    padded = raw + b' ' * (-len(raw) % 4)
    chunks = struct.pack('<II', len(padded), 0x4e4f534a) + padded
    if binary is not None:
        binary += b'\0' * (-len(binary) % 4)
        chunks += struct.pack('<II', len(binary), 0x004e4942) + binary
    return struct.pack('<4sII', b'glTF', 2, 12 + len(chunks)) + chunks


def parse_document(data):
    if data.startswith(b'glTF'):
        require(len(data) >= 20 and struct.unpack_from('<4sII', data) == (b'glTF', 2, len(data)), 'independent GLB framing')
        chunks = []
        at = 12
        while at < len(data):
            require(at + 8 <= len(data), 'GLB chunk header')
            length, kind = struct.unpack_from('<II', data, at)
            at += 8
            require(length % 4 == 0 and at + length <= len(data), 'GLB actual chunk range')
            chunks.append((kind, data[at:at + length]))
            at += length
        require(len(chunks) in (1, 2) and chunks[0][0] == 0x4e4f534a and (len(chunks) == 1 or chunks[1][0] == 0x004e4942), 'bounded GLB chunk profile')
        return json.loads(chunks[0][1]), chunks[1][1] if len(chunks) == 2 else None, True
    return json.loads(data), None, False


def dependency_requests(doc):
    """Validate all declaration shapes and URIs before any dependency I/O."""
    requests = []
    buffers = doc.get('buffers', [])
    images = doc.get('images', [])
    if len(buffers) > 32 or len(images) > 32:
        reject('unsupported', 'logical resource count ceiling')
    if sum(b['byteLength'] for b in buffers) > 32 * MIB:
        reject('unsupported', 'declared buffer sum ceiling')
    for key, values in (('buffers', buffers), ('images', images)):
        for index, item in enumerate(values):
            if key == 'images' and (('uri' in item) == ('bufferView' in item)):
                reject('invalid_input', 'image requires exactly one source')
            if 'uri' in item:
                requests.append((key, index, relative_uri(item['uri'])))
    if len(requests) > 64:
        reject('unsupported', 'dependency reference ceiling')
    return requests


def read_stable(path, maximum, *, dependency=False):
    try:
        before = path.lstat()
        if stat.S_ISLNK(before.st_mode):
            reject('unsupported' if dependency else 'invalid_input', 'source leaf symlink')
        if not stat.S_ISREG(before.st_mode):
            reject('invalid_input', 'nonregular source')
        if before.st_size > maximum:
            reject('unsupported', 'source file ceiling')
        with path.open('rb') as file:
            opened = os.fstat(file.fileno())
            if (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns) != (opened.st_dev, opened.st_ino, opened.st_size, opened.st_mtime_ns):
                reject('invalid_input', 'changed before read')
            data = file.read(maximum + 1)
            after = os.fstat(file.fileno())
        if len(data) != before.st_size or (opened.st_dev, opened.st_ino, opened.st_size, opened.st_mtime_ns) != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns):
            reject('invalid_input', 'changed bounded source')
    except OSError as error:
        raise SourceError('io', 'source I/O: ' + str(path)) from error
    return data, (before.st_dev, before.st_ino)


def bind_source(source, *, scene_reader=None):
    """Own actual dependency bytes, then independently rebase a virtual GLB.

    Rebasing is solely an oracle bridge to the independently authored geometry
    reader. Actual ranges are checked against each original logical buffer first.
    """
    source = Path(source)
    root_data, root_identity = read_stable(source, 32 * MIB)
    doc, embedded, is_glb = parse_document(root_data)
    if not is_glb and len(root_data) > MIB:
        reject('unsupported', 'root JSON ceiling')
    base = source.parent.resolve()
    requests = dependency_requests(doc)
    physical = {}
    by_reference = {}
    all_bytes = len(root_data)
    for key, index, parts in requests:
        path = base
        for part in parts:
            path /= part
            try:
                entry = path.lstat()
            except OSError as error:
                raise SourceError('io', 'dependency lookup') from error
            if stat.S_ISLNK(entry.st_mode):
                reject('unsupported', 'dependency component symlink')
        data, identity = read_stable(path, 32 * MIB, dependency=True)
        if identity == root_identity:
            reject('unsupported', 'dependency aliases source document')
        if identity not in physical:
            physical[identity] = {'data': data, 'path': path, 'sha256': texture.digest(data)}
            all_bytes += len(data)
            if all_bytes > 64 * MIB:
                reject('unsupported', 'unique physical byte ceiling')
        else:
            require(data == physical[identity]['data'], 'independent stable alias bytes')
        by_reference[(key, index)] = physical[identity]['data']
    buffers = []
    for index, item in enumerate(doc.get('buffers', [])):
        declared = item['byteLength']
        require(type(declared) is int and declared > 0, 'buffer declared length')
        if 'uri' in item:
            data = by_reference[('buffers', index)]
            require(len(data) >= declared, 'external buffer actual length')
        else:
            require(is_glb and index == 0 and embedded is not None, 'URI-less buffer BIN binding')
            require(declared <= len(embedded) <= declared + 3 and not any(embedded[declared:]), 'GLB buffer padding')
            data = embedded
        buffers.append(data[:declared])
    virtual = copy.deepcopy(doc)
    rebased = bytearray()
    bases = []
    for buffer in buffers:
        rebased.extend(b'\0' * (-len(rebased) % 4))
        bases.append(len(rebased))
        rebased.extend(buffer)
    for view in virtual.get('bufferViews', []):
        index = view['buffer']
        start, length = view.get('byteOffset', 0), view['byteLength']
        require(type(index) is int and 0 <= index < len(buffers) and type(start) is int and type(length) is int and start >= 0 and length > 0 and start + length <= len(buffers[index]), 'actual original bufferView binding/range')
        view['buffer'] = 0
        view['byteOffset'] = bases[index] + start
    for index, image in enumerate(virtual.get('images', [])):
        if 'uri' in image:
            encoded = by_reference[('images', index)]
            detected = 'image/png' if encoded.startswith(b'\x89PNG\r\n\x1a\n') else 'image/jpeg' if encoded.startswith(b'\xff\xd8\xff') else None
            require(detected is not None and image.get('mimeType', detected) == detected, 'external MIME/signature')
            rebased.extend(b'\0' * (-len(rebased) % 4))
            view = len(virtual.setdefault('bufferViews', []))
            virtual['bufferViews'].append({'buffer': 0, 'byteOffset': len(rebased), 'byteLength': len(encoded)})
            rebased.extend(encoded)
            image.pop('uri')
            image.update(bufferView=view, mimeType=detected)
    virtual['buffers'] = [{'byteLength': len(rebased)}]
    data = texture.encode_glb(virtual, bytes(rebased))
    reader = texture.scene_triangles if scene_reader is None else scene_reader
    triangles, used_images, images, raw_images = reader(data)
    return {'triangles': triangles, 'used_images': used_images, 'images': images, 'raw_images': raw_images,
            'external_files': len(physical), 'external_bytes': sum(len(item['data']) for item in physical.values()),
            'physical': sorted((str(item['path'].relative_to(base)), len(item['data']), item['sha256']) for item in physical.values()),
            'root_bytes': len(root_data), 'root_sha256': texture.digest(root_data), 'virtual_glb': data}


def source_bundle(variant='multibuffer', triangles=8):
    """Raw independently authored JSON, buffer files and image files."""
    authored = texture.fixture(triangles, 'uv-u8' if variant == 'uv-u8-bounds' else 'uv-u16' if variant == 'uv-u16-bounds' else 'jpeg' if variant == 'jpeg' else 'two-images' if variant == 'distinct-basename' else 'standard')
    doc, original = texture.geometry.decode_glb(authored)
    if variant in ('uv-u8-bounds', 'uv-u16-bounds'):
        maximum = 255 if variant == 'uv-u8-bounds' else 65535
        for primitive in doc['meshes'][0]['primitives']:
            item = doc['accessors'][primitive['attributes']['TEXCOORD_0']]
            item.update(min=[1, 0], max=[maximum, maximum - 1])
    raw_images = []
    for image in doc['images']:
        view = doc['bufferViews'][image['bufferView']]
        start = view.get('byteOffset', 0)
        raw_images.append(original[start:start + view['byteLength']])
    image_views = {image['bufferView'] for image in doc['images']}
    mapping = {}
    views = []
    payloads = [bytearray(), bytearray()]
    for old_index, view in enumerate(doc['bufferViews']):
        if old_index in image_views:
            continue
        target = old_index % 2
        payloads[target].extend(b'\0' * (-len(payloads[target]) % 4))
        copied = copy.deepcopy(view)
        copied.update(buffer=target, byteOffset=len(payloads[target]))
        start = view.get('byteOffset', 0)
        payloads[target].extend(original[start:start + view['byteLength']])
        mapping[old_index] = len(views)
        views.append(copied)
    for item in doc['accessors']:
        item['bufferView'] = mapping[item['bufferView']]
    doc['bufferViews'] = views
    files = {'geom/a.bin': bytes(payloads[0]), 'geom/b.bin': bytes(payloads[1]),
             'images/color.png': raw_images[0], 'images/unused.png': raw_images[1]}
    doc['buffers'] = [{'byteLength': len(payloads[0]), 'uri': 'geom/a.bin'}, {'byteLength': len(payloads[1]), 'uri': 'geom/b.bin'}]
    doc['images'] = [{'uri': 'images/color.png', 'mimeType': 'image/jpeg' if variant == 'jpeg' else 'image/png'}, {'uri': 'images/unused.png'}]
    binary = None
    container = 'gltf'
    hardlinks = {}
    if variant in ('glb-mixed', 'glb-json-only'):
        container = 'glb'
        if variant == 'glb-mixed':
            binary = files.pop('geom/a.bin')
            doc['buffers'][0].pop('uri')
    if variant == 'external-view-image':
        blob = raw_images[0]
        doc['buffers'].append({'byteLength': len(blob), 'uri': 'images/view.bin'})
        files['images/view.bin'] = blob
        view = len(doc['bufferViews'])
        doc['bufferViews'].append({'buffer': 2, 'byteOffset': 0, 'byteLength': len(blob)})
        doc['images'][0] = {'bufferView': view, 'mimeType': 'image/png'}
        files.pop('images/color.png')
    if variant == 'mime-inferred':
        doc['images'][0].pop('mimeType')
    if variant == 'trailing-buffer':
        files['geom/a.bin'] += b'opaque trailing bytes beyond declared prefix'
    if variant in ('unicode', 'encoded-unicode', 'space-percent', 'encoded-hash', 'once-percent', 'plus'):
        path, uri = {
            'unicode': ('images/sphère.png', 'images/sphère.png'),
            'encoded-unicode': ('images/sphère.png', 'images/sph%C3%A8re.png'),
            'space-percent': ('images/a 25%.png', 'images/a%2025%25.png'),
            'encoded-hash': ('images/color#alpha.png', 'images/color%23alpha.png'),
            'once-percent': ('images/%2e%2e.png', 'images/%252e%252e.png'),
            'plus': ('images/a+b.png', 'images/a%2bb.png'),
        }[variant]
        files[path] = files.pop('images/color.png')
        doc['images'][0]['uri'] = uri
    if variant == 'contained-dots':
        doc['buffers'][0]['uri'] = './geom/../geom/a.bin'
        doc['images'][0]['uri'] = 'images/%2e/../images/color.png'
    if variant in ('percent-alias', 'repeated-uri', 'hardlink-alias'):
        doc['textures'][1]['source'] = 1
        files.pop('images/unused.png')
        uri = 'images/%63olor.png' if variant == 'percent-alias' else 'images/alias.png' if variant == 'hardlink-alias' else 'images/color.png'
        doc['images'][1] = {'uri': uri}
        if variant == 'hardlink-alias':
            hardlinks['images/alias.png'] = 'images/color.png'
    if variant == 'buffer-alias':
        files.pop('geom/a.bin')
        files.pop('geom/b.bin')
        files['geom/shared.bin'] = original
        doc['buffers'] = [{'byteLength': len(original), 'uri': 'geom/shared.bin'}, {'byteLength': len(original), 'uri': 'geom/%73hared.bin'}]
        doc['bufferViews'] = []
        for old_index, view in enumerate(texture.geometry.decode_glb(authored)[0]['bufferViews']):
            if old_index in image_views:
                continue
            value = copy.deepcopy(view)
            value['buffer'] = old_index % 2
            doc['bufferViews'].append(value)
    if variant == 'distinct-basename':
        files['left/color.png'] = files.pop('images/color.png')
        files['right/color.png'] = texture.encode_png(pixels=texture.PIXELS[3:] + texture.PIXELS[:3])
        files.pop('images/unused.png')
        doc['images'] = [{'uri': 'left/color.png'}, {'uri': 'right/color.png'}]
    if variant == 'unused-buffer':
        files['unused/not-rendered.bin'] = b'\x01\x02\x03\x04'
        doc['buffers'].append({'byteLength': 4, 'uri': 'unused/not-rendered.bin'})
    if variant == 'json-whitespace':
        pass
    return {'document': doc, 'binary': binary, 'container': container, 'files': files, 'hardlinks': hardlinks, 'whitespace': b'\n\t ' if variant == 'json-whitespace' else b''}


def write_bundle(root, bundle):
    root = Path(root)
    root.mkdir(parents=True, exist_ok=True)
    for name, data in bundle['files'].items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    for alias, target in bundle.get('hardlinks', {}).items():
        path = root / alias
        path.parent.mkdir(parents=True, exist_ok=True)
        os.link(root / target, path)
    extension = '.glb' if bundle['container'] == 'glb' else '.gltf'
    source = root / ('source' + extension)
    data = encode_container(bundle['document'], bundle['binary']) if extension == '.glb' else json.dumps(bundle['document'], ensure_ascii=False, separators=(',', ':'), allow_nan=False).encode() + bundle.get('whitespace', b'')
    source.write_bytes(data)
    return source


def write_fixture(root, variant='multibuffer', triangles=8):
    return write_bundle(root, source_bundle(variant, triangles))


def fixture(root, variant='multibuffer', triangles=8):
    return write_fixture(root, variant, triangles)


def inspect(source, archive, leaf_limit):
    model = bind_source(source)
    inspected = texture.inspect(source, archive, leaf_limit, source_model=model)
    inspected.update(external_files=model['external_files'], external_bytes=model['external_bytes'], source_dependencies=model['physical'])
    return inspected


VARIANTS = ('multibuffer', 'glb-mixed', 'glb-json-only', 'external-view-image', 'mime-inferred', 'jpeg',
            'trailing-buffer', 'unicode', 'encoded-unicode', 'space-percent', 'encoded-hash', 'once-percent', 'plus',
            'contained-dots', 'percent-alias', 'repeated-uri', 'hardlink-alias', 'buffer-alias',
            'distinct-basename', 'unused-buffer', 'json-whitespace', 'uv-u8-bounds', 'uv-u16-bounds')


def refusal_bundles():
    cases = []
    bad_uris = (
        ('empty', '', 'invalid_input'), ('percent-short', 'images/a%0', 'invalid_input'),
        ('percent-nonhex', 'images/a%GG.png', 'invalid_input'), ('raw-space', 'images/a b.png', 'invalid_input'),
        ('raw-backslash', 'images\\color.png', 'invalid_input'), ('raw-control', 'images/a\t.png', 'invalid_input'),
        ('raw-C1-control', 'images/a\u0085.png', 'invalid_input'),
        ('encoded-C1-control', 'images/a%C2%85.png', 'invalid_input'),
        ('raw-plus', 'images/a+b.png', 'invalid_input'), ('raw-ampersand', 'images/a&b.png', 'invalid_input'),
        ('raw-semicolon', 'images/a;b.png', 'invalid_input'), ('raw-at', 'images/a@b.png', 'invalid_input'),
        ('terminal-decoded-dot', 'images/%2e', 'invalid_input'), ('terminal-decoded-dotdot', 'images/%2e%2e', 'invalid_input'),
        ('decoded-nul', 'images/a%00.png', 'invalid_input'), ('decoded-invalid-utf8', 'images/a%ff.png', 'invalid_input'),
        ('trailing-slash', 'images/', 'invalid_input'), ('query', 'images/color.png?x', 'unsupported'),
        ('fragment', 'images/color.png#x', 'unsupported'), ('absolute', '/images/color.png', 'unsupported'),
        ('authority', '//server/color.png', 'unsupported'), ('network', 'https://example.invalid/a.png', 'unsupported'),
        ('file-scheme', 'file:images/color.png', 'unsupported'), ('data', 'data:image/png;base64,AAAA', 'unsupported'),
        ('escape', '../outside.png', 'unsupported'), ('encoded-escape', '%2e%2e/outside.png', 'unsupported'),
        ('encoded-slash', 'images%2fcolor.png', 'unsupported'), ('encoded-backslash', 'images%5ccolor.png', 'unsupported'),
        ('encoded-query-character', 'images/a%3f.png', 'unsupported'), ('encoded-colon', 'images/a%3a.png', 'unsupported'),
        ('trailing-dot', 'images/color.', 'unsupported'), ('trailing-space', 'images/color%20', 'unsupported'),
        ('uri-byte-limit', 'a' * 4097, 'unsupported'), ('uri-component-limit', '/'.join(['a'] * 129), 'unsupported'),
    )
    for name, uri, kind in bad_uris:
        bundle = source_bundle()
        bundle['document']['images'][0]['uri'] = uri
        cases.append(('uri-' + name, bundle, kind))
    for name, modify, kind in (
        ('missing-image', lambda b: b['files'].pop('images/color.png'), 'io'),
        ('missing-buffer', lambda b: b['files'].pop('geom/a.bin'), 'io'),
        ('missing-unused-image', lambda b: b['files'].pop('images/unused.png'), 'io'),
        ('short-buffer', lambda b: b['files'].__setitem__('geom/a.bin', b'\0'), 'invalid_input'),
        ('view-beyond-declared-prefix', lambda b: b['document']['bufferViews'][0].__setitem__('byteOffset', b['document']['buffers'][0]['byteLength']), 'invalid_input'),
        ('image-two-sources', lambda b: b['document']['images'][0].__setitem__('bufferView', 0), 'invalid_input'),
        ('buffer-one-without-URI', lambda b: b['document']['buffers'][1].pop('uri'), 'unsupported'),
        ('json-uri-less-buffer-zero', lambda b: b['document']['buffers'][0].pop('uri'), 'invalid_input'),
        ('image-no-source', lambda b: b['document']['images'].__setitem__(0, {}), 'invalid_input'),
        ('external-mime-mismatch', lambda b: b['document']['images'][0].__setitem__('mimeType', 'image/jpeg'), 'invalid_input'),
        ('invalid-unused-png', lambda b: b['files'].__setitem__('images/unused.png', b'not-a-png'), 'invalid_input'),
        ('buffer-count', lambda b: b['document'].__setitem__('buffers', [copy.deepcopy(b['document']['buffers'][0]) for _ in range(33)]), 'unsupported'),
        ('declared-buffer-sum', lambda b: b['document']['buffers'][0].__setitem__('byteLength', 32 * MIB), 'unsupported'),
        ('buffer-file-ceiling', lambda b: b['files'].__setitem__('geom/a.bin', b'\0' * (32 * MIB + 1)), 'unsupported'),
        ('unique-external-byte-ceiling', lambda b: b['files'].update({key: b['files'][key] + b'\0' * (32 * MIB - len(b['files'][key])) for key in ('geom/a.bin', 'geom/b.bin')}), 'unsupported'),
        ('root-json-ceiling', lambda b: b.__setitem__('whitespace', b' ' * MIB), 'unsupported'),
    ):
        bundle = source_bundle()
        modify(bundle)
        cases.append((name, bundle, kind))
    bundle = source_bundle('glb-mixed')
    bundle['document']['buffers'][0]['uri'] = 'geom/b.bin'
    cases.append(('BIN-contradictory-buffer-zero-URI', bundle, 'invalid_input'))
    bundle = source_bundle('glb-mixed')
    bundle['binary'] += b'\0' * (32 * MIB)
    cases.append(('root-GLB-byte-ceiling', bundle, 'unsupported'))

    bundle = source_bundle()
    bundle['document']['accessors'][0].update(min=[-1, -1, -1], max=[2, 2, 2])
    cases.append(('used-position-broad-bounds', bundle, 'invalid_input'))
    bundle = source_bundle()
    unused = copy.deepcopy(bundle['document']['accessors'][2])
    unused.update(min=[-99, -99], max=[99, 99])
    bundle['document']['accessors'].append(unused)
    cases.append(('unused-UV-broad-bounds', bundle, 'invalid_input'))
    for name, index in (('used-inverted-bounds-before-open', 0), ('unused-inverted-bounds-before-open', None)):
        bundle = source_bundle()
        if index is None:
            item = copy.deepcopy(bundle['document']['accessors'][2])
            bundle['document']['accessors'].append(item)
            item.update(min=[5, 5], max=[0, 0])
        else:
            bundle['document']['accessors'][index].update(min=[5, 5, 5], max=[0, 0, 0])
        bundle['files'].pop('geom/a.bin')
        cases.append((name, bundle, 'invalid_input'))
    # A valid early missing URI must not outrank later URI/profile admission.
    for name, modify, kind in (
        ('all-uris-before-open', lambda d: d['images'][0].__setitem__('uri', 'https://example.invalid/a.png'), 'unsupported'),
        ('unsupported-extensions-before-open', lambda d: d.__setitem__('extensionsRequired', ['KHR_texture_transform']), 'unsupported'),
        ('unsupported-unused-extras-before-open', lambda d: d['images'][1].__setitem__('extras', {}), 'unsupported'),
        ('unsupported-attribute-before-open', lambda d: d['meshes'][0]['primitives'][0]['attributes'].__setitem__('TEXCOORD_2', 0), 'unsupported'),
    ):
        bundle = source_bundle()
        bundle['files'].pop('geom/a.bin')
        modify(bundle['document'])
        cases.append((name, bundle, kind))
    return cases


def self_test():
    controls = {}
    for uri, expected in (
        ('images/a%20b%25.png', ('images', 'a b%.png')),
        ('images/%252e%252e.png', ('images', '%2e%2e.png')),
        ('images/color%23alpha.png', ('images', 'color#alpha.png')),
        ('images/sph%C3%A8re.png', ('images', 'sphère.png')),
        ('./geom/../geom/a.bin', ('geom', 'a.bin')),
        ('images/a%2bb.png', ('images', 'a+b.png')),
    ):
        require(relative_uri(uri) == expected, 'analytical URI reference interpretation')
    for name, bundle, wanted in refusal_bundles():
        if not name.startswith('uri-'):
            continue
        try:
            relative_uri(bundle['document']['images'][0]['uri'])
        except SourceError as error:
            require(error.kind == wanted, 'URI reference error classification ' + name)
            controls[name] = 'rejected'
        else:
            raise OracleError('reference accepted ' + name)
    variants = {}
    with tempfile.TemporaryDirectory(prefix='f1b2-selftest-') as temporary:
        root = Path(temporary)
        reference = texture.scene_triangles(texture.fixture(8))[0]
        for variant in VARIANTS:
            source = fixture(root / variant, variant)
            bound = bind_source(source)
            require(len(bound['triangles']) == 8, 'fixture source geometry count')
            if variant not in ('jpeg', 'distinct-basename', 'repeated-uri', 'percent-alias', 'hardlink-alias', 'uv-u8-bounds', 'uv-u16-bounds'):
                texture.match_triangles(reference, bound['triangles'])
            if variant in ('percent-alias', 'repeated-uri', 'hardlink-alias'):
                require(bound['external_files'] == 3 and bound['used_images'] == {0, 1}, 'physical aliases versus logical image identity')
            if variant == 'buffer-alias':
                require(bound['external_files'] == 3, 'physical buffer alias uniqueness')
            variants[variant] = {'external_files': bound['external_files'], 'external_bytes': bound['external_bytes']}
        # Sensitive controls are changes to independently interpreted bytes or
        # binding, and are evaluated before any candidate is executed.
        baseline_source = fixture(root / 'baseline')
        baseline = bind_source(baseline_source)
        other_source = fixture(root / 'different-base')
        (other_source.parent / 'images/color.png').write_bytes(texture.encode_png(pixels=texture.PIXELS[3:] + texture.PIXELS[:3]))
        changed = bind_source(other_source)
        try:
            texture.match_triangles(baseline['triangles'], changed['triangles'])
        except OracleError:
            controls['wrong-uri-base-pixels'] = 'rejected'
        else:
            raise OracleError('wrong source base undetected')
        source = fixture(root / 'wrong-buffer')
        document = json.loads(source.read_bytes())
        document['bufferViews'][0]['buffer'] = 1
        source.write_text(json.dumps(document))
        try:
            changed = bind_source(source)
            texture.match_triangles(baseline['triangles'], changed['triangles'])
        except (OracleError, ValueError, struct.error):
            controls['wrong-logical-buffer-binding'] = 'rejected'
        else:
            raise OracleError('wrong logical buffer undetected')
        source = fixture(root / 'same-sized-payload')
        path = source.parent / 'geom/a.bin'
        data = bytearray(path.read_bytes())
        struct.pack_into('<f', data, 0, 500)
        path.write_bytes(data)
        try:
            texture.match_triangles(baseline['triangles'], bind_source(source)['triangles'])
        except OracleError:
            controls['same-sized-buffer-payload-change'] = 'rejected'
        else:
            raise OracleError('changed buffer payload undetected')
    return {'URI_analytic_reference': 'passed', 'fixture_variants': variants, 'sensitive_binding_controls': controls,
            'f1b_texture_self_test': texture.self_test(),
            'scope': 'independent URI/base/physical-file/buffer interpretation and full-detail resource fidelity; no atomic snapshot claim'}


def run_binary(binary):
    binary = Path(binary).resolve()
    positives, refusals = [], []
    with tempfile.TemporaryDirectory(prefix='f1b2-candidate-') as temporary:
        root = Path(temporary)
        for variant in VARIANTS:
            source = fixture(root / 'sources' / variant, variant)
            for limit in (1, 3, 1000):
                output = root / 'outputs' / (variant + '-' + str(limit) + '.3tz')
                completed = subprocess.run([str(binary), '--json', 'mesh-local-to-3tz', '-i', str(source), '-o', str(output), '--leaf-triangles', str(limit)], text=True, capture_output=True, timeout=60)
                require(completed.returncode == 0, 'candidate ' + variant + ': ' + completed.stdout + completed.stderr)
                summary = json.loads(completed.stdout)
                evidence = inspect(source, output, limit)
                require(summary.get('ok') is True and summary['meshReport'] == evidence['report'], 'CLI/resource/report parity')
                positives.append({'variant': variant, 'leaf_limit': limit, 'status': 'passed', **evidence})
        for name, bundle, kind in refusal_bundles():
            source = write_bundle(root / 'refused-sources' / name, bundle)
            for limit in (1, 1000):
                output = root / 'absent-output' / name / (str(limit) + '.3tz')
                completed = subprocess.run([str(binary), '--json', 'mesh-local-to-3tz', '-i', str(source), '-o', str(output), '--leaf-triangles', str(limit)], text=True, capture_output=True, timeout=60)
                summary = json.loads(completed.stdout)
                require(completed.returncode == {'unsupported': 2, 'invalid_input': 3, 'io': 1}[kind] and summary.get('ok') is False and summary['error']['kind'] == kind, 'source admission precedence/category ' + name + ': ' + completed.stdout + completed.stderr)
                require(not output.exists() and not output.parent.exists(), 'preparation created output work ' + name)
                refusals.append({'case': name, 'leaf_limit': limit, 'kind': kind, 'output_parent_exists': False, 'source_sha256': texture.digest(source.read_bytes())})
        # Filesystem semantics have independent setups; no URI helper creates links.
        for name, setup, kind in (
            ('image-leaf-symlink', 'image-link', 'unsupported'),
            ('image-directory-symlink', 'directory-link', 'unsupported'),
            ('directory-image', 'directory', 'invalid_input'),
            ('document-alias', 'document-hardlink', 'unsupported'),
        ):
            source = fixture(root / 'filesystem' / name)
            image = source.parent / 'images/color.png'
            if setup == 'image-link':
                image.unlink()
                image.symlink_to('unused.png')
            elif setup == 'directory-link':
                moved = source.parent / 'actual-images'
                (source.parent / 'images').rename(moved)
                (source.parent / 'images').symlink_to('actual-images', target_is_directory=True)
            elif setup == 'directory':
                image.unlink()
                image.mkdir()
            else:
                image.unlink()
                os.link(source, image)
            for limit in (1, 1000):
                output = root / 'absent-output' / name / (str(limit) + '.3tz')
                completed = subprocess.run([str(binary), '--json', 'mesh-local-to-3tz', '-i', str(source), '-o', str(output), '--leaf-triangles', str(limit)], text=True, capture_output=True, timeout=60)
                summary = json.loads(completed.stdout)
                require(completed.returncode == {'unsupported': 2, 'invalid_input': 3, 'io': 1}[kind] and summary.get('ok') is False and summary['error']['kind'] == kind and not output.parent.exists(), 'filesystem refusal ' + name + ': ' + completed.stdout)
                refusals.append({'case': name, 'leaf_limit': limit, 'kind': kind, 'output_parent_exists': False})

        for target_name, target in (('document', None), ('buffer', 'geom/a.bin'), ('image', 'images/color.png'), ('unused-image', 'images/unused.png')):
            source = fixture(root / 'output-aliases' / target_name)
            target_path = source if target is None else source.parent / target
            output = source.parent / 'alias.3tz'
            os.link(target_path, output)
            preserved = target_path.read_bytes()
            for limit in (1, 1000):
                completed = subprocess.run([str(binary), '--json', 'mesh-local-to-3tz', '-i', str(source), '-o', str(output), '--leaf-triangles', str(limit), '--force'], text=True, capture_output=True, timeout=60)
                summary = json.loads(completed.stdout)
                require(completed.returncode == 2 and summary.get('ok') is False and summary['error']['kind'] == 'invalid_request', 'output source hardlink alias ' + target_name + ': ' + completed.stdout)
                require(output.read_bytes() == preserved and target_path.read_bytes() == preserved, 'output alias refusal preserved source bytes')
                require(not list(source.parent.glob('.mesh-work-*')), 'output alias created workspace')
                refusals.append({'case': 'output-alias-' + target_name, 'leaf_limit': limit, 'kind': 'invalid_request', 'preserved_sha256': texture.digest(preserved)})
    return {'binary': str(binary), 'binary_sha256': texture.digest(binary.read_bytes()), 'positive_cases': positives, 'refusal_cases': refusals}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--self-test', action='store_true')
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--fixture', type=Path, help='write an independent source directory')
    parser.add_argument('--variant', choices=VARIANTS, default='multibuffer')
    parser.add_argument('--json-output', type=Path)
    args = parser.parse_args()
    result = {}
    if args.self_test or args.binary:
        result['self_test'] = self_test()
    if args.binary:
        result['candidate'] = run_binary(args.binary)
    if args.fixture:
        source = fixture(args.fixture, args.variant)
        model = bind_source(source)
        result['fixture'] = {'source': str(source), 'source_sha256': model['root_sha256'], 'external_files': model['external_files'], 'external_bytes': model['external_bytes']}
    encoded = json.dumps(result, indent=2, allow_nan=False) + '\n'
    if args.json_output:
        args.json_output.parent.mkdir(parents=True, exist_ok=True)
        args.json_output.write_text(encoded)
    else:
        print(encoded, end='')


if __name__ == '__main__':
    main()
