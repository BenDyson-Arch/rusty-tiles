"""Independent, bounded 3D Tiles archive and hierarchy reader (standard library).

``with Archive(path) as a: tiles = expand(a)`` reads directories and ZIP/.3tz
archives without extracting them. Tiles are returned in preorder with object
links in ``parent``/``children``. ``contents`` contains resolved renderable URI
strings; external tileset links remain non-rendered ``external`` placeholders.
``node`` retains explicit properties or holds the synthesized implicit tile.
``transform`` is parent * local, column-major; ``g`` is local geometric error.

Implicit QUADTREE/OCTREE availability follows the 3D Tiles specification, not
producer manifests. JSON and version-1 binary subtrees, external buffers, box
and region subdivision, multiple contents, and numeric tile metadata overrides
are supported. Extension-defined hierarchy/bounds, remote resources, and data
URIs are outside this reader's domain and fail closed. Constants below bound
all input work.
"""

import json
import lzma
import math
import posixpath
import stat
import struct
import zipfile
import zlib
from pathlib import Path
from urllib.parse import unquote, urlsplit


MAX_ENTRIES = 1_000_000
MAX_READ_BYTES = 64 << 30
MAX_MEMBER_BYTES = 256 << 20
MAX_JSON_BYTES = 64 << 20
MAX_VISITS = 1_000_000
MAX_DEPTH = 256
MAX_AVAILABILITY_BITS = 32_000_000  # cumulative, including constant streams
IDENTITY = (1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1)


def _require(condition, message):
    if not condition:
        raise ValueError(message)


def _integer(value, label, minimum=0):
    _require(type(value) is int and value >= minimum, 'invalid ' + label)
    return value


def _number(value, label):
    _require(type(value) in (int, float), 'invalid ' + label)
    try:
        valid = math.isfinite(value)
    except (OverflowError, ValueError):
        valid = False
    _require(valid, 'nonfinite ' + label)
    return value


def _object(value, label):
    _require(isinstance(value, dict), 'invalid ' + label)
    return value


def _array(value, label):
    _require(isinstance(value, list), 'invalid ' + label)
    return value


def _resolve(base, uri):
    _require(isinstance(uri, str) and uri and len(uri) <= 16384,
             'invalid archive URI')
    try:
        parts = urlsplit(uri)
        path = unquote(parts.path, errors='strict')
    except (ValueError, UnicodeError) as exc:
        raise ValueError('invalid archive URI') from exc
    _require(not (parts.scheme or parts.netloc or parts.query or parts.fragment),
             'nonlocal archive URI')
    _require(path and not path.startswith('/') and '\\' not in path and
             not any(ord(c) < 32 for c in path), 'unsafe archive URI')
    name = posixpath.normpath(posixpath.join(posixpath.dirname(base), path))
    _require(name not in ('.', '..') and not name.startswith('../'),
             'archive URI escapes root')
    return name


def _json(data):
    _require(len(data) <= MAX_JSON_BYTES, 'JSON byte limit')

    def pairs(items):
        result = {}
        for key, value in items:
            _require(key not in result, 'duplicate JSON key')
            result[key] = value
        return result

    def constant(value):
        raise ValueError('nonfinite JSON constant: ' + value)

    try:
        return json.loads(data, object_pairs_hook=pairs, parse_constant=constant)
    except (UnicodeError, RecursionError) as exc:
        raise ValueError('invalid or excessively nested JSON') from exc


class Archive:
    """Read archive-relative resources, charging every read to a byte budget.

    Bounds are module constants, not input-controlled. Instances must be used
    inside their context. Missing members and malformed ZIPs raise ValueError.
    """

    root_name = 'tileset.json'

    def __init__(self, path):
        self.path = Path(path)
        self.bytes_read = 0
        self._zip = None
        self._members = None
        self._root = None
        self._open = False

    def __enter__(self):
        _require(not self._open, 'archive already open')
        self.bytes_read = 0
        try:
            if self.path.is_dir():
                self._root = self.path.resolve(strict=True)
            else:
                # Bound central-directory allocation before ZipFile parses it.
                # ZIP64 needs a larger bounded member file and is not guessed.
                self._check_directory_size()
                self._zip = zipfile.ZipFile(self.path)
                infos = self._zip.infolist()
                _require(len(infos) <= MAX_ENTRIES, 'archive entry limit')
                self._members = {}
                for info in infos:
                    name = info.filename
                    _require(len(name) <= 16384 and '\\' not in name and
                             not name.startswith('/') and
                             not any(ord(c) < 32 for c in name),
                             'unsafe ZIP member name')
                    _require(posixpath.normpath(name.rstrip('/')) == name.rstrip('/')
                             and '..' not in name.split('/'), 'unsafe ZIP member name')
                    _require(name not in self._members, 'duplicate ZIP member')
                    _require(not stat.S_ISLNK(info.external_attr >> 16), 'ZIP symlink')
                    self._members[name] = info
            self._open = True
            return self
        except (OSError, zipfile.BadZipFile, zipfile.LargeZipFile, ValueError) as exc:
            self.__exit__(None, None, None)
            raise ValueError('cannot open archive: ' + str(exc)) from exc

    def _check_directory_size(self):
        with self.path.open('rb') as stream:
            stream.seek(0, 2)
            size = stream.tell()
            _require(size <= (1 << 44), 'archive file size limit')
            stream.seek(max(0, size - 65557))
            tail = stream.read(65557)
            self.bytes_read += len(tail)
            offset = tail.rfind(b'PK\x05\x06')
            _require(offset >= 0 and offset + 22 <= len(tail), 'missing ZIP directory')
            end = struct.unpack_from('<4s4H2LH', tail, offset)
            _require(offset + 22 + end[7] == len(tail), 'invalid ZIP end record')
            _require(end[1] == end[2] == 0, 'multipart ZIP unsupported')
            count, length = end[4], end[5]
            if count == 65535 or length == 0xffffffff or end[6] == 0xffffffff:
                absolute = size - len(tail) + offset
                _require(absolute >= 20, 'missing ZIP64 locator')
                stream.seek(absolute - 20)
                locator = stream.read(20)
                self.bytes_read += len(locator)
                signature, disk, position, disks = struct.unpack('<4sLQL', locator)
                _require(signature == b'PK\x06\x07' and disk == 0 and disks == 1,
                         'invalid ZIP64 locator')
                stream.seek(position)
                record = stream.read(56)
                self.bytes_read += len(record)
                _require(len(record) == 56, 'truncated ZIP64 directory')
                fields = struct.unpack('<4sQ2H2L4Q', record)
                _require(fields[0] == b'PK\x06\x06' and fields[1] >= 44 and
                         fields[4] == fields[5] == 0 and fields[6] == fields[7],
                         'invalid ZIP64 directory')
                count, length = fields[7], fields[8]
            _require(count <= MAX_ENTRIES and length <= MAX_JSON_BYTES,
                     'ZIP directory limit')

    def __exit__(self, exc_type, exc, traceback):
        if self._zip is not None:
            self._zip.close()
        self._zip = self._members = self._root = None
        self._open = False

    def _read(self, name, prefix=None):
        _require(self._open, 'archive is closed')
        # Public names are already resolved names, not URI references: decoding
        # here again would corrupt literal percent signs in resolved filenames.
        _require(isinstance(name, str) and name and not name.startswith('/') and
                 '\\' not in name and not any(ord(c) < 32 for c in name) and
                 posixpath.normpath(name) == name and '..' not in name.split('/'),
                 'unsafe archive name')
        try:
            if self._zip is not None:
                info = self._members.get(name)
                _require(info is not None and not info.is_dir(), 'missing archive member: ' + name)
                size = info.file_size
                opener = lambda: self._zip.open(info)
            else:
                path = (self._root / name).resolve(strict=True)
                _require(path.is_relative_to(self._root) and path.is_file(),
                         'archive path escapes root or is not a file')
                size = path.stat().st_size
                opener = lambda: path.open('rb')
            _require(0 <= size <= MAX_MEMBER_BYTES, 'archive member byte limit')
            expected = size if prefix is None else min(size, prefix)
            _require(self.bytes_read + expected <= MAX_READ_BYTES, 'archive read byte limit')
            with opener() as stream:
                data = stream.read(expected + (1 if prefix is None else 0))
            self.bytes_read += len(data)
            _require(len(data) == expected, 'archive member size mismatch')
            return data
        except (OSError, RuntimeError, NotImplementedError, EOFError,
                zipfile.BadZipFile, zlib.error, lzma.LZMAError) as exc:
            raise ValueError('cannot read archive member: ' + name) from exc

    def read(self, name):
        return self._read(name)

    def json(self, name):
        return _json(self.read(name))


def _matrix(value):
    _require(isinstance(value, (list, tuple)) and len(value) == 16, 'invalid transform')
    result = [_number(v, 'transform') for v in value]
    _require([result[i] for i in (3, 7, 11, 15)] == [0, 0, 0, 1],
             'non-affine transform')
    return result


def _multiply(a, b):
    result = [sum(a[k * 4 + r] * b[c * 4 + k] for k in range(4))
              for c in range(4) for r in range(4)]
    return _matrix(result)


def _contents(node):
    _require(not ('content' in node and 'contents' in node), 'both content forms')
    entries = node.get('contents', [node['content']] if 'content' in node else [])
    for entry in _array(entries, 'contents'):
        _object(entry, 'content')
        _require(not ('uri' in entry and 'url' in entry), 'both content URI forms')
        _require(isinstance(entry.get('uri', entry.get('url')), str), 'missing content URI')
    return entries


def _hierarchy_extensions(node):
    extensions = _object(node.get('extensions', {}), 'extensions')
    _require(not any(name in extensions for name in
                     ('3DTILES_implicit_tiling', '3DTILES_multiple_contents')),
             'legacy hierarchy extension unsupported')


def _bounds(root, level, xyz, dimensions):
    volume = _object(root.get('boundingVolume'), 'implicit boundingVolume')
    _require(not volume.get('extensions'), 'implicit bounding volume extension unsupported')
    _require(len([k for k in ('box', 'region', 'sphere') if k in volume]) == 1,
             'invalid implicit boundingVolume')
    scale = 2.0 ** -level
    if 'box' in volume:
        values = _array(volume['box'], 'box')
        _require(len(values) == 12, 'invalid box')
        values = [_number(v, 'box') for v in values]
        center = values[:3]
        axes = [values[3 + 3 * k:6 + 3 * k] for k in range(3)]
        for k in range(dimensions):
            offset = (2 * xyz[k] + 1) * scale - 1
            center = [center[j] + axes[k][j] * offset for j in range(3)]
        result = center + [v * (scale if k < dimensions else 1)
                           for k in range(3) for v in axes[k]]
        _require(all(math.isfinite(v) for v in result), 'implicit box overflow')
        return dict(box=result)
    _require('region' in volume, 'implicit spheres unsupported')
    values = _array(volume['region'], 'region')
    _require(len(values) == 6, 'invalid region')
    west, south, east, north, low, high = [_number(v, 'region') for v in values]
    _require(-math.pi <= west <= math.pi and -math.pi <= east <= math.pi and
             -math.pi / 2 <= south <= north <= math.pi / 2 and low <= high,
             'invalid region extents')
    width = east - west if east >= west else east - west + 2 * math.pi
    w = west + width * xyz[0] * scale
    e = west + width * (xyz[0] + 1) * scale
    wrap = lambda v: (v + math.pi) % (2 * math.pi) - math.pi if v > math.pi else v
    result = [wrap(w), south + (north - south) * xyz[1] * scale,
              wrap(e), south + (north - south) * (xyz[1] + 1) * scale,
              low, high]
    if dimensions == 3:
        result[4:] = [low + (high - low) * xyz[2] * scale,
                      low + (high - low) * (xyz[2] + 1) * scale]
    _require(all(math.isfinite(v) for v in result), 'implicit region overflow')
    return dict(region=result)


def _template(uri, level, xyz, dimensions):
    _require(isinstance(uri, str), 'invalid implicit URI template')
    names = ('level', 'x', 'y', 'z')[:dimensions + 1]
    for name, value in zip(names, (level,) + xyz):
        _require('{' + name + '}' in uri, 'missing implicit template coordinate')
        uri = uri.replace('{' + name + '}', str(value))
    _require('{' not in uri and '}' not in uri, 'unknown implicit template coordinate')
    return uri


def _bit(data, index):
    return (data[index // 8] >> (index % 8)) & 1


class _Subtree:
    def __init__(self, archive, name, levels, branch, content_count, budget, schema):
        data = archive.read(name)
        binary = None
        if data[:4] == b'subt':
            _require(len(data) >= 24, 'truncated subtree header')
            magic, version, json_length, binary_length = struct.unpack_from('<4sLQQ', data)
            _require(version == 1 and json_length % 8 == 0 and binary_length % 8 == 0 and
                     24 + json_length + binary_length == len(data), 'invalid subtree header')
            doc = _json(data[24:24 + json_length])
            binary = memoryview(data)[24 + json_length:]
        else:
            doc = _json(data)
        doc = _object(doc, 'subtree')
        _hierarchy_extensions(doc)
        _require('contentMetadata' not in doc, 'implicit content metadata unsupported')
        buffers = []
        for index, buffer in enumerate(_array(doc.get('buffers', []), 'buffers')):
            buffer = _object(buffer, 'buffer')
            length = _integer(buffer.get('byteLength'), 'buffer byteLength', 1)
            _require(length <= MAX_MEMBER_BYTES, 'buffer byte limit')
            if 'uri' in buffer:
                raw = archive.read(_resolve(name, buffer['uri']))
                _require(len(raw) == length, 'external buffer length mismatch')
            else:
                _require(index == 0 and binary is not None and
                         length <= len(binary) <= length + 7, 'invalid internal subtree buffer')
                raw = binary[:length]
            buffers.append(memoryview(raw))
        views = []
        for view in _array(doc.get('bufferViews', []), 'bufferViews'):
            view = _object(view, 'bufferView')
            index = _integer(view.get('buffer'), 'bufferView buffer')
            start = _integer(view.get('byteOffset', 0), 'bufferView offset')
            length = _integer(view.get('byteLength'), 'bufferView length', 1)
            _require(index < len(buffers) and start % 8 == 0 and
                     start + length <= len(buffers[index]), 'invalid bufferView range')
            views.append(buffers[index][start:start + length])
        tile_count = (branch ** levels - 1) // (branch - 1)
        child_count = branch ** levels
        total = tile_count * (1 + content_count) + child_count
        budget[0] += total
        _require(budget[0] <= MAX_AVAILABILITY_BITS, 'availability bit limit')

        def availability(value, count):
            value = _object(value, 'availability')
            _require(('constant' in value) != ('bitstream' in value), 'invalid availability encoding')
            if 'constant' in value:
                constant = _integer(value['constant'], 'availability constant')
                _require(constant in (0, 1), 'invalid availability constant')
                bits = (1 << count) - 1 if constant else 0
            else:
                index = _integer(value['bitstream'], 'availability bitstream')
                _require(index < len(views) and len(views[index]) * 8 >= count,
                         'truncated availability bitstream')
                bits = int.from_bytes(views[index], 'little')
                _require(bits.bit_length() <= count, 'nonzero availability padding')
            if 'availableCount' in value:
                expected = _integer(value['availableCount'], 'availableCount')
                _require(bits.bit_count() == expected, 'availability count mismatch')
            return bits

        tiles = availability(doc.get('tileAvailability'), tile_count)
        _require(tiles & 1, 'unavailable subtree root')
        content = _array(doc.get('contentAvailability', []), 'contentAvailability')
        _require(len(content) == content_count, 'content availability/template count mismatch')
        contents = [availability(value, tile_count) for value in content]
        for bits in contents:
            _require(not bits & ~tiles, 'content on unavailable tile')
        child = availability(doc.get('childSubtreeAvailability'), child_count)
        # Byte indexing avoids repeatedly shifting an entire large Python int
        # during traversal and the parent-consistency scan.
        self.tiles = tiles.to_bytes((tile_count + 7) // 8, 'little')
        self.contents = [bits.to_bytes((tile_count + 7) // 8, 'little') for bits in contents]
        self.child = child.to_bytes((child_count + 7) // 8, 'little')
        # Validate unreachable islands as well as nodes actually traversed.
        offset = 1
        previous = 0
        for level in range(1, levels):
            for morton in range(branch ** level):
                if _bit(self.tiles, offset + morton):
                    _require(_bit(self.tiles, previous + morton // branch),
                             'available tile has unavailable parent')
            previous, offset = offset, offset + branch ** level
        for morton in range(child_count):
            if _bit(self.child, morton):
                _require(_bit(self.tiles, previous + morton // branch),
                         'available child subtree has unavailable parent')
        self.metadata = self._metadata(doc, schema, views, tiles.bit_count(), tile_count)

    def _metadata(self, doc, schema, views, count, tile_count):
        if 'tileMetadata' not in doc:
            return {}
        _require(count <= MAX_VISITS, 'metadata tile count limit')
        tables = _array(doc.get('propertyTables'), 'propertyTables')
        index = _integer(doc['tileMetadata'], 'tileMetadata')
        _require(index < len(tables), 'invalid tileMetadata table')
        table = _object(tables[index], 'tile property table')
        _require(_integer(table.get('count'), 'metadata count', 1) == count,
                 'tile metadata count mismatch')
        classes = _object(_object(schema, 'metadata schema').get('classes'), 'schema classes')
        class_name = table.get('class')
        _require(isinstance(class_name, str) and class_name in classes, 'invalid metadata class')
        definitions = _object(_object(classes[class_name], 'metadata class').get('properties'),
                              'class properties')
        properties = _object(table.get('properties', {}), 'table properties')
        columns = []
        shapes = {'TILE_BOUNDING_BOX': ('box', 12), 'TILE_BOUNDING_REGION': ('region', 6),
                  'TILE_BOUNDING_SPHERE': ('sphere', 4), 'TILE_GEOMETRIC_ERROR': ('g', 1),
                  'TILE_MINIMUM_HEIGHT': ('minimumHeight', 1),
                  'TILE_MAXIMUM_HEIGHT': ('maximumHeight', 1)}
        for key, definition in definitions.items():
            definition = _object(definition, 'property definition')
            semantic = definition.get('semantic')
            _require(semantic is None or isinstance(semantic, str), 'invalid metadata semantic')
            if semantic not in shapes:
                continue
            label, width = shapes[semantic]
            _require(key in properties, 'missing tile semantic property')
            prop = _object(properties[key], 'semantic property')
            _require(definition.get('type') == 'SCALAR' and
                     definition.get('componentType') in ('FLOAT32', 'FLOAT64') and
                     definition.get('array', False) == (width > 1) and
                     (width == 1 or definition.get('count') == width),
                     'unsupported tile semantic shape')
            for source in (definition, prop):
                _require(not any(k in source for k in ('offset', 'scale', 'noData', 'default')) and
                         not source.get('normalized', False), 'tile semantic modifiers unsupported')
            view = _integer(prop.get('values'), 'semantic values')
            fmt = '<' + ('d' if definition['componentType'] == 'FLOAT64' else 'f') * width
            stride = struct.calcsize(fmt)
            _require(view < len(views) and len(views[view]) == count * stride,
                     'invalid tile semantic buffer range')
            columns.append((label, fmt, stride, views[view]))
        result = {}
        row = 0
        for index in range(tile_count):
            if not _bit(self.tiles, index):
                continue
            values = {}
            for label, fmt, stride, data in columns:
                value = list(struct.unpack_from(fmt, data, row * stride))
                _require(all(math.isfinite(v) for v in value), 'nonfinite tile semantic')
                values[label] = value[0] if len(value) == 1 else value
                if label == 'g':
                    _require(value[0] >= 0, 'negative tile geometric error semantic')
                if label == 'sphere':
                    _require(value[3] >= 0, 'invalid bounding sphere semantic')
            if values:
                result[index] = values
            row += 1
        return result

    def tile(self, level, morton, branch):
        return _bit(self.tiles, (branch ** level - 1) // (branch - 1) + morton)


def expand(archive):
    """Expand a bounded explicit/implicit/external forest into linked tile dicts.

    External links retain their link tile and attach external roots as children.
    Renderable and external contents on the same tile are refused. Explicit
    nodes require finite nonnegative geometricError; ADD/REPLACE is inherited.
    Missing geometry is not opened: this function reads hierarchy resources.
    """
    tiles = []
    budget = [0]
    document = _object(archive.json(archive.root_name), 'tileset')
    documents = {archive.root_name: document}
    _require('root' in document, 'missing tileset root')
    # Each task holds (node, tileset name, parent, active external documents,
    # implicit context). A DFS stack keeps traversal memory bounded by depth.
    stack = [(document['root'], archive.root_name, None,
              frozenset((archive.root_name,)), None)]
    while stack:
        node, base, parent, active, implicit = stack.pop()
        _require(len(tiles) < MAX_VISITS, 'hierarchy visit limit')
        depth = parent['depth'] + 1 if parent else 0
        _require(depth <= MAX_DEPTH, 'hierarchy depth limit')
        node = _object(node, 'tile')
        _hierarchy_extensions(node)
        refine = node.get('refine', parent['refine'] if parent else 'REPLACE')
        _require(refine in ('REPLACE', 'ADD'), 'invalid refinement')
        g = _number(node.get('geometricError'), 'geometricError')
        _require(g >= 0, 'negative geometricError')
        transform = _multiply(parent['transform'] if parent else IDENTITY,
                              _matrix(node.get('transform', IDENTITY)))
        entries = _contents(node)
        if implicit is None and 'implicitTiling' in node:
            settings = _object(node['implicitTiling'], 'implicitTiling')
            scheme = settings.get('subdivisionScheme')
            _require(scheme in ('QUADTREE', 'OCTREE'), 'unsupported subdivision scheme')
            dimensions = 2 if scheme == 'QUADTREE' else 3
            levels = _integer(settings.get('subtreeLevels'), 'subtreeLevels', 1)
            available = _integer(settings.get('availableLevels'), 'availableLevels', 1)
            _require(levels <= 24 and available <= MAX_DEPTH and
                     (1 << (dimensions * levels)) <= MAX_AVAILABILITY_BITS,
                     'implicit level limit')
            _require(not any(k in node for k in ('children', 'metadata')),
                     'invalid implicit root properties')
            for entry in entries:
                _require('boundingVolume' not in entry, 'implicit content boundingVolume unsupported')
            subtree_template = _object(settings.get('subtrees'), 'subtrees').get('uri')
            xyz = (0,) * dimensions
            _bounds(node, 0, xyz, dimensions)
            name = _resolve(base, _template(subtree_template, 0, xyz, dimensions))
            document = documents[base]
            _require(not ('schema' in document and 'schemaUri' in document), 'both metadata schema forms')
            schema = document.get('schema')
            if 'schemaUri' in document:
                schema = archive.json(_resolve(base, document['schemaUri']))
            subtree = _Subtree(archive, name, levels, 1 << dimensions, len(entries), budget, schema)
            implicit = dict(root=node, entries=entries, dimensions=dimensions,
                            levels=levels, available=available, template=subtree_template,
                            subtree=subtree, subtree_name=name, level=0, xyz=xyz,
                            local_level=0, morton=0, subtree_active=frozenset((name,)), schema=schema)
        if implicit is not None:
            context = implicit
            level, xyz = context['level'], context['xyz']
            dimensions, levels = context['dimensions'], context['levels']
            branch = 1 << dimensions
            subtree, local, morton = context['subtree'], context['local_level'], context['morton']
            _require(subtree.tile(local, morton, branch), 'unavailable implicit traversal')
            index = (branch ** local - 1) // (branch - 1) + morton
            # Templates and transform apply at the implicit root only. Generated
            # descendants inherit the composed transform without reapplying it.
            generated = dict(node) if level == 0 else {}
            generated.pop('implicitTiling', None)
            generated.pop('content', None)
            generated.pop('contents', None)
            generated['boundingVolume'] = _bounds(context['root'], level, xyz, dimensions)
            generated['geometricError'] = context['root']['geometricError'] * 2.0 ** -level
            overrides = subtree.metadata.get(index, {})
            for kind in ('box', 'region', 'sphere'):
                if kind in overrides:
                    generated['boundingVolume'] = {kind: overrides[kind]}
                    break
            if 'g' in overrides:
                generated['geometricError'] = overrides['g']
            for label, index_in_region in (('minimumHeight', 4), ('maximumHeight', 5)):
                if label in overrides:
                    _require('region' in generated['boundingVolume'], 'height semantic requires region')
                    generated['boundingVolume']['region'][index_in_region] = overrides[label]
            generated['refine'] = refine
            generated['implicit_coordinates'] = dict(zip(('level', 'x', 'y', 'z'), (level,) + xyz))
            entries = []
            for i, entry in enumerate(context['entries']):
                if _bit(subtree.contents[i], index):
                    item = dict(entry)
                    item.pop('url', None)
                    item['uri'] = _template(entry.get('uri', entry.get('url')), level, xyz, dimensions)
                    entries.append(item)
            if entries:
                generated['contents'] = entries
            node, g = generated, generated['geometricError']
        payloads, external = [], []
        for entry in entries:
            name = _resolve(base, entry.get('uri', entry.get('url')))
            suffix = posixpath.splitext(name)[1].lower()
            # Known render formats need no mesh reads. Other suffixes may name
            # external tilesets; sniff only a bounded prefix before JSON decode.
            is_json = suffix == '.json'
            if suffix not in ('.json', '.glb', '.gltf', '.b3dm', '.i3dm', '.pnts', '.cmpt'):
                is_json = archive._read(name, prefix=64).lstrip().startswith(b'{')
            if is_json:
                linked = _object(archive.json(name), 'JSON content')
                if 'root' in linked:
                    _require(implicit is None, 'implicit external tileset unsupported')
                    _require(name not in active, 'external tileset cycle')
                    external.append((name, linked['root']))
                    documents[name] = linked
                    continue
            payloads.append(name)
        _require(not (payloads and external), 'mixed external and renderable content unsupported')
        tile = dict(id=str(len(tiles)), node=node, parent=parent, children=[], depth=depth,
                    contents=payloads, transform=transform, g=g, refine=refine,
                    external=bool(external), external_uris=[name for name, _ in external])
        tiles.append(tile)
        if parent is not None:
            parent['children'].append(tile)
        tasks = []
        if implicit is None:
            for child in _array(node.get('children', []), 'children'):
                tasks.append((child, base, tile, active, None))
            for name, root in external:
                tasks.append((root, name, tile, active | {name}, None))
        elif context['level'] + 1 < context['available']:
            for child in range(branch):
                next_local = local + 1
                next_morton = morton * branch + child
                boundary = next_local == levels
                available = _bit(subtree.child, next_morton) if boundary else subtree.tile(next_local, next_morton, branch)
                if not available:
                    continue
                next_xyz = tuple(2 * xyz[k] + ((child >> k) & 1) for k in range(dimensions))
                next_context = dict(context, level=level + 1, xyz=next_xyz,
                                    local_level=next_local, morton=next_morton)
                if boundary:
                    name = _resolve(base, _template(context['template'], level + 1, next_xyz, dimensions))
                    _require(name not in context['subtree_active'], 'implicit subtree cycle')
                    next_context.update(subtree=_Subtree(archive, name, levels, branch,
                                                        len(context['entries']), budget, context['schema']),
                                        subtree_name=name, local_level=0, morton=0,
                                        subtree_active=context['subtree_active'] | {name})
                child_node = dict(geometricError=g / 2, refine=refine)
                tasks.append((child_node, base, tile, active, next_context))
        _require(len(tiles) + len(stack) + len(tasks) <= MAX_VISITS, 'hierarchy visit limit')
        stack.extend(reversed(tasks))
    return tiles
