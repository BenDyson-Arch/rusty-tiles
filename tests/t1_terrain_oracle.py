"""Independent T1 LOD GLB/3D Tiles semantic oracle; no producer imports or GDAL warp.

The output is a quadtree implicit forest. The oracle expands it from the
subtree bytes, recovers each vertex's source-lattice node from its decoded
ECEF position, and recomputes every tile's geometric error by dense brute force.

Tiles are EXT_meshopt_compression streams of uint16 KHR_mesh_quantization (or
Float32) positions. ``ext_meshopt_spec`` decodes them; it is written from the
pinned extension text and shares no code with meshoptimizer. Positions are then
dequantized through the node's translation and scale, so every check below
sees what a consumer draws.

Run with ``python tests/t1_terrain_oracle.py DIRECTORY``. Fixture creation uses
GDAL only to write explicitly described source values, never to sample them.
"""
import argparse
from collections import Counter
import json
import math
import pathlib
import struct

import ext_meshopt_spec as meshopt

MESHOPT, QUANTIZATION = 'EXT_meshopt_compression', 'KHR_mesh_quantization'


def ecef(lon, lat, height):
    """WGS84 from a and inverse flattening, independently of producer constants."""
    a = 6378137.0
    f = 1 / 298.257223563
    e2 = f * (2 - f)
    lon, lat = math.radians(lon), math.radians(lat)
    n = a / math.sqrt(1 - e2 * math.sin(lat) ** 2)
    return ((n + height) * math.cos(lat) * math.cos(lon),
            (n + height) * math.cos(lat) * math.sin(lon),
            (n * (1 - e2) + height) * math.sin(lat))


def glb_parts(data):
    assert len(data) >= 28, "truncated GLB"
    magic, version, length = struct.unpack_from('<4sII', data)
    assert (magic, version, length) == (b'glTF', 2, len(data)), "GLB envelope"
    chunks = []
    at = 12
    while at < len(data):
        count, kind = struct.unpack_from('<I4s', data, at)
        at += 8
        assert count % 4 == 0 and at + count <= len(data), "GLB chunk range"
        chunks.append((kind, data[at:at + count]))
        at += count
    assert [kind for kind, _ in chunks] == [b'JSON', b'BIN\x00'], "GLB chunks"
    return json.loads(chunks[0][1]), chunks[1][1]


def decoded_views(document, binary):
    """Every bufferView's bytes, decoding EXT_meshopt_compression streams."""
    used, required = set(document.get('extensionsUsed', [])), set(document.get('extensionsRequired', []))
    assert required <= used <= {MESHOPT, QUANTIZATION}, 'terrain glTF extensions'
    buffers = document['buffers']
    assert 'uri' not in buffers[0] and buffers[0]['byteLength'] <= len(binary), 'GLB binary buffer'
    for fallback in buffers[1:]:
        assert 'uri' not in fallback and fallback.get('extensions', {}).get(MESHOPT, {}).get('fallback') is True, 'fallback buffer'
    views = []
    for view in document['bufferViews']:
        compressed = view.get('extensions', {}).get(MESHOPT)
        if compressed is None:
            assert view.get('buffer', 0) == 0, 'uncompressed view outside the GLB binary'
            start = view.get('byteOffset', 0)
            assert start + view['byteLength'] <= len(binary), 'view range'
            views.append(binary[start:start + view['byteLength']])
            continue
        assert MESHOPT in required, 'compressed view without required EXT_meshopt_compression'
        assert view['buffer'] > 0, 'compressed view must use a fallback buffer'
        assert compressed.get('buffer') == 0 and compressed.get('filter', 'NONE') == 'NONE', 'compressed stream source'
        start, count, stride = compressed.get('byteOffset', 0), compressed['count'], compressed['byteStride']
        assert start + compressed['byteLength'] <= len(binary), 'compressed stream range'
        assert count * stride == view['byteLength'], 'decoded view size'
        stream = binary[start:start + compressed['byteLength']]
        if compressed['mode'] == 'ATTRIBUTES':
            views.append(meshopt.decode_attributes(stream, count, stride))
        else:
            assert compressed['mode'] == 'TRIANGLES' and stride in (2, 4), 'terrain index streams are triangle lists'
            views.append(meshopt.pack_indices(meshopt.decode_triangles(stream, count), stride))
    return views


def glb(path):
    """Document and drawn primitives: decoded positions (node translation and scale
    applied, glTF y-up) and triangles as position tuples."""
    document, binary = glb_parts(pathlib.Path(path).read_bytes())
    assert document['asset']['version'] == '2.0'
    views = decoded_views(document, binary)
    required = set(document.get('extensionsRequired', []))

    def accessor(index):
        record = document['accessors'][index]
        assert not record.get('normalized', False) and 'sparse' not in record
        view = document['bufferViews'][record['bufferView']]
        raw = views[record['bufferView']]
        fmt, size = {5121: ('B', 1), 5123: ('H', 2), 5125: ('I', 4), 5126: ('f', 4)}[record['componentType']]
        width = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3}[record['type']]
        offset = record.get('byteOffset', 0)
        stride = view.get('byteStride', width * size)
        end = offset + (record['count'] - 1) * stride + width * size
        assert end <= len(raw), 'accessor payload'
        values = [struct.unpack_from('<' + fmt * width, raw, offset + i * stride)
                  for i in range(record['count'])]
        assert all(math.isfinite(component) for value in values for component in value)
        if record['type'] == 'VEC3' and 'min' in record:
            for axis in range(3):
                assert record['min'][axis] <= min(v[axis] for v in values)
                assert record['max'][axis] >= max(v[axis] for v in values)
        return values

    assert len(document['nodes']) == 1 and set(document['nodes'][0]) <= {'mesh', 'translation', 'scale'}, 'one TS-only node'
    node = document['nodes'][0]
    translation, scale = node.get('translation', [0., 0., 0.]), node.get('scale', [1., 1., 1.])
    primitives = []
    for mesh in document['meshes']:
        for primitive in mesh['primitives']:
            assert primitive.get('mode', 4) == 4
            record = document['accessors'][primitive['attributes']['POSITION']]
            if record['componentType'] != 5126:
                assert record['componentType'] == 5123 and QUANTIZATION in required, 'uint16 positions require KHR_mesh_quantization'
            positions = [tuple(translation[i] + scale[i] * v[i] for i in range(3))
                         for v in accessor(primitive['attributes']['POSITION'])]
            indices = [v[0] for v in accessor(primitive['indices'])] if 'indices' in primitive else list(range(len(positions)))
            assert len(indices) % 3 == 0 and all(0 <= i < len(positions) for i in indices)
            assert len(set(indices)) == len(positions), 'every vertex is drawn'
            triangles = [tuple(positions[i] for i in indices[start:start + 3]) for start in range(0, len(indices), 3)]
            primitives.append((positions, triangles))
    return document, primitives


def transform(matrix, point):
    return tuple(sum(matrix[row + 4 * col] * point[col] for col in range(3)) + matrix[row + 12]
                 for row in range(3))


def contains_box(box, points):
    """Solve the three box axes; accept zero-axis degenerate coordinate slabs."""
    center = box[:3]
    axes = [box[3:6], box[6:9], box[9:12]]
    # Initial profile uses orthogonal boxes. Fail instead of assuming a sheared box.
    for i in range(3):
        for j in range(i):
            assert abs(sum(a*b for a, b in zip(axes[i], axes[j]))) <= 1e-8
    for p in points:
        d = [p[i] - center[i] for i in range(3)]
        for axis in axes:
            size2 = sum(a*a for a in axis)
            if size2:
                assert abs(sum(a*b for a, b in zip(d, axis))) <= size2 + 1e-7 * math.sqrt(size2), 'content outside box'
            else:
                # With zero axis, require residual to lie in the other axis span.
                residual = d[:]
                for other in axes:
                    norm2 = sum(a*a for a in other)
                    if norm2:
                        t = sum(a*b for a, b in zip(d, other))/norm2
                        residual = [a-t*b for a,b in zip(residual,other)]
                assert math.sqrt(sum(a*a for a in residual)) <= 1e-7, 'outside degenerate box'


IDENTITY = [1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1]


def geodetic(point):
    """ECEF to WGS84 longitude, latitude (degrees) and height by fixed-point iteration."""
    a = 6378137.0
    f = 1 / 298.257223563
    e2 = f * (2 - f)
    x, y, z = point
    r = math.hypot(x, y)
    lat = math.atan2(z, r * (1 - e2))
    for _ in range(12):
        n = a / math.sqrt(1 - e2 * math.sin(lat) ** 2)
        height = r / math.cos(lat) - n
        lat = math.atan2(z, r * (1 - e2 * n / (n + height)))
    n = a / math.sqrt(1 - e2 * math.sin(lat) ** 2)
    return math.degrees(math.atan2(y, x)), math.degrees(lat), r / math.cos(lat) - n


def morton(index, level):
    """Quadtree Morton index to (x, y): x takes the even bits."""
    return (sum(((index >> (2 * bit)) & 1) << bit for bit in range(level)),
            sum(((index >> (2 * bit + 1)) & 1) << bit for bit in range(level)))


def implicit_tree(root, directory, schema):
    """Decode one forest implicit root: a single quadtree subtree, no child subtrees.

    Every available tile takes its box and error from its TILE_BOUNDING_BOX and
    TILE_GEOMETRIC_ERROR row, found through the schema semantics; the level-0 row
    must equal the root tile bitwise. Every terrain tile has content.
    """
    spec = root['implicitTiling']
    assert spec['subdivisionScheme'] == 'QUADTREE', 'quadtree profile'
    levels = spec['subtreeLevels']
    assert spec['availableLevels'] == levels and 1 <= levels <= 4, 'implicit root is one bounded subtree'
    assert 'children' not in root, 'implicit root has explicit children'
    name = spec['subtrees']['uri'].format(level=0, x=0, y=0)
    data = (directory/name).read_bytes()
    magic, version, json_length, binary_length = struct.unpack_from('<4sIQQ', data)
    assert magic == b'subt' and version == 1 and 24 + json_length + binary_length == len(data), 'subtree framing'
    document = json.loads(data[24:24 + json_length])
    binary = data[24 + json_length:]
    assert document['childSubtreeAvailability'] == {'constant': 0}, 'forest subtree has no child subtrees'

    def view(index):
        record = document['bufferViews'][index]
        assert 'uri' not in document['buffers'][record['buffer']], 'internal subtree buffer'
        return binary[record['byteOffset']:record['byteOffset'] + record['byteLength']]

    def bits(declaration, count):
        if 'constant' in declaration:
            return [bool(declaration['constant'])] * count
        raw = view(declaration['bitstream'])
        assert len(raw) == (count + 7) // 8, 'availability bytes'
        return [bool(raw[i // 8] & (1 << (i % 8))) for i in range(count)]

    count = (4 ** levels - 1) // 3
    tiles = bits(document['tileAvailability'], count)
    declarations = document['contentAvailability']
    declarations = declarations if isinstance(declarations, list) else [declarations]
    assert len(declarations) == 1, 'one content slot'
    content = bits(declarations[0], count)
    table = document['propertyTables'][document['tileMetadata']]
    semantics = {value.get('semantic'): key for key, value in schema['classes'][table['class']]['properties'].items()}
    boxes = view(table['properties'][semantics['TILE_BOUNDING_BOX']]['values'])
    errors = view(table['properties'][semantics['TILE_GEOMETRIC_ERROR']]['values'])
    assert table['count'] == sum(tiles) and len(boxes) == 96 * sum(tiles) and len(errors) == 8 * sum(tiles), 'metadata rows'
    template = root['content']['uri']
    nodes, rank = {}, 0
    for level in range(levels):
        offset = (4 ** level - 1) // 3
        for index in range(4 ** level):
            bit = offset + index
            assert content[bit] == tiles[bit], 'every available terrain tile has content'
            if not tiles[bit]:
                continue
            if level:
                assert tiles[(4 ** (level - 1) - 1) // 3 + index // 4], 'availability parent'
            x, y = morton(index, level)
            nodes[(level, x, y)] = dict(box=list(struct.unpack_from('<12d', boxes, rank * 96)),
                                        error=struct.unpack_from('<d', errors, rank * 8)[0],
                                        uri=template.format(level=level, x=x, y=y), children=[],
                                        row=dict(subtree=name, rank=rank))
            rank += 1
    for (level, x, y), node in sorted(nodes.items()):
        if level:
            nodes[(level - 1, x // 2, y // 2)]['children'].append(node)
    top = nodes[(0, 0, 0)]
    assert struct.pack('<12d', *root['boundingVolume']['box']) == struct.pack('<12d', *top['box']), 'level-0 row box'
    assert struct.pack('<d', root['geometricError']) == struct.pack('<d', top['error']), 'level-0 row error'
    return top


def expand(tile, directory, schema, documents):
    """The explicit tree of a bounded implicit forest: skeleton, links and implicit roots."""
    if 'implicitTiling' in tile:
        assert schema, 'implicit root without the metadata schema'
        return implicit_tree(tile, directory, schema)
    uri = tile['content']['uri']
    if uri.endswith('.json'):
        assert not tile.get('children') and uri not in documents, 'link tile has only its document'
        documents.add(uri)
        document = json.loads((directory/uri).read_text())
        linked = document['root']
        assert 'transform' not in linked, 'external document root frame'
        assert linked['boundingVolume'] == tile['boundingVolume'] and linked['geometricError'] == tile['geometricError']
        return expand(linked, directory, document.get('schema', schema), documents)
    assert set(tile) <= {'boundingVolume', 'geometricError', 'refine', 'transform', 'content', 'children'}, 'skeleton keys'
    return dict(box=tile['boundingVolume']['box'], error=tile['geometricError'], uri=uri,
                children=[expand(child, directory, schema, documents) for child in tile.get('children', [])])


def tree(directory):
    """Expand tileset.json into (manifest, root, preorder tiles with parent and depth)."""
    directory = pathlib.Path(directory)
    manifest = json.loads((directory/'tileset.json').read_text())
    assert manifest['asset']['version'] == '1.1'
    root = manifest['root']
    assert root.get('refine') == 'REPLACE', 'REPLACE refinement'
    top = expand(root, directory, manifest.get('schema'), set())
    tiles, stack = [], [(top, None, 0)]
    while stack:
        node, parent, depth = stack.pop()
        node['parent'], node['depth'] = parent, depth
        tiles.append(node)
        stack.extend((child, node, depth + 1) for child in reversed(node['children']))
    return manifest, root, tiles


class Patch:
    """One decoded tile: lattice parameters and frame positions of every vertex.

    The single primitive holds surface triangles (three lattice nodes) and skirt
    triangles (two nodes: a curtain under an edge)."""

    def __init__(self, directory, node, report, frame):
        document, primitives = glb(directory/node['uri'])
        assert len(primitives) == 1, 'one primitive per tile: the skirt shares the surface draw call'
        assert len(document['materials']) == 1, 'one material per tile'
        primitive = document['meshes'][0]['primitives'][0]
        material = document['materials'][primitive['material']]
        assert material['alphaMode'] == 'OPAQUE' and not material['doubleSided'], 'opaque single-sided material'
        self.compressed = all(MESHOPT in view.get('extensions', {}) for view in document['bufferViews'])
        self.scale = document['nodes'][0].get('scale') if QUANTIZATION in document.get('extensionsRequired', []) else None
        west, south, east, north = report['bounds']
        pixel = report['sourcePixelDegrees']

        def decode(position):
            local = (position[0], -position[2], position[1])
            lon, lat, height = geodetic(transform(frame, local))
            x, y = (lon - west) / pixel[0], (lat - south) / pixel[1]
            node = (round(x), round(y))
            assert abs(x - node[0]) < .25 and abs(y - node[1]) < .25, 'vertex off the source lattice'
            return local, node, height

        positions, triangles = primitives[0]
        decoded = {p: decode(p) for p in positions}
        self.triangles, skirt = [], []
        for triangle in triangles:
            distinct = len({decoded[p][1] for p in triangle})
            assert distinct in (2, 3), 'degenerate triangle'
            (self.triangles if distinct == 3 else skirt).append(triangle)
        surface = list(dict.fromkeys(p for triangle in self.triangles for p in triangle))
        self.glb_surface = surface
        self.points = [decoded[p] for p in surface]
        self.local = [p[0] for p in self.points]
        self.node = {position: decoded[position][1] for position in surface}
        self.surface_triangles = [tuple(self.node[p] for p in triangle) for triangle in self.triangles]
        nodes = [point[1] for point in self.points]
        assert len(nodes) == len(set(nodes)), 'surface vertices form one lattice'
        # check_directory replaces these with the full lattice when holes drop nodes.
        self.xs, self.ys = sorted({n[0] for n in nodes}), sorted({n[1] for n in nodes})
        self.footprint = (self.xs[0], self.ys[0], self.xs[-1], self.ys[-1])
        self.position = {point[1]: point[0] for point in self.points}
        self.height = {point[1]: point[2] for point in self.points}
        self.skirt = [tuple(decoded[p] for p in triangle) for triangle in skirt]
        self.skirt_points = [decoded[p] for p in positions if p not in self.node]
        self.cells = {}
        for triangle in self.surface_triangles:
            key = (min(n[0] for n in triangle), min(n[1] for n in triangle))
            self.cells.setdefault(key, []).append(triangle)

    def at(self, x, y):
        """The surface point at lattice parameters (x, y) on the stored triangles, or None."""
        import bisect
        column = min(max(bisect.bisect_right(self.xs, x) - 1, 0), len(self.xs) - 2)
        row = min(max(bisect.bisect_right(self.ys, y) - 1, 0), len(self.ys) - 2)
        for a, b, c in self.cells.get((self.xs[column], self.ys[row]), []):
            denominator = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1])
            u = ((b[1] - c[1]) * (x - c[0]) + (c[0] - b[0]) * (y - c[1])) / denominator
            v = ((c[1] - a[1]) * (x - c[0]) + (a[0] - c[0]) * (y - c[1])) / denominator
            w = 1 - u - v
            if min(u, v, w) >= -1e-12:
                pa, pb, pc = self.position[a], self.position[b], self.position[c]
                return tuple(u * pa[i] + v * pb[i] + w * pc[i] for i in range(3))
        return None


def check_directory(directory, samples=4):
    """Forest structure, payloads, bounds, certified errors, skirts and receipt."""
    directory = pathlib.Path(directory)
    manifest, root, tiles = tree(directory)
    report = json.loads((directory/'conversion.json').read_text())
    frame = root['transform']
    width, height = report['width'], report['height']
    patches = {id(node): Patch(directory, node, report, frame) for node in tiles}
    # Footprints and lattices are quadrants of the parent's, whatever NoData removed.
    def place(node, f):
        patch, stride = patches[id(node)], 2 ** (report['levels'] - 1 - node['depth'])
        patch.footprint, half = f, report['cellsPerLeaf'] * stride // 2
        patch.xs, patch.ys = (sorted(set(range(f[a], f[a + 2], stride)) | {f[a + 2]}) for a in (0, 1))
        for child in node['children']:
            nodes = list(patches[id(child)].node.values())
            x, y = (f[a] + half * (sum(n[a] for n in nodes) / len(nodes) > f[a] + half) for a in (0, 1))
            place(child, (x, y, min(x + half, width), min(y + half, height)))
    place(tiles[0], (0, 0, width, height))
    leaves = [node for node in tiles if not node['children']]
    depth = leaves[0]['depth']
    assert all(leaf['depth'] == depth and leaf['error'] == 0 for leaf in leaves), 'uniform full-detail leaves'
    inventory = {node['uri'] for node in tiles}
    assert len(inventory) == len(tiles), 'one payload per tile'
    # Structure: one stored position per lattice node in each level (a level's
    # tiles share one grid; coarse levels round the same nodes more coarsely),
    # upward triangles, children partition their parent's footprint, and boxes
    # nest and contain content. Every view is a meshopt stream and every coarse
    # tile quantized; only leaves may keep Float32 positions.
    stored = {}
    west, south = report['bounds'][:2]
    for node in tiles:
        patch = patches[id(node)]
        assert patch.compressed, 'uncompressed terrain view'
        assert patch.scale is not None or not node['children'], 'coarse tile without quantized positions'
        for key, position in patch.position.items():
            assert stored.setdefault((node['depth'], key), position) == position, 'shared node changed stored coordinates'
        for triangle in patch.surface_triangles:
            a, b, c = (transform(frame, patch.position[n]) for n in triangle)
            u, v = [b[i] - a[i] for i in range(3)], [c[i] - a[i] for i in range(3)]
            normal = (u[1]*v[2] - u[2]*v[1], u[2]*v[0] - u[0]*v[2], u[0]*v[1] - u[1]*v[0])
            lon = math.radians(west + sum(n[0] for n in triangle) / 3 * report['sourcePixelDegrees'][0])
            lat = math.radians(south + sum(n[1] for n in triangle) / 3 * report['sourcePixelDegrees'][1])
            up = (math.cos(lat) * math.cos(lon), math.cos(lat) * math.sin(lon), math.sin(lat))
            assert sum(n * u for n, u in zip(normal, up)) > 0, 'reversed or collapsed surface triangle'
        x0, y0, x1, y1 = patch.footprint
        if node['children']:
            assert all(node['error'] >= child['error'] for child in node['children']), 'geometricErrorOrder'
            area = sum((c[2] - c[0]) * (c[3] - c[1]) for c in (patches[id(child)].footprint for child in node['children']))
            assert area <= (x1 - x0) * (y1 - y0), 'children partition the parent footprint'
            assert all(x0 <= c[0] and c[2] <= x1 and y0 <= c[1] and c[3] <= y1
                       for c in (patches[id(child)].footprint for child in node['children'])), 'child footprint outside parent'
            for child in node['children']:
                for axis, half in [(0, 3), (1, 7), (2, 11)]:
                    assert node['box'][axis] - node['box'][half] <= child['box'][axis] - child['box'][half], 'child box below parent minimum'
                    assert node['box'][axis] + node['box'][half] >= child['box'][axis] + child['box'][half], 'child box above parent maximum'
        contains_box(node['box'], patch.local + [p[0] for p in patch.skirt_points])
        # A count and outward winding alone permit a duplicated face plus a
        # hole. Census the exact oriented two-triangle stencil of every cell,
        # including clipped coarse cells, for all source-height fixtures.
        # A cell triangle is present exactly when its three nodes are.
        rotate = lambda t: min(t[k:]+t[:k] for k in range(3))
        expected = Counter()
        for y0, y1 in zip(patch.ys, patch.ys[1:]):
            for x0, x1 in zip(patch.xs, patch.xs[1:]):
                for t in (((x0,y0),(x1,y0),(x0,y1)), ((x1,y0),(x1,y1),(x0,y1))):
                    expected[rotate(t)] += all(n in patch.position for n in t)
        expected = +expected
        assert Counter(rotate(t) for t in patch.surface_triangles) == expected, 'missing/duplicated/reversed terrain cells'
    # Certified error: dense parametric distance from each tile to the leaf mesh.
    for node in tiles:
        if not node['children']:
            continue
        patch, measured = patches[id(node)], 0.
        x0, y0, x1, y1 = patch.footprint
        for leaf in leaves:
            fine = patches[id(leaf)]
            lx0, ly0, lx1, ly1 = fine.footprint
            if not (x0 <= lx0 and lx1 <= x1 and y0 <= ly0 and ly1 <= y1):
                continue
            for j in range((ly1 - ly0) * samples + 1):
                for i in range((lx1 - lx0) * samples + 1):
                    x, y = lx0 + i / samples, ly0 + j / samples
                    a, b = patch.at(x, y), fine.at(x, y)
                    if a and b: measured = max(measured, math.dist(a, b))
        node['measured'] = measured
        assert node['error'] >= measured - 1e-9, f'understated geometric error {node["error"]} < {measured}'
    # Skirts: on interior edges only, outward, and their decoded bottoms at least
    # max(e(t) + e(beside), 1) below the decoded surface, e(beside) being the
    # largest error of a non-ancestor across the edge (only geodetic arithmetic
    # is tolerated). Only segments with a triangle in t and its same-level
    # neighbour are skirted.
    root_error = tiles[0]['error']
    tolerance = 1e-6
    def across(node, axis, value, low, high, same_level=False):
        lineage, n = set(), node
        while n: lineage.add(id(n)); n = n['parent']
        a = 0 if axis == 'x' else 1
        far = a + 2 if value == patches[id(node)].footprint[a] else a
        for m in tiles:
            f = patches[id(m)].footprint
            if (id(m) not in lineage and f[far] == value and f[1 - a] < high and low < f[3 - a]
                    and (not same_level or m['depth'] == node['depth'])):
                yield m
    edges = lambda p: {frozenset(pair) for t in p.surface_triangles for pair in ((t[0], t[1]), (t[1], t[2]), (t[2], t[0]))}
    for node in tiles:
        patch = patches[id(node)]
        x0, y0, x1, y1 = patch.footprint
        lines = [('x', x0, x0 > 0), ('x', x1, x1 < width), ('y', y0, y0 > 0), ('y', y1, y1 < height)]
        interior = [(axis, value) for axis, value, inside in lines if inside]
        span = {'x': (y0, y1), 'y': (x0, x1)}
        beside = {(axis, value): max((m['error'] for m in across(node, axis, value, *span[axis])), default=None)
                  for axis, value in interior}
        covered = set()
        centre = [sum(p[i] for p in patch.local) / len(patch.local) for i in range(3)]
        for triangle in patch.skirt:
            nodes = {point[1] for point in triangle}
            edge = [(axis, value) for axis, value in interior
                    if all(n[0 if axis == 'x' else 1] == value for n in nodes)]
            assert edge and len(nodes) == 2 and beside[edge[0]] is not None, 'skirt triangle off an interior edge'
            tops = [p for p in triangle if p[1] in patch.height and p[2] == patch.height[p[1]]]
            assert all(p[0] == patch.position[p[1]] for p in tops), 'skirt top is not the surface vertex'
            for point in triangle:
                drop = patch.height[point[1]] - point[2]
                assert drop == 0 or drop >= max(node['error'] + beside[edge[0]], 1.) - tolerance, 'skirt depth rule'
            a, b, c = (p[0] for p in triangle)
            u, v = [b[i] - a[i] for i in range(3)], [c[i] - a[i] for i in range(3)]
            normal = (u[1]*v[2] - u[2]*v[1], u[2]*v[0] - u[0]*v[2])
            middle = [(a[i] + b[i] + c[i]) / 3 - centre[i] for i in range(2)]
            assert normal[0] * middle[0] + normal[1] * middle[1] > 0, 'skirt faces inward'
            if len(tops) == 2:
                covered.add((edge[0], tuple(sorted(nodes))))
            elif len(tops) == 1:
                covered.add((edge[0], tuple(sorted(nodes)), 'lower'))
        mine = edges(patch)
        for axis, value in interior:
            along = patch.ys if axis == 'x' else patch.xs
            theirs = set().union(*(edges(patches[id(m)]) for m in across(node, axis, value, *span[axis], True)))
            for a, b in zip(along, along[1:]):
                segment = tuple(sorted([(value, a), (value, b)] if axis == 'x' else [(a, value), (b, value)]))
                if frozenset(segment) in mine and frozenset(segment) in theirs:
                    assert ((axis, value), segment) in covered and ((axis, value), segment, 'lower') in covered, 'missing skirt'
                else:
                    assert ((axis, value), segment) not in covered, 'skirt along NoData'
    # Receipt.
    actual = {p.relative_to(directory).as_posix() for p in directory.rglob('*.glb')}
    assert inventory == actual, 'manifest GLB inventory'
    assert report['levels'] == depth + 1 and report['tiles'] == len(tiles), 'report tile inventory'
    assert report['tilesPerLevel'] == [sum(n['depth'] == d for n in tiles) for d in range(depth + 1)], 'report level inventory'
    assert report['geometricErrorMetres'] == [max(n['error'] for n in tiles if n['depth'] == d) for d in range(depth + 1)], 'report level errors'
    assert report['vertices'] == sum(len(p.local) for p in patches.values()), 'report geometry inventory'
    assert report['skirtVertices'] == sum(len(p.skirt_points) for p in patches.values()), 'report skirt inventory'
    triangles = sum(len(patches[id(leaf)].surface_triangles) for leaf in leaves)
    assert triangles <= 2 * width * height, 'report source-cell inventory'
    assert report['rootGeometricErrorMetres'] == root['geometricError'] == root_error, 'root error report'
    assert report['tilesetGeometricErrorMetres'] == manifest['geometricError'] >= 2 * root_error, 'tileset error report'
    members = {p.relative_to(directory).as_posix() for p in directory.rglob('*') if p.is_file()}
    assert all(m in inventory or m in ('tileset.json', 'conversion.json') or m.endswith('.subtree')
               or m.startswith('tileset-d') for m in members), 'unexpected directory member inventory'
    assert report['generatedBytes'] == sum((directory/member).stat().st_size for member in members), 'report byte inventory'
    return {'leaf_tiles': len(leaves), 'tiles': len(tiles), 'triangles': triangles,
            'certified': {str(n['depth']): max(m.get('measured', 0.) for m in tiles if m['depth'] == n['depth'])
                          for n in tiles}}


def check_plane(directory, width=4, height=4, pixel=.25, west=10., north_edge=21.):
    """Complete independent node and oriented-cell inventory for t1-plane.tif leaves.
    Every vertex is its analytic lattice position as stored: Float32-rounded, or
    within half a quantization step per axis; a level's tiles agree on shared nodes."""
    from collections import Counter
    directory=pathlib.Path(directory)
    report=json.loads((directory/'conversion.json').read_text())
    south=north_edge-height*pixel;east_edge=west+width*pixel
    assert report['bounds']==[west,south,east_edge,north_edge] and (report['width'],report['height'])==(width,height)
    _,root,tiles=tree(directory)
    lon,lat=math.radians((west+east_edge)/2),math.radians((south+north_edge)/2)
    east=(-math.sin(lon),math.cos(lon),0.)
    north=(-math.sin(lat)*math.cos(lon),-math.sin(lat)*math.sin(lon),math.cos(lat))
    up=(math.cos(lat)*math.cos(lon),math.cos(lat)*math.sin(lon),math.sin(lat))
    origin=ecef((west+east_edge)/2,(south+north_edge)/2,0.)
    expected_transform=[*east,0.,*north,0.,*up,0.,*origin,1.]
    assert all(abs(a-b)<=1e-8 for a,b in zip(root['transform'],expected_transform)), 'root ENU frame'
    expected,exact={},{}
    for j in range(height+1):
        for i in range(width+1):
            sample_height=100.+2*max(0.,min(width-1.,i-.5))+3*max(0.,min(height-1.,height-j-.5))+report['heightOffsetMetres']
            xyz=ecef(west+i*pixel,south+j*pixel,sample_height)
            relative=[a-b for a,b in zip(xyz,origin)]
            enu=[sum(a*b for a,b in zip(relative,axis)) for axis in [east,north,up]]
            exact[(i,j)]=(enu[0],enu[2],-enu[1])
            expected[(i,j)]=tuple(struct.unpack('<f',struct.pack('<f',v))[0] for v in exact[(i,j)])
    # The index codec may rotate a triangle's corners; winding is what counts.
    rotated=lambda t:min(t[k:]+t[:k] for k in range(3))
    found=Counter();seen={}
    depth=max(n['depth'] for n in tiles)
    for node in sorted(tiles,key=lambda n:-n['depth']):
        patch=Patch(directory,node,report,root['transform'])
        reference=expected if patch.scale is None else exact
        limits=[.002]*3 if patch.scale is None else [step/2+1e-9 for step in patch.scale]
        def identify(p):
            key=min(reference,key=lambda k:sum((a-b)**2 for a,b in zip(p,reference[k])))
            assert all(abs(a-b)<=limit for a,b,limit in zip(p,reference[key],limits)), 'source elevation/position'
            if (node['depth'],key) in seen: assert seen[(node['depth'],key)]==p, 'shared seam changed stored coordinates'
            seen[(node['depth'],key)]=p
            return key
        for p in patch.glb_surface: identify(p)
        if not node['children']:
            for triangle in patch.triangles: found[rotated(tuple(identify(p) for p in triangle))]+=1
    wanted=Counter()
    for j in range(height):
        for i in range(width):
            wanted[rotated(((i,j),(i+1,j),(i,j+1)))]+=1
            wanted[rotated(((i+1,j),(i+1,j+1),(i,j+1)))]+=1
    assert found==wanted, 'missing/duplicated/reversed terrain cells'
    leaf_nodes={key for level,key in seen if level==depth}
    assert leaf_nodes==set(expected), 'source lattice nodes missing'
    return dict(nodes=len(leaf_nodes),triangles=sum(found.values()))


def edit_row(copy, node, box=None, error=None):
    """Overwrite one implicit tile's TILE_BOUNDING_BOX or TILE_GEOMETRIC_ERROR row."""
    path=copy/node['row']['subtree'];data=bytearray(path.read_bytes())
    json_length=struct.unpack_from('<Q',data,8)[0];document=json.loads(data[24:24+json_length])
    schema=json.loads((copy/'tileset.json').read_text())['schema']
    table=document['propertyTables'][document['tileMetadata']]
    semantics={v.get('semantic'):k for k,v in schema['classes'][table['class']]['properties'].items()}
    for semantic,value,size in [('TILE_BOUNDING_BOX',box,96),('TILE_GEOMETRIC_ERROR',error,8)]:
        if value is None: continue
        view=document['bufferViews'][table['properties'][semantics[semantic]]['values']]
        at=24+json_length+view['byteOffset']+node['row']['rank']*size
        struct.pack_into('<%dd'%(size//8),data,at,*(value if box is not None and semantic=='TILE_BOUNDING_BOX' else [value]))
    path.write_bytes(data)


def rewrite_tile(path, mutate):
    """Re-store a tile after ``mutate(document, views)`` edits its decoded views
    (bytearrays) in place; every view becomes a new valid meshopt stream."""
    document, binary = glb_parts(path.read_bytes())
    views = [bytearray(view) for view in decoded_views(document, binary)]
    mutate(document, views)
    out, fallback = bytearray(), 0
    for view, raw in zip(document['bufferViews'], views):
        stream = view['extensions'][MESHOPT]
        stride = stream['byteStride']
        count = len(raw) // stride
        if stream['mode'] == 'ATTRIBUTES':
            encoded = meshopt.encode_attributes(bytes(raw), count, stride)
        else:
            encoded = meshopt.encode_triangles(meshopt.unpack_indices(bytes(raw), stride))
        out += bytes(-len(out) % 4)
        stream.update(byteOffset=len(out), byteLength=len(encoded), count=count)
        out += encoded
        view.update(byteOffset=fallback, byteLength=len(raw))
        fallback += len(raw) + (-len(raw) % 4)
    out += bytes(-len(out) % 4)
    document['buffers'][0]['byteLength'] = len(out)
    document['buffers'][1]['byteLength'] = fallback
    text = json.dumps(document).encode()
    text += b' ' * (-len(text) % 4)
    path.write_bytes(struct.pack('<4sII', b'glTF', 2, 28 + len(text) + len(out)) + struct.pack('<I4s', len(text), b'JSON')
                     + text + struct.pack('<I4s', len(out), b'BIN\x00') + bytes(out))


def restamp_bytes(directory):
    """Make conversion.json's byte receipt match re-stored payloads, so payload
    controls are not rejected by the receipt alone."""
    path = directory/'conversion.json'
    report = json.loads(path.read_text())
    others = sum(p.stat().st_size for p in directory.rglob('*') if p.is_file() and p != path)
    while True:
        text = json.dumps(report, indent=2)
        if report['generatedBytes'] == others + len(text.encode()):
            break
        report['generatedBytes'] = others + len(text.encode())
    path.write_text(text)


def corruption_controls(directory, plane=True, **plane_spec):
    """Demonstrate source, frame, topology, seams, bounds, error, skirt and receipt sensitivity.

    Payload faults edit decoded views and re-store them as new meshopt streams;
    the identity re-store is accepted first, so each rejection is the fault's.
    Without the analytic plane reference, the frame control is omitted."""
    import shutil, tempfile
    directory=pathlib.Path(directory)
    check_directory(directory)
    if plane: check_plane(directory,**plane_spec)
    _,_,tiles=tree(directory)
    report=json.loads((directory/'conversion.json').read_text())
    leaves=[n for n in tiles if not n['children']]
    faults=['position','winding','bounds']+(['frame'] if plane else [])+['row','missing-triangle','duplicate-triangle','report-inventory']
    if len(leaves)>1: faults.append('seam')
    # Pyramid controls need a coarse tile below the root.
    middle=[n for n in tiles if n['children'] and n['parent'] is not None and 'row' in n]
    if middle: faults+=['understated-error','missing-skirt','leaf-box-misses-skirt']
    # Each fault's admissible rejection reasons (the plane reference sees some first).
    expected={'understated-error':('understated geometric error',),'missing-skirt':('missing skirt',),
              'leaf-box-misses-skirt':('content outside box',),'bounds':('level-0 row box',),
              'frame':('root ENU frame',),'report-inventory':('report tile inventory',),
              'position':('source elevation/position','understated geometric error'),
              'winding':('missing/duplicated/reversed terrain cells','reversed or collapsed surface triangle'),
              'row':('missing/duplicated/reversed terrain cells','surface vertices form one lattice'),
              'missing-triangle':('missing/duplicated/reversed terrain cells','skirt triangle off an interior edge'),
              'duplicate-triangle':('missing/duplicated/reversed terrain cells',),
              'seam':('shared seam changed stored coordinates','shared node changed stored coordinates')}
    def patch_of(node,manifest):
        return Patch(directory,node,report,manifest['root']['transform'])
    reasons={}
    for fault in ['identity']+faults:
        with tempfile.TemporaryDirectory() as temporary:
            copy=pathlib.Path(temporary)/'candidate';shutil.copytree(directory,copy)
            manifest=json.loads((copy/'tileset.json').read_text())
            if fault=='identity':
                for node in tiles: rewrite_tile(copy/node['uri'],lambda document,views:None)
            elif fault in ['bounds','frame']:
                if fault=='bounds': manifest['root']['boundingVolume']['box'][3]=0.
                else: manifest['root']['transform'][12]+=10.
                (copy/'tileset.json').write_text(json.dumps(manifest))
            elif fault=='report-inventory':
                path=copy/'conversion.json';changed=json.loads(path.read_text());changed['tiles']+=1
                path.write_text(json.dumps(changed))
            elif fault=='understated-error':
                # Still monotone over its leaf children, but below the measured distance.
                node=max((n for n in middle if not any(c['children'] for c in n['children'])),key=lambda n:n['error'])
                edit_row(copy,node,error=node['error']/4)
            elif fault=='leaf-box-misses-skirt':
                # Lift a skirted leaf's box floor to its lowest surface vertex. The leaf has
                # no child boxes, so only skirt containment can reject it.
                node=next(n for n in leaves if 'row' in n and patch_of(n,manifest).skirt)
                patch=patch_of(node,manifest)
                floor=min(p[2] for p in patch.local);box=list(node['box'])
                top=box[2]+box[11];box[2]=(floor+top)/2;box[11]=(top-floor)/2
                edit_row(copy,node,box=box)
            elif fault=='missing-skirt':
                # Drop the skirt triangles and their bottom vertices, which follow the surface.
                node=next(n for n in middle if patch_of(n,manifest).skirt)
                patch=patch_of(node,manifest)
                vertices,indices=len(patch.glb_surface),3*len(patch.triangles)
                def drop(document,views):
                    primitive=document['meshes'][0]['primitives'][0]
                    position=document['accessors'][primitive['attributes']['POSITION']]
                    index=document['accessors'][primitive['indices']]
                    stride=document['bufferViews'][position['bufferView']]['byteStride']
                    width=document['bufferViews'][index['bufferView']]['extensions'][MESHOPT]['byteStride']
                    kept=meshopt.unpack_indices(bytes(views[index['bufferView']]),width)[:indices]
                    assert max(kept)<vertices, 'surface triangles come first'
                    del views[position['bufferView']][vertices*stride:]
                    views[index['bufferView']][:]=meshopt.pack_indices(kept,width)
                    position['count'],index['count']=vertices,indices
                    if position['componentType']==5123:
                        values=[struct.unpack_from('<3H',views[position['bufferView']],i*stride) for i in range(vertices)]
                    else:
                        values=[struct.unpack_from('<3f',views[position['bufferView']],i*stride) for i in range(vertices)]
                    position['min'],position['max']=[min(v[a] for v in values) for a in range(3)],[max(v[a] for v in values) for a in range(3)]
                rewrite_tile(copy/node['uri'],drop)
            else:
                leaf=leaves[1 if fault=='seam' else 0]
                def corrupt(document,views):
                    primitive=document['meshes'][0]['primitives'][0]
                    position=document['accessors'][primitive['attributes']['POSITION']]
                    index=document['accessors'][primitive['indices']]
                    stride=document['bufferViews'][position['bufferView']]['byteStride']
                    vertices=views[position['bufferView']]
                    quantized=position['componentType']==5123
                    if fault in ['position','seam']:
                        if not quantized:
                            if fault=='position':
                                struct.pack_into('<f',vertices,0,struct.unpack_from('<f',vertices,0)[0]+10.)
                            else:
                                struct.pack_into('<I',vertices,0,struct.unpack_from('<I',vertices,0)[0]+1)
                        else:
                            # Move x by about 10 m (position) or one grid step (seam),
                            # inside the accessor range.
                            q=struct.unpack_from('<H',vertices,0)[0];high=position['max'][0]
                            step=document['nodes'][0]['scale'][0]
                            delta=max(1,round(10./step)) if fault=='position' else 1
                            struct.pack_into('<H',vertices,0,q+min(delta,high-q) if q<high else q-min(delta,q))
                    elif fault=='row':
                        # Exchange complete stored vertices, preserving count and
                        # values while corrupting their row/topology association.
                        other=(position['count']-1)*stride
                        vertices[:stride],vertices[other:other+stride]=vertices[other:other+stride],vertices[:stride]
                    else:
                        width=document['bufferViews'][index['bufferView']]['extensions'][MESHOPT]['byteStride']
                        values=meshopt.unpack_indices(bytes(views[index['bufferView']]),width)
                        if fault=='duplicate-triangle': values[6:9]=values[9:12]
                        elif fault=='missing-triangle': values[1]=values[0]
                        else: values[1],values[2]=values[2],values[1]
                        views[index['bufferView']][:]=meshopt.pack_indices(values,width)
                    # Keep the declared range consistent, so only the geometry is wrong.
                    fmt='<3H' if quantized else '<3f'
                    values=[struct.unpack_from(fmt,vertices,i*stride) for i in range(position['count'])]
                    position['min'],position['max']=[min(v[a] for v in values) for a in range(3)],[max(v[a] for v in values) for a in range(3)]
                rewrite_tile(copy/leaf['uri'],corrupt)
            if fault in ['identity','missing-skirt','position','winding','row','missing-triangle','duplicate-triangle','seam']:
                restamp_bytes(copy)
            try:
                # The duplicate-face control must exercise the generic topology
                # census even on plane fixtures, without the source-plane oracle.
                if plane and fault != 'duplicate-triangle': check_plane(copy,**plane_spec)
                check_directory(copy)
            except (AssertionError,KeyError) as error:
                assert fault!='identity', f'oracle rejected an identity re-store: {error}'
                assert any(reason in str(error) for reason in expected[fault]), f'{fault} rejected for another reason: {error}'
                reasons[fault]=str(error) or type(error).__name__
                continue
            if fault=='identity': continue
            raise AssertionError('oracle accepted corruption: '+fault)
    return {'rejected_corruptions':faults,'reasons':reasons}


def decoded_ecef_triangles(directory):
    """Full-detail leaf surface triangles in ECEF: what MostDetailed queries intersect."""
    directory = pathlib.Path(directory)
    _, root, tiles = tree(directory)
    report = json.loads((directory/'conversion.json').read_text())
    frame = root.get('transform', IDENTITY)
    triangles=[]
    for leaf in (n for n in tiles if not n['children']):
        for face in Patch(directory, leaf, report, frame).triangles:
            triangles.append(tuple(transform(frame,(p[0],-p[2],p[1])) for p in face))
    return triangles


def ray_height(triangles, lon, lat):
    # WGS84 geodetic height is distance along this normal from ellipsoid h=0.
    origin=ecef(lon,lat,0.)
    lon,lat=math.radians(lon),math.radians(lat)
    direction=(math.cos(lat)*math.cos(lon),math.cos(lat)*math.sin(lon),math.sin(lat))
    sub=lambda a,b:tuple(x-y for x,y in zip(a,b))
    dot=lambda a,b:sum(x*y for x,y in zip(a,b))
    cross=lambda a,b:(a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0])
    hits=[]
    for a,b,c in triangles:
        ab,ac=sub(b,a),sub(c,a)
        normal=cross(ab,ac)
        divisor=dot(normal,direction)
        if abs(divisor)<1e-12: continue
        height=dot(normal,sub(a,origin))/divisor
        point=tuple(o+height*d for o,d in zip(origin,direction))
        ap=sub(point,a)
        d00,d01,d11=dot(ab,ab),dot(ab,ac),dot(ac,ac)
        denominator=d00*d11-d01*d01
        if denominator<=0: continue
        v=(d11*dot(ap,ab)-d01*dot(ap,ac))/denominator
        w=(d00*dot(ap,ac)-d01*dot(ap,ab))/denominator
        if v>=-1e-9 and w>=-1e-9 and v+w<=1+1e-9: hits.append(height)
    assert hits, 'query has no decoded mesh intersection'
    assert max(hits)-min(hits)<1e-5, 'multiple incompatible mesh intersections'
    return sum(hits)/len(hits)


def browser_oracle(directory, destination):
    directory=pathlib.Path(directory)
    report=json.loads((directory/'conversion.json').read_text())
    west,south,east,north=report['bounds']
    triangles=decoded_ecef_triangles(directory)
    samples=[]
    outside=[dict(longitude=west-(east-west)*.25,latitude=(south+north)/2)]
    # Interior irrational fractions exercise triangles rather than just vertices;
    # one over a NoData hole has no surface.
    for u,v in [(0.173,0.237),(0.503,0.507),(0.431,0.823)]:
        lon,lat=west+(east-west)*u,south+(north-south)*v
        try:
            samples.append(dict(longitude=lon,latitude=lat,height=ray_height(triangles,lon,lat),tolerance=.05))
        except AssertionError:
            outside.append(dict(longitude=lon,latitude=lat))
    # Constant-latitude Cartesian boundary chords bow poleward. A source ROI
    # guard must reject this point even though a raw decoded-mesh ray hits it.
    # The spherical expression chooses an interior fraction of the bow; the
    # independent WGS84/actual stored-mesh intersection proves the control.
    step=report['sourcePixelDegrees'][0]
    half_step=math.radians(step/2)
    for edge in [north,south]:
        bow=math.degrees(math.atan(math.tan(math.radians(edge))/math.cos(half_step)))-edge
        latitude=edge+bow/4
        if south<=latitude<=north or latitude==edge:
            continue
        longitude=west+step/2
        try:
            underlying_height=ray_height(triangles,longitude,latitude)
        except AssertionError:
            continue
        outside.append(dict(longitude=longitude,latitude=latitude,
                            underlyingMeshHeight=underlying_height,tolerance=.05))
        break
    oracle=dict(samples=samples,outside=outside,
                minimumLeaves=sum(not node['children'] for node in tree(directory)[2]))
    pathlib.Path(destination).write_text(json.dumps(oracle,indent=2)+'\n')
    return oracle


def write_fixture(path, case='plane', width=4, height=4, pixel=.25, options=None):
    from osgeo import gdal, osr
    dataset = gdal.GetDriverByName('GTiff').Create(str(path), width, height, 1, gdal.GDT_Float64, options=options or [])
    dataset.SetGeoTransform([10., pixel, 0., 21., 0., -pixel])
    crs = osr.SpatialReference(); crs.ImportFromEPSG(4326)
    dataset.SetProjection(crs.ExportToWkt())
    # Pixel centre plane h=100+2*column+3*row, north-down source order.
    values = [100.+2*column+3*row for row in range(height) for column in range(width)]
    if case == 'nodata':
        values[5] = -9999.; dataset.GetRasterBand(1).SetNoDataValue(-9999.)
    elif case == 'constant':
        values = [42.] * (width*height)
    elif case == 'all-invalid':
        values = [-9999.] * (width*height); dataset.GetRasterBand(1).SetNoDataValue(-9999.)
    elif case == 'extreme-height':
        values=[9001.] * (width*height)
    elif case == 'nonfinite':
        values[5] = float('inf')
    elif case == 'bump':
        # Smooth relief with a slope: every coarse level has a nonzero error.
        values = [300.+40*math.sin(column/5)*math.cos(row/7)+.5*column for row in range(height) for column in range(width)]
    elif case != 'plane':
        raise ValueError(case)
    dataset.GetRasterBand(1).SetUnitType('m')
    dataset.GetRasterBand(1).WriteRaster(0,0,width,height,struct.pack('<'+str(width*height)+'d',*values),buf_type=gdal.GDT_Float64)
    dataset = None


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('directory')
    parser.add_argument('--browser-oracle')
    parser.add_argument('--browser-imagery',action='store_true')
    parser.add_argument('--plane',action='store_true')
    parser.add_argument('--corruption-controls',action='store_true')
    arguments = parser.parse_args()
    print(json.dumps(check_directory(arguments.directory), sort_keys=True))
    if arguments.plane:
        print(json.dumps(check_plane(arguments.directory),sort_keys=True))
    if arguments.corruption_controls:
        print(json.dumps(corruption_controls(arguments.directory),sort_keys=True))
    if arguments.browser_oracle:
        result=browser_oracle(arguments.directory,arguments.browser_oracle)
        result['imagery']=arguments.browser_imagery
        pathlib.Path(arguments.browser_oracle).write_text(json.dumps(result,indent=2)+'\n')
