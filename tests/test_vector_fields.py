"""Metadata selection and explicit JSON list representation."""
import json
import pathlib
import subprocess
import tempfile
import types
import unittest

from test_vector_lod import vector
from test_vector_reuse import details

from cli_bin import BIN, requires_bin

class VectorFieldTests(unittest.TestCase):
    def source(self,root):
        p=root/'fields.geojson'
        p.write_text(json.dumps(dict(type='FeatureCollection',features=[
            dict(type='Feature',id=i,properties=dict(name='item',tags=tags,numbers=numbers),
                 geometry=dict(type='Point',coordinates=[i,0,0]))
            for i,tags,numbers in [(1,['a','b'],[2**60+3,2]),(2,[],[]),(3,None,None)]])))
        return p

    def args(self,p,out,**options):
        return types.SimpleNamespace(input=str(p),output=str(out),source_crs='local',max_features=64,**options)

    def test_json_lists_preserve_order_empty_missing_and_large_integers(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);p=self.source(root);out=root/'out'
            vector.run(self.args(p,out,list_fields='json'))
            props={key[1]:values[0][0] for key,values in details(out).items()}
            self.assertEqual(json.loads(props['1']['numbers']),[2**60+3,2])
            self.assertEqual(json.loads(props['1']['tags']),['a','b'])
            self.assertEqual(props['2']['tags'],'[]');self.assertIsNone(props['3']['tags'])
            r=json.loads((out/'conversion.json').read_text())
            self.assertEqual(r['metadata']['listFields'],'json')
            self.assertEqual(r['layers'][0]['jsonFields'],['numbers','tags'])

    def test_selection_precedes_unsupported_schema_validation(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);p=self.source(root)
            with self.assertRaisesRegex(ValueError,'unsupported field type'):
                vector.run(self.args(p,root/'strict'))
            for label,opts in [('include',dict(fields=['name'])),('exclude',dict(drop_fields=['tags','numbers']))]:
                out=root/label;vector.run(self.args(p,out,**opts))
                for values in details(out).values():
                    self.assertEqual(set(values[0][0]),{'name','_source_id','_source_layer'})
            with self.assertRaisesRegex(ValueError,'unknown selected'):
                vector.run(self.args(p,root/'typo',fields=['missing']))

    @requires_bin('set RUSTY_TILES_BIN to exercise field argv')
    def test_cli_accepts_field_lists_and_rejects_conflicting_modes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);p=self.source(root)
            args=[BIN,'vector','-i',str(p),'-o',str(root/'out.3tz'),'--sourceCrs','local']
            result=subprocess.run(args+['--fields','name'],capture_output=True)
            self.assertEqual(result.returncode,0,result.stderr.decode())
            result=subprocess.run(args+['--fields','name','--dropFields','tags'],capture_output=True)
            self.assertNotEqual(result.returncode,0)
