import pathlib,struct,json,subprocess,time
import numpy as np
from osgeo import gdal,osr
root=pathlib.Path('/home/bend/.cache/terrain-t1-evidence');binary='/home/bend/.cache/rusty-tiles-132-12d98ab-native'
srs=osr.SpatialReference();srs.ImportFromEPSG(4326)
results=[]
for name,kind in [('constant','constant'),('mask','mask'),('float64','float64')]:
 p=root/(name+'.tif');ds=gdal.GetDriverByName('GTiff').Create(str(p),32,32,1,gdal.GDT_Float64);ds.SetProjection(srs.ExportToWkt());ds.SetGeoTransform([11.25,.02197265625,0,42.1875,0,-.02197265625]);b=ds.GetRasterBand(1);b.SetUnitType('m');values=np.full((32,32),123.5 if kind!='float64' else 100000000.125);b.WriteArray(values)
 if kind=='mask':
  b.CreateMaskBand(gdal.GMF_PER_DATASET);m=np.full((32,32),255,dtype=np.uint8);m[8:24,8:24]=0;b.GetMaskBand().WriteArray(m)
 ds=None
 out=root/(name+'-out');start=time.monotonic();r=subprocess.run([binary,'--json','terrain','-i',str(p),'-o',str(out),'--force','--maxZoom','8','--grid','17','--heightOffset','0','--fillHeight','-999','--maxError','0'],capture_output=True,text=True)
 item={'case':name,'returncode':r.returncode,'elapsed':time.monotonic()-start,'stdout':r.stdout,'stderr':r.stderr}
 if r.returncode==0:
  tile=out/'8'/'272'/'187.terrain'; paths=list((out/'8').glob('*/*.terrain'));tile=paths[0];data=tile.read_bytes();header=struct.unpack_from('<3d2f7d',data);n=struct.unpack_from('<I',data,88)[0];overlay=json.loads(tile.with_suffix('.heights.json').read_text());finite=[x for x in overlay['heights'] if x is not None]
  item.update(gzip_magic=data[:2].hex(),header_range=header[3:5],tile=str(tile),vertices=n,sidecar_min=min(finite) if finite else None,sidecar_max=max(finite) if finite else None,sidecar_nulls=overlay['heights'].count(None))
 results.append(item)
(root/'results.json').write_text(json.dumps(results,indent=2));print(json.dumps(results,indent=2))
