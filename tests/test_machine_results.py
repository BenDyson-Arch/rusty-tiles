"""Machine protocol acceptance: one stdout JSON value and categorized failures."""
import json
import os
import pathlib
import subprocess
import tempfile
import unittest
import zipfile

BIN=os.environ.get('RUSTY_TILES_BIN')

@unittest.skipUnless(BIN,'set RUSTY_TILES_BIN for CLI acceptance')
class MachineResultTests(unittest.TestCase):
    def call(self,*args,env=None):
        result=subprocess.run([BIN,'--json',*args],capture_output=True,text=True,env=env)
        return result,json.loads(result.stdout)

    def source(self,root):
        source=root/'source.geojson'
        source.write_text(json.dumps(dict(type='FeatureCollection',features=[dict(type='Feature',id=1,
            properties=dict(name='example'),geometry=dict(type='Point',coordinates=[0,0,0]))])))
        return source

    def test_success_summary_progress_and_compression_protocol(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=self.source(root);out=root/'out.3tz'
            result,report=self.call('vector','-i',str(source),'-o',str(out),'--sourceCrs','local','--meshopt','--progress','json')
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertTrue(report['ok']);self.assertEqual(report['output'],str(out))
            self.assertEqual(report['counts']['features'],1);self.assertEqual(report['skippedFeatures'],0)
            self.assertEqual(report['conversionReport'],dict(archive=str(out),entry='conversion.json'))
            events=[json.loads(line) for line in result.stderr.splitlines() if line.startswith('{')]
            self.assertEqual(events[0],dict(event='progress',phase='conversion',done=0,total=1))
            self.assertEqual(events[-1]['done'],1);self.assertEqual(events[-1]['total'],1)
            self.assertIn('ingestion',[event['phase'] for event in events])
            before=out.read_bytes()
            result,report=self.call('vector','-i',str(source),'-o',str(out))
            self.assertEqual(result.returncode,5);self.assertEqual(report['error']['code'],'output_conflict')
            self.assertEqual(before,out.read_bytes())

    def test_usage_data_and_environment_have_distinct_codes(self):
        result,report=self.call('vector')
        self.assertEqual(result.returncode,2);self.assertEqual(report['error']['code'],'usage')
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=self.source(root);out=root/'out.3tz'
            args=['vector','-i',str(source),'-o',str(out),'--sourceCrs','local']
            source.write_text('invalid GeoJSON')
            result,report=self.call(*args)
            self.assertEqual(result.returncode,3,result.stderr);self.assertEqual(report['error']['code'],'data')
            self.assertFalse(out.exists())
            source=self.source(root)
            result,report=self.call(*args,env=dict(os.environ,PATH=''))
            self.assertEqual(result.returncode,4);self.assertEqual(report['error']['code'],'environment')
            (root/'numpy.py').write_text("raise ImportError('missing fixture module')")
            result,report=self.call(*args,env=dict(os.environ,PYTHONPATH=str(root)))
            self.assertEqual(result.returncode,4);self.assertIn('missing fixture module',report['error']['message'])
            self.assertFalse(out.exists())

    def test_doctor_failure_keeps_inventory_in_single_result(self):
        result,report=self.call('doctor','--command','vector',env=dict(os.environ,PATH=''))
        self.assertEqual(result.returncode,4);self.assertFalse(report['ok'])
        self.assertEqual(report['error']['code'],'environment');self.assertIn('commands',report)
