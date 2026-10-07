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
            # Settings are separated from counts.
            self.assertEqual(report['settings']['lodLevels'],3);self.assertNotIn('lodLevels',report['counts'])
            self.assertNotIn('lodToleranceMetres',report['counts'])
            self.assertEqual(report['conversionReport'],dict(archive=str(out),entry='conversion.json'))
            events=[json.loads(line) for line in result.stderr.splitlines() if line.startswith('{')]
            self.assertEqual(events[0],dict(event='progress',phase='conversion',done=0,total=1))
            self.assertEqual(events[-1]['done'],1);self.assertEqual(events[-1]['total'],1)
            self.assertIn('ingestion',[event['phase'] for event in events])
            before=out.read_bytes()
            result,report=self.call('vector','-i',str(source),'-o',str(out))
            self.assertEqual(result.returncode,5);self.assertEqual(report['error']['code'],'output_conflict')
            self.assertEqual(before,out.read_bytes())

    def test_human_summary_and_kebab_aliases(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=self.source(root);out=root/'out.3tz'
            result=subprocess.run([BIN,'vector','-i',str(source),'-o',str(out),'--source-crs','local','--max-features','8'],
                capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr);self.assertEqual(result.stdout,'')
            lines=result.stderr.splitlines()
            self.assertEqual(lines[0],f'vector: wrote {out} (1 feature, 1 tile)')
            self.assertEqual(lines[-1],f'next: rusty-tiles validate {out}')
            with zipfile.ZipFile(out) as archive:
                report=json.loads(archive.read('conversion.json'))
            self.assertEqual(report['budgets']['features'],8)

    def test_option_errors_name_the_flag(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=self.source(root);out=root/'out.3tz'
            base=['vector','-i',str(source),'-o',str(out),'--sourceCrs','local']
            for extra,message in [(['--maxBytes','10'],'--maxBytes must be at least 4096'),
                    (['--lodLevels','17'],'--lodLevels must be between 1 and 16'),
                    (['--lodTolerance','0'],'--lodTolerance must be a positive'),
                    (['--maxVertices','3'],'--maxVertices must be at least 4'),
                    (['--jobs','0'],'--jobs must be at least 1'),
                    (['--where',' '],'--where must not be empty')]:
                with self.subTest(extra=extra):
                    result,report=self.call(*base,*extra)
                    self.assertEqual(result.returncode,3);self.assertEqual(report['error']['code'],'data')
                    self.assertIn(message,report['error']['message']);self.assertFalse(out.exists())

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
            missing_database = root/'missing-proj-data';missing_database.mkdir()
            args=['vector','-i',str(source),'-o',str(out),'--sourceCrs','EPSG:4326']
            result,report=self.call(*args,env=dict(os.environ,PROJ_DATA=str(missing_database),PROJ_LIB=str(missing_database)))
            self.assertEqual(result.returncode,4);self.assertEqual(report['error']['code'],'environment')
            self.assertIn('PROJ database',report['error']['message'])
            self.assertFalse(out.exists())

    def test_doctor_failure_keeps_inventory_in_single_result(self):
        with tempfile.TemporaryDirectory() as tmp:
            result,report=self.call('doctor','--command','vector',env=dict(os.environ,PROJ_DATA=tmp,PROJ_LIB=tmp))
        self.assertEqual(result.returncode,4);self.assertFalse(report['ok'])
        self.assertEqual(report['error']['code'],'environment');self.assertIn('commands',report)
