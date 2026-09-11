"""GeoJSON → glTF vector content prototype, pinned to the 2026 draft.

Whole features are partitioned spatially without clipping or simplification.
The glTF extensions preserve polygon rings, holes, feature IDs and scalar
properties. GDAL/GEOS triangulates in a best-fit plane while retaining source
3D positions. Optional repairs and original-outline fallbacks are reported.
"""
import argparse
import json
import math
import pathlib
import struct

import numpy as np
from osgeo import ogr

ogr.UseExceptions()


def coordinates(geometry):
    def walk(v):
        if isinstance(v, list) and v and isinstance(v[0], (float, int)):
            if len(v) not in (2, 3) or not all(math.isfinite(n) for n in v):
                raise ValueError('coordinates must be finite lon/lat[/ellipsoidal height]')
            if not (-180 <= v[0] <= 180 and -90 <= v[1] <= 90):
                raise ValueError('GeoJSON coordinates must be WGS84 longitude/latitude')
            yield [*v, 0][:3] if len(v) == 2 else v
        else:
            for child in v:
                yield from walk(child)
    return list(walk(geometry['coordinates']))


def ecef(points):
    p = np.asarray(points, dtype=float)
    lon, lat, height = np.radians(p[:, 0]), np.radians(p[:, 1]), p[:, 2]
    e2 = 6.6943799901413165e-3
    n = 6378137 / np.sqrt(1-e2*np.sin(lat)**2)
    return np.column_stack(((n+height)*np.cos(lat)*np.cos(lon), (n+height)*np.cos(lat)*np.sin(lon), (n*(1-e2)+height)*np.sin(lat)))


class Glb:
    def __init__(self):
        self.data = bytearray()
        self.doc = dict(asset=dict(version='2.0', generator='rusty-tiles glTF vector prototype'),
                        scene=0, scenes=[dict(nodes=[0])], nodes=[dict(mesh=0)], meshes=[dict(primitives=[])],
                        buffers=[dict(byteLength=0)], bufferViews=[], accessors=[],
                        extensionsUsed=['EXT_mesh_features', 'EXT_structural_metadata'])

    def view(self, data):
        data = data or b"\0"
        self.data.extend(b'\0' * (-len(self.data) % 4))
        i = len(self.doc['bufferViews'])
        self.doc['bufferViews'].append(dict(buffer=0, byteOffset=len(self.data), byteLength=len(data)))
        self.data.extend(data)
        return i

    def accessor(self, values, dtype, kind):
        values = np.asarray(values, dtype=dtype)
        view = self.view(values.tobytes())
        a = dict(bufferView=view, componentType=5126 if dtype == '<f4' else 5125, count=len(values), type=kind)
        if kind == 'VEC3':
            a.update(min=values.min(axis=0).tolist(), max=values.max(axis=0).tolist())
        i = len(self.doc['accessors'])
        self.doc['accessors'].append(a)
        return i

    def finish(self, path):
        self.doc['buffers'][0]['byteLength'] = len(self.data)
        j = json.dumps(self.doc, separators=(',', ':'), allow_nan=False).encode()
        j += b' ' * (-len(j) % 4)
        self.data.extend(b'\0' * (-len(self.data) % 4))
        path.write_bytes(struct.pack('<5I', 0x46546c67, 2, 28+len(j)+len(self.data), len(j), 0x4e4f534a) + j + struct.pack('<2I', len(self.data), 0x004e4942) + self.data)


def polygon(rings, repair=False, report=None):
    rings = [np.asarray(r, dtype=float) for r in rings]
    rings = [r[:-1] if np.array_equal(r[0], r[-1]) else r for r in rings]
    if any(len(r) < 3 for r in rings):
        raise ValueError('polygon rings need three distinct vertices')
    origin = rings[0].mean(axis=0)
    _, _, basis = np.linalg.svd(rings[0]-origin, full_matrices=False)
    deviation = max(np.abs((r-origin) @ basis[2]).max() for r in rings)
    # Projection is only for triangulation. Output positions retain source XYZ.
    projected = [(r-origin) @ basis[:2].T for r in rings]
    shape = ogr.Geometry(ogr.wkbPolygon)
    positions, lookup = [], {}
    segments = []
    duplicates = 0
    for ring, xy in zip(rings, projected):
        ogr_ring = ogr.Geometry(ogr.wkbLinearRing)
        for i, (p, q) in enumerate(zip(ring, xy)):
            key = tuple(q)
            if key not in lookup:
                lookup[key] = len(positions)
                positions.append(p)
            else:
                duplicates += 1
                if not repair:
                    raise ValueError('duplicate/shared polygon ring vertices require --repair')
            ogr_ring.AddPoint_2D(*q)
            segments.append((q, xy[(i+1)%len(xy)], p, ring[(i+1)%len(ring)]))
        ogr_ring.CloseRings()
        shape.AddGeometry(ogr_ring)
    valid = shape.IsValid()
    if not valid and not repair:
        raise ValueError('invalid polygon topology; inspect source or explicitly use --repair')
    repaired = shape.MakeValid() if not valid else shape
    shapes = []
    def collect(g):
        kind = ogr.GT_Flatten(g.GetGeometryType())
        if kind == ogr.wkbPolygon:
            shapes.append(g.Clone())
        elif kind in (ogr.wkbMultiPolygon, ogr.wkbGeometryCollection):
            for child in g:
                collect(child)
        elif not g.IsEmpty():
            raise ValueError('repair produced collapsed non-polygon geometry; source needs review')
    collect(repaired)
    if not shapes:
        raise ValueError('polygon has no filled area')
    added = 0
    max_spread = 0.0
    def position(q):
        nonlocal added, max_spread
        key = tuple(q[:2])
        if key in lookup:
            return lookup[key]
        q = np.asarray(key)
        candidates = []
        for a, b, p, r in segments:
            ab = b-a
            t = float(np.clip(np.dot(q-a,ab)/max(np.dot(ab,ab),1e-30),0,1))
            distance = np.linalg.norm(a+t*ab-q)
            candidates.append((distance, p+t*(r-p)))
        closest = min(d for d, _ in candidates)
        if closest > 1e-6:
            raise ValueError('triangulator added a point away from source edges')
        hits = [p for d,p in candidates if d < max(1e-8,closest+1e-9)]
        point = np.mean(hits,axis=0)
        spread = max(np.linalg.norm(p-point) for p in hits)
        max_spread = max(max_spread,float(spread))
        # A crossing of unrelated 3D surfaces cannot be repaired as one vertex.
        if spread > .02:
            raise ValueError('projected intersection differs by more than 2 cm in 3D; source needs review')
        lookup[key] = len(positions)
        positions.append(point)
        added += 1
        return lookup[key]
    indices, loops, triangle_offsets, loop_offsets = [], [], [], []
    for part in shapes:
        triangle_offsets.append(len(indices))
        loop_offsets.append(len(loops))
        for r, ring in enumerate(part):
            points = [p[:2] for p in ring.GetPoints()[:-1]]
            # MakeValid can retain repeated adjacent points in otherwise valid rings.
            cleaned = []
            for q in points:
                if not cleaned or q != cleaned[-1]:
                    cleaned.append(q)
            points = cleaned
            xy = np.asarray(points)
            area = np.sum(xy[:,0]*np.roll(xy[:,1],-1)-xy[:,1]*np.roll(xy[:,0],-1))
            if (area > 0) != (r == 0):
                points.reverse()
            loops.extend(position(q) for q in points)
            loops.append(0xffffffff)
        for t in part.ConstrainedDelaunayTriangulation():
            points = t.GetGeometryRef(0).GetPoints()[:3]
            tri = [position(q) for q in points]
            pts = np.asarray([q[:2] for q in points])
            if np.linalg.det(np.stack((pts[1]-pts[0],pts[2]-pts[0]))) < 0:
                tri.reverse()
            indices.extend(tri)
    loops.pop()  # separators between loops, no terminal empty loop
    if not indices:
        raise ValueError('polygon produced no triangles')
    if report is not None:
        report.update(planarityDeviationMetres=float(deviation), topologyRepaired=not valid,
                      duplicateVertices=duplicates, addedIntersectionVertices=added,
                      maximumIntersectionAdjustmentMetres=max_spread, polygonParts=len(shapes))
    return positions, indices, loops, triangle_offsets, loop_offsets


def emit(items, path, project, repair=False, reports=None, ambiguous_outlines=False):
    glb = Glb()
    # Preserve scalar property types; unsupported schemas fail explicitly.
    keys = set().union(*(f['properties'] for f in items))
    schema, columns = {}, {}
    for key in sorted(keys):
        values = [f['properties'].get(key) for f in items]
        present = [v for v in values if v is not None]
        missing = len(present) != len(values)
        if missing:
            if all(isinstance(v, str) for v in present):
                sentinel = '__RUSTY_TILES_MISSING__'
                while sentinel in present:
                    sentinel += '_'
            elif present and all(type(v) in (int,float) for v in present):
                sentinel = -1.7976931348623157e308
                if sentinel in present:
                    raise ValueError('reserved missing-value sentinel occurs in source')
            else:
                raise ValueError(f'nullable boolean/complex property {key!r} requires an explicit schema')
            values = [sentinel if v is None else v for v in values]
        if all(isinstance(v, bool) for v in values):
            schema[key] = dict(type='BOOLEAN')
            columns[key] = dict(values=glb.view(np.packbits(values, bitorder='little').tobytes()))
        elif all(type(v) is int for v in values):
            schema[key] = dict(type='SCALAR', componentType='INT64')
            columns[key] = dict(values=glb.view(np.asarray(values, dtype='<i8').tobytes()))
        elif all(type(v) in (int, float) and math.isfinite(v) for v in values):
            schema[key] = dict(type='SCALAR', componentType='FLOAT64')
            columns[key] = dict(values=glb.view(np.asarray(values, dtype='<f8').tobytes()))
        elif all(isinstance(v, str) for v in values):
            chunks = [v.encode() for v in values]
            offsets = np.cumsum([0]+[len(v) for v in chunks], dtype=np.uint32)
            schema[key] = dict(type='STRING')
            columns[key] = dict(values=glb.view(b''.join(chunks)), stringOffsets=glb.view(offsets.astype('<u4').tobytes()), stringOffsetType='UINT32')
        else:
            raise ValueError(f'unsupported/null/mixed property {key!r}; retain source and normalize explicitly')
        if missing:
            schema[key]['noData'] = sentinel
    glb.doc['extensions'] = {'EXT_structural_metadata': dict(schema=dict(id='rusty-tiles-vector', classes={'feature':dict(properties=schema)}),
        propertyTables=[dict(name='features', **{'class':'feature'}, count=len(items), properties=columns)])}
    all_positions = []
    for fid, feature in enumerate(items):
        g = feature['geometry']
        kind, c = g['type'], g['coordinates']
        if kind == 'Point':
            parts = [('Point', [c])]
        elif kind == 'MultiPoint':
            parts = [('Point', c)]
        elif kind in ('LineString', 'Polygon'):
            parts = [(kind, c)]
        elif kind in ('MultiLineString', 'MultiPolygon'):
            parts = [(kind.removeprefix('Multi'), p) for p in c]
        else:
            raise ValueError(f'unsupported geometry {kind}')
        for kind, c in parts:
            ext = {'EXT_mesh_features': dict(featureIds=[dict(featureCount=len(items), attribute=0, propertyTable=0)])}
            if kind == 'Polygon':
                report = dict(sourceId=feature['properties']['_source_id'])
                try:
                    points, indices, loops, triangle_offsets, loop_offsets = polygon([project(r) for r in c], repair, report)
                except ValueError as e:
                    if ambiguous_outlines and 'projected intersection differs by more than 2 cm in 3D' in str(e):
                        if reports is not None:
                            reports.append(dict(sourceId=feature['properties']['_source_id'], topologyRepaired=False,
                                                outputGeometry='outline', reason=str(e)))
                        # Keep every original ring coordinate, including its closing
                        # segment; feature identity and properties remain unchanged.
                        parts.extend(('LineString', ring) for ring in c)
                        continue
                    raise ValueError(f"feature {feature['properties']['_source_id']}: {e}") from e
                if reports is not None:
                    reports.append(report)
                ext['EXT_mesh_polygon'] = dict(count=len(triangle_offsets), indicesOffsets=glb.accessor(triangle_offsets, '<u4', 'SCALAR'),
                    loopIndices=glb.accessor(loops, '<u4', 'SCALAR'), loopIndicesOffsets=glb.accessor(loop_offsets, '<u4', 'SCALAR'))
                if 'EXT_mesh_polygon' not in glb.doc['extensionsUsed']:
                    glb.doc['extensionsUsed'].append('EXT_mesh_polygon')
                mode = 4
            else:
                points = project(c)
                indices = list(range(len(points)))
                mode = 0 if kind == 'Point' else 3
                if len(points) < (1 if mode == 0 else 2):
                    raise ValueError('empty/degenerate feature')
            primitive = dict(mode=mode, attributes=dict(POSITION=glb.accessor(points, '<f4', 'VEC3'),
                _FEATURE_ID_0=glb.accessor([fid]*len(points), '<u4', 'SCALAR')), indices=glb.accessor(indices, '<u4', 'SCALAR'), extensions=ext)
            glb.doc['meshes'][0]['primitives'].append(primitive)
            all_positions.extend(points)
    glb.finish(path)
    # glTF Y-up → tile Z-up.
    p = np.asarray(all_positions)[:, [0, 2, 1]] * [1, -1, 1]
    low, high = p.min(axis=0), p.max(axis=0)
    center, half = (low+high)/2, np.maximum((high-low)/2, .001)
    return dict(box=[*center.tolist(), half[0], 0, 0, 0, half[1], 0, 0, 0, half[2]])


def run(args):
    doc = json.loads(pathlib.Path(args.input).read_text())
    crs = (doc.get('crs') or {}).get('properties',{}).get('name','')
    if doc.get('type') != 'FeatureCollection' or crs not in ('','urn:ogc:def:crs:OGC:1.3:CRS84','urn:ogc:def:crs:EPSG::4979','urn:ogc:def:crs:EPSG::4326'):
        raise ValueError('input must be a WGS84 longitude/latitude[/height] FeatureCollection')
    features = doc['features']
    if not features:
        raise ValueError('empty feature collection')
    points = []
    for i, f in enumerate(features):
        p = coordinates(f['geometry'])
        if not p:
            raise ValueError('empty geometry')
        points.extend(p)
        f['properties'] = dict(f.get('properties') or {})
        if '_source_id' in f['properties']:
            raise ValueError('_source_id is reserved for stable feature identity')
        f['properties']['_source_id'] = json.dumps(f.get('id', i), separators=(',', ':'))
        f['_center'] = np.mean(p, axis=0)
    origin = np.mean(points, axis=0)
    lon, lat = np.radians(origin[:2])
    east = [-math.sin(lon), math.cos(lon), 0]
    north = [-math.sin(lat)*math.cos(lon), -math.sin(lat)*math.sin(lon), math.cos(lat)]
    up = [math.cos(lat)*math.cos(lon), math.cos(lat)*math.sin(lon), math.sin(lat)]
    basis = np.array([east, north, up])
    anchor = ecef([origin])[0]
    def project(c):
        c = [list(p) if len(p) == 3 else [*p, 0] for p in c]
        return ((ecef(c)-anchor) @ basis.T)[:, [0, 2, 1]] * [1, 1, -1]
    # Partition in metre coordinates, never mix angular degrees with height.
    for feature in features:
        feature['_center'] = project([feature['_center']])[0]
    output = pathlib.Path(args.output)
    output.mkdir(exist_ok=True)
    (output/'t').mkdir()
    counter = 0
    reports = []
    def build(items):
        nonlocal counter
        if len(items) <= args.max_features:
            uri = f't/{counter}.glb'
            counter += 1
            bounds = emit(items, output/uri, project, getattr(args, 'repair', False), reports, getattr(args, 'ambiguous_outlines', False))
            return dict(boundingVolume=bounds, geometricError=0, content=dict(uri=uri,
                extensions={'3DTILES_content_gltf_vector':dict(vector=True)}))
        centers = np.asarray([f['_center'] for f in items])
        axis = np.ptp(centers, axis=0).argmax()
        items = sorted(items, key=lambda f:f['_center'][axis])
        children = [build(items[:len(items)//2]), build(items[len(items)//2:])]
        b = [c['boundingVolume']['box'] for c in children]
        lows = [np.asarray(v[:3])-np.asarray([v[3], v[7], v[11]]) for v in b]
        highs = [np.asarray(v[:3])+np.asarray([v[3], v[7], v[11]]) for v in b]
        lo, hi = np.min(lows, axis=0), np.max(highs, axis=0)
        m, h = (lo+hi)/2, (hi-lo)/2
        return dict(boundingVolume=dict(box=[*m, h[0], 0, 0, 0, h[1], 0, 0, 0, h[2]]), geometricError=float(np.linalg.norm(hi-lo)), refine='REPLACE', children=children)
    root = build(features)
    root['transform'] = [*east, 0, *north, 0, *up, 0, *anchor, 1]
    result = dict(asset=dict(version='1.1'), extensionsUsed=['3DTILES_content_gltf_vector'],
        geometricError=root['geometricError'], root=root)
    (output/'tileset.json').write_text(json.dumps(result, indent=2, allow_nan=False))
    (output/'conversion.json').write_text(json.dumps(dict(features=len(features),leafTiles=counter,repairEnabled=getattr(args,'repair',False),polygons=reports),indent=2))
    print(f'vector: {len(features)} features, {counter} leaf tiles, {sum(r["topologyRepaired"] for r in reports)} topology repairs')


if __name__ == '__main__':
    p = argparse.ArgumentParser()
    p.add_argument('input')
    p.add_argument('output')
    p.add_argument('--max-features', type=int, default=64)
    p.add_argument('--repair', action='store_true')
    p.add_argument('--ambiguous-outlines', action='store_true')
    a = p.parse_args()
    if a.max_features < 1:
        p.error('max-features must be positive')
    run(a)
