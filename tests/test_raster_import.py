"""Conversion acceptance checks with invented coordinates and signed measurements."""
import json
import math
import os
import pathlib
import subprocess
import tempfile
import unittest
import numpy as np
from osgeo import gdal, osr

ROOT=pathlib.Path(__file__).resolve().parents[1]
BIN=pathlib.Path(os.environ.get('RUSTY_TILES_BIN',ROOT/'target/debug/rusty-tiles'))
gdal.UseExceptions(); osr.UseExceptions()
if os.environ.get('RUSTY_TILES_BIN') and not BIN.is_file():
    raise RuntimeError(f'configured CLI is missing: {BIN}')

@unittest.skipUnless(BIN.exists(), 'build the local rusty-tiles executable first')
class RasterImportTests(unittest.TestCase):
    def test_numeric_values_and_masks(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp); source=root/'survey.tif'
            ds=gdal.GetDriverByName('GTiff').Create(str(source),32,32,1,gdal.GDT_Float32)
            crs=osr.SpatialReference();crs.ImportFromEPSG(4326)
            ds.SetProjection(crs.ExportToWkt());ds.SetGeoTransform([12.5,.00001,0,41.9,0,-.00001])
            values=np.tile(np.arange(32,dtype=np.float32)-16,(32,1));values[0,0]=-32768
            ds.GetRasterBand(1).WriteArray(values);ds.GetRasterBand(1).SetNoDataValue(-32768);ds=None
            out=root/'raster'
            subprocess.run([str(BIN),'raster','-i',str(source),'-o',str(out),'--maxZoom','16','--display','gray','--band','1','--displayMin','-10','--displayMax','10'],check=True,capture_output=True)
            np.testing.assert_array_equal(gdal.Open(str(out/'source.cog.tif')).ReadAsArray(),values)
            cog=gdal.Open(str(out/'source.cog.tif'))
            self.assertEqual(cog.GetRasterBand(1).GetNoDataValue(),-32768)
            tiles=list((out/'tiles/16').rglob('*.png'));self.assertTrue(tiles)
            rgba=gdal.Open(str(tiles[0])).ReadAsArray();self.assertEqual(rgba.shape[0],4)
            self.assertTrue(np.any(rgba[3]==0));self.assertTrue(np.any(rgba[3]>0))

    def test_byte_rgb_preserves_valid_black_and_source_mask(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=root/'image.tif'
            ds=gdal.GetDriverByName('GTiff').Create(str(source),256,256,3,gdal.GDT_Byte)
            crs=osr.SpatialReference();crs.ImportFromEPSG(4326)
            ds.SetProjection(crs.ExportToWkt());ds.SetGeoTransform([12.5,.00001,0,41.9,0,-.00001])
            data=np.zeros((3,256,256),dtype=np.uint8);data[:,:,128:]=np.array([120,170,220])[:,None,None]
            for i in range(3): ds.GetRasterBand(i+1).WriteArray(data[i])
            ds.GetRasterBand(1).CreateMaskBand(gdal.GMF_PER_DATASET)
            mask=np.full((256,256),255,dtype=np.uint8);mask[:64,:64]=0
            ds.GetRasterBand(1).GetMaskBand().WriteArray(mask);ds=None
            out=root/'out'
            subprocess.run([str(BIN),'raster','-i',str(source),'-o',str(out),'--minZoom','16','--maxZoom','16'],check=True,capture_output=True)
            tiles=[gdal.Open(str(p)).ReadAsArray() for p in (out/'tiles/16').rglob('*.png')]
            self.assertTrue(any(np.any((a[3]==255)&np.all(a[:3]==0,axis=0)) for a in tiles))
            self.assertTrue(any(np.any(a[3]==0) for a in tiles))
            cog=gdal.Open(str(out/'source.cog.tif'))
            np.testing.assert_array_equal(cog.ReadAsArray(),data)
            np.testing.assert_array_equal(cog.GetRasterBand(1).GetMaskBand().ReadAsArray(),mask)

    def test_numeric_gray_respects_internal_mask_at_covered_pixel(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=root/'numeric-mask.tif'
            ds=gdal.GetDriverByName('GTiff').Create(str(source),256,256,1,gdal.GDT_Float32)
            crs=osr.SpatialReference();crs.ImportFromEPSG(4326)
            ds.SetProjection(crs.ExportToWkt());ds.SetGeoTransform([12.5,.00001,0,41.9,0,-.00001])
            ds.GetRasterBand(1).WriteArray(np.full((256,256),.5,dtype=np.float32))
            ds.GetRasterBand(1).CreateMaskBand(gdal.GMF_PER_DATASET)
            mask=np.full((256,256),255,dtype=np.uint8);mask[32:96,32:96]=0
            ds.GetRasterBand(1).GetMaskBand().WriteArray(mask);ds=None
            out=root/'out'
            subprocess.run([str(BIN),'raster','-i',str(source),'-o',str(out),'--minZoom','16','--maxZoom','16',
                '--display','gray','--displayMin','0','--displayMax','1'],check=True,capture_output=True)
            n=2**16;lon=12.5+64*.00001;lat=41.9-64*.00001
            tx=(lon+180)/360*n;ty=(1-math.asinh(math.tan(math.radians(lat)))/math.pi)/2*n
            tile=gdal.Open(str(out/'tiles/16'/str(math.floor(tx))/f'{math.floor(ty)}.png')).ReadAsArray()
            px=int((tx%1)*256);py=int((ty%1)*256)
            self.assertEqual(int(tile[3,py,px]),0,'masked source pixel must remain transparent inside coverage')

    def test_numeric_image_mode_rejected_without_publishing(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=root/'numeric.tif'
            ds=gdal.GetDriverByName('GTiff').Create(str(source),2,2,1,gdal.GDT_Float32)
            crs=osr.SpatialReference();crs.ImportFromEPSG(4326)
            ds.SetProjection(crs.ExportToWkt());ds.SetGeoTransform([12,.0001,0,42,0,-.0001])
            ds.GetRasterBand(1).WriteArray(np.array([[-10,0],[300,400]],dtype=np.float32));ds=None
            out=root/'out'
            result=subprocess.run([str(BIN),'raster','-i',str(source),'-o',str(out),'--maxZoom','1'],capture_output=True)
            self.assertNotEqual(result.returncode,0)
            self.assertIn(b'image display requires byte imagery',result.stderr)
            self.assertFalse(out.exists())

    def test_invalid_numeric_style_does_not_publish(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=root/'survey.tif'
            ds=gdal.GetDriverByName('GTiff').Create(str(source),2,2,1,gdal.GDT_Byte)
            crs=osr.SpatialReference();crs.ImportFromEPSG(4326);ds.SetProjection(crs.ExportToWkt());ds.SetGeoTransform([12,.0001,0,42,0,-.0001]);ds=None
            out=root/'out'
            result=subprocess.run([str(BIN),'raster','-i',str(source),'-o',str(out),'--maxZoom','1','--display','gray','--displayMin','10','--displayMax','-10'],capture_output=True)
            self.assertNotEqual(result.returncode,0);self.assertFalse(out.exists())
