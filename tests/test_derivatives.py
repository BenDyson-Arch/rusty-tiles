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
    path = ROOT/'scripts'/f'{name}.py'
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


from vector_test_support import vector, to_source


class VectorTests(unittest.TestCase):
    def test_archive_content_has_typed_properties_and_polygon_topology(self):
        with tempfile.TemporaryDirectory() as tmp:
            vector.run(types.SimpleNamespace(input=str(ROOT/'tests/fixtures/vector.geojson'),output=tmp,max_features=2))
            manifest=json.loads((pathlib.Path(tmp)/'tileset.json').read_text())
            self.assertEqual(manifest['extensionsUsed'],['3DTILES_content_gltf_vector'])
            primitives=[]
            def leaves(node):
                if 'children' in node:
                    for child in node['children']:
                        yield from leaves(child)
                else:
                    yield pathlib.Path(tmp)/node['content']['uri']
            for file in leaves(manifest['root']):
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
            np.testing.assert_array_equal(to_source(path,positions),ring)

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



if __name__ == '__main__':
    unittest.main()
