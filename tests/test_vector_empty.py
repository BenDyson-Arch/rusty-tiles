"""Geometry-free records are visible in reports, not in tile content."""
import json
import pathlib
import tempfile
import unittest
from osgeo import ogr
import test_vector_fields as field_tests
from test_vector_gpkg import gpkg
from test_vector_reuse import archive, details
from test_vector_lod import vector

class EmptyGeometryTests(unittest.TestCase):
    args=field_tests.VectorFieldTests.args
    def test_null_and_empty_records_keep_ids_in_reports(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);p=root/'empty.gpkg'
            gpkg(p,[('roads',3857,[(1,dict(type='Point',coordinates=[1,0]),None)])])
            ds=ogr.Open(str(p),1);layer=ds.GetLayer(0)
            for fid,geom in [(2,None),(3,ogr.Geometry(ogr.wkbLineString))]:
                f=ogr.Feature(layer.GetLayerDefn());f.SetFID(fid)
                if geom is not None:f.SetGeometry(geom)
                layer.CreateFeature(f)
            ds=None;out=root/'out';vector.run(self.args(p,out))
            r=json.loads((out/'conversion.json').read_text())
            self.assertEqual(r['features'],1);self.assertEqual(r['featuresWithoutGeometry'],2)
            self.assertEqual(r['layers'][0]['featuresWithoutGeometry'],2);self.assertEqual(r['skippedFeatures'],0)
            rows=[json.loads(s) for s in (out/'geometry-reports.jsonl').read_text().splitlines()]
            self.assertEqual({r['sourceId'] for r in rows if r.get('outcome')=='no-geometry'},{'2','3'})
            self.assertEqual({k[1] for k in details(out)},{'1'})

    def test_last_drawable_geometry_becoming_null_removes_old_content(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);p=root/'only.geojson';before=root/'before';previous=root/'previous.3tz'
            feature=dict(type='Feature',id='placeholder',properties=dict(name='same'),geometry=dict(type='Point',coordinates=[1,0,0]))
            p.write_text(json.dumps(dict(type='FeatureCollection',features=[feature])))
            vector.run(self.args(p,before));archive(before,previous)
            feature['geometry']=None;p.write_text(json.dumps(dict(type='FeatureCollection',features=[feature])))
            for label,options in [('fresh',{}),('reuse',dict(reuse_tileset=str(previous)))]:
                out=root/label;vector.run(self.args(p,out,**options))
                self.assertEqual(details(out),{});self.assertEqual(list((out/'t').iterdir()),[])
                self.assertEqual(json.loads((out/'conversion.json').read_text())['featuresWithoutGeometry'],1)
