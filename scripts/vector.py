"""GeoJSON → glTF vector content prototype, pinned to the 2026 draft.

Full-detail features are partitioned spatially; parents contain conservative
3D line and planar polygon simplifications with metre-based error bounds.
The glTF extensions preserve polygon rings, holes, feature IDs and scalar
properties. GDAL/GEOS triangulates in a best-fit plane while retaining source
3D positions. Optional repairs and original-outline fallbacks are reported.
"""
import argparse
import json
import math
import os
import pathlib
import struct

import numpy as np
from osgeo import gdal, ogr

ogr.UseExceptions()



def geometry_operation(operation):
    """Quiet native validity/repair warnings; retain exceptions and debug output."""
    if os.environ.get('RUSTY_TILES_PYTHON_TRACEBACK') == '1':
        return operation()
    gdal.PushErrorHandler('CPLQuietErrorHandler')
    try:
        return operation()
    finally:
        gdal.PopErrorHandler()


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
        self.data.extend(b'\0' * (-len(self.data) % 8))
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


class OutlineFallback(ValueError):
    """Source rings can be retained without inventing a filled surface."""


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
    valid = geometry_operation(shape.IsValid)
    if not valid and not repair:
        raise ValueError('invalid polygon topology; inspect source or explicitly use --repair')
    repaired = geometry_operation(shape.MakeValid) if not valid else shape
    shapes = []
    def collect(g):
        kind = ogr.GT_Flatten(g.GetGeometryType())
        if kind == ogr.wkbPolygon:
            shapes.append(g.Clone())
        elif kind in (ogr.wkbMultiPolygon, ogr.wkbGeometryCollection):
            for child in g:
                collect(child)
        elif not g.IsEmpty():
            raise OutlineFallback('repair produced collapsed non-polygon geometry; source needs review')
    collect(repaired)
    if not shapes:
        raise OutlineFallback('polygon has no filled area')
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
            raise OutlineFallback('projected intersection differs by more than 2 cm in 3D; source needs review')
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


def geometry_parts(g):
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
    return parts


def validate_feature(feature, repair=False, ambiguous_outlines=False):
    reports=[]
    parts=geometry_parts(feature['geometry'])
    for kind, points in parts:
        if kind == 'Polygon':
            try:
                polygon(points, repair)
            except OutlineFallback as error:
                if not ambiguous_outlines:raise
                reports.append(dict(sourceId=feature['properties']['_source_id'],sourceLayer=feature['properties']['_source_layer'],
                    topologyRepaired=False,outputGeometry='outline',reason=str(error)))
        elif len(points) < (1 if kind == 'Point' else 2):
            raise ValueError('empty/degenerate feature')
    if reports:
        # Normalize before budgeting, so a large collapsed outline can be split
        # with the ordinary line fragmenter while retaining every source segment.
        feature['geometry']=dict(type='MultiLineString',coordinates=[ring for kind,rings in parts for ring in rings])
    return reports


def emit(items, path, project, repair=False, reports=None, ambiguous_outlines=False, encoding_report=None, schema_types=None, fill_only=False):
    glb = Glb()
    # Preserve scalar property types; unsupported schemas fail explicitly.
    keys = set().union(*(f['properties'] for f in items))
    schema, columns = {}, {}
    for key in sorted(keys):
        values = [f['properties'].get(key) for f in items]
        present = [v for v in values if v is not None]
        missing = len(present) != len(values)
        expected = (schema_types or {}).get(key)
        if expected == 'real' or (not expected and present and all(type(v) in (int,float) for v in present) and any(type(v) is float for v in present)):
            if any(type(v) is int and abs(v)>2**53 for v in present):
                raise ValueError(f'property {key!r} cannot represent large integers as float64 without loss')
            values = [float(v) if v is not None else None for v in values]
            present = [v for v in values if v is not None]
        if missing:
            if expected == 'integer' or (not expected and present and all(type(v) is int for v in present)):
                sentinel = -(2**53)  # exactly representable in JSON/JavaScript too
                while sentinel in present:
                    sentinel += 1
            elif expected == 'real' or (present and all(type(v) in (int,float) for v in present)):
                sentinel = -1.7976931348623157e308
                if sentinel in present:
                    raise ValueError('reserved missing-value sentinel occurs in source')
            elif expected == 'boolean':
                raise ValueError(f'nullable boolean property {key!r} requires an explicit schema')
            elif expected == 'string' or all(isinstance(v,str) for v in present):
                sentinel = '__RUSTY_TILES_MISSING__'
                while sentinel in present:
                    sentinel += '_'
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
        parts = geometry_parts(feature['geometry'])
        for kind, c in parts:
            ext = {'EXT_mesh_features': dict(featureIds=[dict(featureCount=len(items), attribute=0, propertyTable=0)])}
            if kind == 'Polygon':
                report = dict(sourceId=feature['properties']['_source_id'])
                try:
                    points, indices, loops, triangle_offsets, loop_offsets = polygon([project(r) for r in c], repair, report)
                except ValueError as e:
                    if ambiguous_outlines and isinstance(e,OutlineFallback):
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
                if fill_only:
                    ext.pop('EXT_mesh_polygon')
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
    if fill_only:
        glb.doc['extensionsUsed'] = [e for e in glb.doc['extensionsUsed'] if e != 'EXT_mesh_polygon']
        glb.doc['extensionsUsed'].append('KHR_materials_unlit')
        glb.doc['materials'] = [dict(doubleSided=True,extensions={'KHR_materials_unlit':{}},
            pbrMetallicRoughness=dict(baseColorFactor=[1,1,1,1],metallicFactor=0,roughnessFactor=1))]
        for primitive in glb.doc['meshes'][0]['primitives']:
            primitive['material']=0
    glb.finish(path)
    # glTF Y-up → tile Z-up.
    p = np.asarray(all_positions)[:, [0, 2, 1]] * [1, -1, 1]
    rounding = float(np.linalg.norm(p-p.astype('<f4'),axis=1).max())
    if encoding_report is not None:
        encoding_report['rounding'] = rounding
        encoding_report['vertices'] = len(all_positions)
    low, high = p.min(axis=0), p.max(axis=0)
    center, half = (low+high)/2, np.maximum((high-low)/2+rounding, .001)
    return dict(box=[*center.tolist(), half[0], 0, 0, 0, half[1], 0, 0, 0, half[2]])


def paths(geometry):
    """Yield coordinate paths without their duplicate closing polygon vertex."""
    kind, c = geometry['type'], geometry['coordinates']
    if kind == 'Point':
        return [[c]]
    if kind == 'MultiPoint':
        return [[p] for p in c]
    if kind == 'LineString':
        return [c]
    if kind == 'MultiLineString':
        return c
    if kind == 'Polygon':
        return [r[:-1] if r[0] == r[-1] else r for r in c]
    if kind == 'MultiPolygon':
        return [r[:-1] if r[0] == r[-1] else r for poly in c for r in poly]
    raise ValueError(f'unsupported geometry {kind}')


def simplify_path(source, tolerance, locked, closed=False):
    """Iterative 3D RDP; retained source vertices, fixed junctions/boundaries.

    Every replaced source segment chain lies within tolerance of its chord.
    Endpoint projection and continuity also bound chord-to-chain distance.
    """
    points = np.asarray(source, dtype=float)
    if closed and np.array_equal(points[0], points[-1]):
        points = points[:-1]
    if len(points) < (4 if closed else 3) or tolerance <= 0:
        return points.tolist(), 0.0
    if closed:
        split = int(np.argmax(np.linalg.norm(points - points[0], axis=1)))
        if split == 0:
            return points.tolist(), 0.0
        points = np.vstack([points, points[0]])
        anchors = {0, split, len(points)-1}
    else:
        anchors = {0, len(points)-1}
    anchors.update(i for i,p in enumerate(points) if tuple(p) in locked)
    keep = set(anchors)
    anchors = sorted(anchors)
    pending = list(zip(anchors[:-1], anchors[1:]))
    error = 0.0
    while pending:
        a, b = pending.pop()
        if b <= a + 1:
            continue
        segment = points[b] - points[a]
        interior = points[a+1:b]
        t = np.clip((interior - points[a]) @ segment / max(float(segment @ segment), 1e-30), 0, 1)
        distances = np.linalg.norm(interior - points[a] - t[:, None] * segment, axis=1)
        i = int(np.argmax(distances))
        if distances[i] > tolerance:
            index = a + 1 + i
            keep.add(index)
            pending.extend(((a,index),(index,b)))
        else:
            error = max(error, float(distances[i]))
    result = points[sorted(keep)]
    if closed:
        result = result[:-1]
        if len(result) < 3:
            return points[:-1].tolist(), 0.0
    return result.tolist(), error


def simplify_polygon(rings, tolerance, locked):
    rings = [np.asarray(r, dtype=float) for r in rings]
    opened = [r[:-1] if np.array_equal(r[0],r[-1]) else r for r in rings]
    origin = opened[0].mean(axis=0)
    _, _, basis = np.linalg.svd(opened[0]-origin, full_matrices=False)
    deviation = max(float(np.abs((r-origin) @ basis[2]).max()) for r in opened)
    if 2*deviation >= tolerance:
        return [r.tolist() for r in rings], 0.0, 'nonplanar polygon retained'
    def shape(values):
        poly = ogr.Geometry(ogr.wkbPolygon)
        for r in values:
            ring = ogr.Geometry(ogr.wkbLinearRing)
            for p in (np.asarray(r)-origin) @ basis[:2].T:
                ring.AddPoint_2D(*p)
            ring.CloseRings()
            poly.AddGeometry(ring)
        return poly
    original = shape(opened)
    if not geometry_operation(original.IsValid):
        return [r.tolist() for r in rings], 0.0, 'invalid source topology retained for existing repair policy'
    candidates, errors = [], []
    for ring in opened:
        simplified, error = simplify_path(ring, tolerance-2*deviation, locked, closed=True)
        candidates.append(simplified)
        errors.append(error)
    candidate = shape(candidates)
    if not geometry_operation(candidate.IsValid) or candidate.GetArea() <= 0:
        return [r.tolist() for r in rings], 0.0, 'simplification would change polygon topology'
    if sum(map(len,candidates)) == sum(map(len,opened)):
        return [r.tolist() for r in rings], 0.0, None
    # All rings and holes retained; locks keep shared source boundaries fixed.
    return [r+[r[0]] for r in candidates], max(errors) + 2*deviation, None


def simplify_feature(feature, tolerance, locked, reports):
    import copy
    result = copy.deepcopy(feature)
    geometry = result['geometry']
    kind, c = geometry['type'], geometry['coordinates']
    error = 0.0
    if kind in ('Point','MultiPoint'):
        return result, error  # semantic point features are never silently thinned
    if kind == 'LineString':
        geometry['coordinates'], error = simplify_path(c, tolerance, locked)
    elif kind == 'MultiLineString':
        parts = [simplify_path(p, tolerance, locked) for p in c]
        geometry['coordinates'] = [p for p,e in parts]
        error = max(e for p,e in parts)
    else:
        parts = [c] if kind == 'Polygon' else c
        values = []
        for rings in parts:
            value, part_error, reason = simplify_polygon(rings, tolerance, locked)
            values.append(value)
            error = max(error, part_error)
            if reason:
                reports.append(dict(sourceId=feature['properties']['_source_id'], reason=reason))
        geometry['coordinates'] = values[0] if kind == 'Polygon' else values
    return result, error


def union_bounds(boxes):
    lows = [np.asarray(v[:3])-np.asarray([v[3],v[7],v[11]]) for v in boxes]
    highs = [np.asarray(v[:3])+np.asarray([v[3],v[7],v[11]]) for v in boxes]
    lo, hi = np.min(lows,axis=0), np.max(highs,axis=0)
    m, h = (lo+hi)/2, (hi-lo)/2
    return dict(box=[*m, h[0],0,0,0,h[1],0,0,0,h[2]])


def run(args):
    import sys
    import types
    if '__file__' in globals():
        sys.path.insert(0,str(pathlib.Path(__file__).resolve().parent))
    import hashlib
    import vector_pipeline
    import vector_reuse
    import vector_source
    sources=[globals().get('__source__') or pathlib.Path(__file__).read_text()]
    for module in (vector_source,vector_reuse,vector_pipeline):
        sources.append(getattr(module,'__source__',None) or pathlib.Path(module.__file__).read_text())
    encoder=hashlib.sha256('\0'.join(sources).encode()).hexdigest()
    return vector_pipeline.run(args,types.SimpleNamespace(emit=emit,polygon=polygon,validate_feature=validate_feature,simplify_feature=simplify_feature,encoder_digest=encoder))


if __name__ == '__main__':
    p = argparse.ArgumentParser()
    p.add_argument('input')
    p.add_argument('output')
    p.add_argument('--lod-tolerance', type=float, default=.1)
    p.add_argument('--lod-levels', type=int, default=3)
    p.add_argument('--max-features', type=int, default=64)
    p.add_argument('--layer', dest='layers', action='append', default=[])
    p.add_argument('--all-layers', action='store_true')
    p.add_argument('--reuse-tileset')
    p.add_argument('--source-crs')
    p.add_argument('--height-offset', type=float)
    p.add_argument('--max-vertices', type=int, default=65536)
    p.add_argument('--max-bytes', type=int, default=4194304)
    p.add_argument('--max-tiles', type=int, default=100000)
    p.add_argument('--max-source-vertices', type=int, default=1000000)
    p.add_argument('--where')
    p.add_argument('--list-fields', choices=['error','json'], default='error')
    p.add_argument('--field', dest='fields', action='append', default=[])
    p.add_argument('--drop-field', dest='drop_fields', action='append', default=[])
    p.add_argument('--skip-invalid', action='store_true')
    p.add_argument('--repair', action='store_true')
    p.add_argument('--ambiguous-outlines', action='store_true')
    a = p.parse_args()
    if a.max_features < 1:
        p.error('max-features must be positive')
    run(a)
