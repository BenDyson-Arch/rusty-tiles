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


BIN=os.environ.get('RUSTY_TILES_BIN')

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
