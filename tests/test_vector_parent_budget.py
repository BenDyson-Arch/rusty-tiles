"""Coarse content is not limited by the leaf feature partition budget."""
import json
import math
import pathlib
import subprocess
import tempfile
import types
import unittest
import zipfile
from test_vector_lod import vector, read

from cli_bin import BIN, requires_bin

class ParentFeatureBudgetTests(unittest.TestCase):
    def source(self,root):
        features=[]
        for i in range(16):
            ring=[[i*5+math.cos(a),math.sin(a),0] for a in [2*math.pi*j/40 for j in range(40)]]
            features.append(dict(type='Feature',id=i,properties=dict(name=str(i)),
                geometry=dict(type='Polygon',coordinates=[ring+[ring[0]]])))
        source=root/'dense.geojson';source.write_text(json.dumps(dict(type='FeatureCollection',features=features)))
        return source

    def test_all_features_render_at_root_while_leaves_keep_their_budget(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=self.source(root);out=root/'out'
            vector.run(types.SimpleNamespace(input=str(source),output=str(out),source_crs='local',max_features=2))
            node=json.loads((out/'tileset.json').read_text())['root']
            self.assertIn('content',node)
            _,_,ids=read(out/node['content']['uri'])
            self.assertEqual(set(ids),{str(i) for i in range(16)})
            self.assertEqual(node['extras']['featureFragments'],16)
            def leaves(n):
                if not n.get('children'):
                    self.assertLessEqual(n['extras']['featureFragments'],2)
                    self.assertEqual(n['geometricError'],0)
                for c in n.get('children',[]):leaves(c)
            leaves(node)

    def test_separate_parent_feature_and_vertex_limits_report_binding_reason(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=self.source(root)
            for name,options,reason in [('features',dict(max_parent_features=2),'parentFeatures'),
                    ('vertices',dict(max_vertices=8),'vertices')]:
                out=root/name
                vector.run(types.SimpleNamespace(input=str(source),output=str(out),source_crs='local',max_features=2,**options))
                node=json.loads((out/'tileset.json').read_text())['root']
                self.assertNotIn('content',node)
                self.assertEqual(node['extras']['routingReason'],reason)

    @requires_bin('set RUSTY_TILES_BIN for CLI acceptance')
    def test_cli_parent_budget_option_is_wired_and_zero_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=self.source(root);out=root/'result.3tz'
            command=[BIN,'vector','--explicit','-i',str(source),'-o',str(out),'--sourceCrs','local',
                '--maxFeatures','2','--maxParentFeatures','32']
            result=subprocess.run(command,capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr)
            with zipfile.ZipFile(out) as archive:
                report=json.loads(archive.read('conversion.json'))
                self.assertEqual(report['budgets']['parentFeatures'],32)
                self.assertIn('content',json.loads(archive.read('tileset.json'))['root'])
            result=subprocess.run(command[:-1]+['0'],capture_output=True,text=True)
            self.assertNotEqual(result.returncode,0)
