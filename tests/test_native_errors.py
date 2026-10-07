"""Actual native CLI failures stay concise and independent of Python imports."""
import json
import os
import pathlib
import subprocess
import tempfile
import unittest

BIN=os.environ.get('RUSTY_TILES_BIN')

@unittest.skipUnless(BIN,'set RUSTY_TILES_BIN for diagnostics acceptance')
class NativeErrorTests(unittest.TestCase):
    def test_invalid_data_has_short_diagnostics_and_native_debug_warnings(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);p=root/'invalid.geojson';out=root/'out.3tz'
            p.write_text(json.dumps(dict(type='FeatureCollection',features=[dict(type='Feature',id=2,properties={},geometry=dict(type='Polygon',coordinates=[[[0,0,0],[2,2,0],[2,0,0],[0,2,0],[0,0,0]]]))])))
            argv=[BIN,'vector','-i',str(p),'-o',str(out),'--sourceCrs','local']
            # The deprecated RUSTY_TILES_PYTHON_TRACEBACK name remains a fallback.
            for debug,name in ((False,None),(True,'RUSTY_TILES_NATIVE_DIAGNOSTICS'),(True,'RUSTY_TILES_PYTHON_TRACEBACK')):
                env=dict(os.environ)
                for variable in ('RUSTY_TILES_NATIVE_DIAGNOSTICS','RUSTY_TILES_PYTHON_TRACEBACK'):env.pop(variable,None)
                if name:env[name]='1'
                r=subprocess.run(argv,env=env,capture_output=True)
                self.assertNotEqual(r.returncode,0);self.assertFalse(out.exists())
                self.assertIn(b'invalid polygon topology',r.stderr)
                self.assertNotIn(b'NumPy are required',r.stderr);self.assertNotIn(b'__source__=',r.stderr)
                self.assertLess(len(r.stderr),5000 if debug else 2000)
                self.assertNotIn(b'Traceback',r.stderr)
                self.assertEqual(b'Warning 1:' in r.stderr,debug)

    def test_missing_python_modules_do_not_hide_invalid_input(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=root/'source.geojson';source.write_text('{}')
            (root/'numpy.py').write_text("raise ImportError('test dependency unavailable')")
            env=dict(os.environ,PYTHONPATH=str(root));out=root/'out.3tz'
            r=subprocess.run([BIN,'vector','-i',str(source),'-o',str(out)],env=env,capture_output=True)
            self.assertNotEqual(r.returncode,0);self.assertFalse(out.exists())
            self.assertNotIn(b'Python dependency import failed',r.stderr)
            self.assertIn(b'OGR cannot open vector input',r.stderr)
            self.assertNotIn(b'Traceback',r.stderr)
