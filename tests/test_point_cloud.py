"""Decode generated content, independently check fidelity, bounds and LOD."""
import importlib.util
import json
import pathlib
import struct
import tempfile
import types
import unittest

import laspy
import numpy as np

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('point_cloud', ROOT / 'scripts/point_cloud.py')
cloud = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cloud)


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


def fixture(path, count=257, identical=False, crs=None):
    header = laspy.LasHeader(point_format=3, version='1.2')
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


class PointCloudTests(unittest.TestCase):
    def convert(self, tmp, suffix='.las', **kwargs):
        source = pathlib.Path(tmp) / ('cloud' + suffix)
        data = fixture(source, **kwargs)
        out = pathlib.Path(tmp) / 'tiles'
        args = types.SimpleNamespace(input=str(source), output=str(out), source_crs='local',
                                     height_offset=None, max_points=16, chunk_points=11)
        report = cloud.run(args)
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
                    for name in ('red', 'green', 'blue', 'intensity', 'classification', 'return_number',
                                 'number_of_returns', 'temperature', 'X', 'Y', 'Z'):
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
            cloud.run(args)
            root = json.loads((out / 'tileset.json').read_text())['root']
            # UTM central meridian 9°; inverse northing latitude independently tabulated.
            lon, lat, h = np.radians(9), np.radians(45.153477183356024), float(data.z[0]) + 10
            a, e2 = 6378137., 6.6943799901413165e-3
            n = a / np.sqrt(1 - e2 * np.sin(lat)**2)
            expected = [(n+h)*np.cos(lat)*np.cos(lon), (n+h)*np.cos(lat)*np.sin(lon), (n*(1-e2)+h)*np.sin(lat)]
            np.testing.assert_allclose(root['transform'][12:15], expected, atol=.001, rtol=0)

    def test_errors_clean_staging_and_do_not_replace_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = pathlib.Path(tmp) / 'cloud.las'
            fixture(source)
            out = pathlib.Path(tmp) / 'tiles'
            args = types.SimpleNamespace(input=str(source), output=str(out), source_crs='EPSG:32632',
                                         height_offset=None, max_points=16, chunk_points=11)
            with self.assertRaisesRegex(ValueError, 'explicit finite'):
                cloud.run(args)
            self.assertFalse(out.exists())
            out.mkdir()
            (out / 'keep').write_text('original')
            with self.assertRaisesRegex(ValueError, 'already exists'):
                cloud.run(args)
            self.assertEqual((out / 'keep').read_text(), 'original')

    def test_scaled_extra_dimension_is_not_truncated(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = pathlib.Path(tmp) / 'cloud.las'
            data = fixture(source, count=3)
            data.add_extra_dim(laspy.ExtraBytesParams(name='scaled', type='i2', scales=[.1], offsets=[3]))
            data.scaled = np.array([3.1, 4.2, 5.3])
            data.write(source)
            out = pathlib.Path(tmp) / 'tiles'
            cloud.run(types.SimpleNamespace(input=str(source), output=str(out), source_crs='local',
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
                cloud.run(args)
            self.assertFalse(out.exists())


if __name__ == '__main__':
    unittest.main()
