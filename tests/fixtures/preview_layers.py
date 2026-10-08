"""Create invented inputs and convert all five preview layers with the native CLI.
Usage: RUSTY_TILES_BIN=/absolute/native/binary python preview_layers.py new/output
Requires development GDAL, NumPy, laspy and pyproj; downloads nothing.
"""
import json,pathlib,struct,subprocess,zipfile,os,sys,types
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
from test_point_cloud import fixture,run
from pyproj import CRS
from vector_compat import generate
import numpy as np
from osgeo import gdal,osr
gdal.UseExceptions();osr.UseExceptions()
root=pathlib.Path(sys.argv[1]);root.mkdir(parents=True,exist_ok=False)
cli_binary=os.environ['RUSTY_TILES_BIN']
def call(args): subprocess.run([cli_binary,*args],env=dict(os.environ,PATH=''),check=True)
positions=np.array([[-30,0,-30],[30,0,-30],[30,0,30],[-30,0,30]],dtype='<f4')
indices=np.array([0,2,1,0,3,2],dtype='<u2');binary=positions.tobytes()+indices.tobytes()
doc=dict(asset={'version':'2.0'},buffers=[{'byteLength':len(binary)}],bufferViews=[{'buffer':0,'byteOffset':0,'byteLength':positions.nbytes},{'buffer':0,'byteOffset':positions.nbytes,'byteLength':indices.nbytes}],
accessors=[{'bufferView':0,'componentType':5126,'count':4,'type':'VEC3','min':positions.min(axis=0).tolist(),'max':positions.max(axis=0).tolist()},
 {'bufferView':1,'componentType':5123,'count':6,'type':'SCALAR'}],materials=[{'doubleSided':True,'pbrMetallicRoughness':{'baseColorFactor':[1,0,0,1],'metallicFactor':0,'roughnessFactor':1}}],
meshes=[{'primitives':[{'attributes':{'POSITION':0},'indices':1,'material':0,'mode':4}]}],nodes=[{'mesh':0}],scenes=[{'nodes':[0]}],scene=0)
data=json.dumps(doc,separators=(',',':')).encode();data+=b' '*((-len(data))%4);binary+=b'\0'*((-len(binary))%4)
mesh=root/'quad.glb';mesh.write_bytes(struct.pack('<4sII',b'glTF',2,28+len(data)+len(binary))+struct.pack('<I4s',len(data),b'JSON')+data+struct.pack('<I4s',len(binary),b'BIN\0')+binary)
archive=root/'mesh.3tz'
call(['mesh-to-3tz','-i',str(mesh),'-o',str(archive),'--maxTriangles','1','--maxBytes','0',
      '--cartographicPositionDegrees','12.1','41.9','200',])
with zipfile.ZipFile(archive) as z:z.extractall(root/'mesh')
source=root/'imagery.tif';ds=gdal.GetDriverByName('GTiff').Create(str(source),32,32,3,gdal.GDT_Byte)
srs=osr.SpatialReference();srs.ImportFromEPSG(4326);ds.SetProjection(srs.ExportToWkt());ds.SetGeoTransform([12,.01,0,42,0,-.01])
for band,value in enumerate((255,100,20),1):ds.GetRasterBand(band).Fill(value)
ds=None
call(['raster','-i',str(source),'-o',str(root/'imagery'),'--maxZoom','10',])

source=root/'dem.tif';ds=gdal.GetDriverByName('GTiff').Create(str(source),32,32,1,gdal.GDT_Float32)
ds.SetProjection(srs.ExportToWkt());ds.SetGeoTransform([12,.01,0,42,0,-.01])
values=np.full((32,32),123.5,dtype='f4');values[12:20,12:20]=-32768
band=ds.GetRasterBand(1);band.WriteArray(values);band.SetNoDataValue(-32768);band=None;ds=None
call(['terrain','-i',str(source),'-o',str(root/'terrain'),'--maxZoom','9','--grid','65',
      '--heightOffset','10.25','--fillHeight','-999.125'])
source=root/'cloud.las';fixture(source,crs=CRS.from_epsg(32632))
run(types.SimpleNamespace(input=str(source),output=str(root/'cloud'),source_crs='header',
    max_points=32,chunk_points=31,height_offset=10,explicit=False))
generate(root/'annotations',quantize=True,meshopt_helper=cli_binary,batch=True,aggregate_points=True)
print('Serve the selected mesh, cloud, annotations, imagery and terrain directories with rusty-tiles preview.')
