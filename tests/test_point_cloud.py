"""Decode generated content, independently check fidelity, bounds and LOD."""
import json
import os
import pathlib
import struct
import subprocess
import tempfile
import types
import unittest
import zipfile

import laspy
import numpy as np

ROOT = pathlib.Path(__file__).resolve().parents[1]
BIN = os.environ.get('RUSTY_TILES_BIN')


def run(args):
    archive = pathlib.Path(args.output).with_suffix('.3tz')
    if pathlib.Path(args.output).exists():
        raise ValueError('output directory already exists')
    command = [BIN, '--json', 'point-cloud', '-i', args.input, '-o', str(archive),
               '--sourceCrs', args.source_crs, '--maxPoints', str(args.max_points),
               '--chunkPoints', str(args.chunk_points)]
    if args.height_offset is not None:
        command += ['--heightOffset', str(args.height_offset)]
    result = subprocess.run(command, capture_output=True, text=True, env=dict(os.environ, PATH=''))
    response = json.loads(result.stdout)
    if result.returncode:
        raise ValueError(response['error']['message'])
    with zipfile.ZipFile(archive) as z:
        report = json.loads(z.read('conversion.json'))
        for name in z.namelist():
            if name == '@3dtilesIndex1@':
                continue
            path = pathlib.Path(args.output) / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(z.read(name))
    return report


def read_glb(path):
    data = path.read_bytes()
    magic, version, length = struct.unpack_from('<4sII', data)
    assert (magic, version, length) == (b'glTF', 2, len(data))
    size = struct.unpack_from('<I', data, 12)[0]
    doc = json.loads(data[20:20 + size])
    binary = data[28 + size:]
    def view(index, dtype, width=1):
        v = doc['bufferViews'][index]
        a = np.frombuffer(binary[v.get('byteOffset', 0):v.get('byteOffset', 0) + v['byteLength']], dtype=dtype)
        return a.reshape(-1, width) if width != 1 else a
    primitive = doc['meshes'][0]['primitives'][0]
    accessor = doc['accessors'][primitive['attributes']['POSITION']]
    positions = view(accessor['bufferView'], '<f4', 3)[:, [0, 2, 1]] * [1, -1, 1]
    metadata = doc['extensions']['EXT_structural_metadata']
    table = metadata['propertyTables'][0]
    mapping = {'UINT8': '<u1', 'UINT16': '<u2', 'UINT32': '<u4', 'UINT64': '<u8',
               'INT8': '<i1', 'INT16': '<i2', 'INT32': '<i4', 'INT64': '<i8',
               'FLOAT32': '<f4', 'FLOAT64': '<f8'}
    props = {name: view(column['values'], mapping[metadata['schema']['classes']['point']['properties'][name]['componentType']])
             for name, column in table['properties'].items()}
    return positions, props


def fixture(path, count=257, identical=False, crs=None, point_format=3):
    header = laspy.LasHeader(point_format=point_format, version='1.4' if point_format >= 6 else '1.2')
    header.scales = [0.001] * 3
    header.offsets = [500000, 5000000, 80]
    header.add_extra_dim(laspy.ExtraBytesParams(name='temperature', type='float64'))
    if crs:
        header.add_crs(crs)
    data = laspy.LasData(header)
    i = np.arange(count)
    data.x = 500000 + (i % 17) * .713
    data.y = 5000000 + (i // 17) * .627
    data.z = 80 + np.sin(i) * 2
    if identical:
        data.x = np.full(count, 500000.)
        data.y = np.full(count, 5000000.)
        data.z = np.full(count, 80.)
    if 'red' in data.point_format.dimension_names:
        data.red = ((i * 233) % 65536).astype('u2')
        data.green = ((i * 431) % 65536).astype('u2')
        data.blue = ((i * 717) % 65536).astype('u2')
    data.classification = (i % 20).astype('u1')
    data.intensity = (i * 13).astype('u2')
    data.return_number = (i % 3 + 1).astype('u1')
    data.number_of_returns = np.full(count, 3, dtype='u1')
    data.temperature = i / 7
    data.write(path)
    return data


def evlr_fixture(path, extended_extra=False):
    from pyproj import CRS
    from laspy.vlrs.vlrlist import VLRList
    data = fixture(path, count=5, point_format=7, crs=CRS.from_epsg(32632))
    wkt = data.header.vlrs.extract('WktCoordinateSystemVlr')[0]
    data.evlrs = VLRList([
        laspy.VLR(user_id='unrelated', record_id=1, record_data=b'ignored'),
        wkt,
    ])
    if extended_extra:
        data.evlrs.append(data.header.vlrs.extract('ExtraBytesVlr')[0])
    data.write(path)
    return data


@unittest.skipUnless(BIN, 'set RUSTY_TILES_BIN for native CLI acceptance')
class PointCloudTests(unittest.TestCase):
    def convert(self, tmp, suffix='.las', **kwargs):
        source = pathlib.Path(tmp) / ('cloud' + suffix)
        data = fixture(source, **kwargs)
        out = pathlib.Path(tmp) / 'tiles'
        args = types.SimpleNamespace(input=str(source), output=str(out), source_crs='local',
                                     height_offset=None, max_points=16, chunk_points=11)
        report = run(args)
        return data, out, report

    def test_las_and_laz_fidelity_bounds_and_parent_error(self):
        for suffix in ('.las', '.laz'):
            with self.subTest(suffix=suffix), tempfile.TemporaryDirectory() as tmp:
                data, out, report = self.convert(tmp, suffix)
                manifest = json.loads((out / 'tileset.json').read_text())
                originals = np.column_stack([data.x, data.y, data.z])
                seen = []
                def walk(node, parent_center):
                    center = parent_center + np.array(node['transform'][12:15])
                    positions, props = read_glb(out / node['content']['uri'])
                    global_points = positions + center
                    ids = props['source_index'].astype(int)
                    np.testing.assert_allclose(global_points, originals[ids], atol=2e-6, rtol=0)
                    for name in data.point_format.dimension_names:
                        np.testing.assert_array_equal(props[name], np.asarray(data[name])[ids])
                    np.testing.assert_array_equal(np.column_stack([props['source_x'], props['source_y'], props['source_z']]), originals[ids])
                    self.assertLessEqual(len(ids), 16)
                    box = node['boundingVolume']['box']
                    half = np.array([box[3], box[7], box[11]])
                    self.assertTrue((np.abs(positions) <= half + 1e-8).all())
                    if 'children' not in node:
                        self.assertEqual(node['geometricError'], 0)
                        seen.extend(ids.tolist())
                        return ids
                    child_ids = np.concatenate([walk(child, center) for child in node['children']])
                    # Reconstruct voxel membership from the original records,
                    # independently of the converter's scratch files or sampling.
                    lo = originals[child_ids].min(axis=0)
                    extent = originals[child_ids].max(axis=0) - lo
                    safe = np.where(extent > 0, extent, 1)
                    first = {}
                    for source_id in sorted(child_ids):
                        key = tuple(np.minimum(((originals[source_id] - lo) / safe * 2).astype(int), 1))
                        first.setdefault(key, source_id)
                    self.assertEqual(sorted(ids), sorted(first.values()))
                    # Independent exhaustive nearest-representative distance on small fixture.
                    distances = np.linalg.norm(originals[child_ids, None, :] - global_points[None, :, :], axis=2).min(axis=1)
                    self.assertLessEqual(float(distances.max()), node['geometricError'] + 2e-6)
                    self.assertTrue(all(node['geometricError'] >= c['geometricError'] for c in node['children']))
                    self.assertTrue((np.abs(originals[child_ids] - center) <= half + 1e-8).all())
                    return child_ids
                walk(manifest['root'], np.zeros(3))
                self.assertEqual(sorted(seen), list(range(len(data.points))))
                self.assertFalse((out / 'scratch').exists())
                self.assertGreater(report['tiles'], 1)

    def test_duplicate_positions_terminate_without_losing_records(self):
        with tempfile.TemporaryDirectory() as tmp:
            data, out, _ = self.convert(tmp, identical=True)
            ids = []
            def walk(node):
                if 'children' in node:
                    for child in node['children']:
                        walk(child)
                else:
                    ids.extend(read_glb(out / node['content']['uri'])[1]['source_index'].tolist())
            walk(json.loads((out / 'tileset.json').read_text())['root'])
            self.assertEqual(sorted(ids), list(range(len(data.points))))

    def test_projected_crs_and_explicit_height(self):
        from pyproj import CRS
        with tempfile.TemporaryDirectory() as tmp:
            source = pathlib.Path(tmp) / 'cloud.las'
            data = fixture(source, count=1, crs=CRS.from_epsg(32632))
            out = pathlib.Path(tmp) / 'tiles'
            args = types.SimpleNamespace(input=str(source), output=str(out), source_crs='header',
                                         height_offset=10., max_points=16, chunk_points=11)
            run(args)
            root = json.loads((out / 'tileset.json').read_text())['root']
            # UTM central meridian 9°; inverse northing latitude independently tabulated.
            lon, lat, h = np.radians(9), np.radians(45.153477183356024), float(data.z[0]) + 10
            a, e2 = 6378137., 6.6943799901413165e-3
            n = a / np.sqrt(1 - e2 * np.sin(lat)**2)
            expected = [(n+h)*np.cos(lat)*np.cos(lon), (n+h)*np.cos(lat)*np.sin(lon), (n*(1-e2)+h)*np.sin(lat)]
            np.testing.assert_allclose(root['transform'][12:15], expected, atol=.001, rtol=0)

    def test_projected_tree_positions_match_an_independent_pyproj_reader(self):
        from pyproj import CRS, Transformer
        with tempfile.TemporaryDirectory() as tmp:
            source = pathlib.Path(tmp) / 'cloud.laz'
            data = fixture(source, crs=CRS.from_epsg(32632))
            out = pathlib.Path(tmp) / 'tiles'
            run(types.SimpleNamespace(input=str(source), output=str(out), source_crs='header',
                height_offset=10., max_points=16, chunk_points=11))
            operation = Transformer.from_crs(CRS.from_epsg(32632).to_3d(), CRS.from_epsg(4978),
                always_xy=True, allow_ballpark=False, only_best=True)
            expected = np.column_stack(operation.transform(data.x, data.y, np.asarray(data.z) + 10., errcheck=True))
            seen = []
            def walk(node, origin):
                center = origin + np.asarray(node['transform'][12:15])
                positions, props = read_glb(out / node['content']['uri'])
                ids = props['source_index'].astype(int)
                np.testing.assert_allclose(positions + center, expected[ids], atol=2e-6, rtol=0)
                for name in data.point_format.dimension_names:
                    np.testing.assert_array_equal(props[name], np.asarray(data[name])[ids])
                if 'children' in node:
                    for child in node['children']:
                        walk(child, center)
                else:
                    seen.extend(ids.tolist())
            walk(json.loads((out / 'tileset.json').read_text())['root'], np.zeros(3))
            self.assertEqual(sorted(seen), list(range(len(data.points))))

    def test_errors_clean_staging_and_do_not_replace_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = pathlib.Path(tmp) / 'cloud.las'
            fixture(source)
            out = pathlib.Path(tmp) / 'tiles'
            args = types.SimpleNamespace(input=str(source), output=str(out), source_crs='EPSG:32632',
                                         height_offset=None, max_points=16, chunk_points=11)
            with self.assertRaisesRegex(ValueError, 'explicit finite'):
                run(args)
            self.assertFalse(out.exists())
            out.mkdir()
            (out / 'keep').write_text('original')
            with self.assertRaisesRegex(ValueError, 'already exists'):
                run(args)
            self.assertEqual((out / 'keep').read_text(), 'original')

    def test_scaled_extra_dimension_is_not_truncated(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = pathlib.Path(tmp) / 'cloud.las'
            data = fixture(source, count=3)
            data.add_extra_dim(laspy.ExtraBytesParams(name='scaled', type='i2', scales=[.1], offsets=[3]))
            data.scaled = np.array([3.1, 4.2, 5.3])
            data.write(source)
            out = pathlib.Path(tmp) / 'tiles'
            run(types.SimpleNamespace(input=str(source), output=str(out), source_crs='local',
                height_offset=None, max_points=16, chunk_points=11))
            root = json.loads((out / 'tileset.json').read_text())['root']
            props = read_glb(out / root['content']['uri'])[1]
            np.testing.assert_array_equal(props['scaled'], np.asarray(data.scaled))

    def test_reject_vector_extra_dimensions(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = pathlib.Path(tmp) / 'cloud.las'
            data = fixture(source)
            data.add_extra_dim(laspy.ExtraBytesParams(name='vector', type='3f4'))
            data.write(source)
            out = pathlib.Path(tmp) / 'tiles'
            args = types.SimpleNamespace(input=str(source), output=str(out), source_crs='local',
                                         height_offset=None, max_points=16, chunk_points=11)
            with self.assertRaisesRegex(ValueError, 'unsupported LAS dimension'):
                run(args)
            self.assertFalse(out.exists())

    def test_all_supported_formats_preserve_original_scalar_fields_and_flags(self):
        for fmt in (0, 1, 2, 3, 6, 7, 8):
            for suffix in ('.las', '.laz'):
                with self.subTest(fmt=fmt, suffix=suffix), tempfile.TemporaryDirectory() as tmp:
                    source = pathlib.Path(tmp) / ('cloud' + suffix)
                    data = fixture(source, count=23, point_format=fmt)
                    i = np.arange(23)
                    for name in ('synthetic', 'key_point', 'withheld', 'scan_direction_flag', 'edge_of_flight_line'):
                        data[name] = (i % 2).astype('u1')
                    data.user_data = (i * 7).astype('u1')
                    data.point_source_id = (i * 2333).astype('u2')
                    if fmt >= 6:
                        data.classification = (i * 11).astype('u1')
                        data.scanner_channel = (i % 4).astype('u1')
                        data.overlap = (i % 3 == 0).astype('u1')
                        data.scan_angle = (i * 113 - 2300).astype('i2')
                    else:
                        data.scan_angle_rank = (i * 7 - 88).astype('i1')
                    if 'gps_time' in data.point_format.dimension_names:
                        data.gps_time = i / 7
                    if fmt == 8:
                        data.nir = (i * 431).astype('u2')
                    data.write(source)
                    out = pathlib.Path(tmp) / 'tiles'
                    run(types.SimpleNamespace(input=str(source), output=str(out), source_crs='local',
                        height_offset=None, max_points=30, chunk_points=3))
                    doc = json.loads((out / 'tileset.json').read_text())
                    props = read_glb(out / doc['root']['content']['uri'])[1]
                    for name in data.point_format.dimension_names:
                        np.testing.assert_array_equal(props[name], np.asarray(data[name]), err_msg=f'{fmt} {name}')

    def test_all_extra_scalar_types_keep_large_integers_and_decoded_scaling(self):
        cases = [('u1', [0, 255, 13]), ('i1', [-128, 127, -13]),
                 ('u2', [0, 65535, 13]), ('i2', [-32768, 32767, -13]),
                 ('u4', [0, 4294967295, 13]), ('i4', [-2147483648, 2147483647, -13]),
                 ('u8', [0, 18446744073709551615, 9007199254740993]),
                 ('i8', [-9223372036854775808, 9223372036854775807, -9007199254740993]),
                 ('f4', [.125, -.375, 12345.25]), ('f8', [.125, -.375, 12345.250000001])]
        for suffix in ('.las', '.laz'):
            with self.subTest(suffix=suffix), tempfile.TemporaryDirectory() as tmp:
                source = pathlib.Path(tmp) / ('cloud' + suffix)
                data = fixture(source, count=3)
                for kind, values in cases:
                    data.add_extra_dim(laspy.ExtraBytesParams(name='extra_' + kind, type=kind))
                    data['extra_' + kind] = np.array(values, dtype=kind)
                for name, scales, offsets in [('both', [.1], [3.]), ('scale_only', [.25], None), ('offset_only', None, [.125])]:
                    data.add_extra_dim(laspy.ExtraBytesParams(name=name, type='i2', scales=scales or [1.], offsets=offsets or [0.]))
                    data[name] = np.array([1, -2, 13]) * (scales[0] if scales else 1) + (offsets[0] if offsets else 0)
                data.write(source)
                # laspy writes both scale/offset flags together. LAS permits
                # either independently; create those valid declarations directly.
                content = bytearray(source.read_bytes())
                content[227 + 54 + 12 * 192 + 3] &= ~16
                content[227 + 54 + 13 * 192 + 3] &= ~8
                source.write_bytes(content)
                out = pathlib.Path(tmp) / 'tiles'
                run(types.SimpleNamespace(input=str(source), output=str(out), source_crs='local',
                    height_offset=None, max_points=30, chunk_points=1))
                props = read_glb(out / 't/0.glb')[1]
                for name in data.point_format.extra_dimension_names:
                    np.testing.assert_array_equal(props[name], np.asarray(data[name]), err_msg=name)
                for name in ('both', 'scale_only', 'offset_only'):
                    self.assertEqual(props[name].dtype, np.dtype('<f8'))

    def test_wkt_header_crs_and_declared_3d_heights_policy(self):
        from pyproj import CRS
        with tempfile.TemporaryDirectory() as tmp:
            source = pathlib.Path(tmp) / 'cloud.laz'
            data = fixture(source, count=1, point_format=7, crs=CRS.from_epsg(32632))
            out = pathlib.Path(tmp) / 'tiles'
            report = run(types.SimpleNamespace(input=str(source), output=str(out), source_crs='header',
                height_offset=10., max_points=16, chunk_points=11))
            self.assertIn('WGS 84', report['resolvedCrs'])
            props = read_glb(out / 't/0.glb')[1]
            np.testing.assert_array_equal(props['source_z'], np.asarray(data.z))
        for definition in ('EPSG:4979', 'EPSG:4978', 'EPSG:4326+5773'):
            with self.subTest(definition=definition), tempfile.TemporaryDirectory() as tmp:
                source = pathlib.Path(tmp) / 'cloud.las'
                fixture(source, count=1, point_format=7, crs=CRS.from_user_input(definition))
                out = pathlib.Path(tmp) / 'tiles'
                with self.assertRaisesRegex(ValueError, '2D horizontal CRS'):
                    run(types.SimpleNamespace(input=str(source), output=str(out), source_crs='header',
                        height_offset=0., max_points=16, chunk_points=11))
                self.assertFalse(out.exists())

    def test_wkt_and_extra_bytes_after_unrelated_evlr(self):
        from pyproj import CRS, Transformer
        transform = Transformer.from_crs(32632, 4978, always_xy=True)
        for suffix in ('.las', '.laz'):
            for extended_extra in (False, True):
                with self.subTest(suffix=suffix, extended_extra=extended_extra), tempfile.TemporaryDirectory() as tmp:
                    source = pathlib.Path(tmp) / ('cloud' + suffix)
                    data = evlr_fixture(source, extended_extra)
                    independent = laspy.read(source)
                    self.assertEqual(len(independent.evlrs), 3 if extended_extra else 2)
                    self.assertEqual(independent.header.parse_crs(), CRS.from_epsg(32632))
                    out = pathlib.Path(tmp) / 'tiles'
                    report = run(types.SimpleNamespace(input=str(source), output=str(out), source_crs='header',
                        height_offset=7., max_points=16, chunk_points=2))
                    self.assertEqual(CRS.from_wkt(report['resolvedCrs']), CRS.from_epsg(32632))
                    self.assertEqual(report['points'], len(data.points))
                    manifest = json.loads((out / 'tileset.json').read_text())
                    positions, props = read_glb(out / 't/0.glb')
                    placed = positions + np.array(manifest['root']['transform'][12:15])
                    expected = np.column_stack(transform.transform(data.x, data.y, np.asarray(data.z) + 7.))
                    np.testing.assert_allclose(placed, expected, atol=2e-6, rtol=0)
                    for name in data.point_format.dimension_names:
                        np.testing.assert_array_equal(props[name], np.asarray(data[name]), err_msg=name)

    def test_invalid_later_evlrs_fail_without_replacing_output(self):
        for suffix in ('.las', '.laz'):
            for case in ('duplicate_wkt', 'truncated'):
                with self.subTest(suffix=suffix, case=case), tempfile.TemporaryDirectory() as tmp:
                    source = pathlib.Path(tmp) / ('cloud' + suffix)
                    data = evlr_fixture(source)
                    if case == 'duplicate_wkt':
                        data.evlrs.append(data.evlrs[1])
                        data.write(source)
                    else:
                        source.write_bytes(source.read_bytes()[:-1])
                    output = pathlib.Path(tmp) / 'cloud.3tz'
                    output.write_bytes(b'original')
                    before = set(pathlib.Path(tmp).iterdir())
                    result = subprocess.run([BIN, '--json', 'point-cloud', '-i', str(source), '-o', str(output),
                        '--sourceCrs', 'header', '--heightOffset', '7', '--force'],
                        capture_output=True, text=True, env=dict(os.environ, PATH=''))
                    response = json.loads(result.stdout)
                    self.assertEqual(result.returncode, 3, result.stdout)
                    self.assertEqual(response['error']['code'], 'data')
                    if case == 'duplicate_wkt':
                        self.assertIn('multiple LAS WKT', response['error']['message'])
                    else:
                        self.assertIn('invalid LAS/LAZ', response['error']['message'])
                    self.assertEqual(output.read_bytes(), b'original')
                    self.assertEqual(set(pathlib.Path(tmp).iterdir()), before)

    def test_waveform_formats_are_explicitly_rejected(self):
        for fmt in (4, 5, 9, 10):
            with self.subTest(fmt=fmt), tempfile.TemporaryDirectory() as tmp:
                source = pathlib.Path(tmp) / 'cloud.las'
                header = laspy.LasHeader(point_format=fmt, version='1.4' if fmt >= 9 else '1.3')
                data = laspy.LasData(header);data.x=[0.];data.y=[0.];data.z=[0.];data.write(source)
                out = pathlib.Path(tmp) / 'tiles'
                with self.assertRaisesRegex(ValueError, 'waveform'):
                    run(types.SimpleNamespace(input=str(source), output=str(out), source_crs='local',
                        height_offset=None, max_points=16, chunk_points=11))
                self.assertFalse(out.exists())

    def test_nonfinite_extra_values_fail_before_publication(self):
        for value in (np.nan, np.inf, -np.inf):
            with self.subTest(value=value), tempfile.TemporaryDirectory() as tmp:
                source = pathlib.Path(tmp) / 'cloud.las'
                data = fixture(source, count=3);data.temperature=[1., 2., value];data.write(source)
                out = pathlib.Path(tmp) / 'tiles'
                with self.assertRaisesRegex(ValueError, 'nonfinite dimension'):
                    run(types.SimpleNamespace(input=str(source), output=str(out), source_crs='local',
                        height_offset=None, max_points=16, chunk_points=1))
                self.assertFalse(out.exists())

    def test_malformed_extra_descriptors_and_crs_fail_without_staging_leaks(self):
        for case in ('reserved', 'tail', 'malformed', 'scale', 'crs'):
            with self.subTest(case=case), tempfile.TemporaryDirectory() as tmp:
                source = pathlib.Path(tmp) / 'cloud.las'
                data = fixture(source, count=3)
                if case == 'reserved':
                    data.add_extra_dim(laspy.ExtraBytesParams(name='source_index', type='u8'))
                    data.write(source)
                elif case == 'tail':
                    # A valid file without the description of its extra data.
                    data.write(source)
                    content = bytearray(source.read_bytes())
                    content[227 + 2:227 + 18] = b'unknown'.ljust(16, b'\0')
                    source.write_bytes(content)
                else:
                    data.write(source)
                    content = bytearray(source.read_bytes())
                    if case == 'malformed':
                        struct.pack_into('<H', content, 227 + 20, 191)
                    elif case == 'scale':
                        content[227 + 54 + 3] |= 8
                        struct.pack_into('<d', content, 227 + 54 + 112, float('nan'))
                    else:
                        # Explicit invalid CRS exercises native diagnostics.
                        pass
                    source.write_bytes(content)
                out = pathlib.Path(tmp) / 'out.3tz'
                options = ['--sourceCrs', 'invalid CRS', '--heightOffset', '0'] if case == 'crs' else ['--sourceCrs', 'local']
                before = set(pathlib.Path(tmp).iterdir())
                result = subprocess.run([BIN, '--json', 'point-cloud', '-i', str(source), '-o', str(out), *options],
                    capture_output=True, text=True, env=dict(os.environ, PATH=''))
                response = json.loads(result.stdout)
                self.assertEqual(result.returncode, 3, result.stdout)
                self.assertEqual(response['error']['code'], 'data')
                self.assertEqual(set(pathlib.Path(tmp).iterdir()), before)

    def test_native_point_cloud_doctor_needs_no_python(self):
        result = subprocess.run([BIN, 'doctor', '--json', '--command', 'point-cloud'],
            capture_output=True, text=True, env=dict(os.environ, PATH=''))
        self.assertEqual(result.returncode, 0, result.stdout)
        report = json.loads(result.stdout)
        self.assertTrue(report['ready'])
        self.assertEqual(report['commands']['point-cloud']['requires'], [])
        self.assertTrue(report['commands']['point-cloud']['geospatial']['ready'])


if __name__ == '__main__':
    unittest.main()
