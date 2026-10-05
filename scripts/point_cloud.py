"""Disk-backed LAS/LAZ → glTF POINTS in a replacement-refined 3D Tiles tree.

Each parent samples its own source points on a bounded voxel grid. The cell
diagonal bounds source-to-sample distance; leaves keep every record. No source
application axis conventions or guessed CRS/vertical datum are used.
"""
import argparse
import contextlib
import json
import math
import pathlib
import struct

import laspy
import numpy as np
from pyproj import CRS, Transformer, network
from pyproj.exceptions import ProjError


class Glb:
    def __init__(self):
        self.data = bytearray()
        self.doc = dict(asset=dict(version='2.0', generator='rusty-tiles point cloud'),
                        buffers=[dict(byteLength=0)], bufferViews=[], accessors=[],
                        scenes=[dict(nodes=[0])], scene=0, nodes=[dict(mesh=0)], meshes=[])

    def view(self, values):
        self.data.extend(b'\0' * (-len(self.data) % 8))
        raw = np.ascontiguousarray(values).tobytes()
        index = len(self.doc['bufferViews'])
        self.doc['bufferViews'].append(dict(buffer=0, byteOffset=len(self.data), byteLength=len(raw)))
        self.data.extend(raw)
        return index

    def accessor(self, values, component, kind, normalized=False):
        item = dict(bufferView=self.view(values), componentType=component, count=len(values), type=kind)
        if normalized:
            item['normalized'] = True
        if kind == 'VEC3' and component == 5126:
            item.update(min=values.min(axis=0).tolist(), max=values.max(axis=0).tolist())
        index = len(self.doc['accessors'])
        self.doc['accessors'].append(item)
        return index

    def write(self, path):
        self.data.extend(b'\0' * (-len(self.data) % 4))
        self.doc['buffers'][0]['byteLength'] = len(self.data)
        header = json.dumps(self.doc, separators=(',', ':'), allow_nan=False).encode()
        header += b' ' * (-len(header) % 4)
        total = 12 + 8 + len(header) + 8 + len(self.data)
        with path.open('wb') as file:
            file.write(struct.pack('<4sII', b'glTF', 2, total))
            file.write(struct.pack('<I4s', len(header), b'JSON'))
            file.write(header)
            file.write(struct.pack('<I4s', len(self.data), b'BIN\0'))
            file.write(self.data)


def chunks(path, dtype, budget):
    with path.open('rb') as file:
        while True:
            rows = np.fromfile(file, dtype=dtype, count=budget)
            if not len(rows):
                return
            yield rows


def bounds(path, dtype, budget):
    lo, hi = np.full(3, np.inf), np.full(3, -np.inf)
    count = 0
    for rows in chunks(path, dtype, budget):
        lo = np.minimum(lo, rows['position'].min(axis=0))
        hi = np.maximum(hi, rows['position'].max(axis=0))
        count += len(rows)
    return lo, hi, count


def sample(path, dtype, budget, lo, hi):
    # At most side**3 representatives, independent of source point count.
    side = max(1, int(round(budget ** (1 / 3))))
    while side ** 3 > budget:
        side -= 1
    extent = hi - lo
    cell = extent / side
    safe = np.where(extent > 0, extent, 1)
    representatives = {}
    for rows in chunks(path, dtype, budget):
        keys = np.minimum(((rows['position'] - lo) / safe * side).astype(np.int64), side - 1)
        keys = (keys[:, 0] * side + keys[:, 1]) * side + keys[:, 2]
        _, indices = np.unique(keys, return_index=True)
        for i in indices:
            representatives.setdefault(int(keys[i]), rows[i].copy())
    return np.array(list(representatives.values()), dtype=dtype), float(np.linalg.norm(cell))


def emit(rows, center, properties, path):
    glb = Glb()
    positions = (rows['position'] - center)[:, [0, 2, 1]] * [1, 1, -1]
    encoded = positions.astype('<f4')
    rounding = float(np.linalg.norm(positions - encoded, axis=1).max())
    attrs = dict(POSITION=glb.accessor(encoded, 5126, 'VEC3'),
                 _FEATURE_ID_0=glb.accessor(np.arange(len(rows), dtype='<u4'), 5125, 'SCALAR'))
    if all(name in rows.dtype.names for name in ('red', 'green', 'blue')):
        rgb = np.column_stack([*[rows[name] for name in ('red', 'green', 'blue')],
                               np.full(len(rows), 65535, dtype='<u2')]).astype('<u2')
        attrs['COLOR_0'] = glb.accessor(rgb, 5123, 'VEC4', normalized=True)
    schema, columns = {}, {}
    for name, dtype in properties.items():
        component = {('u', 1): 'UINT8', ('u', 2): 'UINT16', ('u', 4): 'UINT32', ('u', 8): 'UINT64',
                     ('i', 1): 'INT8', ('i', 2): 'INT16', ('i', 4): 'INT32', ('i', 8): 'INT64',
                     ('f', 4): 'FLOAT32', ('f', 8): 'FLOAT64'}[(dtype.kind, dtype.itemsize)]
        schema[name] = dict(type='SCALAR', componentType=component)
        columns[name] = dict(values=glb.view(rows[name]))
    glb.doc['extensionsUsed'] = ['EXT_mesh_features', 'EXT_structural_metadata', 'KHR_materials_unlit']
    glb.doc['extensions'] = dict(EXT_structural_metadata=dict(
        schema=dict(id='rusty-tiles-point-cloud', classes=dict(point=dict(properties=schema))),
        propertyTables=[dict(name='points', **{'class': 'point'}, count=len(rows), properties=columns)]))
    glb.doc['materials'] = [dict(extensions=dict(KHR_materials_unlit={}),
                                  pbrMetallicRoughness=dict(metallicFactor=0, roughnessFactor=1))]
    glb.doc['meshes'] = [dict(primitives=[dict(mode=0, attributes=attrs, material=0,
        extensions=dict(EXT_mesh_features=dict(featureIds=[dict(featureCount=len(rows), attribute=0, propertyTable=0)])))])]
    glb.write(path)
    return rounding


def transform_for(header, args):
    if args.source_crs == 'local':
        if args.height_offset is not None:
            raise ValueError('heightOffset applies only to geospatial CRS; local XYZ is in metres')
        return None, None
    if args.height_offset is None or not math.isfinite(args.height_offset):
        raise ValueError('geospatial input requires explicit finite --height-offset to ellipsoidal metres (0 if established)')
    crs = header.parse_crs() if args.source_crs == 'header' else CRS.from_user_input(args.source_crs)
    if crs is None:
        raise ValueError('LAS has no CRS; supply --source-crs or explicitly choose local')
    if crs.is_compound or crs.is_geocentric or len(crs.axis_info) != 2 or not (crs.is_projected or crs.is_geographic):
        raise ValueError('use a 2D horizontal CRS and explicit ellipsoidal height offset; compound/geocentric CRS is unsupported')
    network.set_network_enabled(False)
    try:
        transformer = Transformer.from_crs(crs.to_3d(), CRS.from_epsg(4978), always_xy=True,
                                           allow_ballpark=False, only_best=True)
    except ProjError as error:
        error.environment_error = True
        raise

    return transformer, crs.to_string()


def run(args):
    if args.max_points < 1 or args.chunk_points < 1:
        raise ValueError('point budgets must be positive')
    output = pathlib.Path(args.output)
    if output.exists():
        raise ValueError('output directory already exists')
    # Rust owns this staging directory. Direct helper use also cleans failed jobs.
    output.mkdir(parents=True)
    try:
        return convert(args, output)
    except BaseException:
        import shutil
        shutil.rmtree(output)
        raise


def convert(args, output):
    scratch = output / 'scratch'
    scratch.mkdir()
    (output / 't').mkdir()
    properties = {'source_index': np.dtype('<u8'), 'source_x': np.dtype('<f8'),
                  'source_y': np.dtype('<f8'), 'source_z': np.dtype('<f8')}
    source = scratch / 'source.bin'
    origin = None
    count = 0
    with laspy.open(args.input) as reader, source.open('wb') as file:
        transformer, crs = transform_for(reader.header, args)
        scales = reader.header.scales.tolist()
        offsets = reader.header.offsets.tolist()
        if reader.header.point_format.id in (4, 5, 9, 10):
            raise ValueError('waveform LAS point formats are unsupported')
        dimensions = []
        for dim in reader.header.point_format.dimensions:
            if dim.name in properties or dim.name == 'position':
                raise ValueError(f'reserved LAS dimension: {dim.name}')
            dtype = dim.dtype
            if dim.num_elements != 1 or dim.name.startswith('wave'):
                raise ValueError(f'unsupported LAS dimension: {dim.name}')
            if dtype is None:  # packed bit fields, expanded losslessly
                dtype = np.dtype('u1')
            dtype = np.dtype(dtype).newbyteorder('<')
            if dtype.kind not in 'uif' or dtype.itemsize not in (1, 2, 4, 8):
                raise ValueError(f'unsupported LAS dimension type: {dim.name}')
            if dim.is_scaled:
                dtype = np.dtype('<f8')
            properties[dim.name] = dtype
            dimensions.append(dim.name)
        dtype = np.dtype([('position', '<f8', (3,)), *properties.items()])
        for points in reader.chunk_iterator(args.chunk_points):
            rows = np.zeros(len(points), dtype=dtype)
            xyz = np.column_stack([points.x, points.y, points.z])
            if not np.isfinite(xyz).all():
                raise ValueError('nonfinite source coordinates')
            rows['source_index'] = np.arange(count, count + len(points), dtype='<u8')
            for i, key in enumerate(('source_x', 'source_y', 'source_z')):
                rows[key] = xyz[:, i]
            for name in dimensions:
                rows[name] = points[name]
                if not np.isfinite(rows[name]).all():
                    raise ValueError(f'nonfinite dimension: {name}')
            if transformer:
                xyz = np.column_stack(transformer.transform(xyz[:, 0], xyz[:, 1],
                    xyz[:, 2] + args.height_offset, errcheck=True))
            if not np.isfinite(xyz).all():
                raise ValueError('nonfinite projected coordinates')
            if origin is None:
                origin = xyz[0].copy()
            rows['position'] = xyz - origin
            rows.tofile(file)
            count += len(rows)
        if count != reader.header.point_count:
            raise ValueError('LAS point count differs from header')
    if not count:
        raise ValueError('empty point cloud')
    counter = 0
    max_rounding = 0.0
    def build(path, parent_center, depth=0):
        nonlocal counter, max_rounding
        lo, hi, size = bounds(path, dtype, args.chunk_points)
        center = lo + (hi - lo) / 2
        leaf = size <= args.max_points
        if leaf:
            rows = np.fromfile(path, dtype=dtype)
            error = 0.0
        else:
            rows, error = sample(path, dtype, args.max_points, lo, hi)
        index = counter
        counter += 1
        uri = f't/{index}.glb'
        rounding = emit(rows, center, properties, output / uri)
        max_rounding = max(max_rounding, rounding)
        del rows
        half = np.maximum((hi - lo) / 2 + rounding, 1e-6)
        delta = center - parent_center
        transform = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, *delta.tolist(), 1]
        node = dict(boundingVolume=dict(box=[0, 0, 0, half[0], 0, 0, 0, half[1], 0, 0, 0, half[2]]),
                    transform=transform, geometricError=error + (rounding if not leaf else 0),
                    refine='REPLACE', content=dict(uri=uri))
        if not leaf:
            if depth >= 64:
                raise ValueError('point partition exceeds 64 levels; inspect source extent or raise maxPoints')
            axis = int(np.argmax(hi - lo))
            paths = [scratch / f'{index}-0.bin', scratch / f'{index}-1.bin']
            counts = [0, 0]
            with contextlib.ExitStack() as stack:
                files = [stack.enter_context(p.open('wb')) for p in paths]
                seen = 0
                for batch in chunks(path, dtype, args.chunk_points):
                    if hi[axis] == lo[axis]:
                        mask = np.arange(seen, seen + len(batch)) < size // 2
                    else:
                        mask = batch['position'][:, axis] < center[axis]
                    for i, select in enumerate((mask, ~mask)):
                        batch[select].tofile(files[i])
                        counts[i] += int(select.sum())
                    seen += len(batch)
            if not all(counts):
                raise ValueError('spatial split made no progress')
            path.unlink()
            node['children'] = [build(p, center, depth + 1) for p in paths]
            node['geometricError'] = max(node['geometricError'], *(c['geometricError'] for c in node['children']))
        else:
            path.unlink()
        return node
    root = build(source, np.zeros(3))
    root['transform'][12:15] = (np.array(root['transform'][12:15]) + origin).tolist()
    scratch.rmdir()
    box = root['boundingVolume']['box']
    tileset_error = max(1.0, root['geometricError'], 2 * float(np.linalg.norm([box[3], box[7], box[11]])))
    (output / 'tileset.json').write_text(json.dumps(dict(asset=dict(version='1.1'),
        geometricError=tileset_error, root=root), indent=2, allow_nan=False))
    report = dict(points=count, tiles=counter, sourceCrs=args.source_crs, resolvedCrs=crs,
        sourceScales=scales, sourceOffsets=offsets, heightOffset=args.height_offset,
        properties=list(properties), maxPositionRoundingMetres=max_rounding,
        sampling='first source point per voxel; celldiagonal bounds source-to-sample distance',
        maxPoints=args.max_points, chunkPoints=args.chunk_points)
    (output / 'conversion.json').write_text(json.dumps(report, indent=2, allow_nan=False))
    print(f'point cloud: {count} points, {counter} tiles; max local float32 rounding {max_rounding:.6g} m')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('input')
    parser.add_argument('output')
    parser.add_argument('--source-crs', required=True)
    parser.add_argument('--height-offset', type=float)
    parser.add_argument('--max-points', type=int, default=50000)
    parser.add_argument('--chunk-points', type=int, default=100000)
    run(parser.parse_args())
