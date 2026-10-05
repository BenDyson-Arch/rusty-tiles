"""Actual replacements succeed; failed jobs retain all previous output bytes."""
import os
import pathlib
import subprocess
import tempfile
import unittest
import numpy as np
try:
    import laspy
except ImportError:
    laspy=None
from osgeo import gdal,osr

BIN=os.environ.get('RUSTY_TILES_BIN')
ROOT=pathlib.Path(__file__).resolve().parents[1]

@unittest.skipUnless(BIN,'set RUSTY_TILES_BIN for replacement acceptance')
class ForceTests(unittest.TestCase):
    def test_each_command_replaces_only_successful_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);raster=root/'source.tif';cloud=root/'cloud.las'
            ds=gdal.GetDriverByName('GTiff').Create(str(raster),8,8,1,gdal.GDT_Byte)
            crs=osr.SpatialReference();crs.ImportFromEPSG(4326)
            ds.SetProjection(crs.ExportToWkt());ds.SetGeoTransform([12,.01,0,42,0,-.01])
            ds.GetRasterBand(1).WriteArray(np.ones((8,8),dtype=np.uint8));ds=None
            if laspy is not None:
                las=laspy.LasData(laspy.LasHeader(point_format=3,version='1.2'));las.x=[0,1];las.y=[0,1];las.z=[0,1];las.write(cloud)
            jobs=[('vector',ROOT/'tests/fixtures/vector.geojson',[]),('point-cloud',cloud,['--sourceCrs','local']),
                  ('raster',raster,['--maxZoom','0']),('terrain',raster,['--maxZoom','0','--heightOffset','0','--fillHeight','0'])]
            for command,source,opts in jobs:
                if command=='point-cloud' and laspy is None:continue
                with self.subTest(command=command):
                    out=root/command
                    directory=command in ('raster','terrain')
                    if directory:out.mkdir();(out/'previous').write_bytes(b'original')
                    else:out.write_bytes(b'original')
                    argv=[BIN,command,'-i',str(source),'-o',str(out),*opts]
                    rejected=subprocess.run(argv,capture_output=True)
                    self.assertNotEqual(rejected.returncode,0);self.assertIn(b'--force',rejected.stderr)
                    bad=root/'bad';bad.write_bytes(b'invalid data')
                    failure=subprocess.run([BIN,command,'-i',str(bad),'-o',str(out),*opts,'--force'],capture_output=True)
                    self.assertNotEqual(failure.returncode,0)
                    self.assertEqual((out/'previous' if directory else out).read_bytes(),b'original')
                    success=subprocess.run(argv+['-f'],capture_output=True)
                    self.assertEqual(success.returncode,0,success.stderr.decode())
                    if directory:self.assertFalse((out/'previous').exists());self.assertTrue(any(out.iterdir()))
                    else:self.assertNotEqual(out.read_bytes(),b'original')
