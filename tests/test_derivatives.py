"""Independent binary checks for the GDAL-backed lab derivatives.
Run: python3 -m unittest discover -s tests -p 'test_*.py'
"""
import importlib.util
import json
import pathlib
import struct
import tempfile
import types
import unittest

import numpy as np

ROOT = pathlib.Path(__file__).resolve().parents[1]


def module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT/'scripts'/f'{name}.py')
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


terrain = module('terrain')
vector = module('vector')


def decode_terrain(data):
    header = struct.unpack_from('<3d2f7d', data)
    n, = struct.unpack_from('<I', data, 88)
    cursor = 92
    attributes = []
    for _ in range(3):
        values = np.frombuffer(data, '<u2', n, cursor).astype(np.int64)
        attributes.append(np.cumsum((values >> 1) ^ -(values & 1)))
        cursor += n*2
    count, = struct.unpack_from('<I', data, cursor)
    cursor += 4
    codes = np.frombuffer(data, '<u2', count*3, cursor)
    cursor += count*6
    highest, indices = 0, []
    for code in codes:
        indices.append(highest-int(code))
        if code == 0:
            highest += 1
    edges = []
    for _ in range(4):
        count, = struct.unpack_from('<I', data, cursor)
        cursor += 4
        edges.append(np.frombuffer(data, '<u2', count, cursor))
        cursor += count*2
    return header, np.array(attributes).T, np.asarray(indices).reshape(-1, 3), edges


class TerrainTests(unittest.TestCase):
    def test_root_horizon_point_is_not_earth_center(self):
        data=terrain.encode(0,-90,180,np.zeros((17,17)),0,100)
        header,*_=decode_terrain(data)
        self.assertGreater(np.linalg.norm(header[9:12]),1e12)

    def test_edges_winding_quantization_and_bounds(self):
        a = np.add.outer(np.arange(17), np.arange(17)).astype(float)
        b = a+16
        left = decode_terrain(terrain.encode(133, -13, .01, a, 0, 100))
        right = decode_terrain(terrain.encode(133.01, -13, .01, b, 0, 100))
        header, attrs, tris, edges = left
        self.assertTrue(np.all((tris >= 0) & (tris < len(attrs))))
        xy = attrs[tris, :2]
        cross = (xy[:, 1, 0]-xy[:, 0, 0])*(xy[:, 2, 1]-xy[:, 0, 1])-(xy[:, 1, 1]-xy[:, 0, 1])*(xy[:, 2, 0]-xy[:, 0, 0])
        self.assertTrue(np.all(cross > 0))
        np.testing.assert_array_equal(attrs[edges[2], 2], right[1][right[3][0], 2])
        heights = attrs[:, 2]/32767*100
        expected = (attrs[:, 0]+attrs[:, 1])/32767*16
        self.assertLess(np.max(np.abs(heights-expected)), .002)
        xyz = terrain.ecef(133+attrs[:, 0]/32767*.01, -13+attrs[:, 1]/32767*.01, heights)
        self.assertLessEqual(np.linalg.norm(xyz-np.asarray(header[5:8]), axis=1).max(), header[8]+1e-6)


class VectorTests(unittest.TestCase):
    def test_hole_and_vertical_polygon_triangulation(self):
        outer = [[0,0,0],[10,0,0],[10,10,0],[0,10,0],[0,0,0]]
        hole = [[3,3,0],[3,7,0],[7,7,0],[7,3,0],[3,3,0]]
        for vertical in (False, True):
            rings = [outer,hole]
            if vertical:
                rings = [[[p[0],p[2],p[1]] for p in r] for r in rings]
            pos, indices, loops, _, _ = vector.polygon(rings)
            p = np.asarray(pos)[np.asarray(indices).reshape(-1,3)]
            area = np.linalg.norm(np.cross(p[:,1]-p[:,0],p[:,2]-p[:,0]),axis=1).sum()/2
            self.assertAlmostEqual(area,84,places=8)
            self.assertEqual(loops.count(0xffffffff),1)
            self.assertEqual(set(indices),set(range(8)))

    def test_archive_content_has_typed_properties_and_polygon_topology(self):
        with tempfile.TemporaryDirectory() as tmp:
            vector.run(types.SimpleNamespace(input=str(ROOT/'tests/fixtures/vector.geojson'),output=tmp,max_features=2))
            manifest=json.loads((pathlib.Path(tmp)/'tileset.json').read_text())
            self.assertEqual(manifest['extensionsUsed'],['3DTILES_content_gltf_vector'])
            primitives=[]
            for file in pathlib.Path(tmp).glob('t/*.glb'):
                data=file.read_bytes()
                self.assertEqual(struct.unpack_from('<I',data,8)[0],len(data))
                size=struct.unpack_from('<I',data,12)[0]
                doc=json.loads(data[20:20+size])
                schema=doc['extensions']['EXT_structural_metadata']['schema']['classes']['feature']['properties']
                self.assertEqual(schema['height'],dict(type='SCALAR',componentType='FLOAT64'))
                self.assertEqual(schema['visible'],dict(type='BOOLEAN'))
                primitives.extend(doc['meshes'][0]['primitives'])
            self.assertEqual(len(primitives),4)
            self.assertEqual(sum('EXT_mesh_polygon' in p['extensions'] for p in primitives),2)
            self.assertTrue(all('_FEATURE_ID_0' in p['attributes'] for p in primitives))

    def test_partition_uses_metres_instead_of_mixing_degrees_and_height(self):
        with tempfile.TemporaryDirectory() as tmp:
            source=pathlib.Path(tmp)/'input.geojson'
            features=[dict(type='Feature',properties={},geometry=dict(type='Point',coordinates=[lon,44,h])) for lon in (0,.001) for h in (0,10)]
            source.write_text(json.dumps(dict(type='FeatureCollection',features=features)))
            out=pathlib.Path(tmp)/'output'
            vector.run(types.SimpleNamespace(input=str(source),output=str(out),max_features=2))
            tiles=json.loads((out/'tileset.json').read_text())
            self.assertTrue(all(c['boundingVolume']['box'][3]<1 for c in tiles['root']['children']))

    def test_ambiguous_polygon_outline_preserves_original_coordinates(self):
        ring=[[0,0,0],[2,2,0],[0,2,1],[2,0,1],[0,0,0]]
        with tempfile.TemporaryDirectory() as tmp:
            path=pathlib.Path(tmp)/'tile.glb'
            reports=[]
            vector.emit([dict(properties=dict(name='cross',_source_id='0'),geometry=dict(type='Polygon',coordinates=[ring]))],path,
                        lambda p:np.asarray(p),repair=True,reports=reports,ambiguous_outlines=True)
            data=path.read_bytes();n=struct.unpack_from('<I',data,12)[0];doc=json.loads(data[20:20+n]);binary=data[28+n:]
            primitive=doc['meshes'][0]['primitives'][0]
            self.assertEqual(primitive['mode'],3)
            self.assertEqual(reports[0]['outputGeometry'],'outline')
            accessor=doc['accessors'][primitive['attributes']['POSITION']]
            view=doc['bufferViews'][accessor['bufferView']]
            positions=np.frombuffer(binary,'<f4',accessor['count']*3,view.get('byteOffset',0)).reshape(-1,3)
            np.testing.assert_array_equal(positions,ring)

    def test_missing_string_properties_use_explicit_nodata(self):
        with tempfile.TemporaryDirectory() as tmp:
            path=pathlib.Path(tmp)/'tile.glb'
            features=[dict(properties=dict(_source_id='0',optional='present'),geometry=dict(type='Point',coordinates=[0,0,0])),
                      dict(properties=dict(_source_id='1'),geometry=dict(type='Point',coordinates=[1,0,0]))]
            vector.emit(features,path,lambda p:np.asarray(p))
            data=path.read_bytes();n=struct.unpack_from('<I',data,12)[0];doc=json.loads(data[20:20+n])
            prop=doc['extensions']['EXT_structural_metadata']['schema']['classes']['feature']['properties']['optional']
            self.assertEqual(prop['type'],'STRING')
            self.assertIn('noData',prop)

    def test_invalid_coordinates_rejected_and_nonplanar_vertices_retained(self):
        with self.assertRaises(ValueError):
            vector.coordinates(dict(coordinates=[[200,0]]))
        source=[[0,0,0],[1,0,0],[1,1,1],[0,1,0]]
        positions,*_=vector.polygon([source+[source[0]]])
        self.assertEqual({tuple(p) for p in positions},{tuple(p) for p in source})

    def test_explicit_repair_preserves_bowtie_as_two_polygons(self):
        ring=[[0,0,0],[2,2,0],[0,2,0],[2,0,0],[0,0,0]]
        with self.assertRaises(ValueError):
            vector.polygon([ring])
        report={}
        p,i,loops,offsets,loop_offsets=vector.polygon([ring],True,report)
        self.assertTrue(report['topologyRepaired'])
        self.assertEqual(len(offsets),2)
        self.assertEqual(len(loop_offsets),2)
        triangles=np.asarray(p)[np.asarray(i).reshape(-1,3)]
        self.assertAlmostEqual(np.linalg.norm(np.cross(triangles[:,1]-triangles[:,0],triangles[:,2]-triangles[:,0]),axis=1).sum()/2,2)


if __name__ == '__main__':
    unittest.main()
