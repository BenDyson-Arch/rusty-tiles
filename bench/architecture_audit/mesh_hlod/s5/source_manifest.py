#!/usr/bin/env python3
"""Independent S5 source manifests (analytic JSON or disk-backed SQLite).

``with load_manifest(path) as m:`` exposes ``m['metadata']`` and
``m['triangles']`` as mappings. Triangle iteration preserves source traversal
order. ``m.source_rank(key)`` is its zero-based rank; ``m.tie_order(key)`` is
(expanded primitive traversal ordinal, triangle ordinal, source rank), never
lexicographic identity order. Keys are exact decimal ``node,mesh,primitive,tri``.
Legacy analytic JSON retains insertion order when no explicit rank is supplied.
SQLite keeps triangle rows on disk; JSON is intended for small analytic inputs.

The generator below owns its GLB geometry reader, with no producer imports.
Usage: source_manifest.py SOURCE.glb EXPECTED.sqlite [--transform16 M ... M].
An explicit baseline directory/.3tz/.zip/tileset.json is also a SOURCE. Root
frame is the default; --content-frame names it explicitly. --coordinate-frame
world applies the mandatory glTF Y-up conversion before the archive placement.
For direct GLB world output the caller supplies that entire placement matrix.

Supported source geometry: embedded, uncompressed glTF 2.0; selected-scene
single-parent traversal (depth <128); affine matrix/TRS; indexed/nonindexed
TRIANGLES/STRIP/FAN including degenerates; scalar/vector signed/unsigned/f32
accessors with stride, normalization and sparse overrides; NORMAL, TANGENT,
UV0/UV1 and COLOR0. uvs/texcoords are aliases for the UV0 three-corner list;
texcoord_sets retains both sets under string keys. Materials are original JSON
objects, including authored defaults/texture indices: this is not texture-pixel
correspondence, canonical material equivalence or an appearance acceptance test.
Animation, skins, morphs, source compression and unknown semantic extensions
refuse. Baseline leaves additionally admit the adjacent independent meshopt
decoder and require exact published identity; implicit/external hierarchies
need an independent expansion and are refused. No producer facts are read.

Limits: 16 MiB GLB/tileset JSON; 64 MiB baseline member and cached decoded views;
250,000 triangles per baseline leaf; 256 MiB identity-decoder JSON and 512 MiB
Node heap; hierarchy <=1,000,000 visits; JSON output <=100,000 triangles. Source
GLB geometry is mmap-backed and triangle storage/sorting is SQLite-backed, with
an 8 MiB page cache and disk temporary sort. There is no archive-wide Python
triangle map for SQLite output, but JSON metadata, one baseline leaf and its
identity decoder remain resident; no measured total-RSS/scale claim is made.
Source inputs must remain unchanged during generation. Placement after capture
is f64; explicit placement never rounds ECEF coordinates to f32. Failures leave
the destination untouched; success replaces it atomically from the same volume.
"""
from collections.abc import Mapping
import argparse
from contextlib import contextmanager
import hashlib
import json
import math
import mmap
import os
from pathlib import Path
from pathlib import PurePosixPath
import sqlite3
import struct
import subprocess
import tempfile
import zipfile


MAX_JSON_BYTES = 16 * 1024 * 1024
MAX_BASELINE_BYTES = 64 * 1024 * 1024
MAX_BASELINE_TRIANGLES = 250_000
MAX_JSON_TRIANGLES = 100_000
IDENTITY = (1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.)
# glTF Y-up -> 3D Tiles Z-up. Column-major, as are --transform16 and GLB matrices.
Y_UP = (1., 0., 0., 0., 0., 0., 1., 0., 0., -1., 0., 0., 0., 0., 0., 1.)
Y_UP_INVERSE = (1., 0., 0., 0., 0., 0., -1., 0., 0., 1., 0., 0., 0., 0., 0., 1.)


class ManifestError(ValueError):
    """Malformed, unsupported or resource-limited independent source input."""


def _identity(key):
    try:
        values = tuple(int(v) for v in key.split(','))
    except (AttributeError, ValueError) as exc:
        raise ManifestError(f'invalid source key: {key!r}') from exc
    if len(values) != 4 or any(v < 0 or v >= 2**64 for v in values):
        raise ManifestError(f'invalid source key: {key!r}')
    if ','.join(map(str, values)) != key:
        raise ManifestError(f'noncanonical source key: {key!r}')
    return values


class _JSONTriangles(Mapping):
    def __init__(self, rows):
        self.rows = rows
        self.ranks = {key: row.get('source_rank', rank)
                      for rank, (key, row) in enumerate(rows.items())}

    def __getitem__(self, key):
        return self.rows[key]

    def __iter__(self):
        return iter(self.rows)

    def __len__(self):
        return len(self.rows)

    def source_rank(self, key):
        return self.ranks[key]


class _SQLiteTriangles(Mapping):
    def __init__(self, connection):
        self.connection = connection
        self.count = connection.execute('SELECT count(*) FROM triangles').fetchone()[0]

    def __getitem__(self, key):
        row = self.connection.execute(
            'SELECT t.data,p.data,t.rank FROM triangles t JOIN primitives p '
            'ON p.key=t.primitive WHERE t.key=?', (key,)).fetchone()
        if row is None:
            raise KeyError(key)
        return dict(json.loads(row[0]), **json.loads(row[1]), source_rank=row[2])

    def __iter__(self):
        return (row[0] for row in self.connection.execute(
            'SELECT key FROM triangles ORDER BY source_order'))

    def items(self):
        for key, data, primitive, rank in self.connection.execute(
                'SELECT t.key,t.data,p.data,t.rank FROM triangles t '
                'JOIN primitives p ON p.key=t.primitive ORDER BY t.source_order'):
            yield key, dict(json.loads(data), **json.loads(primitive), source_rank=rank)

    def __len__(self):
        return self.count

    def source_rank(self, key):
        row = self.connection.execute('SELECT rank FROM triangles WHERE key=?', (key,)).fetchone()
        if row is None:
            raise KeyError(key)
        return row[0]


class Manifest(Mapping):
    def __init__(self, metadata, triangles, connection=None):
        self.metadata, self.triangles = metadata, triangles
        self.connection = connection

    def __getitem__(self, key):
        if key == 'metadata':
            return self.metadata
        if key == 'triangles':
            return self.triangles
        raise KeyError(key)

    def __iter__(self):
        return iter(('metadata', 'triangles'))

    def __len__(self):
        return 2

    def source_rank(self, key):
        return self.triangles.source_rank(key)

    def tie_order(self, key):
        row = self.triangles[key]
        return row.get('expanded_ordinal', self.source_rank(key)), _identity(key)[3], self.source_rank(key)

    def close(self):
        if self.connection is not None:
            self.connection.close()
            self.connection = None

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()


def load_manifest(path):
    """Open read-only; close the returned context manager after consuming rows.

    Triangle values use xyz/material/vertices/expanded_ordinal/material_index;
    optional normals, texcoords (also uvs), tangents and colors follow corner
    order. JSON without metadata is the existing analytic interchange.
    """
    path = Path(path)
    with path.open('rb') as stream:
        sqlite = stream.read(16) == b'SQLite format 3\0'
    if sqlite:
        connection = sqlite3.connect(path.resolve().as_uri() + '?mode=ro', uri=True)
        try:
            metadata = json.loads(connection.execute(
                "SELECT value FROM metadata WHERE key='manifest'").fetchone()[0])
            if metadata.get('format') != 's5-source-manifest-v1':
                raise ManifestError('unknown SQLite source manifest format')
            return Manifest(metadata, _SQLiteTriangles(connection), connection)
        except Exception:
            connection.close()
            raise
    doc = json.loads(path.read_text())
    if not isinstance(doc.get('triangles'), dict):
        raise ManifestError('manifest requires a triangles mapping')
    return Manifest(doc.get('metadata', {}), _JSONTriangles(doc['triangles']))


def _require(ok, message):
    if not ok:
        raise ManifestError(message)


def _uint(value, label, maximum=2**64 - 1):
    _require(type(value) is int and 0 <= value <= maximum, f'invalid unsigned {label}')
    return value


def _at(values, index, label):
    _uint(index, label)
    _require(index < len(values), f'{label} out of range')
    return values[index]


def _vector(value, length, label):
    _require(isinstance(value, (list, tuple)) and len(value) == length, f'invalid {label}')
    _require(all(type(v) in (int, float) and math.isfinite(v) for v in value), f'nonfinite {label}')
    return tuple(float(v) for v in value)


def _matrix(value):
    m = _vector(value, 16, 'affine transform')
    _require(m[3::4] == (0., 0., 0., 1.), 'non-affine transform')
    return m


def _multiply(a, b):
    result = tuple(sum(a[r + k*4] * b[k + c*4] for k in range(4))
                   for c in range(4) for r in range(4))
    _require(all(math.isfinite(v) for v in result), 'nonfinite composed transform')
    return result


def _point(m, p):
    # Sum the three products before translation, then round only at capture.
    result = [sum(m[r + k*4] * p[k] for k in range(3)) + m[r + 12] for r in range(3)]
    _require(all(math.isfinite(v) for v in result), 'nonfinite transformed position')
    return result


def _f32(v):
    try:
        out = struct.unpack('<f', struct.pack('<f', v))[0]
    except (OverflowError, struct.error) as exc:
        raise ManifestError('value outside finite f32 domain') from exc
    _require(math.isfinite(out), 'value outside finite f32 domain')
    return out


def _normal_matrix(m):
    a, b, c = m[0], m[4], m[8]
    d, e, f = m[1], m[5], m[9]
    g, h, i = m[2], m[6], m[10]
    cof = ((e*i-f*h, f*g-d*i, d*h-e*g),
           (c*h-b*i, a*i-c*g, b*g-a*h),
           (b*f-c*e, c*d-a*f, a*e-b*d))
    det = a*cof[0][0] + b*cof[0][1] + c*cof[0][2]
    _require(math.isfinite(det) and det != 0, 'singular transform')
    return det, tuple(tuple(v/det for v in row) for row in cof)


def _unit(v):
    length = math.sqrt(sum(x*x for x in v))
    _require(math.isfinite(length) and length > 0, 'zero/nonfinite transformed direction')
    return [x/length for x in v]


def _normal(m, n, rounding=True):
    _, inverse_transpose = _normal_matrix(m)
    value = _unit([sum(row[k]*n[k] for k in range(3)) for row in inverse_transpose])
    return [_f32(x) for x in value] if rounding else value


def _local(node):
    if 'matrix' in node:
        _require(not any(k in node for k in ('translation', 'rotation', 'scale')), 'matrix with TRS')
        return _matrix(node['matrix'])
    t = _vector(node.get('translation', [0, 0, 0]), 3, 'translation')
    s = _vector(node.get('scale', [1, 1, 1]), 3, 'scale')
    q = _vector(node.get('rotation', [0, 0, 0, 1]), 4, 'rotation')
    length = math.sqrt(sum(v*v for v in q))
    _require(abs(length-1) <= 1e-6, 'non-unit quaternion')
    x, y, z, w = (v/length for v in q)
    return ((1-2*(y*y+z*z))*s[0], (2*(x*y+z*w))*s[0], (2*(x*z-y*w))*s[0], 0.,
            (2*(x*y-z*w))*s[1], (1-2*(x*x+z*z))*s[1], (2*(y*z+x*w))*s[1], 0.,
            (2*(x*z+y*w))*s[2], (2*(y*z-x*w))*s[2], (1-2*(x*x+y*y))*s[2], 0., *t, 1.)


class _Accessor:
    """Random access into mmap; sparse indices use binary search, not a resident map."""
    TYPES = {5120: ('b', 1, 127), 5121: ('B', 1, 255),
             5122: ('h', 2, 32767), 5123: ('H', 2, 65535),
             5125: ('I', 4, 4294967295), 5126: ('f', 4, 1)}
    WIDTHS = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4}

    def __init__(self, glb, index):
        a = _at(glb.doc.get('accessors', []), index, 'accessor')
        _require(not a.get('extensions'), 'unsupported accessor extension')
        self.count = _uint(a.get('count'), 'accessor count')
        self.component = a.get('componentType')
        _require(self.component in self.TYPES and a.get('type') in self.WIDTHS, 'unsupported accessor type')
        code, width, self.maximum = self.TYPES[self.component]
        self.width = self.WIDTHS[a['type']]
        self.unpack = struct.Struct('<' + code*self.width)
        self.normalized = a.get('normalized', False)
        _require(type(self.normalized) is bool, 'invalid normalization flag')
        _require(not self.normalized or self.component in (5120, 5121, 5122, 5123), 'invalid normalized component')
        self.raw, self.start, self.stride = None, 0, self.unpack.size
        if 'bufferView' in a:
            self.raw, view = glb.view(a['bufferView'])
            self.start = _uint(a.get('byteOffset', 0), 'accessor offset')
            self.stride = _uint(view.get('byteStride', self.unpack.size), 'accessor stride')
            _require(self.stride >= self.unpack.size and self.stride % width == 0 and self.start % width == 0,
                     'invalid accessor alignment/stride')
            end = self.start + (self.count-1)*self.stride + self.unpack.size if self.count else self.start
            _require(end <= len(self.raw), 'accessor outside bufferView')
        else:
            _require(a.get('byteOffset', 0) == 0, 'offset on absent accessor bufferView')
        self.sparse = None
        if 'sparse' in a:
            sparse = a['sparse']
            count = _uint(sparse.get('count'), 'sparse count', self.count)
            indices, values = sparse['indices'], sparse['values']
            component = indices.get('componentType')
            _require(component in (5121, 5123, 5125), 'invalid sparse index component')
            code, iw, _ = self.TYPES[component]
            iraw, iv = glb.view(indices['bufferView'])
            vraw, vv = glb.view(values['bufferView'])
            _require('byteStride' not in iv and 'byteStride' not in vv, 'strided sparse bufferView')
            io = _uint(indices.get('byteOffset', 0), 'sparse indices offset')
            vo = _uint(values.get('byteOffset', 0), 'sparse values offset')
            _require(io % iw == 0 and vo % width == 0 and io+count*iw <= len(iraw)
                     and vo+count*self.unpack.size <= len(vraw), 'sparse data outside bufferView')
            unpack = struct.Struct('<'+code)
            previous = -1
            for n in range(count):
                vertex = _unpack_from(unpack, iraw, io+n*iw)[0]
                _require(previous < vertex < self.count, 'unordered/out-of-range sparse index')
                previous = vertex
            self.sparse = count, iraw, io, unpack, vraw, vo

    def __getitem__(self, index):
        _require(0 <= index < self.count, 'vertex/index accessor out of range')
        values = None
        if self.sparse is not None:
            count, iraw, io, unpack, vraw, vo = self.sparse
            low, high = 0, count
            while low < high:
                mid = (low+high)//2
                if _unpack_from(unpack, iraw, io+mid*unpack.size)[0] < index:
                    low = mid+1
                else:
                    high = mid
            if low < count and _unpack_from(unpack, iraw, io+low*unpack.size)[0] == index:
                values = _unpack_from(self.unpack, vraw, vo+low*self.unpack.size)
        if values is None:
            values = _unpack_from(self.unpack, self.raw, self.start+index*self.stride) if self.raw is not None else (0,)*self.width
        _require(all(math.isfinite(v) for v in values), 'nonfinite accessor value')
        if self.normalized:
            values = tuple(max(-1., v/self.maximum) for v in values)
        return values


class _GLB:
    """Embedded GLB geometry, mmap on source and at most one bounded baseline leaf."""
    def __init__(self, path, baseline=False):
        self.file = Path(path).open('rb')
        self.data = None
        self.views = {}
        self.baseline = baseline
        try:
            _require(os.fstat(self.file.fileno()).st_size >= 28, 'short GLB')
            self.data = mmap.mmap(self.file.fileno(), 0, access=mmap.ACCESS_READ)
            magic, version, length = struct.unpack_from('<4sII', self.data)
            _require(magic == b'glTF' and version == 2 and length == len(self.data), 'invalid GLB header')
            jl, jt = struct.unpack_from('<I4s', self.data, 12)
            _require(jt == b'JSON' and jl <= MAX_JSON_BYTES and jl % 4 == 0 and 28+jl <= length,
                     'invalid/oversize GLB JSON chunk')
            self.doc = json.loads(self.data[20:20+jl])
            _require(self.doc.get('asset', {}).get('version') == '2.0', 'requires glTF 2.0')
            bl, bt = struct.unpack_from('<I4s', self.data, 20+jl)
            _require(bt == b'BIN\0' and bl % 4 == 0 and 28+jl+bl == length, 'requires one embedded GLB BIN')
            buffers = self.doc.get('buffers', [])
            _require(buffers and all('uri' not in b for b in buffers), 'external GLB buffer unsupported')
            size = _uint(buffers[0].get('byteLength'), 'buffer length')
            _require(size <= bl and bl-size <= 3, 'logical buffer/BIN length mismatch')
            _require(len(buffers) == 1 or (baseline and all(
                b.get('extensions', {}).get('EXT_meshopt_compression', {}).get('fallback') is True
                for b in buffers[1:])), 'unsupported additional GLB buffers')
            self.bin_start, self.bin_length = 28+jl, size
            self._profile()
        except Exception:
            self.close()
            raise

    def _profile(self):
        j = self.doc
        _require(not any(k in j for k in ('animations', 'skins')), 'animated/skinned source unsupported')
        # Image codecs are opaque here: material dictionaries are retained,
        # but no image/appearance result is inferred from reading geometry.
        allowed = {'KHR_mesh_quantization', 'KHR_materials_unlit', 'KHR_texture_transform',
                   'KHR_texture_basisu', 'EXT_texture_webp', 'EXT_texture_avif'}
        if self.baseline:
            allowed |= {'EXT_meshopt_compression', 'EXT_mesh_features', 'EXT_structural_metadata'}
        _require(set(j.get('extensionsRequired', [])) <= allowed, 'unsupported required GLB extension')
        _require(set(j.get('extensions', {})) <= ({'EXT_structural_metadata'} if self.baseline else set()),
                 'unsupported scene extension')
        for n in j.get('nodes', []):
            _require(not n.get('extensions') and not any(k in n for k in ('skin', 'weights')),
                     'node extension/skin/morph unsupported')
        for mesh in j.get('meshes', []):
            _require(not mesh.get('extensions') and 'weights' not in mesh, 'mesh extension/morph unsupported')
            for p in mesh.get('primitives', []):
                _require('targets' not in p and set(p.get('extensions', {})) <=
                         ({'EXT_mesh_features'} if self.baseline else set()), 'primitive extension/morph unsupported')

    def view(self, index):
        v = _at(self.doc.get('bufferViews', []), index, 'bufferView')
        extensions = v.get('extensions', {})
        _require(set(extensions) <= ({'EXT_meshopt_compression'} if self.baseline else set()),
                 'compressed/extended source bufferView unsupported')
        if index not in self.views:
            e = extensions.get('EXT_meshopt_compression')
            if e is not None:
                filter_name = e.get('filter', 'NONE')
                _require(e.get('buffer') == 0 and filter_name in
                         ('NONE', 'OCTAHEDRAL', 'QUATERNION', 'EXPONENTIAL'), 'unsupported meshopt buffer/filter')
                start = _uint(e.get('byteOffset', 0), 'meshopt offset')
                length = _uint(e.get('byteLength'), 'meshopt length')
                count = _uint(e.get('count'), 'meshopt count')
                stride = _uint(e.get('byteStride'), 'meshopt stride')
                size = count*stride
                _require(size == v.get('byteLength') and size <= MAX_BASELINE_BYTES,
                         'oversize/inconsistent meshopt view')
                _require(start+length <= self.bin_length, 'meshopt outside logical buffer')
                mode = e.get('mode')
                _require(mode in ('ATTRIBUTES', 'TRIANGLES', 'INDICES'), 'unsupported meshopt mode')
                result = subprocess.run(['python3', str(Path(__file__).with_name('meshopt.py')),
                                         mode, str(count), str(stride), filter_name],
                                        input=self.data[self.bin_start+start:self.bin_start+start+length],
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60)
                _require(result.returncode == 0 and len(result.stdout) == size,
                         'independent meshopt decode failed: '+result.stderr[-1000:].decode(errors='replace'))
                _require(sum(len(x) for x in self.views.values())+size <= MAX_BASELINE_BYTES,
                         'baseline decoded view limit exceeded')
                self.views[index] = result.stdout
            else:
                _require(v.get('buffer') == 0, 'bufferView cannot read virtual/external buffer')
                start = _uint(v.get('byteOffset', 0), 'bufferView offset')
                length = _uint(v.get('byteLength'), 'bufferView length')
                _require(start+length <= self.bin_length, 'bufferView outside logical buffer')
                # A small slice descriptor avoids copying large source views.
                self.views[index] = _Window(self.data, self.bin_start+start, length)
        return self.views[index], v

    def accessor(self, index):
        return _Accessor(self, index)

    def close(self):
        self.views.clear()
        if self.data is not None:
            self.data.close()
            self.data = None
        self.file.close()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()


class _Window:
    def __init__(self, data, offset, length):
        self.data, self.offset, self.length = data, offset, length

    def __len__(self):
        return self.length


def _unpack_from(unpack, raw, offset):
    if isinstance(raw, _Window):
        return unpack.unpack_from(raw.data, raw.offset+offset)
    return unpack.unpack_from(raw, offset)


def _instances(glb):
    j = glb.doc
    scenes, nodes = j.get('scenes', []), j.get('nodes', [])
    _require('scene' in j or len(scenes) == 1, 'requires default or sole scene')
    scene = _at(scenes, j.get('scene', 0), 'scene')
    parents = {}
    for i, n in enumerate(nodes):
        for child in n.get('children', []):
            _at(nodes, child, 'child node')
            _require(child not in parents, 'multiple/duplicate node parents')
            parents[child] = i
    roots = scene.get('nodes', [])
    _require(len(roots) == len(set(roots)) and not any(r in parents for r in roots),
             'duplicate/nonroot scene root')
    stack = [(r, IDENTITY, 0) for r in reversed(roots)]
    seen = set()
    ordinal = 0
    while stack:
        ni, parent, depth = stack.pop()
        _require(depth < 128 and ni not in seen, 'scene depth/cycle limit')
        seen.add(ni)
        n = _at(nodes, ni, 'scene node')
        world = _multiply(parent, _local(n))
        _normal_matrix(world)
        if 'mesh' in n:
            mi = _uint(n['mesh'], 'mesh index', 2**32-1)
            mesh = _at(j.get('meshes', []), mi, 'mesh')
            for pi, primitive in enumerate(mesh.get('primitives', [])):
                yield ni, mi, pi, ordinal, world, primitive
                ordinal += 1
        stack.extend((child, world, depth+1) for child in reversed(n.get('children', [])))


class _Primitive:
    def __init__(self, glb, descriptor):
        self.node, self.mesh, self.primitive, self.ordinal, self.world, p = descriptor
        attrs = p.get('attributes', {})
        allowed = {'POSITION', 'NORMAL', 'TANGENT', 'TEXCOORD_0', 'TEXCOORD_1', 'COLOR_0'}
        _require('POSITION' in attrs and all(k in allowed or (glb.baseline and k.startswith('_FEATURE_ID_'))
                                         for k in attrs), 'unsupported/missing primitive attribute')
        self.attrs = {k: glb.accessor(v) for k, v in attrs.items() if k in allowed}
        self.position = self.attrs['POSITION']
        widths = {'POSITION': (3,), 'NORMAL': (3,), 'TANGENT': (4,),
                  'TEXCOORD_0': (2,), 'TEXCOORD_1': (2,), 'COLOR_0': (3, 4)}
        _require(all(a.width in widths[k] and a.count == self.position.count for k, a in self.attrs.items()),
                 'primitive attribute shape/count mismatch')
        self.indices = glb.accessor(p['indices']) if 'indices' in p else None
        if self.indices is not None:
            _require(self.indices.width == 1 and self.indices.component in (5121, 5123, 5125)
                     and not self.indices.normalized, 'invalid index accessor')
        self.mode = p.get('mode', 4)
        _require(self.mode in (4, 5, 6), 'only TRIANGLES/STRIP/FAN supported')
        count = self.indices.count if self.indices is not None else self.position.count
        _require(self.mode != 4 or count % 3 == 0, 'incomplete TRIANGLES primitive')
        self.count = count//3 if self.mode == 4 else max(0, count-2)
        self.det, self.normal_matrix = _normal_matrix(self.world)
        material_index = p.get('material', 2**32-1)
        _uint(material_index, 'material index', 2**32-1)
        material = _at(glb.doc.get('materials', []), material_index, 'material') if 'material' in p else None
        self.shared = dict(expanded_ordinal=self.ordinal, material_index=material_index, material=material)

    def vertices(self, triangle, reflected=True):
        _require(0 <= triangle < self.count, 'triangle ordinal out of range')
        if self.mode == 4:
            offsets = [3*triangle, 3*triangle+1, 3*triangle+2]
        elif self.mode == 5:
            offsets = [triangle, triangle+1, triangle+2] if triangle % 2 == 0 else [triangle+1, triangle, triangle+2]
        else:
            offsets = [0, triangle+1, triangle+2]
        vertices = [self.indices[o][0] for o in offsets] if self.indices is not None else offsets
        _require(all(0 <= v < self.position.count for v in vertices), 'source vertex out of range')
        if self.indices is not None:
            _require(all(v != self.indices.maximum for v in vertices), 'primitive restart index unsupported')
        if reflected and self.det < 0:
            vertices[1], vertices[2] = vertices[2], vertices[1]
        return vertices

    def row(self, triangle, capture=True):
        vertices = self.vertices(triangle, reflected=capture)
        xyz = [_point(self.world, self.position[v]) for v in vertices]
        if capture:
            xyz = [[0. if x == 0 else _f32(x) for x in p] for p in xyz]
        row = dict(xyz=xyz, vertices=vertices)
        if 'NORMAL' in self.attrs:
            row['normals'] = [_normal(self.world, self.attrs['NORMAL'][v], rounding=capture) for v in vertices]
        texcoords = {}
        for k in (0, 1):
            if f'TEXCOORD_{k}' in self.attrs:
                texcoords[str(k)] = [[_f32(x) for x in self.attrs[f'TEXCOORD_{k}'][v]] for v in vertices]
        if texcoords:
            row['texcoord_sets'] = texcoords
            if '0' in texcoords:
                row['texcoords'] = texcoords['0']
                row['uvs'] = texcoords['0']
        if 'TANGENT' in self.attrs:
            tangents = []
            for v in vertices:
                t = self.attrs['TANGENT'][v]
                direction = _unit([sum(self.world[r+k*4]*t[k] for k in range(3)) for r in range(3)])
                tangents.append([*map(_f32, direction), _f32(t[3]*(-1 if self.det < 0 else 1))])
            row['tangents'] = tangents
        if 'COLOR_0' in self.attrs:
            row['colors'] = [list(map(_f32, self.attrs['COLOR_0'][v])) for v in vertices]
            row['colors'] = [v+[1.] if len(v) == 3 else v for v in row['colors']]
        return row


def _placed(row, matrix):
    if matrix == IDENTITY:
        return row
    row['xyz'] = [_point(matrix, p) for p in row['xyz']]
    if 'normals' in row:
        row['normals'] = [_normal(matrix, n, rounding=False) for n in row['normals']]
    if 'tangents' in row:
        sign = -1 if _normal_matrix(matrix)[0] < 0 else 1
        row['tangents'] = [[*_unit([sum(matrix[r+k*4]*t[k] for k in range(3)) for r in range(3)]),
                            sign*t[3]] for t in row['tangents']]
    return row


def _source_rows(path, transform):
    with _GLB(path) as glb:
        for descriptor in _instances(glb):
            primitive = _Primitive(glb, descriptor)
            for triangle in range(primitive.count):
                key = f'{primitive.node},{primitive.mesh},{primitive.primitive},{triangle}'
                yield key, dict(_placed(primitive.row(triangle), transform), **primitive.shared)


def _safe_name(base, uri):
    _require(isinstance(uri, str) and '\\' not in uri and not any(c in uri for c in ('%', '?', '#', ':')),
             'requires plain relative archive URI')
    name = PurePosixPath(base).parent / uri
    _require(not name.is_absolute() and '..' not in name.parts, 'unsafe archive URI')
    return str(name)


class _Archive:
    def __init__(self, path):
        path = Path(path)
        self.directory = path if path.is_dir() else path.parent if path.suffix.lower() == '.json' else None
        self.root = path.name if path.suffix.lower() == '.json' else 'tileset.json'
        self.zip = zipfile.ZipFile(path) if self.directory is None else None
        if self.zip is not None:
            names = self.zip.namelist()
            _require(len(names) == len(set(names)), 'duplicate baseline ZIP members')

    @contextmanager
    def open(self, name, limit):
        if self.directory is not None:
            path = (self.directory/name).resolve()
            _require(path.is_relative_to(self.directory.resolve()), 'baseline member outside directory')
            _require(path.stat().st_size <= limit, f'baseline member exceeds {limit} bytes: {name}')
            with path.open('rb') as stream:
                yield stream
        else:
            info = self.zip.getinfo(name)
            _require(info.file_size <= limit, f'baseline member exceeds {limit} bytes: {name}')
            with self.zip.open(info) as stream:
                yield stream

    def document(self):
        with self.open(self.root, MAX_JSON_BYTES) as stream:
            return json.load(stream)

    def close(self):
        if self.zip is not None:
            self.zip.close()


def _leaves(doc, root_name, coordinate_frame):
    root = doc['root']
    start = IDENTITY if coordinate_frame == 'root' else _matrix(root.get('transform', IDENTITY))
    stack = [(root, start, 0)]
    visits = 0
    while stack:
        node, tile_world, depth = stack.pop()
        visits += 1
        _require(visits <= 1_000_000 and depth < 128, 'baseline hierarchy limit')
        _require('implicitTiling' not in node, 'implicit baseline requires independent expansion')
        _require(not ('content' in node and 'contents' in node), 'both tile content forms')
        entries = node.get('contents', [node['content']] if 'content' in node else [])
        _require(not any(str(e.get('uri', e.get('url', ''))).lower().endswith('.json') for e in entries),
                 'external baseline tilesets need independent expansion')
        children = node.get('children', [])
        if children:
            stack.extend((child, _multiply(tile_world, _matrix(child.get('transform', IDENTITY))), depth+1)
                         for child in reversed(children))
        else:
            _require(bool(entries), 'empty baseline leaf')
            for entry in entries:
                name = _safe_name(root_name, entry.get('uri', entry.get('url')))
                _require(name.lower().endswith('.glb'), 'baseline requires inline explicit GLB leaves')
                # In root frame conjugate relative tile transforms into glTF Y-up.
                m = _multiply(tile_world, Y_UP)
                if coordinate_frame == 'root':
                    m = _multiply(Y_UP_INVERSE, m)
                yield name, m


def _exact_id(value, label):
    # decode.mjs transports UINT64 through decimal strings, never JS Number.
    if isinstance(value, str):
        _require(value.isascii() and value.isdecimal(), f'invalid identity string {label}')
        value = int(value)
    return _uint(value, label)


def _baseline_faces(path):
    """Existing independent identity decoder, reading GLB via stdin, not a filename API."""
    decoder = Path(__file__).with_name('decode.mjs')
    with Path(path).open('rb') as source, tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as errors:
        result = subprocess.run(['node', '--max-old-space-size=512', str(decoder), '/dev/stdin'],
                                stdin=source, stdout=output, stderr=errors, timeout=120)
        errors.seek(0)
        detail = errors.read(2000).decode(errors='replace')
        _require(result.returncode == 0, 'baseline identity decode failed: '+detail)
        _require(output.tell() <= 256*1024*1024, 'baseline decoder output limit')
        output.seek(0)
        return json.load(output)['faces']


def _baseline_rows(path, coordinate_frame, transform):
    archive = _Archive(path)
    try:
        for name, placement in _leaves(archive.document(), archive.root, coordinate_frame):
            with tempfile.TemporaryDirectory(prefix='s5-source-leaf-') as scratch:
                leaf = Path(scratch)/'leaf.glb'
                with archive.open(name, MAX_BASELINE_BYTES) as source, leaf.open('wb') as destination:
                    while chunk := source.read(1024*1024):
                        destination.write(chunk)
                with _GLB(leaf, baseline=True) as glb:
                    descriptors = list(_instances(glb))
                    primitives = [_Primitive(glb, d) for d in descriptors]
                    _require(sum(p.count for p in primitives) <= MAX_BASELINE_TRIANGLES, 'baseline leaf triangle limit')
                    faces = iter(_baseline_faces(leaf))
                    for p in primitives:
                        _require(p.mode == 4, 'baseline identity decoder requires TRIANGLES')
                        for triangle in range(p.count):
                            face = next(faces, None)
                            _require(face is not None, 'baseline decoder lost faces')
                            row = p.row(triangle, capture=False)
                            # Published geometry is already baked: do not swap reflection corners again.
                            _require(row['xyz'] == face['xyz'], 'baseline independent geometry decode mismatch')
                            ids = face.get('ids', {})
                            _require('source_triangle' in ids and 'source_primitive' in ids,
                                     'baseline leaf missing source identity sets')
                            t = {k: _exact_id(v, k) for k, v in ids['source_triangle'].items()}
                            s = ids['source_primitive']
                            fields = ('node_index', 'mesh_index', 'primitive_index', 'triangle_index',
                                      'expanded_ordinal', 'material_index', 'vertex_0', 'vertex_1', 'vertex_2')
                            _require(all(k in t for k in fields), 'incomplete baseline source identity')
                            _require(all(_exact_id(s.get(k), k) == t[k] for k in
                                         ('node_index', 'mesh_index', 'primitive_index', 'expanded_ordinal', 'material_index')),
                                     'baseline source identity sets disagree')
                            key = ','.join(str(t[k]) for k in fields[:4])
                            row.update(vertices=[t[f'vertex_{k}'] for k in range(3)], material=face['material'],
                                       material_index=t['material_index'], expanded_ordinal=t['expanded_ordinal'])
                            yield key, _placed(row, _multiply(transform, placement))
                    _require(next(faces, None) is None, 'baseline decoder added faces')
    finally:
        archive.close()


def _sha256(path):
    digest = hashlib.sha256()
    with Path(path).open('rb') as source:
        while chunk := source.read(1024*1024):
            digest.update(chunk)
    return digest.hexdigest()


def _compact(value):
    return json.dumps(value, separators=(',', ':'), allow_nan=False)


def _write_database(path, rows, metadata):
    connection = sqlite3.connect(path)
    try:
        connection.executescript('''
            PRAGMA journal_mode=OFF;
            PRAGMA synchronous=OFF;
            PRAGMA temp_store=FILE;
            PRAGMA cache_size=-8192;
            CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE primitives (key TEXT PRIMARY KEY, data TEXT NOT NULL);
            CREATE TABLE triangles (
                key TEXT PRIMARY KEY, source_order BLOB NOT NULL UNIQUE,
                primitive TEXT NOT NULL, data TEXT NOT NULL, rank INTEGER);
        ''')
        count = 0
        previous_primitive = None
        previous_shared = None
        for key, original in rows:
            indices = _identity(key)
            row = dict(original)
            shared = {k: row.pop(k) for k in ('expanded_ordinal', 'material_index', 'material')}
            ordinal = _uint(shared['expanded_ordinal'], 'expanded ordinal')
            primitive_key = ','.join(map(str, indices[:3]))
            shared_json = _compact(shared)
            if primitive_key != previous_primitive:
                existing = connection.execute('SELECT data FROM primitives WHERE key=?', (primitive_key,)).fetchone()
                if existing is None:
                    connection.execute('INSERT INTO primitives VALUES (?,?)', (primitive_key, shared_json))
                else:
                    _require(json.loads(existing[0]) == shared, 'inconsistent primitive identity/material across baseline leaves')
                previous_primitive, previous_shared = primitive_key, shared_json
            else:
                _require(previous_shared == shared_json, 'inconsistent primitive identity/material')
            try:
                connection.execute('INSERT INTO triangles VALUES (?,?,?,?,NULL)',
                                   (key, struct.pack('>QQ', ordinal, indices[3]), primitive_key, _compact(row)))
            except sqlite3.IntegrityError as exc:
                raise ManifestError('duplicate triangle identity or expanded source order: '+key) from exc
            count += 1
            if count % 8192 == 0:
                connection.commit()
        _require(count > 0, 'source contains no selected triangles')
        # SQLite performs the sort/join on disk; no archive-wide Python ID map.
        connection.executescript('''
            CREATE TEMP TABLE ranks AS SELECT key,
                row_number() OVER (ORDER BY source_order)-1 AS rank FROM triangles;
            CREATE UNIQUE INDEX ranks_key ON ranks(key);
            UPDATE triangles SET rank=(SELECT rank FROM ranks WHERE ranks.key=triangles.key);
            DROP TABLE ranks;
            CREATE UNIQUE INDEX triangle_rank ON triangles(rank);
        ''')
        metadata['triangleCount'] = count
        connection.execute('INSERT INTO metadata VALUES (?,?)', ('manifest', _compact(metadata)))
        connection.commit()
    finally:
        connection.close()


def generate_manifest(source, output, *, transform16=None, coordinate_frame='root'):
    """Generate atomically from a source .glb or untouched explicit baseline archive.

    Source .glb: selected scene transforms, reflection correction and f32 capture;
    no published identity extensions needed. Baseline: only hierarchy leaves,
    independent identity decode via stdin. Default ``root`` ignores the common
    archive root placement and conjugates relative tile transforms by the glTF
    Y-up/Z-up conversion. ``world`` applies full tile transforms *after* that
    conversion, entirely in f64. Direct GLB world placement requires transform16.
    transform16 is a caller-specified column-major affine matrix applied last,
    after capture rounding; global coordinates are never rounded back to f32.
    """
    source, output = Path(source), Path(output)
    _require(output.suffix.lower() in ('.json', '.sqlite'), 'output must end in .json or .sqlite')
    _require(coordinate_frame in ('root', 'world'), 'unknown coordinate frame')
    _require(source.resolve() != output.resolve(), 'source and output must differ')
    transform = IDENTITY if transform16 is None else _matrix(transform16)
    direct = source.suffix.lower() == '.glb'
    _require(not direct or coordinate_frame != 'world' or transform16 is not None,
             'direct source world frame needs explicit --transform16 (including Y-up conversion)')
    metadata = dict(format='s5-source-manifest-v1', sourceKind='sourceGLB' if direct else 'baselineLeaves',
                    source=str(source.resolve()), coordinateFrame=coordinate_frame,
                    transform16=list(transform), sourceOrder='expanded_ordinal,triangle_index',
                    tieOrder='expanded_ordinal,triangle_index,source_rank',
                    geometry='scene-world f32 capture; placement f64',
                    attributes='normals: inverse transpose/unit/f32; texcoords: decoded f32')
    if source.is_file():
        metadata['sourceSha256'] = _sha256(source)
    else:
        metadata['sourceTilesetSha256'] = _sha256(source/'tileset.json')
    rows = _source_rows(source, transform) if direct else _baseline_rows(source, coordinate_frame, transform)
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.s5-source-', dir=output.parent) as scratch:
        db = Path(scratch)/'manifest.sqlite'
        _write_database(db, rows, metadata)
        if output.suffix.lower() == '.sqlite':
            os.replace(db, output)
        else:
            _require(metadata['triangleCount'] <= MAX_JSON_TRIANGLES,
                     f'JSON limited to {MAX_JSON_TRIANGLES} triangles; use .sqlite')
            destination = Path(scratch)/'manifest.json'
            with load_manifest(db) as manifest, destination.open('w') as stream:
                stream.write('{"metadata":'+_compact(metadata)+',"triangles":{')
                for rank, (key, row) in enumerate(manifest['triangles'].items()):
                    stream.write((',' if rank else '')+_compact(key)+':'+_compact(row))
                stream.write('}}\n')
            os.replace(destination, output)
    return metadata


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', help='source GLB, explicit baseline .3tz/.zip, directory or tileset.json')
    parser.add_argument('output', help='.json (at most 100000 triangles) or .sqlite (disk-backed)')
    parser.add_argument('--transform16', nargs=16, type=float, metavar='M',
                        help='explicit column-major affine placement, applied after f32 capture')
    parser.add_argument('--coordinate-frame', choices=('root', 'world'), default='root',
                        help='baseline root-relative glTF frame (default) or full world frame')
    parser.add_argument('--content-frame', dest='coordinate_frame', action='store_const', const='root',
                        help='explicitly strip common root placement; retain relative tile transforms')
    args = parser.parse_args()
    try:
        print(json.dumps(generate_manifest(args.source, args.output, transform16=args.transform16,
                                           coordinate_frame=args.coordinate_frame), sort_keys=True))
    except (ManifestError, OSError, sqlite3.Error, subprocess.SubprocessError, KeyError,
            TypeError, StopIteration) as exc:
        parser.exit(1, f'sourceManifest: {exc}\n')


if __name__ == '__main__':
    main()
