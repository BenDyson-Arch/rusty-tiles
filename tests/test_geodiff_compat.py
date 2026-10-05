"""Opt-in end-to-end producer/apply compatibility; no changeset code in the tiler.
GEODIFF_CPP_BIN=/path/to/geodiff GO_GEODIFF_DRIVER=/path/to/driver \
  python -m unittest discover -s tests -p 'test_geodiff_compat.py'
The Go driver exposes diff base modified output, and apply target diff.
"""
from contextlib import closing
import json
import os
import pathlib
import shutil
import sqlite3
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
        self.check_cross_apply(spatial_index=False)

    def test_indexed_geopackage_cross_apply_maintains_index_and_reuses_tiles(self):
        self.check_cross_apply(spatial_index=True)

    def check_cross_apply(self,spatial_index):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);base=self.make_source(tmp,32,spatial_index=spatial_index);modified=root/'modified.gpkg';shutil.copy2(base,modified)
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
                if spatial_index:
                    with closing(sqlite3.connect(modified)) as expected, closing(sqlite3.connect(applied)) as actual:
                        query='SELECT id,minx,maxx,miny,maxy FROM rtree_roads_geom ORDER BY id'
                        self.assertEqual(actual.execute(query).fetchall(),expected.execute(query).fetchall())
                    # Use OGR's spatial filter as well as checking the index rows.
                    expected=ogr.Open(str(modified));actual=ogr.Open(str(applied))
                    for bounds in [(-101,-1,-99,1),(3999,-1,4021,2),(1699,-1,1761,1),(-1,-1,61,1)]:
                        hits=[]
                        for ds in (expected,actual):
                            layer=ds.GetLayer(0);layer.SetSpatialFilterRect(*bounds)
                            hits.append(sorted(f.GetFID() for f in layer))
                        self.assertEqual(hits[0],hits[1])
                    expected=actual=None
                incremental=root/label;vector.run(self.args(applied,incremental,previous))
                self.assert_same_details(incremental,fresh)
                r=report(incremental)['reuse'];self.assertGreater(r['reusedContents'],0)
                self.assertLess(r['rebuiltContents'],r['publishedContents']//2)
                print(label,json.dumps(r,sort_keys=True))
