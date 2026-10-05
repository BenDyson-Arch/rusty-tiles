"""Actual CLI failures are concise and distinguish imports from bad data."""
import json
import os
import pathlib
import subprocess
import tempfile
import unittest

BIN=os.environ.get('RUSTY_TILES_BIN')

@unittest.skipUnless(BIN,'set RUSTY_TILES_BIN for diagnostics acceptance')
class PythonErrorTests(unittest.TestCase):
    def test_invalid_data_has_short_diagnostics_and_named_debug_traceback(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);p=root/'invalid.geojson';out=root/'out.3tz'
            p.write_text(json.dumps(dict(type='FeatureCollection',features=[dict(type='Feature',id=2,properties={},geometry=dict(type='Polygon',coordinates=[[[0,0,0],[2,2,0],[2,0,0],[0,2,0],[0,0,0]]]))])))
            argv=[BIN,'vector','-i',str(p),'-o',str(out),'--sourceCrs','local']
            for debug in (False,True):
                env=dict(os.environ);env.pop('RUSTY_TILES_PYTHON_TRACEBACK',None)
                if debug:env['RUSTY_TILES_PYTHON_TRACEBACK']='1'
                r=subprocess.run(argv,env=env,capture_output=True)
                self.assertNotEqual(r.returncode,0);self.assertFalse(out.exists())
                self.assertIn(b'invalid polygon topology',r.stderr)
                self.assertNotIn(b'NumPy are required',r.stderr);self.assertNotIn(b'__source__=',r.stderr)
                self.assertLess(len(r.stderr),5000 if debug else 2000)
                self.assertEqual(b'Traceback' in r.stderr,debug)
                if debug:self.assertIn(b'<rusty-tiles/vector_pipeline.py>',r.stderr)

    def test_missing_dependency_is_reported_as_an_import_failure(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=root/'source.geojson';source.write_text('{}')
            (root/'numpy.py').write_text("raise ImportError('test dependency unavailable')")
            env=dict(os.environ,PYTHONPATH=str(root));out=root/'out.3tz'
            r=subprocess.run([BIN,'vector','-i',str(source),'-o',str(out)],env=env,capture_output=True)
            self.assertNotEqual(r.returncode,0);self.assertFalse(out.exists())
            self.assertIn(b'Python dependency import failed',r.stderr);self.assertIn(b'GDAL/GEOS and NumPy',r.stderr)
            self.assertNotIn(b'Traceback',r.stderr)
