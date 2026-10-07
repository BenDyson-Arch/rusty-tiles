"""Native CLI readiness inventory is offline and independent of Python."""
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
        self.assertTrue(report['ready'])
        vector = report['commands']['vector']
        self.assertTrue(vector['geometry']['ready'])
        self.assertTrue(vector['geospatial']['ready'])
        self.assertEqual(vector['backend'], 'native GDAL/GEOS')
        self.assertTrue(vector['geospatial']['versions']['gdal'])
        self.assertNotIn('python', report)

    def test_missing_module_is_reported_and_native_commands_remain_ready(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);(root/'numpy.py').write_text("raise ImportError('doctor fixture: numpy missing')")
            env=dict(os.environ,PYTHONPATH=str(root))
            result,report=self.run_doctor('--command','vector',env=env)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertTrue(report['ready'])
            self.assertNotIn('modules', report)
            result,report=self.run_doctor('--command','mesh-to-3tz',env=env)
            self.assertEqual(result.returncode,0,result.stderr);self.assertTrue(report['ready'])

    def test_missing_python_does_not_block_native_readiness(self):
        env=dict(os.environ,PATH='')
        result,report=self.run_doctor('--command','vector',env=env)
        self.assertEqual(result.returncode,0,result.stderr);self.assertTrue(report['ready'])
        self.assertNotIn('python', report)
        result,report=self.run_doctor('--command','convert',env=env)
        self.assertEqual(result.returncode,0,result.stderr);self.assertTrue(report['ready'])

    def test_all_commands_without_python(self):
        result,report=self.run_doctor(env=dict(os.environ,PATH=''))
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertTrue(report['ready'])
        self.assertEqual(set(report['selectedCommands']),set(report['commands']))
        self.assertNotIn('python',report)
        self.assertNotIn('modules',report)
        self.assertFalse(report['proj']['networkEnabled'])
        self.assertTrue(report['proj']['database']['ready'])

    def test_missing_database_is_environment_error_only_for_selected_geospatial(self):
        with tempfile.TemporaryDirectory() as tmp:
            env=dict(os.environ,PATH='',PROJ_DATA=tmp,PROJ_LIB=tmp)
            result,report=self.run_doctor('--command','terrain',env=env)
            self.assertEqual(result.returncode,4,result.stderr)
            self.assertFalse(report['ok'])
            self.assertEqual(report['error']['code'],'environment')
            self.assertIn('database',report['proj']['database']['error'])
            self.assertTrue(report['nativeGeospatial']['versions']['gdal'])
            result,report=self.run_doctor('--command','mesh-to-3tz',env=env)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertTrue(report['ready'])
            self.assertFalse(report['proj']['database']['ready'])

    def test_local_grid_inventory_is_read_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);grid=root/'fixture.gtx';grid.write_bytes(b'inventory marker')
            result,report=self.run_doctor('--command','mesh-to-3tz',env=dict(os.environ,PROJ_DATA=str(root)))
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertIn(str(grid),report['proj']['availableGrids'])
            self.assertEqual(list(root.iterdir()),[grid]);self.assertEqual(grid.read_bytes(),b'inventory marker')
