"""Attribute selection precedes validation and participates in reuse compatibility."""
import json
import pathlib
import subprocess
import tempfile
import unittest
import test_vector_fields as field_tests
from test_vector_reuse import archive,details
from test_vector_lod import vector
from test_vector_gpkg import gpkg

from cli_bin import BIN, requires_bin

class FilterTests(unittest.TestCase):
    args=field_tests.VectorFieldTests.args
    def source(self,root):
        p=root/'filter.geojson'
        p.write_text(json.dumps(dict(type='FeatureCollection',features=[
            dict(type='Feature',id=1,properties=dict(name='public'),geometry=dict(type='Point',coordinates=[0,0,0])),
            dict(type='Feature',id=2,properties=dict(name='private'),geometry=dict(type='Point',coordinates=[10,0,0])),
            dict(type='Feature',id=3,properties=dict(name='bad'),geometry=dict(type='Polygon',coordinates=[[[0,0,0],[2,2,0],[2,0,0],[0,2,0],[0,0,0]]]))])))
        return p

    def test_filter_excludes_bad_features_and_changed_filter_rebuilds(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);p=self.source(root);out=root/'public';previous=root/'prior.3tz'
            vector.run(self.args(p,out,where="name = 'public'"));archive(out,previous)
            self.assertEqual({k[1] for k in details(out)},{'1'})
            same=root/'same';vector.run(self.args(p,same,where="name = 'public'",reuse_tileset=str(previous)))
            self.assertGreater(json.loads((same/'conversion.json').read_text())['reuse']['reusedContents'],0)
            changed=root/'changed';vector.run(self.args(p,changed,where="name = 'private'",reuse_tileset=str(previous)))
            self.assertEqual({k[1] for k in details(changed)},{'2'})
            r=json.loads((changed/'conversion.json').read_text());self.assertEqual(r['attributeFilter'],"name = 'private'")
            self.assertEqual(r['reuse']['reusedContents'],0);self.assertEqual(r['reuse']['incompatibleReason'],'attribute filter changed')
            empty=root/'empty';vector.run(self.args(p,empty,where="name = 'absent'",reuse_tileset=str(previous)))
            self.assertEqual(details(empty),{})
            with self.assertRaisesRegex(ValueError,'attribute filter'):
                vector.run(self.args(p,root/'invalid',where='does_not_exist = 1'))

    def test_filter_is_applied_to_each_selected_geopackage_layer(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);p=root/'layers.gpkg';g=dict(type='Point',coordinates=[0,0])
            gpkg(p,[('a',3857,[(1,g,1),(2,g,2)]),('b',3857,[(3,g,1),(4,g,2)])])
            out=root/'out';vector.run(self.args(p,out,all_layers=True,where='large = 1',drop_fields=['large']))
            self.assertEqual({(k[0],k[1]) for k in details(out)},{('a','1'),('b','3')})

    @requires_bin('set RUSTY_TILES_BIN for filter argv')
    def test_cli_accepts_expression_as_one_argument(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);p=self.source(root);out=root/'out.3tz'
            r=subprocess.run([BIN,'vector','-i',str(p),'-o',str(out),'--sourceCrs','local','--where',"name = 'public'"],capture_output=True)
            self.assertEqual(r.returncode,0,r.stderr.decode());self.assertTrue(out.exists())
