#!/usr/bin/env python3
"""Full decoded road/line fidelity audit for the opt-in Natural Earth fixture.
Usage: python audit_vector.py natural-earth-roads.gpkg native-output.3tz
Requires EPSG:4326 multiline geometry, zero ellipsoidal source height, all scalar
fields, and uncompressed native GLB output. Downloads and modifies nothing.
"""
import collections,json,pathlib,struct,sys,zipfile
import numpy as np
from osgeo import ogr
from pyproj import Transformer
if len(sys.argv)!=3:
    raise SystemExit(__doc__)
source,archive=sys.argv[1:]
ogr.UseExceptions()
ds=ogr.Open(source);layer=ds.GetLayer(0)
if layer.GetSpatialRef().GetAuthorityCode(None)!='4326':
    raise ValueError('this audit requires the EPSG:4326 Natural Earth roads fixture')
transform=Transformer.from_crs(4979,4978,always_xy=True)
records={}
for feature in layer:
    geometry=json.loads(feature.GetGeometryRef().ExportToJson())
    paths=geometry['coordinates'] if geometry['type']=='MultiLineString' else [geometry['coordinates']]
    coords=[np.array(transform.transform(*np.c_[np.array(p)[:,:2],np.zeros(len(p))].T)).T for p in paths]
    records[str(feature.GetFID())]=(feature.items(),coords,[np.zeros(len(p)-1,dtype='u4') for p in paths])
maximum=0.;segments=0;vertices=0;size=0;leaves=0
with zipfile.ZipFile(archive) as z:
    root=json.loads(z.read('tileset.json'))['root']
    def walk(node,parent):
        frame=parent@np.array(node.get('transform',np.eye(4).T.flatten())).reshape(4,4).T
        if node.get('children'):
            for child in node['children']:walk(child,frame)
            return
        global maximum,segments,vertices,size,leaves
        leaves+=1
        for content in node.get('contents',[node['content']] if 'content' in node else []):
            data=z.read(content['uri']);size=max(size,len(data));n=struct.unpack_from('<I',data,12)[0]
            doc=json.loads(data[20:20+n]);binary=data[28+n:]
            table=doc['extensions']['EXT_structural_metadata']['propertyTables'][0]
            schema=doc['extensions']['EXT_structural_metadata']['schema']['classes']['feature']['properties']
            def view(i,dtype):
                v=doc['bufferViews'][i];return np.frombuffer(binary,dtype,offset=v.get('byteOffset',0),count=v['byteLength']//np.dtype(dtype).itemsize)
            props=[{} for _ in range(table['count'])]
            for name,col in table['properties'].items():
                sc=schema[name]
                if sc['type']=='STRING':
                    raw=view(col['values'],'u1').tobytes();offs=view(col['stringOffsets'],'<u4');values=[raw[a:b].decode() for a,b in zip(offs[:-1],offs[1:])]
                elif sc['type']=='BOOLEAN':values=np.unpackbits(view(col['values'],'u1'),bitorder='little')[:table['count']].tolist()
                else:values=view(col['values'],{'INT64':'<i8','FLOAT64':'<f8'}[sc['componentType']]).tolist()
                for prop,value in zip(props,values):prop[name]=None if value==sc.get('noData',object()) else value
            bound=node['extras']['positionRoundingMetres']+1e-7
            for prim in doc['meshes'][0]['primitives']:
                assert prim['mode']==3
                ac=doc['accessors'][prim['attributes']['POSITION']];positions=view(ac['bufferView'],'<f4')[:ac['count']*3].reshape(-1,3)
                vertices=max(vertices,len(positions));positions=positions[:,[0,2,1]]*[1,-1,1];positions=(np.c_[positions,np.ones(len(positions))]@frame.T)[:,:3]
                ids=view(doc['accessors'][prim['attributes']['_FEATURE_ID_0']]['bufferView'],'<u4')
                indices=view(doc['accessors'][prim['indices']]['bufferView'],'<u4')
                for group in np.split(indices,np.flatnonzero(indices==0xffffffff)):
                    group=group[group!=0xffffffff]
                    if not len(group):continue
                    fid=int(ids[group[0]]);prop=props[fid];actual=positions[group]
                    expected,paths,seen=records[prop['_source_id']]
                    assert all(prop[k]==v for k,v in expected.items()), (prop,expected)
                    matched=False
                    for path,hits in zip(paths,seen):
                        candidates=np.flatnonzero(np.linalg.norm(path-actual[0],axis=1)<=bound)
                        for start in candidates:
                            if start+len(actual)>len(path):continue
                            error=np.linalg.norm(path[start:start+len(actual)]-actual,axis=1).max()
                            if error<=bound and not hits[start:start+len(actual)-1].any():
                                hits[start:start+len(actual)-1]+=1;maximum=max(maximum,float(error));matched=True;break
                        if matched:break
                    assert matched, prop['_source_id']
                    segments+=len(actual)-1
    walk(root,np.eye(4))
assert all((hits==1).all() for _,_,parts in records.values() for hits in parts)
print(json.dumps(dict(features=len(records),leafSegments=segments,leaves=leaves,allScalarPropertiesMatch=True,
 allSourceSegmentsExactlyOnce=True,maximumPositionErrorMetres=maximum,maximumGlbBytes=size,maximumGlbVertices=vertices),indent=2))
