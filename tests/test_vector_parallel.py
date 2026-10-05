"""Parallel completion must not change manifests, payloads or reuse semantics."""
import json
import os
import pathlib
import sqlite3
import subprocess
import sys
import tempfile
import types
import unittest
from unittest import mock
import weakref
import zipfile
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]/'scripts'))
import vector_parallel

BIN=os.environ.get('RUSTY_TILES_BIN')

class ParentStreamingTests(unittest.TestCase):
    def test_budget_guards_stop_decoding_and_retaining_source_candidates(self):
        class SourceFeature(dict):pass
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp)
            for reason,property_size,expected_loads in [('vertices',0,3),('estimatedBytes',10000,1)]:
                with self.subTest(reason=reason):
                    spool=root/f'{reason}.sqlite';db=sqlite3.connect(spool)
                    db.executescript('CREATE TABLE features(id INTEGER PRIMARY KEY,path TEXT,data TEXT);'
                        'CREATE TABLE vertices(x REAL,y REAL,z REAL,shared INTEGER);')
                    for i in range(30):
                        coordinates=[[i*1000+j,0,0] for j in range(256)]
                        feature=dict(properties=dict(_source_id=str(i),label='x'*property_size),
                            geometry=dict(type='LineString',coordinates=coordinates))
                        db.execute('INSERT INTO features VALUES(?,?,?)',(i,'',json.dumps(feature)))
                        db.executemany('INSERT INTO vertices VALUES(?,?,?,0)',coordinates)
                    db.commit();db.close()
                    references=[];original_loads=json.loads
                    def load(data):
                        feature=SourceFeature(original_loads(data));references.append(weakref.ref(feature))
                        return feature
                    def simplify(feature,tolerance,locked,reports,**kwargs):
                        retained=[reference() for reference in references if reference() is not None]
                        self.assertEqual(len(retained),1,'only the current original feature may be retained')
                        self.assertLessEqual(sum(len(f['geometry']['coordinates']) for f in retained),256)
                        return dict(feature,geometry=dict(type='LineString',coordinates=feature['geometry']['coordinates'][:32])),0.
                    args=types.SimpleNamespace(max_features=1,max_parent_features=4096,max_vertices=64,max_bytes=4096)
                    with mock.patch.object(vector_parallel.json,'loads',side_effect=load):
                        result=vector_parallel.encode((str(spool),'',[0,0,0],1,args,{},str(root)),
                            types.SimpleNamespace(simplify_feature=simplify))
                    self.assertIsNone(result[0]);self.assertEqual(result[2],reason)
                    self.assertEqual(len(references),expected_loads,'stop loading as soon as a budget fails')
                    self.assertTrue(all(reference() is None for reference in references))

@unittest.skipUnless(BIN,'set RUSTY_TILES_BIN for parallel acceptance')
class ParallelVectorTests(unittest.TestCase):
    def test_workers_preserve_content_manifest_and_reuse(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=root/'lines.geojson'
            source.write_text(json.dumps(dict(type='FeatureCollection',features=[dict(type='Feature',id=i,
                properties=dict(name=f'line-{i}'),geometry=dict(type='LineString',coordinates=[
                    [i*100+j,float(j%3)*.01,0] for j in range(80)])) for i in range(12)])))
            outputs=[];reports=[]
            for jobs in (1,2):
                out=root/f'jobs-{jobs}.3tz';outputs.append(out)
                result=subprocess.run([BIN,'vector','-i',str(source),'-o',str(out),'--sourceCrs','local',
                    '--jobs',str(jobs),'--maxFeatures','2','--quantize','--meshopt'],capture_output=True,text=True)
                self.assertEqual(result.returncode,0,result.stderr)
                with zipfile.ZipFile(out) as archive:reports.append(json.loads(archive.read('conversion.json')))
            self.assertEqual(reports[0]['performance']['workersUsed'],1)
            self.assertEqual(reports[1]['performance']['workersUsed'],2)
            with zipfile.ZipFile(outputs[0]) as a,zipfile.ZipFile(outputs[1]) as b:
                self.assertEqual(a.namelist(),b.namelist())
                for name in a.namelist():
                    if name not in ('conversion.json','@3dtilesIndex1@'):
                        self.assertEqual(a.read(name),b.read(name),name)
            out=root/'reused.3tz'
            result=subprocess.run([BIN,'vector','-i',str(source),'-o',str(out),'--sourceCrs','local',
                '--jobs','2','--maxFeatures','2','--quantize','--meshopt','--reuseTileset',str(outputs[0])],capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr)
            with zipfile.ZipFile(out) as archive:
                report=json.loads(archive.read('conversion.json'))
                self.assertEqual(report['performance']['workersUsed'],0)
                self.assertEqual(report['reuse']['rebuiltContents'],0)
                self.assertGreater(report['reuse']['reusedTiles'],0)

    def test_zero_jobs_rejects_without_publishing(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=root/'source.geojson';source.write_text('{}');out=root/'out.3tz'
            result=subprocess.run([BIN,'vector','-i',str(source),'-o',str(out),'--jobs','0'],capture_output=True,text=True)
            self.assertNotEqual(result.returncode,0);self.assertFalse(out.exists())
