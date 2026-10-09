"""Independent analytic D1 raster oracle; Python standard library only.

Run: python tests/d1_raster_oracle.py /path/to/native/rusty-tiles
No GDAL writer, production transform helper, or historical raster oracle is used.
"""
import argparse
import hashlib
import itertools
import shutil
import json
import math
from pathlib import Path
import struct
import subprocess
import tempfile
import zlib


def rgb(column, row):
    return bytes((column, row, (3 * column + 5 * row) % 256))


def affine(zoom, x, y):
    half = math.pi * 6378137.0
    span = 2 * half / (2 ** zoom)
    return (-half + x * span, half - y * span, span / 256)


def write_fixture(path, zoom=3, x=5, y=2, *, missing_crs=False,
                  shifted=False, nodata=False, alpha=False, mask=False, large_metadata=False, point=False, color_profile=False, oversized_block=False, transfer_profile=False, orientation=False, bigtiff=False, little_endian=True, codec=1, predictor=None):
    """Write one raw TIFF strip with explicit GeoTIFF tags from first principles."""
    endian = '<' if little_endian else '>'
    inline = 8 if bigtiff else 4
    header_size = 16 if bigtiff else 8
    count_size, entry_size = (8, 20) if bigtiff else (2, 12)
    left, top, step = affine(zoom, x, y)
    if point:
        # GDAL shifts PixelIsPoint tiepoints by half a pixel; compensate so
        # the resulting affine still matches the requested PixelIsArea tile.
        left += step / 2
        top -= step / 2
    if shifted:
        left += step / 2
    bands = 4 if alpha else 3
    pixels = b''.join(rgb(c, r) + (b'\xff' if alpha else b'')
                      for r in range(256) for c in range(256))
    # TIFF types: BYTE=1, ASCII=2, SHORT=3, LONG=4, DOUBLE=12.
    entries = [
        (256, 4, 1, struct.pack(endian + 'I', 256)),
        (257, 4, 1, struct.pack(endian + 'I', 256)),
        (258, 3, bands, struct.pack(endian + '' + 'H' * bands, *([8] * bands))),
        (259, 3, 1, struct.pack(endian + 'H', codec)),
        (262, 3, 1, struct.pack(endian + 'H', 2)),
        (273, 4, 1, b'OFFSET'),
        (277, 3, 1, struct.pack(endian + 'H', bands)),
        (278, 4, 1, struct.pack(endian + 'I', 256)),
        (279, 4, 1, struct.pack(endian + 'I', len(pixels))),
        (284, 3, 1, struct.pack(endian + 'H', 1)),
        (33550, 12, 3, struct.pack(endian + '3d', step, step, 0)),
        (33922, 12, 6, struct.pack(endian + '6d', 0, 0, 0, left, top, 0)),
    ]
    if not missing_crs:
        keys = (1, 1, 0, 4, 1024, 0, 1, 1, 1025, 0, 1, 2 if point else 1,
                3072, 0, 1, 3857, 3076, 0, 1, 9001)
        entries.append((34735, 3, len(keys), struct.pack(endian + '20H', *keys)))
    if predictor is not None:
        entries.append((317, 3, 1, struct.pack(endian + 'H', predictor)))
    if nodata:
        entries.append((42113, 2, 2, b'0\0'))
    if alpha:
        entries.append((338, 3, 1, struct.pack(endian + 'H', 2)))
    if orientation:
        entries.append((274, 3, 1, struct.pack(endian + 'H', 4)))
    if color_profile:
        # Opaque embedded ICC data is surfaced by GTiff COLOR_PROFILE metadata.
        profile = bytearray(128)
        struct.pack_into('>I', profile, 0, len(profile))
        profile[12:16], profile[16:20], profile[20:24] = b'mntr', b'RGB ', b'XYZ '
        profile[36:40] = b'acsp'
        entries.append((34675, 7, len(profile), bytes(profile)))
    if transfer_profile:
        curve = tuple(value * 257 for value in range(256)) * 3
        entries.append((301, 3, len(curve), struct.pack(endian + '768H', *curve)))
    if oversized_block:
        entries = [entry for entry in entries if entry[0] not in (273, 278, 279)]
        pixels = b''.join(rgb(c % 256, r % 256) for r in range(512) for c in range(512))
        entries.extend([(322, 4, 1, struct.pack(endian + 'I', 512)),
                        (323, 4, 1, struct.pack(endian + 'I', 512)),
                        (324, 4, 1, b'OFFSET'),
                        (325, 4, 1, struct.pack(endian + 'I', len(pixels)))])
    if large_metadata:
        description = b'A' * (30 * 1024 * 1024 - 1) + b'\0'
        entries.append((270, 2, len(description), description))
    entries.sort()
    offset = header_size + count_size + entry_size * len(entries) + inline
    payload = bytearray()
    directory = bytearray(struct.pack(endian + ('Q' if bigtiff else 'H'), len(entries)))
    # Strip location depends on the size of all out-of-line tag data.
    extra_size = sum(len(data) + len(data) % 2 for _, _, _, data in entries
                     if len(data) > inline and data != b'OFFSET')
    pixel_offset = offset + extra_size
    for tag, kind, count, data in entries:
        if data == b'OFFSET':
            if bigtiff:
                kind = 16  # LONG8 offset; essential for big-endian BigTIFF.
            value = struct.pack(endian + ('Q' if bigtiff else 'I'), pixel_offset)
        elif len(data) <= inline:
            value = data.ljust(inline, b'\0')
        else:
            value = struct.pack(endian + ('Q' if bigtiff else 'I'), offset + len(payload))
            payload.extend(data)
            if len(data) % 2:
                payload.append(0)
        directory.extend(struct.pack(endian + ('HHQ' if bigtiff else 'HHI'), tag, kind, count) + value)
    directory.extend(struct.pack(endian + ('Q' if bigtiff else 'I'), 0))
    marker = b'II' if little_endian else b'MM'
    header = marker + (struct.pack(endian + 'HHHQ', 43, 8, 0, 16) if bigtiff else struct.pack(endian + 'HI', 42, 8))
    image = bytearray(header + directory + payload + pixels)
    if mask:
        # A second IFD explicitly declares an internal transparency mask.
        mask_offset = len(image)
        if mask_offset % 2:
            image.append(0)
            mask_offset += 1
        struct.pack_into('<I', image, 8 + 2 + 12 * len(entries), mask_offset)
        mask_tags = [(254, 4, 4), (256, 4, 256), (257, 4, 256),
                     (258, 3, 1), (259, 3, 1), (262, 3, 4),
                     (273, 4, mask_offset + 2 + 12 * 10 + 4),
                     (277, 3, 1), (278, 4, 256), (279, 4, 8192)]
        image.extend(struct.pack(endian + 'H', len(mask_tags)))
        for tag, kind, value in mask_tags:
            field = struct.pack(endian + 'H', value).ljust(4, b'\0') if kind == 3 else struct.pack(endian + 'I', value)
            image.extend(struct.pack(endian + 'HHI', tag, kind, 1) + field)
        image.extend(struct.pack(endian + 'I', 0))
        image.extend(b'\xff' * 8192)
    Path(path).write_bytes(image)


def write_storage_fixture(path, *, compression, planar, tiled, bigtiff, little_endian):
    """Independent finite storage matrix: two strips or four 128-square tiles."""
    endian = '<' if little_endian else '>'
    inline, header_size, count_size, entry_size = (8, 16, 8, 20) if bigtiff else (4, 8, 2, 12)
    def pack(code, *values):
        return struct.pack(endian + code, *values)
    blocks = []
    planes = range(3) if planar == 2 else (None,)
    for plane in planes:
        for row_origin in (0, 128):
            for col_origin in ((0, 128) if tiled else (0,)):
                width = 128 if tiled else 256
                block = b''.join(rgb(c, r) if plane is None else rgb(c, r)[plane:plane + 1]
                                 for r in range(row_origin, row_origin + 128)
                                 for c in range(col_origin, col_origin + width))
                blocks.append(zlib.compress(block) if compression in (8, 32946) else block)
    left, top, step = affine(3, 5, 2)
    keys = (1, 1, 0, 4, 1024, 0, 1, 1, 1025, 0, 1, 1, 3072, 0, 1, 3857, 3076, 0, 1, 9001)
    entries = [(256, 4, 1, pack('I', 256)), (257, 4, 1, pack('I', 256)),
               (258, 3, 3, pack('3H', 8, 8, 8)), (259, 3, 1, pack('H', compression)),
               (262, 3, 1, pack('H', 2)), (277, 3, 1, pack('H', 3)),
               (284, 3, 1, pack('H', planar)), (317, 3, 1, pack('H', 1)),
               (33550, 12, 3, pack('3d', step, step, 0)),
               (33922, 12, 6, pack('6d', 0, 0, 0, left, top, 0)),
               (34735, 3, 20, pack('20H', *keys))]
    offset_tag, count_tag = (324, 325) if tiled else (273, 279)
    if tiled:
        entries.extend([(322, 4, 1, pack('I', 128)), (323, 4, 1, pack('I', 128))])
    else:
        entries.append((278, 4, 1, pack('I', 128)))
    offset_code, offset_type = ('Q', 16) if bigtiff else ('I', 4)
    entries.extend([(offset_tag, offset_type, len(blocks), bytes(len(blocks) * inline)),
                    (count_tag, 4, len(blocks), pack('I' * len(blocks), *(len(b) for b in blocks)))])
    entries.sort()
    external_start = header_size + count_size + entry_size * len(entries) + inline
    data_start = external_start + sum(len(value) + len(value) % 2 for _, _, _, value in entries if len(value) > inline)
    offsets, position = [], data_start
    for block in blocks:
        offsets.append(position)
        position += len(block)
    entries = [(tag, kind, count, pack(offset_code * len(offsets), *offsets) if tag == offset_tag else value)
               for tag, kind, count, value in entries]
    directory = bytearray(pack('Q' if bigtiff else 'H', len(entries)))
    external = bytearray()
    for tag, kind, count, value in entries:
        if len(value) <= inline:
            field = value.ljust(inline, b'\0')
        else:
            field = pack('Q' if bigtiff else 'I', external_start + len(external))
            external.extend(value)
            if len(value) % 2:
                external.append(0)
        directory.extend(pack('HHQ' if bigtiff else 'HHI', tag, kind, count) + field)
    directory.extend(pack('Q' if bigtiff else 'I', 0))
    marker = b'II' if little_endian else b'MM'
    header = marker + (pack('HHHQ', 43, 8, 0, 16) if bigtiff else pack('HI', 42, 8))
    Path(path).write_bytes(header + directory + external + b''.join(blocks))


def decode_png(path):
    """Decode noninterlaced 8-bit RGB/RGBA PNG, independently of image crates."""
    data = Path(path).read_bytes()
    assert data[:8] == b'\x89PNG\r\n\x1a\n'
    position, compressed = 8, bytearray()
    width = height = channels = None
    while position < len(data):
        size = struct.unpack_from('>I', data, position)[0]
        kind = data[position + 4:position + 8]
        chunk = data[position + 8:position + 8 + size]
        crc = struct.unpack_from('>I', data, position + 8 + size)[0]
        assert zlib.crc32(kind + chunk) & 0xffffffff == crc
        if kind == b'IHDR':
            width, height, depth, color, compression, filtering, interlace = struct.unpack('>IIBBBBB', chunk)
            assert depth == 8 and color in (2, 6)
            assert compression == filtering == interlace == 0
            channels = 3 if color == 2 else 4
        elif kind == b'IDAT':
            compressed.extend(chunk)
        elif kind == b'IEND':
            break
        position += 12 + size
    assert width == height == 256
    raw = zlib.decompress(compressed)
    stride = width * channels
    assert len(raw) == height * (stride + 1)
    previous = bytearray(stride)
    pixels = bytearray()
    for row in range(height):
        start = row * (stride + 1)
        filter_type = raw[start]
        assert filter_type in range(5)
        current = bytearray(raw[start + 1:start + 1 + stride])
        for index in range(stride):
            left = current[index - channels] if index >= channels else 0
            up = previous[index]
            upper_left = previous[index - channels] if index >= channels else 0
            if filter_type == 0:
                predictor = 0
            elif filter_type == 1:
                predictor = left
            elif filter_type == 2:
                predictor = up
            elif filter_type == 3:
                predictor = (left + up) // 2
            else:
                estimate = left + up - upper_left
                distances = (abs(estimate - left), abs(estimate - up), abs(estimate - upper_left))
                predictor = (left, up, upper_left)[distances.index(min(distances))]
            current[index] = (current[index] + predictor) & 255
        pixels.extend(current)
        previous = current
    return channels, pixels


def validate(output, source, zoom, x, y):
    expected_files = {f'tiles/{zoom}/{x}/{y}.png', 'tilejson.json', 'report.json'}
    assert {str(p.relative_to(output)) for p in output.rglob('*') if p.is_file()} == expected_files
    channels, pixels = decode_png(output / f'tiles/{zoom}/{x}/{y}.png')
    validate_pixels(channels, pixels)
    tilejson = json.loads((output / 'tilejson.json').read_text())
    validate_tilejson(tilejson, zoom, x, y)
    report = json.loads((output / 'report.json').read_text())
    expected_report = dict(schema_version=1, profile='d1-web-mercator-rgb',
                           source_bytes=source.stat().st_size, width=256, height=256,
                           z=zoom, x=x, y=y)
    assert report == expected_report
    return report


def validate_pixels(channels, pixels):
    for row in range(256):
        for column in range(256):
            start = (row * 256 + column) * channels
            assert pixels[start:start + 3] == rgb(column, row), (column, row)
            if channels == 4:
                assert pixels[start + 3] == 255


def validate_tilejson(tilejson, zoom, x, y):
    assert tilejson['tilejson'] == '3.0.0'
    assert tilejson['scheme'] == 'xyz'
    assert tilejson['minzoom'] == tilejson['maxzoom'] == zoom
    assert tilejson['tiles'] == ['tiles/{z}/{x}/{y}.png']
    n = 2 ** zoom
    west, east = x / n * 360 - 180, (x + 1) / n * 360 - 180
    south = math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * (y + 1) / n))))
    north = math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * y / n))))
    assert len(tilejson['bounds']) == 4
    for actual, expected in zip(tilejson['bounds'], (west, south, east, north)):
        assert abs(actual - expected) <= 1e-9, (actual, expected)


def must_detect(action):
    try:
        action()
    except AssertionError:
        return
    raise AssertionError('Oracle sensitivity control failed to detect corruption')


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def run(binary, json_output=None):
    evidence = dict(binary_sha256=digest(binary), positive_cases=[], refusal_cases=[],
                    sensitivity_controls=[], resource_method='small C exec/fork/wait4 launcher (child max RSS KiB on Linux)',
                    resource_limits='Observed child RSS only; no bound inferred for all TIFF metadata/codecs')
    with tempfile.TemporaryDirectory(prefix='d1-raster-oracle-') as temporary:
        root = Path(temporary)
        launcher = root / 'measure-child'
        launcher_source = Path(__file__).resolve().parents[1] / 'bench/architecture_audit/f1a/measure_child.c'
        subprocess.run(['cc', '-O2', str(launcher_source), '-o', str(launcher)], check=True, capture_output=True)
        evidence['measurement_helper_sha256'] = digest(launcher_source)
        def invoke(input_path, output_path, zoom=3, x=5, y=2):
            rss = root / 'child-rss.txt'
            command = [str(launcher), str(rss), str(binary), '--json',
                       'raster-tile-to-directory', '-i', str(input_path), '-o', str(output_path),
                       '--zoom', str(zoom), '--x', str(x), '--y', str(y)]
            result = subprocess.run(command, capture_output=True, text=True)
            measurements = rss.read_text().splitlines()
            peak = int(measurements[-1])
            return result, peak
        storage_cases = {}
        for compression, planar, tiled, bigtiff, little_endian in itertools.product((1, 8), (1, 2), (False, True), (False, True), (False, True)):
            name = f'storage-c{compression}-p{planar}-t{int(tiled)}-b{int(bigtiff)}-le{int(little_endian)}'
            storage_cases[name] = dict(compression=compression, planar=planar, tiled=tiled, bigtiff=bigtiff, little_endian=little_endian)
        positive_cases = [('analytic', 3, 5, 2, False),
                                        ('zoom0', 0, 0, 0, False),
                                        ('zoom24', 24, 12345678, 8765432, False),
                                        ('input-ceiling', 3, 5, 2, True),
                                        ('large-metadata', 3, 5, 2, False),
                                        ('classic-big-endian', 3, 5, 2, False),
                                        ('bigtiff-little-endian', 3, 5, 2, False),
                                        ('bigtiff-big-endian', 3, 5, 2, False)]
        positive_cases.extend((name, 3, 5, 2, False) for name in storage_cases)
        for name, zoom, x, y, padded in positive_cases:
            source, output = root / (name + '.tif'), root / (name + '-out')
            write_fixture(source, zoom, x, y, large_metadata=name == 'large-metadata',
                          bigtiff=name.startswith('bigtiff-'), little_endian=not name.endswith('big-endian'))
            if name in storage_cases:
                write_storage_fixture(source, **storage_cases[name])
            if padded:
                with source.open('r+b') as stream:
                    stream.truncate(32 * 1024 * 1024)
            result, peak = invoke(source, output, zoom, x, y)
            assert result.returncode == 0, (name, result.stdout, result.stderr)
            report = validate(output, source, zoom, x, y)
            assert json.loads(result.stdout)['rasterReport'] == report
            assert not list(root.glob('.tiles-dir-*'))
            evidence['positive_cases'].append(dict(name=name, source_sha256=digest(source),
                source_bytes=source.stat().st_size, child_peak_rss_kib=peak, report=report,
                checked_pixels=65536, report_fact_checks=len(report)))
        source, output = root / 'analytic.tif', root / 'analytic-out'
        tile = output / 'tiles/3/5/2.png'
        channels, pixels = decode_png(tile)
        pixels[0] ^= 1
        must_detect(lambda: validate_pixels(channels, pixels))
        evidence['sensitivity_controls'].append('corrupt-decoded-pixel-detected')
        tilejson = json.loads((output / 'tilejson.json').read_text())
        tilejson['bounds'][0] += 1
        must_detect(lambda: validate_tilejson(tilejson, 3, 5, 2))
        evidence['sensitivity_controls'].append('wrong-bounds-detected')
        missing = root / 'missing-member'
        shutil.copytree(output, missing)
        (missing / 'report.json').unlink()
        must_detect(lambda: validate(missing, source, 3, 5, 2))
        evidence['sensitivity_controls'].append('missing-member-detected')
        for name, options in [('missing-crs', {'missing_crs': True}),
                              ('shifted', {'shifted': True}), ('nodata', {'nodata': True}),
                              ('alpha', {'alpha': True}), ('mask', {'mask': True}),
                              ('above-input-ceiling', {}),
                              ('pixel-is-point-compensated', {'point': True}),
                              ('embedded-color-profile', {'color_profile': True}),
                              ('transfer-profile', {'transfer_profile': True}),
                              ('oversized-storage-block', {'oversized_block': True}),
                              ('bottom-up-orientation', {'orientation': True}),
                              ('truncated-header', {}), ('bad-table-offset', {}),
                              ('oversized-entry-count', {}),
                              ('codec-lzw-refused', {'codec': 5}),
                              ('codec-jpeg-refused', {'codec': 7}),
                              ('codec-zstd-refused', {'codec': 50000}),
                              ('codec-legacy-deflate-refused', {}),
                              ('predictor-two-refused', {'predictor': 2})]:
            invalid, destination = root / (name + '.tif'), root / (name + '-out')
            write_fixture(invalid, **options)
            if name == 'codec-legacy-deflate-refused':
                write_storage_fixture(invalid, compression=32946, planar=1, tiled=False, bigtiff=False, little_endian=True)
            if name == 'above-input-ceiling':
                with invalid.open('r+b') as stream:
                    stream.truncate(32 * 1024 * 1024 + 1)
            if name == 'truncated-header':
                invalid.write_bytes(b'II')
            elif name == 'bad-table-offset':
                image = bytearray(invalid.read_bytes())
                struct.pack_into('<I', image, 4, len(image) + 1024)
                invalid.write_bytes(image)
            elif name == 'oversized-entry-count':
                image = bytearray(invalid.read_bytes())
                struct.pack_into('<H', image, 8, 4097)
                invalid.write_bytes(image)
            refused, peak = invoke(invalid, destination)
            assert refused.returncode != 0, name
            expected_kind = 'invalid_input' if name in ('truncated-header', 'bad-table-offset') else 'unsupported'
            assert json.loads(refused.stdout)['error']['kind'] == expected_kind, (name, refused.stdout)
            assert not list(root.glob('.tiles-dir-*'))
            assert not destination.exists(), name
            evidence['refusal_cases'].append(dict(name=name, error_kind=expected_kind, exit_code=refused.returncode,
                                                   child_peak_rss_kib=peak))
        for bigtiff, little_endian in itertools.product((True, False), (True, False)):
            for label in ('high-bit', 'maximum', 'file-end', 'tail-seven', 'small-offset'):
                if not bigtiff and label == 'high-bit':
                    continue
                invalid, destination = root / 'offset.tif', root / 'offset-out'
                write_fixture(invalid, bigtiff=bigtiff, little_endian=little_endian)
                data = bytearray(invalid.read_bytes())
                offset = {'high-bit': 2 ** 63, 'maximum': 2 ** (64 if bigtiff else 32) - 1,
                          'file-end': len(data), 'tail-seven': len(data) - (7 if bigtiff else 1),
                          'small-offset': 4}[label]
                struct.pack_into(('<' if little_endian else '>') + ('Q' if bigtiff else 'I'), data, 8 if bigtiff else 4, offset)
                invalid.write_bytes(data)
                refused, peak = invoke(invalid, destination)
                failure = json.loads(refused.stdout)
                assert refused.returncode == 3 and failure['error']['kind'] == 'invalid_input', (label, failure)
                assert not destination.exists() and not list(root.glob('.tiles-dir-*'))
                evidence['refusal_cases'].append(dict(name=f'offset-b{int(bigtiff)}-le{int(little_endian)}-{label if bigtiff or label != "tail-seven" else "tail-one"}',
                    error_kind='invalid_input', exit_code=refused.returncode, child_peak_rss_kib=peak))
        before = {p.relative_to(output): p.read_bytes() for p in output.rglob('*') if p.is_file()}
        refused, peak = invoke(source, output)
        assert refused.returncode != 0
        assert json.loads(refused.stdout)['error']['kind'] == 'output_conflict'
        assert not list(root.glob('.tiles-dir-*'))
        after = {p.relative_to(output): p.read_bytes() for p in output.rglob('*') if p.is_file()}
        assert before == after
        evidence['refusal_cases'].append(dict(name='existing-output-preserved', error_kind='output_conflict', exit_code=refused.returncode,
                                               child_peak_rss_kib=peak))
    evidence['num_positive_cases'] = len(evidence['positive_cases'])
    evidence['num_refusal_cases'] = len(evidence['refusal_cases'])
    evidence['num_sensitivity_controls'] = len(evidence['sensitivity_controls'])
    if json_output:
        Path(json_output).write_text(json.dumps(evidence, indent=2) + '\n')
    print(f"Independent D1 raster oracle passed: {evidence['num_positive_cases']} positives, {evidence['num_refusal_cases']} refusals, {evidence['num_sensitivity_controls']} sensitivity controls")
    return evidence


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('--json-output', type=Path)
    arguments = parser.parse_args()
    run(arguments.binary.resolve(), arguments.json_output)
