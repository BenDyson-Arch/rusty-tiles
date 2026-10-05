"""Actual CLI readiness inventory handles missing interpreters and modules."""
import json
import os
import pathlib
import subprocess
import tempfile
import unittest

BIN=os.environ.get('RUSTY_TILES_BIN')

@unittest.skipUnless(BIN,'set RUSTY_TILES_BIN for doctor acceptance')
class DoctorTests(unittest.TestCase):
    def run_doctor(self,*args,env=None):
        result=subprocess.run([BIN,'doctor','--json',*args],capture_output=True,text=True,env=env)
        return result,json.loads(result.stdout)

    def test_installed_vector_profile_reports_versions_and_capabilities(self):
        result,report=self.run_doctor('--command','vector')
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertTrue(report['ready']);self.assertTrue(report['vectorTriangulation'])
        self.assertTrue(report['geos']['available'])
        self.assertTrue(report['python']['executable']);self.assertTrue(report['modules']['gdal']['version'])
        self.assertIn('availableGrids',report['proj']);self.assertTrue(report['proj']['dataDirectories'])

    def test_missing_module_is_reported_and_native_commands_remain_ready(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);(root/'numpy.py').write_text("raise ImportError('doctor fixture: numpy missing')")
            env=dict(os.environ,PYTHONPATH=str(root))
            result,report=self.run_doctor('--command','vector',env=env)
            self.assertNotEqual(result.returncode,0);self.assertFalse(report['ready'])
            self.assertIn('numpy',report['commands']['vector']['missing'])
            self.assertIn('doctor fixture',report['modules']['numpy']['error'])
            result,report=self.run_doctor('--command','mesh-to-3tz',env=env)
            self.assertEqual(result.returncode,0,result.stderr);self.assertTrue(report['ready'])

    def test_missing_python_does_not_block_native_readiness(self):
        env=dict(os.environ,PATH='')
        result,report=self.run_doctor('--command','vector',env=env)
        self.assertNotEqual(result.returncode,0);self.assertFalse(report['python']['available'])
        result,report=self.run_doctor('--command','convert',env=env)
        self.assertEqual(result.returncode,0,result.stderr);self.assertTrue(report['ready'])

    def test_local_grid_inventory_is_read_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);grid=root/'fixture.gtx';grid.write_bytes(b'inventory marker')
            result,report=self.run_doctor('--command','mesh-to-3tz',env=dict(os.environ,PROJ_DATA=str(root)))
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertIn(str(grid),report['proj']['availableGrids'])
            self.assertEqual(list(root.iterdir()),[grid]);self.assertEqual(grid.read_bytes(),b'inventory marker')
