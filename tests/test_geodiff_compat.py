"""Opt-in end-to-end producer/apply compatibility; no changeset code in the tiler.
GEODIFF_CPP_BIN=/path/to/geodiff GO_GEODIFF_DRIVER=/path/to/driver \
  python -m unittest discover -s tests -p 'test_geodiff_compat.py'
The Go driver exposes diff base modified output, and apply target diff.
"""
import json
import os
import pathlib
import shutil
import subprocess
import tempfile
import unittest

from osgeo import ogr
import test_vector_reuse as reuse
archive=reuse.archive;vector=reuse.vector;report=reuse.report


@unittest.skipUnless(os.environ.get('GEODIFF_CPP_BIN') and os.environ.get('GO_GEODIFF_DRIVER'),
                     'set upstream C++ and go-geodiff drivers to run cross compatibility')
class GeodiffCompatibilityTests(unittest.TestCase):
    make_source=reuse.ReuseTests.make_source
    args=reuse.ReuseTests.args
    assert_same_details=reuse.ReuseTests.assert_same_details

    def run_tool(self,binary,*args):
        result=subprocess.run([binary,*map(str,args)],capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr.decode())

    def test_both_producers_and_cross_apply_match_fresh_tiles(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);base=self.make_source(tmp,32,spatial_index=False);modified=root/'modified.gpkg';shutil.copy2(base,modified)
            a=root/'baseline';previous=root/'baseline.3tz'
            vector.run(self.args(base,a));archive(a,previous)
            ds=ogr.Open(str(modified),1);layer=ds.GetLayer(0)
            f=layer.GetFeature(7);f.SetField('name','property edit');f.SetField('large',None);layer.SetFeature(f)
            f=layer.GetFeature(18);f.SetGeometry(ogr.CreateGeometryFromJson(json.dumps(dict(type='LineString',coordinates=[[4000,0,0],[4010,1,0],[4020,0,0]]))));layer.SetFeature(f)
            layer.DeleteFeature(1)
            f=ogr.Feature(layer.GetLayerDefn());f.SetFID(99);f.SetField('name','insert');f.SetField('large',2**60+3)
            f.SetGeometry(ogr.CreateGeometryFromJson(json.dumps(dict(type='Point',coordinates=[-100,0,0]))));layer.CreateFeature(f);ds=None
            cpp=os.environ['GEODIFF_CPP_BIN'];go=os.environ['GO_GEODIFF_DRIVER'];cpp_diff=root/'cpp.diff';go_diff=root/'go.diff'
            self.run_tool(cpp,'diff',base,modified,cpp_diff);self.run_tool(go,'diff',base,modified,go_diff)
            self.assertEqual(cpp_diff.read_bytes(),go_diff.read_bytes())
            fresh=root/'fresh';vector.run(self.args(modified,fresh))
            for label,binary,changeset in [('cpp-applies-go',cpp,go_diff),('go-applies-cpp',go,cpp_diff)]:
                applied=root/(label+'.gpkg');shutil.copy2(base,applied);self.run_tool(binary,'apply',applied,changeset)
                incremental=root/label;vector.run(self.args(applied,incremental,previous))
                self.assert_same_details(incremental,fresh)
                r=report(incremental)['reuse'];self.assertGreater(r['reusedContents'],0)
                self.assertLess(r['rebuiltContents'],r['publishedContents']//2)
                print(label,json.dumps(r,sort_keys=True))

    def test_indexed_geopackage_cross_apply_exposes_go_trigger_gap(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);base=self.make_source(tmp,4);modified=root/'modified.gpkg';shutil.copy2(base,modified)
            ds=ogr.Open(str(modified),1);layer=ds.GetLayer(0);f=layer.GetFeature(2)
            f.SetGeometry(ogr.CreateGeometryFromJson(json.dumps(dict(type='LineString',coordinates=[[1,0,0],[2,1,0]]))))
            layer.SetFeature(f);ds=None
            cpp=os.environ['GEODIFF_CPP_BIN'];go=os.environ['GO_GEODIFF_DRIVER'];changeset=root/'changes.diff'
            self.run_tool(go,'diff',base,modified,changeset)
            applied_cpp=root/'cpp.gpkg';applied_go=root/'go.gpkg';shutil.copy2(base,applied_cpp);shutil.copy2(base,applied_go)
            self.run_tool(cpp,'apply',applied_cpp,changeset)
            # Upstream succeeds with the actual GDAL R-tree triggers present.
            fresh=root/'fresh';converted=root/'cpp';vector.run(self.args(modified,fresh));vector.run(self.args(applied_cpp,converted))
            self.assert_same_details(converted,fresh)
            result=subprocess.run([go,'apply',str(applied_go),str(changeset)],capture_output=True)
            if result.returncode and b'no such function: ST_IsEmpty' in result.stderr:
                before=root/'before';after=root/'after';vector.run(self.args(base,before));vector.run(self.args(applied_go,after))
                self.assert_same_details(before,after)
                self.skipTest('go-geodiff bbdf585 lacks ST_IsEmpty for indexed GPKG geometry apply; upstream apply passed and failed Go apply remained atomic')
            self.assertEqual(result.returncode,0,result.stderr.decode())
            converted=root/'go';vector.run(self.args(applied_go,converted));self.assert_same_details(converted,fresh)
