"""Verify custom coverage sidecars against independently decoded terrain values."""
import importlib.util
import json
import pathlib
import tempfile
import types
import unittest

import numpy as np
from osgeo import gdal, osr
from test_derivatives import decode_terrain

ROOT=pathlib.Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('terrain_overlay',ROOT/'scripts/terrain.py')
terrain=importlib.util.module_from_spec(spec)
spec.loader.exec_module(terrain)


class TerrainOverlayTests(unittest.TestCase):
    def test_coverage_nulls_height_offset_and_south_to_north_rows(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=root/'dem.tif';out=root/'tiles'
            ds=gdal.GetDriverByName('GTiff').Create(str(source),32,32,1,gdal.GDT_Float32)
            crs=osr.SpatialReference();crs.ImportFromEPSG(4326)
            ds.SetProjection(crs.ExportToWkt());ds.SetGeoTransform([12,.01,0,42,0,-.01])
            values=100+np.add.outer(np.arange(32)*10,np.arange(32)).astype(np.float32)
            values[12:20,12:20]=-32768
            ds.GetRasterBand(1).WriteArray(values);ds.GetRasterBand(1).SetNoDataValue(-32768);ds=None
            terrain.run(types.SimpleNamespace(input=str(source),output=str(out),max_zoom=9,grid=17,
                height_offset=10.,fill_height=-999.,max_tiles=1000))
            manifest=json.loads((out/'layer.json').read_text())
            self.assertEqual(manifest['heightOverlay'],dict(version=1,tiles=['{z}/{x}/{y}.heights.json'],grid=17,rowOrder='south-to-north'))
            report=json.loads((out/'conversion.json').read_text())
            self.assertEqual(report['heightRange'][0],-999)
            active=0
            ordered=0
            for path in out.glob('*/*/*.heights.json'):
                overlay=json.loads(path.read_text())
                self.assertEqual((overlay['width'],overlay['height']),(17,17))
                self.assertEqual(len(overlay['heights']),289)
                heights=np.array([np.nan if h is None else h for h in overlay['heights']]).reshape(17,17)
                valid=np.isfinite(heights)
                header,attrs,*_=decode_terrain(path.with_name(path.name.replace('.heights.json','.terrain')).read_bytes())
                decoded=header[3]+attrs[:,2]/32767*(header[4]-header[3])
                x=np.rint(attrs[:,0]/32767*16).astype(int)
                y=np.rint(attrs[:,1]/32767*16).astype(int)
                sampled=np.where(valid,heights,-999.)[y,x]
                np.testing.assert_allclose(decoded,sampled,atol=report['heightQuantizationStep']/2+1e-8,rtol=0)
                if valid.any():
                    active+=1
                    rows=np.where(valid.any(axis=1))[0]
                    if len(rows)>1:
                        ordered+=1
                        self.assertGreater(np.nanmean(heights[rows[0]]),np.nanmean(heights[rows[-1]]))
                    self.assertTrue(np.isnan(heights).any())
                    self.assertGreaterEqual(float(heights[valid].min()),110)
            self.assertGreater(active,0)
            self.assertGreater(ordered,0)
