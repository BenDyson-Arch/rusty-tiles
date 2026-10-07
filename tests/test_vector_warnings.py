"""Native warnings stay quiet without hiding diagnostics or leaking handlers."""
import json
import os
import pathlib
import subprocess
import tempfile
import unittest
import zipfile
from unittest import mock

from osgeo import gdal
from test_vector_lod import vector

from cli_bin import BIN, requires_bin

class GeometryWarningTests(unittest.TestCase):
    @requires_bin('set RUSTY_TILES_BIN for native stderr acceptance')
    def test_skip_and_repair_are_quiet_and_debug_retains_native_warnings(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            source = root/'bowties.geojson'
            rings = [[[x,0,0],[x+2,2,0],[x+2,0,0],[x,2,0],[x,0,0]] for x in (0,4,8)]
            source.write_text(json.dumps(dict(type='FeatureCollection', features=[
                dict(type='Feature',id=0,properties=dict(name='valid'),geometry=dict(type='Polygon',
                    coordinates=[[[12,0,0],[14,0,0],[14,2,0],[12,2,0],[12,0,0]]]))] + [
                dict(type='Feature',id=i,properties=dict(name=str(i)),
                     geometry=dict(type='Polygon',coordinates=[ring]))
                for i,ring in enumerate(rings,1)])))
            for index, (flags, debug) in enumerate([
                (['--skipInvalid'], False), (['--repair'], False), (['--repair'], True),
            ]):
                output = root/f'out-{index}.3tz'
                env = dict(os.environ, RUSTY_TILES_NATIVE_DIAGNOSTICS='1' if debug else '0')
                result = subprocess.run([BIN,'vector','-i',str(source),'-o',str(output),
                    '--sourceCrs','local',*flags],capture_output=True,text=True,env=env)
                self.assertEqual(result.returncode,0,result.stderr)
                self.assertEqual('Warning 1:' in result.stderr,debug,result.stderr)
                with zipfile.ZipFile(output) as archive:
                    reports = [json.loads(line) for line in archive.read('geometry-reports.jsonl').decode().splitlines()]
                if '--skipInvalid' in flags:
                    self.assertEqual({r['sourceId'] for r in reports if r.get('outcome')=='skipped'}, {'1','2','3'})
                else:
                    self.assertTrue(any(r.get('topologyRepaired') for r in reports))
