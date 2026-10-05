"""Collapsed repairs keep source 3D outlines only when explicitly requested."""
import json
import pathlib
import tempfile
import unittest
from test_vector_fields import vector
import test_vector_fields as field_tests
from test_vector_reuse import details

class CollapsedRepairTests(unittest.TestCase):
    args=field_tests.VectorFieldTests.args
    def test_collapsed_repair_retains_closed_outline_and_metadata(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);p=root/'collapse.geojson'
            ring=[[10,0,0],[11,0,0],[11.5,0,0],[11,0,0],[10,0,0]]
            p.write_text(json.dumps(dict(type='FeatureCollection',features=[
                dict(type='Feature',id=1,properties=dict(name='ok'),geometry=dict(type='Polygon',coordinates=[[[0,0,0],[2,0,0],[2,2,0],[0,2,0],[0,0,0]]])),
                dict(type='Feature',id=2,properties=dict(name='sliver'),geometry=dict(type='Polygon',coordinates=[ring]))])))
            with self.assertRaisesRegex(ValueError,'collapsed'):
                vector.run(self.args(p,root/'strict',repair=True))
            out=root/'fallback';vector.run(self.args(p,out,repair=True,ambiguous_outlines=True))
            decoded=details(out);self.assertIn(('collapse','1',4),decoded)
            prop,xyz=decoded[('collapse','2',3)][0]
            self.assertEqual(prop['name'],'sliver');self.assertNotIn(('collapse','2',4),decoded)
            import numpy as np
            np.testing.assert_allclose(xyz,ring,atol=1e-6)
            rows=[json.loads(s) for s in (out/'geometry-reports.jsonl').read_text().splitlines()]
            self.assertTrue(any(r.get('outputGeometry')=='outline' and r['sourceId']=='2' and 'collapsed' in r['reason'] for r in rows))

    def test_large_collapsed_outline_uses_line_budgets(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);p=root/'long.geojson'
            ring=[[i,0,0] for i in range(100)]+[[i,0,0] for i in range(98,-1,-1)]
            p.write_text(json.dumps(dict(type='FeatureCollection',features=[dict(type='Feature',id=7,properties={},geometry=dict(type='Polygon',coordinates=[ring]))])))
            out=root/'out';vector.run(self.args(p,out,repair=True,ambiguous_outlines=True,max_vertices=32,max_bytes=8192))
            decoded=details(out);lines=decoded[('long','7',3)]
            self.assertEqual(sum(len(x)-1 for _,x in lines),len(ring)-1)
            self.assertTrue(all(len(x)<=32 for _,x in lines))
