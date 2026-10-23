"""Finite 0.4 vector oracle: published bytes only, no producer/build-state imports.

Stdlib forest/metadata/accessor reader plus the independent specification-written
meshopt decoder. Intentionally limited to the producer's quadtree/box/TS profile;
this is source fidelity evidence, not a general 3D Tiles validator.
"""
import collections
import json
import itertools
import math
import struct
import zipfile

from ext_meshopt_spec import decode_attributes
from t1_terrain_oracle import IDENTITY, contains_box, ecef, glb_parts, morton, transform


def multiply(a, b):
    return [sum(a[r + 4*k]*b[k + 4*c] for k in range(4))
            for c in range(4) for r in range(4)]


def inverse_transform(matrix, point):
    axes = [matrix[i:i+3] for i in (0,4,8)]
    def cross3(a,b):
        return (a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0])
    def dot(a,b):
        return sum(x*y for x,y in zip(a,b))
    dual = [cross3(axes[1],axes[2]),cross3(axes[2],axes[0]),cross3(axes[0],axes[1])]
    determinant = dot(axes[0],dual[0])
    assert abs(determinant) > 1e-20
    delta = [point[i]-matrix[12+i] for i in range(3)]
    return tuple(dot(delta,axis)/determinant for axis in dual)


def node_matrix(node):
    if 'matrix' in node:
        return node['matrix']
    assert 'rotation' not in node, 'finite TS node profile'
    m = list(IDENTITY)
    for i, value in enumerate(node.get('scale', [1, 1, 1])):
        m[5*i] = value
    m[12:15] = node.get('translation', [0, 0, 0])
    return m


def columns(table, schema, view):
    result = [{} for _ in range(table['count'])]
    for name, column in table['properties'].items():
        spec = schema['classes'][table['class']]['properties'][name]
        raw = view(column['values'])
        if spec['type'] == 'STRING':
            fmt = {'UINT32': 'I', 'UINT64': 'Q'}[column.get('stringOffsetType', 'UINT32')]
            offsets = struct.unpack('<' + fmt*(table['count'] + 1), view(column['stringOffsets']))
            values = [raw[a:b].decode('utf8') for a, b in zip(offsets, offsets[1:])]
        elif spec['type'] == 'BOOLEAN':
            values = [bool(raw[i//8] & (1 << (i % 8))) for i in range(table['count'])]
        else:
            fmt = {'INT64': 'q', 'UINT64': 'Q', 'FLOAT64': 'd', 'FLOAT32': 'f'}[spec['componentType']]
            width = {'SCALAR': 1, 'VEC3': 3, 'MAT4': 16}.get(spec['type'])
            # TILE_BOUNDING_BOX is a fixed-size array of twelve FLOAT64 values.
            width = spec.get('count', width) if spec.get('array') else width
            size = struct.calcsize(fmt)*width
            values = [struct.unpack_from('<' + fmt*width, raw, i*size) for i in range(table['count'])]
            if width == 1:
                values = [v[0] for v in values]
        for row, value in zip(result, values):
            row[name] = None if value == spec.get('noData', object()) else value
    return result


def forest(archive):
    """Expand explicit links and actual subtree availability/metadata, never vector-build.json."""
    documents, members = set(), set()
    def contents(node):
        return node.get('contents', [node['content']] if 'content' in node else [])

    def expand(tile, schema, frame):
        frame = multiply(frame, tile.get('transform', IDENTITY))
        if 'implicitTiling' not in tile:
            links = contents(tile)
            if links and links[0]['uri'].endswith('.json'):
                assert len(links) == 1 and not tile.get('children')
                name = links[0]['uri']
                assert name not in documents
                documents.add(name)
                doc = json.loads(archive.read(name))
                return expand(doc['root'], doc.get('schema', schema), frame)
            return dict(box=tile['boundingVolume']['box'], error=tile['geometricError'],
                        frame=frame, extras=tile.get('extras', {}), uris=[c['uri'] for c in contents(tile)],
                        children=[expand(c, schema, frame) for c in tile.get('children', [])])
        spec = tile['implicitTiling']
        assert spec['subdivisionScheme'] == 'QUADTREE'
        levels = spec['subtreeLevels']
        assert 1 <= levels <= 4 and spec['availableLevels'] == levels
        data = archive.read(spec['subtrees']['uri'].format(level=0, x=0, y=0))
        magic, version, jl, bl = struct.unpack_from('<4sIQQ', data)
        assert (magic, version) == (b'subt', 1) and 24 + jl + bl == len(data)
        doc, binary = json.loads(data[24:24+jl]), data[24+jl:]
        assert doc['childSubtreeAvailability'] == {'constant': 0}
        def view(i):
            v = doc['bufferViews'][i]
            return binary[v.get('byteOffset', 0):v.get('byteOffset', 0)+v['byteLength']]
        def bit(declaration, i):
            return bool(declaration['constant']) if 'constant' in declaration else bool(view(declaration['bitstream'])[i//8] & (1 << (i % 8)))
        table = doc['propertyTables'][doc['tileMetadata']]
        rows = columns(table, schema, view)
        semantics = {s.get('semantic'): n for n, s in schema['classes'][table['class']]['properties'].items()}
        declarations = doc['contentAvailability']
        declarations = declarations if isinstance(declarations, list) else [declarations]
        templates = contents(tile)
        assert len(templates) == len(declarations)
        nodes, rank = {}, 0
        for level in range(levels):
            for index in range(4**level):
                b = (4**level-1)//3 + index
                if not bit(doc['tileAvailability'], b):
                    assert not any(bit(d, b) for d in declarations)
                    continue
                x, y = morton(index, level)
                row = rows[rank]
                nodes[level, x, y] = dict(box=row[semantics['TILE_BOUNDING_BOX']],
                    error=row[semantics['TILE_GEOMETRIC_ERROR']], frame=frame, extras=json.loads(row.get('extras', '{}')),
                    uris=[t['uri'].format(level=level, x=x, y=y) for t, d in zip(templates, declarations) if bit(d, b)], children=[])
                rank += 1
        assert rank == len(rows)
        for (level, x, y), node in nodes.items():
            if level:
                nodes[level-1, x//2, y//2]['children'].append(node)
        top = nodes[0, 0, 0]
        assert list(top['box']) == tile['boundingVolume']['box']
        assert top['error'] == tile['geometricError']
        return top

    doc = json.loads(archive.read('tileset.json'))
    root = expand(doc['root'], doc.get('schema'), IDENTITY)
    def walk(node, ancestors):
        assert math.isfinite(node['error']) and node['error'] >= 0
        if ancestors:
            assert node['error'] <= ancestors[-1]['error'] + 1e-8
        for uri in node['uris']:
            assert uri not in members, 'duplicate published payload placement'
            members.add(uri)
        yield node, ancestors
        for child in node['children']:
            yield from walk(child, ancestors+[node])
    nodes = list(walk(root, []))
    assert members == {n for n in archive.namelist() if n.endswith('.glb')}, 'published inventory'
    return nodes


def primitives(data):
    doc, binary = glb_parts(data)
    views = []
    for v in doc['bufferViews']:
        ext = v.get('extensions', {}).get('EXT_meshopt_compression')
        if ext:
            assert ext['mode'] == 'ATTRIBUTES' and ext.get('filter', 'NONE') == 'NONE'
            start = ext.get('byteOffset', 0)
            raw = decode_attributes(binary[start:start+ext['byteLength']], ext['count'], ext['byteStride'])
        else:
            start = v.get('byteOffset', 0)
            raw = binary[start:start+v['byteLength']]
        assert len(raw) == v['byteLength']
        views.append(raw)
    def accessor(i):
        a = doc['accessors'][i]
        fmt = {5121: 'B', 5123: 'H', 5125: 'I', 5126: 'f'}[a['componentType']]
        width = {'SCALAR': 1, 'VEC3': 3}[a['type']]
        stride = doc['bufferViews'][a['bufferView']].get('byteStride', struct.calcsize(fmt)*width)
        values = [struct.unpack_from('<'+fmt*width, views[a['bufferView']], a.get('byteOffset', 0)+j*stride) for j in range(a['count'])]
        if a.get('normalized'):
            assert a['componentType'] == 5123
            values = [tuple(v/65535 for v in row) for row in values]
        return [v[0] for v in values] if width == 1 else values
    meta = doc['extensions']['EXT_structural_metadata']
    tables = [columns(t, meta['schema'], lambda i: views[i]) for t in meta['propertyTables']]
    def walk(i, parent):
        node = doc['nodes'][i]
        matrix = multiply(parent, node_matrix(node))
        if 'mesh' in node:
            for p in doc['meshes'][node['mesh']]['primitives']:
                mapping, = p['extensions']['EXT_mesh_features']['featureIds']
                ids = accessor(p['attributes']['_FEATURE_ID_' + str(mapping['attribute'])])
                pos = [transform(matrix, v) for v in accessor(p['attributes']['POSITION'])]
                # glTF y-up -> 3D Tiles z-up, after the complete glTF node frame.
                pos = [(x, -z, y) for x, y, z in pos]
                indices = accessor(p['indices']) if 'indices' in p else list(range(len(pos)))
                mode = p.get('mode', 4)
                if mode == 0:
                    groups = [(i,) for i in indices]
                elif mode == 4:
                    assert len(indices) % 3 == 0
                    groups = [indices[i:i+3] for i in range(0, len(indices), 3)]
                else:
                    assert mode == 3
                    groups, previous = [], None
                    for i in indices:
                        if i == 0xffffffff:
                            previous = None
                            continue
                        if previous is not None:
                            groups.append((previous, i))
                        previous = i
                for group in groups:
                    assert len({ids[i] for i in group}) == 1, 'cross-feature join'
                    row = tables[mapping['propertyTable']][ids[group[0]]]
                    yield mode, row, tuple(pos[i] for i in group)
        for child in node.get('children', []):
            yield from walk(child, matrix)
    for i in doc['scenes'][doc.get('scene', 0)]['nodes']:
        yield from walk(i, IDENTITY)


def decode(path):
    result = collections.defaultdict(list)
    with zipfile.ZipFile(path) as archive:
        report = json.loads(archive.read('conversion.json'))
        nodes = forest(archive)
        rounding = max(n['extras'].get('positionRoundingMetres', 0) for n,_ in nodes)
        assert math.isfinite(rounding) and 0 <= rounding <= 1e-4, 'finite fixture rounding'
        report['oracleMaximumRoundingMetres'] = rounding
        for node, ancestors in nodes:
            for uri in node['uris']:
                for mode, row, positions in primitives(archive.read(uri)):
                    world = [transform(node['frame'], p) for p in positions]
                    for owner in ancestors+[node]:
                        contains_box(owner['box'], [inverse_transform(owner['frame'], p) for p in world])
                    if not node['children']:
                        result[row['_source_id']].append((mode, row, world))
    return dict(result), report


def cross(a, b, c):
    return (b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])


def clip_area(triangle, rectangle):
    """Exact convex clipping against an analytic rectangle; no topology library."""
    vertices = list(triangle)
    x0, y0, x1, y1 = rectangle
    for axis, value, sign in [(0,x0,1),(0,x1,-1),(1,y0,1),(1,y1,-1)]:
        clipped = []
        for a, b in zip(vertices, vertices[1:]+vertices[:1]):
            da, db = (a[axis]-value)*sign, (b[axis]-value)*sign
            if da >= 0:
                clipped.append(a)
            if (da < 0) != (db < 0):
                t = da/(da-db)
                clipped.append(tuple(a[i]+t*(b[i]-a[i]) for i in range(2)))
        vertices = clipped
    return abs(sum(a[0]*b[1]-b[0]*a[1] for a,b in zip(vertices, vertices[1:]+vertices[:1])))/2


def check_source(actual, expected, tolerance):
    """Expected rows carry literal points/segments or disjoint filled rectangles.

    Polygon fixtures are axis-aligned rings: triangle containment, pairwise
    interior-disjointness and total area prove their union (including the hole).
    """
    assert set(actual) == set(expected), 'missing/extra source identity'
    for identity, spec in expected.items():
        groups = actual[identity]
        mode = spec['mode']
        for m, row, points in groups:
            assert m == mode, 'geometry kind'
            assert row == spec['properties'], 'source metadata/identity'
            assert all(type(value) is type(spec['properties'][name]) for name,value in row.items()), 'source metadata type'
            assert all(math.isfinite(v) for p in points for v in p)
        if mode in (0, 3):
            observed = [points for _, _, points in groups]
            wanted = list(spec['parts'])
            assert len(observed) == len(wanted), 'point/segment multiplicity'
            for part in observed:
                distances = [max(math.dist(a,b) for a,b in zip(part, target)) for target in wanted]
                closest = min(range(len(wanted)), key=distances.__getitem__)
                assert distances[closest] <= tolerance, 'source coordinate/line coverage'
                wanted.pop(closest)
        elif 'triangles' in spec:
            wanted = list(spec['triangles'])
            assert len(groups) == len(wanted), 'repaired triangle multiplicity'
            for _, _, triangle in groups:
                matches = [i for i,target in enumerate(wanted) if any(
                    max(math.dist(a,b) for a,b in zip(triangle,permutation)) <= tolerance
                    for permutation in itertools.permutations(target))]
                assert matches, 'repaired triangle vertex bijection'
                wanted.pop(matches[0])
        else:
            triangles = [p for _, _, p in groups]
            rectangles = spec['rectangles']
            area = 0
            for triangle in triangles:
                assert max(abs(p[2]-spec['height']) for p in triangle) <= tolerance, 'polygon height'
                a = abs(cross(*triangle))/2
                assert a > 0, 'degenerate triangle'
                inside = sum(clip_area(triangle, r) for r in rectangles)
                assert abs(inside-a) <= tolerance*max(1, math.sqrt(a))*8, 'hole-fill/outside polygon'
                area += a
            expected_area = sum((x1-x0)*(y1-y0) for x0,y0,x1,y1 in rectangles)
            assert abs(area-expected_area) <= tolerance*max(1, math.sqrt(expected_area))*16, 'polygon coverage area'
            # Distinct triangles must not overlap with positive area. SAT is a
            # complete convex intersection test; shrink by tolerance for shared edges.
            for i, a in enumerate(triangles):
                for b in triangles[:i]:
                    separated = False
                    for poly in (a,b):
                        for p,q in zip(poly,poly[1:]+poly[:1]):
                            axis = (q[1]-p[1], p[0]-q[0])
                            aa = [v[0]*axis[0]+v[1]*axis[1] for v in a]
                            bb = [v[0]*axis[0]+v[1]*axis[1] for v in b]
                            if min(max(aa),max(bb))-max(min(aa),min(bb)) <= tolerance*math.hypot(*axis)*2:
                                separated = True
                    assert separated, 'overlapping polygon triangles'
