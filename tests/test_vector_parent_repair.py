"""Explicit parent display stand-ins have a conservative surface error bound."""
import json
import os
import pathlib
import subprocess
import tempfile
import types
import unittest
import zipfile
import numpy as np
from test_vector_lod import vector, distance
from test_vector_gpkg import nodes

BIN=os.environ.get('RUSTY_TILES_BIN')

def feature():
    corners=np.array([[0,0,0],[2,2,0],[2,0,0],[0,2,0],[0,0,0]],float)
    ring=np.concatenate([a+(b-a)*np.linspace(0,1,25,endpoint=False)[:,None]
        for a,b in zip(corners[:-1],corners[1:])]).tolist()
    return dict(type='Feature',properties=dict(_source_id='1',_source_layer='bow'),
        geometry=dict(type='Polygon',coordinates=[ring+[ring[0]]]))

class ParentRepairTests(unittest.TestCase):
    def test_parent_option_preserves_leaf_bytes_and_records_substitutions(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=feature();source['id']=1;source['properties']=dict(name='bow')
            p=root/'bow.geojson';p.write_text(json.dumps(dict(type='FeatureCollection',features=[source])))
            leaves=[]
            for enabled in (False,True):
                out=root/str(enabled)
                vector.run(types.SimpleNamespace(input=str(p),output=str(out),source_crs='local',max_features=64,
                    repair=True,parent_repair=enabled,lod_tolerance=1,lod_levels=3))
                manifest=json.loads((out/'tileset.json').read_text())
                leaves.append([ (out/n['content']['uri']).read_bytes() for n in nodes(manifest['root']) if not n.get('children')])
                if enabled:
                    report=json.loads((out/'conversion.json').read_text())
                    self.assertTrue(report['parentRepairEnabled'])
                    self.assertTrue(any(r.get('substitution')=='parentOutline' for r in report['lodFallbacks']))
                    self.assertTrue(manifest['root'].get('children'))
            self.assertEqual(leaves[0],leaves[1])

    @unittest.skipUnless(BIN,'set RUSTY_TILES_BIN for parentRepair CLI acceptance')
    def test_cli_option_is_wired(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=feature();source['id']=1;source['properties']=dict(name='bow');p=root/'bow.geojson';out=root/'out.3tz'
            p.write_text(json.dumps(dict(type='FeatureCollection',features=[source])))
            result=subprocess.run([BIN,'vector','-i',str(p),'-o',str(out),'--sourceCrs','local',
                '--repair','--parentRepair','--lodTolerance','1'],capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr)
            with zipfile.ZipFile(out) as archive:
                self.assertTrue(json.loads(archive.read('conversion.json'))['parentRepairEnabled'])
