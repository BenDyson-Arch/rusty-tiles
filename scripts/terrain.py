"""Quantized-mesh lab encoder. GDAL handles source CRS and windowed sampling.

Regular shared grids keep adjacent tile edges deterministic. Outside coverage
and NoData use an explicitly selected fill height. Source height datum is never
inferred: the caller supplies an offset to ellipsoidal metres.
"""
import argparse
import json
import math
import pathlib
import struct
import sys

import numpy as np
from osgeo import gdal, osr

gdal.UseExceptions()
osr.UseExceptions()
A = 6378137.0
B = 6356752.314245179


def ecef(lon, lat, height):
    lon, lat = np.radians(lon), np.radians(lat)
    e2 = 1 - (B / A) ** 2
    n = A / np.sqrt(1 - e2 * np.sin(lat) ** 2)
    return np.stack(((n + height) * np.cos(lat) * np.cos(lon),
                     (n + height) * np.cos(lat) * np.sin(lon),
                     (n * (1 - e2) + height) * np.sin(lat)), axis=-1)


def encode(west, south, size, heights, floor, ceiling):
    count = heights.shape[0]
    uv = np.rint(np.linspace(0, 32767, count)).astype(np.int64)
    u, v = np.meshgrid(uv, uv)
    h = np.rint((heights - floor) / max(ceiling - floor, 1e-30) * 32767).astype(np.int64)
    h = np.clip(h, 0, 32767)
    # Same global height interval makes shared edges decode identically.
    decoded = floor + h / 32767 * (ceiling - floor)
    xyz = ecef(west + u / 32767 * size, south + v / 32767 * size, decoded).reshape(-1, 3)
    center = (xyz.min(axis=0) + xyz.max(axis=0)) / 2
    radius = np.linalg.norm(xyz - center, axis=1).max()
    # Hemispheric roots have an occlusion point at infinity along their centre
    # direction. Zero is the Earth centre and would incorrectly hide the tile.
    # Small tiles use the conservative ellipsoid-scaled occlusion construction.
    scaled = xyz / [A, A, B]
    direction = center / [A, A, B]
    norm = np.linalg.norm(direction)
    direction = direction / norm if norm > 0 else np.array([1., 0., 0.])
    hop = direction * 1e15
    if size < 90 and floor >= 0:
        lengths = np.linalg.norm(scaled, axis=1)
        unit = scaled / lengths[:, None]
        cosa = unit @ direction
        sina = np.linalg.norm(np.cross(unit, direction), axis=1)
        denom = cosa / np.maximum(lengths, 1) - sina * np.sqrt(np.maximum(lengths * lengths - 1, 0)) / np.maximum(lengths, 1)
        if np.all(denom > 0):
            hop = direction * (1 / denom).max()
    triangles = []
    for y in range(count - 1):
        for x in range(count - 1):
            a = y * count + x
            triangles.extend((a, a + 1, a + count, a + 1, a + count + 1, a + count))
    order = list(dict.fromkeys(triangles))
    remap = np.empty(count * count, dtype=np.int64)
    remap[order] = np.arange(len(order))
    indices = remap[triangles]
    high = 0
    codes = []
    for index in indices:
        codes.append(high - int(index))
        if index == high:
            high += 1
    out = bytearray(struct.pack('<3d2f7d', *center, floor, ceiling, *center, radius, *hop))
    out.extend(struct.pack('<I', len(order)))
    for values in (u, v, h):
        delta = np.diff(values.ravel()[order], prepend=0)
        out.extend(((delta << 1) ^ (delta >> 63)).astype('<u2').tobytes())
    out.extend(struct.pack('<I', len(indices) // 3))
    out.extend(np.asarray(codes, dtype='<u2').tobytes())
    # West south east north; runtime sorts these lists and constructs skirts.
    for edge in (np.arange(count) * count, np.arange(count),
                 np.arange(count) * count + count - 1, np.arange(count) + count * (count - 1)):
        out.extend(struct.pack('<I', count))
        out.extend(remap[edge].astype('<u2').tobytes())
    return out


def run(args):
    source = gdal.Open(args.input)
    if source.RasterCount != 1 or source.GetSpatialRef() is None:
        raise ValueError('terrain requires a single-band DEM with a declared CRS')
    if not (0 <= args.max_zoom <= 24) or args.grid not in (17, 33, 65, 129):
        raise ValueError('maxZoom must be 0..24; grid must be 17, 33, 65 or 129')
    band = source.GetRasterBand(1)
    unit = band.GetUnitType().lower()
    if unit not in ('', 'm', 'metre', 'meter', 'metres', 'meters'):
        raise ValueError(f'height units must be metres, found {unit!r}')
    if band.GetScale() not in (None, 1) or band.GetOffset() not in (None, 0):
        raise ValueError('scaled DEM bands must be converted to actual metre values first')
    low, high = band.ComputeRasterMinMax(False)
    low = min(low + args.height_offset, args.fill_height)
    high = max(high + args.height_offset, args.fill_height)
    # Header stores float32; encode with those exact endpoints.
    low, high = float(np.float32(low)), float(np.float32(high))
    vrt = gdal.Warp('', source, format='VRT', dstSRS='EPSG:4326', dstNodata=float('nan'), outputType=gdal.GDT_Float32)
    gt = vrt.GetGeoTransform()
    bounds = [gt[0], gt[3] + gt[5] * vrt.RasterYSize, gt[0] + gt[1] * vrt.RasterXSize, gt[3]]
    if bounds[2] - bounds[0] >= 180 or not (-180 <= bounds[0] < bounds[2] <= 180):
        raise ValueError('antimeridian/global DEMs require a split before this lab conversion')
    dest = pathlib.Path(args.output)
    available = []
    tiles = 0
    for z in range(args.max_zoom + 1):
        size = 180 / 2**z
        x0 = max(0, math.floor((bounds[0] + 180) / size))
        x1 = min(2**(z+1)-1, math.floor((bounds[2] + 180) / size))
        y0 = max(0, math.floor((bounds[1] + 90) / size))
        y1 = min(2**z-1, math.floor((bounds[3] + 90) / size))
        if z == 0:
            x0, x1, y0, y1 = 0, 1, 0, 0
        total = (x1-x0+1)*(y1-y0+1)
        if tiles + total > args.max_tiles:
            raise ValueError(f'tile count exceeds --maxTiles={args.max_tiles}')
        available.append([dict(startX=x0, startY=y0, endX=x1, endY=y1)])
        for x in range(x0, x1+1):
            folder = dest / str(z) / str(x)
            folder.mkdir(parents=True, exist_ok=True)
            for y in range(y0, y1+1):
                west, south = -180 + x*size, -90 + y*size
                step = size / (args.grid-1)
                # Expanded pixel envelope puts sample centres exactly on edges.
                tile = gdal.Warp('', vrt, format='MEM', outputBounds=[west-step/2, south-step/2, west+size+step/2, south+size+step/2],
                                 width=args.grid, height=args.grid, resampleAlg='bilinear', dstNodata=float('nan'), outputType=gdal.GDT_Float32)
                heights = np.flipud(tile.ReadAsArray()).astype(np.float64)
                # Preserve actual coverage independently of the standalone terrain fill.
                # The client uses these heights to supplement its selected base terrain.
                overlay = [float(h)+args.height_offset if np.isfinite(h) else None for h in heights.ravel()]
                (folder / f'{y}.heights.json').write_text(json.dumps(dict(width=args.grid, height=args.grid, heights=overlay), separators=(',', ':'), allow_nan=False))
                heights = np.where(np.isfinite(heights), heights+args.height_offset, args.fill_height)
                (folder / f'{y}.terrain').write_bytes(encode(west, south, size, heights, low, high))
                tiles += 1
        print(f'terrain level {z}: {total} tiles', file=sys.stderr)
    (dest / 'layer.json').write_text(json.dumps(dict(tilejson='2.1.0', format='quantized-mesh-1.0', version='1.0.0',
        scheme='tms', projection='EPSG:4326', minzoom=0, maxzoom=args.max_zoom, bounds=bounds,
        tiles=['{z}/{x}/{y}.terrain'], available=available,
        heightOverlay=dict(version=1, tiles=['{z}/{x}/{y}.heights.json'], grid=args.grid, rowOrder='south-to-north')), indent=2))
    (dest / 'conversion.json').write_text(json.dumps(dict(sourceCrs=source.GetProjection(), heightOffset=args.height_offset,
        fillHeight=args.fill_height, grid=args.grid, tiles=tiles, heightRange=[low,high], heightQuantizationStep=(high-low)/32767,
        sourcePixelDegrees=[gt[1],abs(gt[5])], finestGridDegrees=180/2**args.max_zoom/(args.grid-1),
        limitations='Regular-grid prototype. NoData/outside filled explicitly. Height datum supplied by caller. No certified maximum surface-error bound.'), indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('input')
    parser.add_argument('output')
    parser.add_argument('--max-zoom', type=int, required=True)
    parser.add_argument('--grid', type=int, default=65)
    parser.add_argument('--height-offset', type=float, required=True)
    parser.add_argument('--fill-height', type=float, required=True)
    parser.add_argument('--max-tiles', type=int, default=100000)
    options = parser.parse_args()
    if not math.isfinite(options.height_offset) or not math.isfinite(options.fill_height):
        parser.error('heights must be finite')
    run(options)
