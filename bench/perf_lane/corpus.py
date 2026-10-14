"""Deterministic invented performance-lane inputs, using only the Python stdlib.

Usage: python3 bench/perf_lane/corpus.py DIR
Writes LAS, GeoJSON, GLB and GeoTIFF inputs whose bytes are independent of the
zlib/GDAL/NumPy stack: PNG and TIFF data are stored uncompressed, and noise comes
from a fixed linear congruential generator. PINNED holds the expected digests.
"""
import argparse, hashlib, json, math, pathlib, struct, zlib

PINNED = {  # regenerate and update only when a generator deliberately changes
    'points.las': '67eb91b0839cb8f244d52fc1c4ca7210aa4fd5d1e2bf4fbd5252ac57f50516e3',
    'vector.geojson': '4f11f4ca75ea9045babcda0d368d9f70ca4bc50a41926fb49feb85cd78f6c4c4',
    'mesh.glb': '6c53d2fbed365f976c4a0f897e3bb10cea2ecd9e2651348d00cb8619735cde06',
    'rgba.tif': '2409501400b7641c3a503e10ba75129f8f30e965ed803af45d1684e6cdfa0d35',
    'dem.tif': '197a8f6677d961d19e73a15dd42f517f17c311847ef141240da51a816cf32fa6',
}


def lcg(seed):
    while True:
        seed = (seed * 6364136223846793005 + 1442695040888963407) % 2**64
        yield seed >> 33


def write_las(path, count=200_000):
    """LAS 1.2 point format 3 (RGB + GPS time) on an invented UTM 56S hill."""
    side = math.isqrt(count)
    noise = lcg(20261011)
    records = bytearray()
    bounds = [2**31, -2**31] * 3
    for i in range(count):
        gx, gy = i % side, i // side
        x = gx * 50 + next(noise) % 40  # centimetres
        y = gy * 50 + next(noise) % 40
        z = int(4000 * math.sin(gx / 37) * math.cos(gy / 53)) + next(noise) % 200 + 20000
        for axis, value in enumerate((x, y, z)):
            bounds[2 * axis] = min(bounds[2 * axis], value)
            bounds[2 * axis + 1] = max(bounds[2 * axis + 1], value)
        records += struct.pack('<iiiHBBbBHdHHH', x, y, z, i % 4096, 0x09, 2 + i % 5, 0, 0, i % 7,
                               i * 0.001, (gx * 257) % 65536, (gy * 263) % 65536, (z * 3) % 65536)
    header = bytearray(227)
    header[:4] = b'LASF'
    header[24:26] = bytes((1, 2))
    header[26:37] = b'rusty-tiles'
    header[58:72] = b'perf-lane demo'
    struct.pack_into('<HIIBHI', header, 94, 227, 227, 0, 3, 34, count)
    struct.pack_into('<5I', header, 111, count, 0, 0, 0, 0)
    struct.pack_into('<3d', header, 131, 0.01, 0.01, 0.01)
    offsets = (500000, 6960000, 0)
    struct.pack_into('<3d', header, 155, *offsets)
    struct.pack_into('<6d', header, 179, *(offsets[a] + bounds[2 * a + m] * .01 for a in range(3) for m in (1, 0)))
    path.write_bytes(bytes(header) + bytes(records))


def write_geojson(path):
    """Mixed polygons (some with holes), lines and points near Rome, with attributes."""
    noise, features = lcg(7), []
    for row in range(60):
        for col in range(60):
            x, y, s = 12.40 + col * .002, 41.85 + row * .002, .0006 + next(noise) % 400 * 1e-6
            ring = [[x, y], [x + s, y], [x + s, y + s * .8], [x + s * .4, y + s * 1.2], [x, y + s], [x, y]]
            rings = [ring]
            if (row + col) % 5 == 0:
                h = s * .25
                rings.append([[x + h, y + h], [x + h, y + 2 * h], [x + 2 * h, y + 2 * h], [x + 2 * h, y + h], [x + h, y + h]])
            features.append(dict(type='Feature', properties=dict(id=len(features), kind='building', height=3 + next(noise) % 40,
                name=f'b{row}-{col}'), geometry=dict(type='Polygon', coordinates=rings)))
    for line in range(400):
        coords = [[12.40 + step * .0012, 41.85 + line * .0003 + .0004 * math.sin(step / 3 + line)] for step in range(100)]
        features.append(dict(type='Feature', properties=dict(id=len(features), kind='road', lanes=1 + line % 4),
            geometry=dict(type='LineString', coordinates=coords)))
    for point in range(6000):
        features.append(dict(type='Feature', properties=dict(id=len(features), kind='tree', girth=next(noise) % 300 / 100),
            geometry=dict(type='Point', coordinates=[12.40 + next(noise) % 120000 * 1e-6, 41.85 + next(noise) % 120000 * 1e-6])))
    path.write_text(json.dumps(dict(type='FeatureCollection', features=features), separators=(',', ':')))


def stored_zlib(data):
    """zlib stream of stored deflate blocks: identical bytes under every zlib build."""
    out = bytearray(b'\x78\x01')
    for start in range(0, len(data), 65535):
        block = data[start:start + 65535]
        out += bytes((start + 65535 >= len(data),)) + struct.pack('<HH', len(block), len(block) ^ 0xFFFF) + block
    return bytes(out) + struct.pack('>I', zlib.adler32(data))


def png_rgb(size):
    rows = bytearray()
    for y in range(size):
        rows.append(0)
        rows += bytes(((x * 5 + y) & 255, (x ^ y) & 255, (y * 3 + (x >> 2)) & 255)[c] for x in range(size) for c in range(3))
    chunk = lambda kind, body: struct.pack('>I', len(body)) + kind + body + struct.pack('>I', zlib.crc32(kind + body))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', size, size, 8, 2, 0, 0, 0))
            + chunk(b'IDAT', stored_zlib(bytes(rows))) + chunk(b'IEND', b''))


def write_glb(path, grid=220, extent=400.0, texture=512):
    """Textured Y-up metre height field: 96,800 triangles, under the 100k mesh-archive ceiling."""
    n = grid + 1
    positions, uvs, indices = bytearray(), bytearray(), bytearray()
    for j in range(n):
        for i in range(n):
            x, z = i / grid * extent, j / grid * extent
            positions += struct.pack('<3f', x, 12 * math.sin(i / 9) * math.cos(j / 11), -z)
            uvs += struct.pack('<2f', i / grid, j / grid)
    for j in range(grid):
        for i in range(grid):
            a = j * n + i
            indices += struct.pack('<6I', a, a + 1, a + n, a + 1, a + n + 1, a + n)
    values = struct.unpack(f'<{3 * n * n}f', positions)
    low, high = ([f(values[axis::3]) for axis in range(3)] for f in (min, max))
    image = png_rgb(texture)
    pad = lambda b: b + b'\0' * (-len(b) % 4)
    views, payload = [], bytearray()
    for blob, target in ((positions, 34962), (uvs, 34962), (indices, 34963), (image, None)):
        view = dict(buffer=0, byteOffset=len(payload), byteLength=len(blob))
        if target: view['target'] = target
        views.append(view); payload += pad(bytes(blob))
    document = dict(asset=dict(version='2.0', generator='rusty-tiles perf-lane invented mesh'), scene=0,
        scenes=[dict(nodes=[0])], nodes=[dict(mesh=0)],
        meshes=[dict(primitives=[dict(attributes=dict(POSITION=0, TEXCOORD_0=1), indices=2, material=0)])],
        materials=[dict(pbrMetallicRoughness=dict(baseColorTexture=dict(index=0), metallicFactor=0))],
        textures=[dict(source=0, sampler=0)], samplers=[dict(magFilter=9729, minFilter=9729)],
        images=[dict(bufferView=3, mimeType='image/png')],
        accessors=[dict(bufferView=0, componentType=5126, count=n * n, type='VEC3', min=low, max=high),
                   dict(bufferView=1, componentType=5126, count=n * n, type='VEC2'),
                   dict(bufferView=2, componentType=5125, count=6 * grid * grid, type='SCALAR')],
        bufferViews=views, buffers=[dict(byteLength=len(payload))])
    encoded = json.dumps(document, separators=(',', ':')).encode()
    encoded += b' ' * (-len(encoded) % 4)
    path.write_bytes(struct.pack('<III', 0x46546C67, 2, 28 + len(encoded) + len(payload))
        + struct.pack('<II', len(encoded), 0x4E4F534A) + encoded + struct.pack('<II', len(payload), 0x004E4942) + payload)


def write_tiff(path, width, height, spp, bits, sample_format, photometric, rows, geokeys, scale, origin,
               extra=(), nodata=None):
    """Little-endian strip GeoTIFF, one uncompressed strip per 16 rows."""
    row_bytes = width * spp * bits // 8
    strips = [(y, min(16, height - y)) for y in range(0, height, 16)]
    entries = [(256, 4, [width]), (257, 4, [height]), (258, 3, [bits] * spp), (259, 3, [1]), (262, 3, [photometric]),
               (273, 4, [0] * len(strips)), (277, 3, [spp]), (278, 4, [16]), (279, 4, [n * row_bytes for _, n in strips]),
               (284, 3, [1]), (339, 3, [sample_format] * spp), (33550, 12, [scale[0], scale[1], 0.0]),
               (33922, 12, [0.0, 0.0, 0.0, origin[0], origin[1], 0.0]), (34735, 3, [1, 1, 0, len(geokeys) // 4] + geokeys)]
    if extra: entries.append((338, 3, list(extra)))
    if nodata is not None: entries.append((42113, 2, nodata.encode() + b'\0'))
    entries.sort()
    sizes, codes = {2: 1, 3: 2, 4: 4, 12: 8}, {2: 'B', 3: 'H', 4: 'I', 12: 'd'}
    ifd_size = 2 + 12 * len(entries) + 4
    extra_offset, blobs = 8 + ifd_size, bytearray()
    raw, inline = {}, {}
    for tag, kind, values in entries:
        data = bytes(values) if kind == 2 else struct.pack(f'<{len(values)}{codes[kind]}', *values)
        raw[tag] = data
        if len(data) > 4:
            inline[tag] = extra_offset + len(blobs); blobs += data + b'\0' * (len(data) % 2)
    data_offset = extra_offset + len(blobs)
    offsets = [data_offset + y * row_bytes for y, _ in strips]
    raw[273] = struct.pack(f'<{len(offsets)}I', *offsets)
    if len(raw[273]) > 4: blobs[inline[273] - extra_offset:inline[273] - extra_offset + len(raw[273])] = raw[273]
    ifd = bytearray(struct.pack('<H', len(entries)))
    for tag, kind, values in entries:
        count = len(raw[tag]) // sizes[kind]
        value = struct.pack('<I', inline[tag]) if tag in inline else raw[tag].ljust(4, b'\0')
        ifd += struct.pack('<HHI', tag, kind, count) + value
    ifd += b'\0\0\0\0'
    with path.open('wb') as stream:
        stream.write(b'II*\0' + struct.pack('<I', 8) + ifd + blobs)
        for row in rows: stream.write(row)


def rgba_rows(size, hole):
    """Opaque sinusoidal colour with a transparent centre square and a half-alpha band."""
    noise = bytes(next(g) & 15 for g in [lcg(3)] for _ in range(2 * size))
    wave = [int(110 + 100 * math.sin(x / 41)) for x in range(size)]
    shift = [bytes((i + k) & 255 for i in range(256)) for k in range(256)]
    lo, hi = (size - hole) // 2, (size + hole) // 2
    for y in range(size):
        row = bytearray(size * 4)
        n = noise[y % size:y % size + size]
        row[0::4] = bytes(wave).translate(shift[(y // 7) & 255])
        row[1::4] = bytes((x * 3 // 8) & 255 for x in range(size)).translate(shift[(y * 5 // 16) & 255])
        row[2::4] = n.translate(shift[(160 + y // 32) & 255])
        alpha = bytearray(b'\xff' * size)
        if size // 10 <= y < size // 10 + 64: alpha = bytearray(b'\x80' * size)
        if lo <= y < hi:
            row[lo * 4:hi * 4] = bytes((hi - lo) * 4); alpha[lo:hi] = bytes(hi - lo)
        row[3::4] = alpha
        yield bytes(row)


def dem_rows(size):
    for y in range(size):
        yield struct.pack(f'<{size}f', *(150 + 80 * math.sin(x / 47) * math.cos(y / 53) + (x + y) * .05 for x in range(size)))


# EPSG:3857 at zoom-14 resolution near Rome; the origin is deliberately not tile-aligned.
Z14 = 2 * math.pi * 6378137 / 2**14 / 256
GEO_3857 = [1024, 0, 1, 1, 1025, 0, 1, 1, 3072, 0, 1, 3857]
GEO_4326 = [1024, 0, 1, 2, 1025, 0, 1, 1, 2048, 0, 1, 4326]
RGBA_ORIGIN, RGBA_SIZE, RGBA_HOLE = (1394000.0, 5160000.0), 3000, 1000


def mercator_degrees(x, y):
    return math.degrees(x / 6378137), math.degrees(math.atan(math.sinh(y / 6378137)))


def hole_degrees(origin, pixel, size, hole):
    """W,S,E,N of the centred transparent square of a north-up EPSG:3857 raster."""
    lo, hi = (size - hole) // 2 * pixel, (size + hole) // 2 * pixel
    return [*mercator_degrees(origin[0] + lo, origin[1] - hi), *mercator_degrees(origin[0] + hi, origin[1] - lo)]


FILES = {
    'points.las': write_las,
    'vector.geojson': write_geojson,
    'mesh.glb': write_glb,
    'rgba.tif': lambda p: write_tiff(p, RGBA_SIZE, RGBA_SIZE, 4, 8, 1, 2, rgba_rows(RGBA_SIZE, RGBA_HOLE), GEO_3857,
                                     (Z14, Z14), RGBA_ORIGIN, extra=(2,)),
    'dem.tif': lambda p: write_tiff(p, 1025, 1025, 1, 32, 3, 1, dem_rows(1025), GEO_4326, (1e-4, 1e-4), (12.4, 41.95),
                                    nodata='-32768'),
}


def sha256(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1 << 20), b''): digest.update(block)
    return digest.hexdigest()


def build(directory):
    """Create missing inputs, check pinned digests and return {name: sha256}."""
    directory.mkdir(parents=True, exist_ok=True)
    digests = {}
    for name, writer in FILES.items():
        path = directory / name
        if not path.exists():
            writer(path.with_suffix('.partial')); path.with_suffix('.partial').rename(path)
        digests[name] = sha256(path)
        if PINNED.get(name, digests[name]) != digests[name]:
            raise SystemExit(f'{path}: sha256 {digests[name]} differs from pinned {PINNED[name]}')
    return digests


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=pathlib.Path)
    print(json.dumps(build(parser.parse_args().directory), indent=2))
