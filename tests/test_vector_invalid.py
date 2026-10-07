"""Opt-in omissions, complete diagnostics and publication safety."""
import contextlib
import io
import json
import pathlib
import subprocess
import tempfile
import types
import unittest

from test_vector_lod import vector, read

from cli_bin import BIN, requires_bin

class InvalidFeatureTests(unittest.TestCase):
    def source(self,root):
        square=[[0,0,0],[2,0,0],[2,2,0],[0,2,0],[0,0,0]]
        bow=[[4,0,0],[6,2,0],[6,0,0],[4,2,0],[4,0,0]]
        p=root/'mixed.geojson'
        p.write_text(json.dumps(dict(type='FeatureCollection',features=[
            dict(type='Feature',id=i,properties=dict(name=str(i)),geometry=dict(type='Polygon',coordinates=[ring]))
            for i,ring in [(1,square),(2,bow),(3,bow)]])))
        return p

    def args(self,p,out,skip=False):
        return types.SimpleNamespace(input=str(p),output=str(out),source_crs='local',max_features=64,skip_invalid=skip)

    def test_strict_mode_reports_all_invalid_identities_before_failing(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);p=self.source(root);errors=io.StringIO()
            with contextlib.redirect_stderr(errors),self.assertRaisesRegex(ValueError,'2 unconvertible'):
                vector.run(self.args(p,root/'out'))
            self.assertIn('feature 2:',errors.getvalue());self.assertIn('feature 3:',errors.getvalue())
            self.assertFalse((root/'out/tileset.json').exists())

    def test_skip_mode_preserves_valid_metadata_and_reports_every_omission(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);p=self.source(root);out=root/'out'
            vector.run(self.args(p,out,True))
            report=json.loads((out/'conversion.json').read_text())
            self.assertEqual(report['features'],1);self.assertEqual(report['skippedFeatures'],2)
            self.assertEqual(report['layers'][0]['invalidFeatures'],2)
            records=[json.loads(line) for line in (out/'geometry-reports.jsonl').read_text().splitlines()]
            skipped=[r for r in records if r.get('outcome')=='skipped']
            self.assertEqual({r['sourceId'] for r in skipped},{'2','3'})
            self.assertTrue(all(r['sourceLayer']=='mixed' and r['reason'] for r in skipped))
            manifest=json.loads((out/'tileset.json').read_text())
            _,positions,ids=read(out/manifest['root']['content']['uri'])
            self.assertEqual(ids,['1']);self.assertEqual(len(positions[0]),4)

    @requires_bin('set RUSTY_TILES_BIN for actual CLI publication checks')
    def test_cli_strict_publishes_nothing_and_skip_mode_publishes_archive(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);p=self.source(root);out=root/'result.3tz'
            args=[BIN,'vector','-i',str(p),'-o',str(out),'--sourceCrs','local']
            result=subprocess.run(args,capture_output=True)
            self.assertNotEqual(result.returncode,0);self.assertFalse(out.exists())
            self.assertIn(b'feature 2:',result.stderr);self.assertIn(b'feature 3:',result.stderr)
            result=subprocess.run(args+['--skipInvalid'],capture_output=True)
            self.assertEqual(result.returncode,0,result.stderr.decode());self.assertTrue(out.is_file())
