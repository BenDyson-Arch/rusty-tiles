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
    def call(self, *args, **kwargs):
        # The development oracle uses Python; the actual CLI gets no executable PATH.
        return subprocess.run(*args, env=dict(os.environ, PATH=''), **kwargs)

    def test_json_stdout_and_explicit_alpha_intersect_source_mask(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=root/'rgba-mask.tif'
            ds=gdal.GetDriverByName('GTiff').Create(str(source),256,256,4,gdal.GDT_Byte)
            crs=osr.SpatialReference();crs.ImportFromEPSG(4326)
            ds.SetProjection(crs.ExportToWkt());ds.SetGeoTransform([12.5,.00001,0,41.9,0,-.00001])
            data=np.full((4,256,256),255,dtype=np.uint8)
            data[:3]=0  # Valid black remains opaque where both coverage and alpha permit it.
            data[3,128:192,128:192]=0
            data[3,192:,192:]=128
            for i in range(4):ds.GetRasterBand(i+1).WriteArray(data[i])
            ds.GetRasterBand(4).SetColorInterpretation(gdal.GCI_AlphaBand)
            ds.GetRasterBand(1).CreateMaskBand(gdal.GMF_PER_DATASET)
            mask=np.full((256,256),255,dtype=np.uint8);mask[32:96,32:96]=0
            ds.GetRasterBand(1).GetMaskBand().WriteArray(mask);ds=None
            out=root/'out'
            result=self.call([str(BIN),'raster','-i',str(source),'-o',str(out),'--json',
                '--minZoom','16','--maxZoom','16','--alphaBand','4'],capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertTrue(json.loads(result.stdout)['ok'],'stdout must contain exactly one JSON result')
            def alpha_at(x,y):
                n=2**16;lon=12.5+(x+.5)*.00001;lat=41.9-(y+.5)*.00001
                tx=(lon+180)/360*n;ty=(1-math.asinh(math.tan(math.radians(lat)))/math.pi)/2*n
                tile=gdal.Open(str(out/'tiles/16'/str(math.floor(tx))/f'{math.floor(ty)}.png')).ReadAsArray()
                return int(tile[3,int((ty%1)*256),int((tx%1)*256)])
            self.assertEqual(alpha_at(64,64),0,'separate source mask must override opaque alpha')
            self.assertEqual(alpha_at(160,160),0,'explicit alpha must remain transparent')
            self.assertEqual(alpha_at(112,112),255,'valid black must remain opaque')
            self.assertEqual(alpha_at(224,224),128,'partial alpha must be preserved')
            cog=gdal.Open(str(out/'source.cog.tif'))
            np.testing.assert_array_equal(cog.ReadAsArray(),data)
            np.testing.assert_array_equal(cog.GetRasterBand(1).GetMaskBand().ReadAsArray(),mask)

    def test_numeric_values_and_masks(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp); source=root/'survey.tif'
            ds=gdal.GetDriverByName('GTiff').Create(str(source),32,32,1,gdal.GDT_Float32)
            crs=osr.SpatialReference();crs.ImportFromEPSG(4326)
            ds.SetProjection(crs.ExportToWkt());ds.SetGeoTransform([12.5,.00001,0,41.9,0,-.00001])
            values=np.tile(np.arange(32,dtype=np.float32)-16,(32,1));values[0,0]=-32768
            ds.GetRasterBand(1).WriteArray(values);ds.GetRasterBand(1).SetNoDataValue(-32768);ds=None
            out=root/'raster'
            self.call([str(BIN),'raster','-i',str(source),'-o',str(out),'--maxZoom','16','--display','gray','--band','1','--displayMin','-10','--displayMax','10'],check=True,capture_output=True)
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
            self.call([str(BIN),'raster','-i',str(source),'-o',str(out),'--minZoom','16','--maxZoom','16'],check=True,capture_output=True)
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
            self.call([str(BIN),'raster','-i',str(source),'-o',str(out),'--minZoom','16','--maxZoom','16',
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
            result=self.call([str(BIN),'raster','-i',str(source),'-o',str(out),'--maxZoom','1'],capture_output=True)
            self.assertNotEqual(result.returncode,0)
            self.assertIn(b'image display requires byte imagery',result.stderr)
            self.assertFalse(out.exists())

    def test_invalid_numeric_style_does_not_publish(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=root/'survey.tif'
            ds=gdal.GetDriverByName('GTiff').Create(str(source),2,2,1,gdal.GDT_Byte)
            crs=osr.SpatialReference();crs.ImportFromEPSG(4326);ds.SetProjection(crs.ExportToWkt());ds.SetGeoTransform([12,.0001,0,42,0,-.0001]);ds=None
            out=root/'out'
            result=self.call([str(BIN),'raster','-i',str(source),'-o',str(out),'--maxZoom','1','--display','gray','--displayMin','10','--displayMax','-10'],capture_output=True)
            self.assertNotEqual(result.returncode,0);self.assertFalse(out.exists())

    def test_native_matches_frozen_python_pixels_and_source_metadata(self):
        import importlib.util
        import types
        spec=importlib.util.spec_from_file_location('raster_oracle', ROOT/'tests/fixtures/raster_oracle.py')
        oracle=importlib.util.module_from_spec(spec);spec.loader.exec_module(oracle)
        for kind in ('rgba','gray','projected-gray','byte-gray'):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as tmp:
                root=pathlib.Path(tmp);source=root/'source with spaces.tif'
                numeric='gray' in kind and kind!='byte-gray'
                count=2 if numeric else (4 if kind=='rgba' else 1)
                dtype=gdal.GDT_Float32 if numeric else gdal.GDT_Byte
                ds=gdal.GetDriverByName('GTiff').Create(str(source),257,259,count,dtype)
                crs=osr.SpatialReference();crs.ImportFromEPSG(32633 if kind=='projected-gray' else 4326)
                ds.SetProjection(crs.ExportToWkt())
                ds.SetGeoTransform([500000,1,0,4650000,0,-1] if kind=='projected-gray' else [12.5,.00001,0,41.9,0,-.00001])
                ds.SetMetadata({'fixture':'invented source metadata'})
                for i in range(1,count+1):
                    band=ds.GetRasterBand(i)
                    values=np.tile(np.linspace(-80,80,257,dtype=np.float32),(259,1)) if numeric else np.full((259,257),min(255,64*i),dtype=np.uint8)
                    if kind=='rgba':
                        if i<4:values[:128,:128]=0
                        else:values[:]=255;values[100:180,100:180]=0;values[192:,192:]=128
                    if numeric:values[0,0]=-9999;band.SetNoDataValue(-9999);band.SetUnitType('m')
                    band.WriteArray(values);band.SetDescription(f'measurement-{i}')
                    band.SetMetadata({'meaning':f'field-{i}'})
                    if numeric and i==1:band.SetScale(2);band.SetOffset(10)  # Unstyled source values/metadata remain intact.
                masked=ds.GetRasterBand(2 if numeric else 1)
                masked.CreateMaskBand(gdal.GMF_PER_DATASET)
                mask=np.full((259,257),255,dtype=np.uint8);mask[32:96,32:96]=0
                masked.GetMaskBand().WriteArray(mask)
                if kind=='rgba':ds.GetRasterBand(4).SetColorInterpretation(gdal.GCI_AlphaBand)
                ds=None
                native=root/'native';legacy=root/'legacy';legacy.mkdir()
                recipe=dict(display='gray' if numeric else 'image',band=2 if numeric else 1,
                    alpha_band=4 if kind=='rgba' else 0,display_min=-100 if numeric else None,
                    display_max=100 if numeric else None,min_zoom=15,max_zoom=16)
                argv=[str(BIN),'raster','-i',str(source),'-o',str(native),'--minZoom','15','--maxZoom','16',
                    '--display',recipe['display'],'--band',str(recipe['band']),'--alphaBand',str(recipe['alpha_band'])]
                if numeric:argv+=['--displayMin','-100','--displayMax','100']
                result=self.call(argv,capture_output=True,text=True)
                self.assertEqual(result.returncode,0,result.stderr)
                oracle.run(types.SimpleNamespace(input=str(source),output=str(legacy),**recipe))
                self.assertEqual(json.loads((native/'tilejson.json').read_text()),json.loads((legacy/'tilejson.json').read_text()))
                self.assertEqual(json.loads((native/'conversion.json').read_text()),json.loads((legacy/'conversion.json').read_text()))
                files=lambda out:{str(p.relative_to(out)) for p in (out/'tiles').rglob('*.png')}
                self.assertEqual(files(native),files(legacy));self.assertTrue(files(native))
                for path in files(native):
                    np.testing.assert_array_equal(gdal.Open(str(native/path)).ReadAsArray(),gdal.Open(str(legacy/path)).ReadAsArray(),err_msg=path)
                original=gdal.Open(str(source));cog=gdal.Open(str(native/'source.cog.tif'))
                np.testing.assert_array_equal(cog.ReadAsArray(),original.ReadAsArray())
                self.assertEqual(cog.GetMetadata(),original.GetMetadata())
                self.assertEqual(cog.GetGeoTransform(),original.GetGeoTransform());self.assertEqual(cog.GetProjection(),original.GetProjection())
                self.assertEqual(cog.GetMetadata('IMAGE_STRUCTURE')['LAYOUT'],'COG')
                for i in range(1,count+1):
                    a=original.GetRasterBand(i);b=cog.GetRasterBand(i)
                    for name in ('GetNoDataValue','GetScale','GetOffset','GetUnitType','GetDescription','GetMetadata','GetColorInterpretation'):
                        self.assertEqual(getattr(a,name)(),getattr(b,name)(),(kind,i,name))
                    np.testing.assert_array_equal(b.GetMaskBand().ReadAsArray(),a.GetMaskBand().ReadAsArray())
                self.assertEqual({p.name for p in native.iterdir()},{'source.cog.tif','tiles','tilejson.json','conversion.json'})

    def test_invalid_inputs_and_styles_preserve_previous_directory(self):
        cases=[
            ({},['--minZoom','2','--maxZoom','1'],'minZoom'),
            ({},['--maxZoom','25'],'maxZoom'),
            ({},['--maxZoom','1','--display','other'],'display must'),
            ({},['--maxZoom','1','--band','0','--display','gray','--displayMin','0','--displayMax','1'],'band does not exist'),
            ({},['--maxZoom','1','--alphaBand','4'],'band does not exist'),
            ({},['--maxZoom','1','--displayMin','0'],'require gray'),
            ({},['--maxZoom','1','--display','gray','--displayMin','nan','--displayMax','1'],'finite increasing range'),
            ({},['--maxZoom','1','--display','gray','--displayMin','0','--displayMax','1','--alphaBand','1'],'alphaBand is for image'),
            ({'scale':2},['--maxZoom','1','--display','gray','--displayMin','0','--displayMax','1'],'scaled bands'),
            ({'crs':None},['--maxZoom','1'],'declared CRS'),
            ({'gt':[12,.01,0,86,0,-.01]},['--maxZoom','1'],'polar coverage'),
            ({'gt':[179,.1,0,42,0,-.01]},['--maxZoom','1'],'antimeridian'),
            ({'gt':[12,.1,0,42,0,-.1]},['--maxZoom','24'],'100000 tiles'),
            ({'crs':3857,'gt':[19900000,5000,0,5000000,0,-5000]},['--maxZoom','1'],'antimeridian'),
        ]
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp)
            for config,opts,message in cases:
                with self.subTest(config=config,opts=opts):
                    source=root/'source.tif'
                    ds=gdal.GetDriverByName('GTiff').Create(str(source),32,32,1,gdal.GDT_Byte)
                    crs=osr.SpatialReference();crs.ImportFromEPSG(config.get('crs') or 4326)
                    if config.get('crs',True):ds.SetProjection(crs.ExportToWkt())
                    ds.SetGeoTransform(config.get('gt',[12,.01,0,42,0,-.01]))
                    ds.GetRasterBand(1).WriteArray(np.ones((32,32),dtype=np.uint8))
                    if 'scale' in config:ds.GetRasterBand(1).SetScale(config['scale'])
                    ds=None
                    out=root/'out';out.mkdir(exist_ok=True);(out/'previous').write_bytes(b'keep original bytes')
                    result=self.call([str(BIN),'raster','--json','--progress','json','-i',str(source),'-o',str(out),'-f',*opts],capture_output=True,text=True)
                    self.assertEqual(result.returncode,3,result.stderr)
                    report=json.loads(result.stdout);self.assertEqual(report['error']['code'],'data')
                    self.assertIn(message,report['error']['message'])
                    self.assertEqual({p.name for p in out.iterdir()},{'previous'})
                    self.assertEqual((out/'previous').read_bytes(),b'keep original bytes')
                    self.assertEqual({p.name for p in root.iterdir()},{'source.tif','out'})

    def test_native_raster_doctor_and_force_publication_without_executables(self):
        result=self.call([str(BIN),'doctor','--command','raster','--json'],capture_output=True,text=True)
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertTrue(json.loads(result.stdout)['commands']['raster']['tiling']['ready'])
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=root/'source.tif';out=root/'out';out.mkdir()
            (out/'previous').write_bytes(b'old result')
            ds=gdal.GetDriverByName('GTiff').Create(str(source),32,32,1,gdal.GDT_Byte)
            crs=osr.SpatialReference();crs.ImportFromEPSG(4326);ds.SetProjection(crs.ExportToWkt());ds.SetGeoTransform([12,.01,0,42,0,-.01])
            ds.GetRasterBand(1).WriteArray(np.ones((32,32),dtype=np.uint8));ds=None
            argv=[str(BIN),'raster','--json','--progress','json','-i',str(source),'-o',str(out),'--maxZoom','0']
            result=self.call(argv,capture_output=True,text=True)
            self.assertEqual(result.returncode,5);self.assertEqual(json.loads(result.stdout)['error']['code'],'output_conflict')
            result=self.call(argv+['-f'],capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr);self.assertTrue(json.loads(result.stdout)['ok'])
            self.assertFalse((out/'previous').exists())
            events=[json.loads(line) for line in result.stderr.splitlines()]
            self.assertEqual(events[0]['done'],0);self.assertEqual(events[-1]['done'],1)
            before={str(p.relative_to(out)):p.read_bytes() for p in out.rglob('*') if p.is_file()}
            result=self.call(argv+['-f'],capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr)
            after={str(p.relative_to(out)):p.read_bytes() for p in out.rglob('*') if p.is_file()}
            self.assertEqual(before,after,'same revision/dependency stack produces identical output')

    def test_wide_geographic_raster_without_wrap_remains_supported(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=root/'wide.tif';out=root/'out'
            ds=gdal.GetDriverByName('GTiff').Create(str(source),32,32,1,gdal.GDT_Byte)
            crs=osr.SpatialReference();crs.ImportFromEPSG(4326);ds.SetProjection(crs.ExportToWkt())
            ds.SetGeoTransform([-170,340/32,0,20,0,-40/32]);ds.GetRasterBand(1).WriteArray(np.ones((32,32),dtype=np.uint8));ds=None
            result=self.call([str(BIN),'raster','-i',str(source),'-o',str(out),'--maxZoom','0'],capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr)
            manifest=json.loads((out/'tilejson.json').read_text())
            self.assertEqual(manifest['bounds'],[-170,-20,170,20])
            rgba=gdal.Open(str(out/'tiles/0/0/0.png')).ReadAsArray()
            self.assertTrue(np.any(rgba[-1]>0))
