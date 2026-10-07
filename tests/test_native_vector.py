"""Native migration boundaries and OGR reader coverage beyond GeoPackage."""
import json
import os
import pathlib
import struct
import tempfile
import types
import unittest

import numpy as np
from osgeo import ogr, osr
from pyproj import Transformer
from vector_test_support import native_run, python_run
from test_vector_reuse import archive, details


@unittest.skipUnless(os.environ.get('RUSTY_TILES_BIN'), 'select the native CLI')
class NativeVectorTests(unittest.TestCase):
    def test_mixed_numeric_columns_reject_integers_outside_exact_float64_bounds(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            source = root/'mixed.geojson'
            for number in (2**53+1, -(2**53+1), 2**63-1, -(2**63), 2**64-1):
                with self.subTest(number=number):
                    case = root/str(number)
                    case.mkdir()
                    source.write_text(json.dumps(dict(type='FeatureCollection', features=[
                        dict(type='Feature', id=i, properties=dict(value=value),
                             geometry=dict(type='Point', coordinates=[i, 0, 0]))
                        for i, value in enumerate((number, 1.5))])))
                    for name, run in (('native', native_run), ('python', python_run)):
                        args = types.SimpleNamespace(input=str(source), output=str(case/name),
                            source_crs='local', max_features=64)
                        with self.assertRaisesRegex(ValueError, 'large integers as float64 without loss'):
                            run(args)
                    self.assertFalse((case/'native').exists())

    def test_mixed_numeric_columns_keep_exact_float64_integer_boundaries(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp); source = root/'mixed.geojson'; output = root/'native'
            numbers = (2**53, -(2**53), 2**53-1, -(2**53-1), 1.5)
            source.write_text(json.dumps(dict(type='FeatureCollection', features=[
                dict(type='Feature', id=i, properties=dict(value=value),
                     geometry=dict(type='Point', coordinates=[i, 0, 0]))
                for i, value in enumerate(numbers)])))
            native_run(types.SimpleNamespace(input=str(source), output=str(output), source_crs='local'))
            decoded = {key[1]:fragments[0][0]['value'] for key,fragments in details(output).items()}
            self.assertEqual(decoded, {str(i):float(v) for i,v in enumerate(numbers)})
            data = next((output/'t').glob('*.glb')).read_bytes()
            length = struct.unpack_from('<I', data, 12)[0]; doc = json.loads(data[20:20+length])
            schema = doc['extensions']['EXT_structural_metadata']['schema']['classes']['feature']['properties']
            self.assertEqual(schema['value']['componentType'], 'FLOAT64')

    def test_python_archives_require_a_fresh_native_baseline(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            source = root / 'source.geojson'
            source.write_text(json.dumps(dict(type='FeatureCollection', features=[
                dict(type='Feature', id='original', properties=dict(name='original'),
                     geometry=dict(type='Point', coordinates=[0, 0, 0]))])))
            args = types.SimpleNamespace(input=str(source), output=str(root/'python'),
                source_crs='local', max_features=64)
            python_run(args)
            previous = root/'python.3tz'
            archive(root/'python', previous)
            original = previous.read_bytes()
            args.output = str(root/'native')
            args.reuse_tileset = str(previous)
            with self.assertRaisesRegex(ValueError, 'previous encoder differs.*fresh native'):
                native_run(args)
            self.assertFalse((root/'native').exists())
            self.assertEqual(previous.read_bytes(), original)
            args.reuse_tileset = None
            native_run(args)
            self.assertEqual(next(iter(details(root/'native').values()))[0][0]['name'], 'original')

    def test_shapefile_projection_filter_and_typed_fields(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            source = root/'survey.shp'
            ds = ogr.GetDriverByName('ESRI Shapefile').CreateDataSource(str(source))
            crs = osr.SpatialReference(); crs.ImportFromEPSG(32632)
            layer = ds.CreateLayer('survey', crs, ogr.wkbPoint)
            layer.CreateField(ogr.FieldDefn('number', ogr.OFTInteger64))
            layer.CreateField(ogr.FieldDefn('label', ogr.OFTString))
            for i in range(2):
                f = ogr.Feature(layer.GetLayerDefn())
                f.SetField('number', 2**53+3); f.SetField('label', 'keep' if i == 0 else 'omit')
                g = ogr.Geometry(ogr.wkbPoint); g.AddPoint_2D(500000+i*100, 4600000)
                f.SetGeometry(g); layer.CreateFeature(f)
            ds = None
            out = root/'native'
            native_run(types.SimpleNamespace(input=str(source), output=str(out), where="label = 'keep'"))
            result = details(out); self.assertEqual(len(result), 1)
            properties, xyz = next(iter(result.values()))[0]
            self.assertEqual(properties['number'], 2**53+3)
            self.assertEqual(properties['label'], 'keep')
            reference = Transformer.from_crs(32632, 4978, always_xy=True).transform(500000, 4600000, 0)
            np.testing.assert_allclose(xyz[0], reference, atol=.001, rtol=0)

    def test_boolean_lists_and_empty_lists_are_explicit_json_strings(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            source = root/'booleans.geojson'
            source.write_text(json.dumps(dict(type='FeatureCollection', features=[
                dict(type='Feature', id=i, properties=dict(flags=value),
                     geometry=dict(type='Point', coordinates=[i, 0, 0]))
                for i, value in enumerate(([True, False], [], None))])))
            out = root/'native'
            native_run(types.SimpleNamespace(input=str(source), output=str(out), source_crs='local', list_fields='json'))
            props = {key[1]:values[0][0]['flags'] for key,values in details(out).items()}
            self.assertEqual(props, {'0':'[true,false]', '1':'[]', '2':None})

    def test_native_quantized_leaf_decodes_within_its_metre_error_bound(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=root/'line.geojson';out=root/'native'
            x=np.linspace(1000000,1001000,3000)
            original=np.column_stack((x,np.sin(x*.01),np.cos(x*.01)))
            source.write_text(json.dumps(dict(type='FeatureCollection',features=[dict(type='Feature',
                id=1,properties=dict(large=2**60+3),geometry=dict(type='LineString',coordinates=original.tolist()))])))
            native_run(types.SimpleNamespace(input=str(source),output=str(out),source_crs='local',quantize=True))
            node=json.loads((out/'tileset.json').read_text())['root'];world=np.eye(4)
            while True:
                world=world@np.array(node.get('transform',np.eye(4).T.flatten())).reshape(4,4).T
                if not node.get('children'):break
                node=node['children'][0]
            data=(out/node['content']['uri']).read_bytes();n=struct.unpack_from('<I',data,12)[0]
            doc=json.loads(data[20:20+n]);binary=data[28+n:]
            ac=doc['accessors'][doc['meshes'][0]['primitives'][0]['attributes']['POSITION']]
            view=doc['bufferViews'][ac['bufferView']]
            self.assertEqual(ac['componentType'],5123);self.assertTrue(ac['normalized'])
            packed=np.ndarray((ac['count'],3),'<u2',buffer=binary,offset=view['byteOffset'],strides=(8,2))
            p=packed.astype(float)/65535*np.array(doc['nodes'][0]['scale'])+doc['nodes'][0]['translation']
            p=p[:,[0,2,1]]*[1,-1,1];decoded=(np.c_[p,np.ones(len(p))]@world.T)[:,:3]
            self.assertEqual(decoded.shape,original.shape)
            error=float(np.linalg.norm(decoded-original,axis=1).max())
            self.assertLessEqual(error,node['geometricError']+1e-9)
            self.assertLessEqual(error,node['extras']['quantizationErrorMetres']+node['extras']['positionRoundingMetres']+1e-9)
            column=doc['extensions']['EXT_structural_metadata']['propertyTables'][0]['properties']['large']
            view=doc['bufferViews'][column['values']]
            self.assertEqual(struct.unpack_from('<q',binary,view['byteOffset'])[0],2**60+3)
