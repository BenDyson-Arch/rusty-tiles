#!/usr/bin/env python3
"""Literal topology-source controls independent of every product encoder.

Root materializes these tiny GLBs and runs topology.py. No producer imports,
converter/consumer/build/process launches. Controls are source-oracle checks,
not a claim that the preservation-only codec validates rendered semantics.
"""
import argparse,copy,hashlib,json,struct
from pathlib import Path
def write_json(p,x):p.write_text(json.dumps(x,indent=2,ensure_ascii=False)+'\n')
def build(label,positions,indices,mode,polygon=None,fill=False):
 binary=b'';views=[];accessors=[]
 def view(raw,stride=None):
  nonlocal binary
  binary+=b'\0'*(-len(binary)%8);v={'buffer':0,'byteOffset':len(binary),'byteLength':len(raw)}
  if stride:v['byteStride']=stride
  binary+=raw;views.append(v);return len(views)-1
 def accessor(raw,component,kind,count,stride=None,ext=None):
  a={'bufferView':view(raw,stride),'componentType':component,'type':kind,'count':count}
  if ext:a.update(ext)
  accessors.append(a);return len(accessors)-1
 pos=accessor(struct.pack('<'+'f'*3*len(positions),*(x for p in positions for x in p)),5126,'VEC3',len(positions),
  ext={'min':[min(p[i] for p in positions) for i in range(3)],'max':[max(p[i] for p in positions) for i in range(3)]})
 fid=accessor(b'\0\0\0\0'*len(positions),5123,'SCALAR',len(positions),4)
 ix=accessor(struct.pack('<'+'I'*len(indices),*indices),5125,'SCALAR',len(indices))
 primitive={'mode':mode,'attributes':{'POSITION':pos,'_FEATURE_ID_0':fid},'indices':ix,
  'extensions':{'EXT_mesh_features':{'featureIds':[{'featureCount':1,'attribute':0,'propertyTable':0}]}}}
 used=['EXT_mesh_features','EXT_structural_metadata'];required=[]
 if 0xffffffff in indices:used+=['KHR_mesh_primitive_restart'];required+=['KHR_mesh_primitive_restart']
 if polygon:
  ext={'count':len(polygon['indicesOffsets'])}
  for k,v in polygon.items():ext[k]=accessor(struct.pack('<'+'I'*len(v),*v),5125,'SCALAR',len(v))
  primitive['extensions']['EXT_mesh_polygon']=ext;used+=['EXT_mesh_polygon']
 source_id=json.dumps(label,separators=(',',':')).encode();sv=view(source_id);so=view(struct.pack('<2I',0,len(source_id)))
 doc={'asset':{'version':'2.0'},'scene':0,'scenes':[{'nodes':[0]}],'nodes':[{'mesh':0}],
  'meshes':[{'primitives':[primitive]}],'buffers':[{'byteLength':len(binary)}],'bufferViews':views,'accessors':accessors,
  'extensionsUsed':used,'extensions':{'EXT_structural_metadata':{
    'schema':{'id':'independent-topology','classes':{'feature':{'properties':{'_source_id':{'type':'STRING'}}}}},
    'propertyTables':[{'class':'feature','count':1,'properties':{'_source_id':{'values':sv,'stringOffsets':so,'stringOffsetType':'UINT32'}}}]}}}
 if required:doc['extensionsRequired']=required
 if fill:
  used+=['KHR_materials_unlit'];primitive['material']=0
  doc['materials']=[{'doubleSided':True,'extensions':{'KHR_materials_unlit':{}},
   'pbrMetallicRoughness':{'baseColorFactor':[1,1,1,1],'metallicFactor':0,'roughnessFactor':1}}]
 return doc,binary
def envelope(doc,binary):
 j=json.dumps(doc,separators=(',',':'),ensure_ascii=False).encode();j+=b' '*(-(20+len(j))%8)
 b=binary+b'\0'*(-len(binary)%8)
 return struct.pack('<4sII',b'glTF',2,28+len(j)+len(b))+struct.pack('<I4s',len(j),b'JSON')+j+struct.pack('<I4s',len(b),b'BIN\0')+b
def feature(label,kind,coordinates):return {'type':'FeatureCollection','features':[{'type':'Feature','id':label,'properties':{},'geometry':{'type':kind,'coordinates':coordinates}}]}
def main():
 p=argparse.ArgumentParser();p.add_argument('--work',type=Path,required=True);a=p.parse_args();w=a.work.resolve()
 assert w.parent==Path('/tmp') and w.name.startswith('rusty-tiles-vector-p3-topology-controls-');w.mkdir(mode=0o700)
 restart=0xffffffff
 cases=[('points',[[0,0,0],[2,1,0]],[0,1],0,None,False,
   feature('points','MultiPoint',[[0,0,0],[2,1,0]])),
  ('lines',[[0,0,0],[2,1,0],[4,3,0],[6,4,0],[8,3,0]],[0,1,restart,2,3,4],3,None,False,
   feature('lines','MultiLineString',[[[0,0,0],[2,1,0]],[[4,3,0],[6,4,0],[8,3,0]]])),
  ('hole',[[0,0,0],[8,0,0],[8,8,0],[0,8,0],[2,2,0],[2,4,0],[4,4,0],[4,2,0]],
   [0,1,7,0,7,4,0,4,3,3,4,5,3,5,6,3,6,2,2,6,1,1,6,7],4,
   {'indicesOffsets':[0],'loopIndices':[0,1,2,3,restart,4,5,6,7],'loopIndicesOffsets':[0]},False,
   feature('hole','Polygon',[[[0,0,0],[8,0,0],[8,8,0],[0,8,0],[0,0,0]],
                           [[2,2,0],[2,4,0],[4,4,0],[4,2,0],[2,2,0]]])),
  ('multiple',[[0,0,0],[2,0,0],[2,2,0],[0,2,0],[4,0,0],[6,0,0],[6,2,0],[4,2,0]],
   [0,1,2,0,2,3,4,5,6,4,6,7],4,
   {'indicesOffsets':[0,6],'loopIndices':[0,1,2,3,restart,4,5,6,7],'loopIndicesOffsets':[0,5]},False,
   feature('multiple','MultiPolygon',[[[[0,0,0],[2,0,0],[2,2,0],[0,2,0],[0,0,0]]],
                                    [[[4,0,0],[6,0,0],[6,2,0],[4,2,0],[4,0,0]]]])),
  ('fill',[[0,0,0],[2,0,0],[2,2,0],[0,2,0]],[0,1,2,0,2,3],4,None,True,
   feature('fill','Polygon',[[[0,0,0],[2,0,0],[2,2,0],[0,2,0],[0,0,0]]]))]
 records=[];original={}
 def emit(name,doc,binary,source,expected):
  path=w/(name+'.glb');raw=envelope(doc,binary);path.write_bytes(raw);sp=w/(name+'.source.geojson');write_json(sp,source)
  records.append({'name':name,'input':str(path),'source':str(sp),'input_sha256':hashlib.sha256(raw).hexdigest(),
   'expected_source_oracle':expected,'scope':'independent literal source semantic control; not target/codec execution'})
 for label,pos,indices,mode,poly,fill,source in cases:
  doc,b=build(label,pos,indices,mode,poly,fill);original[label]=(doc,b,source);emit(label,doc,b,source,'pass')
 def changed(label,name,change):
  doc,b,source=original[label];doc=copy.deepcopy(doc);b=bytearray(b);change(doc,b);emit(name,doc,bytes(b),source,'fail')
 def ac_offset(d,ai):a=d['accessors'][ai];return d['bufferViews'][a['bufferView']]['byteOffset']+a.get('byteOffset',0)
 changed('lines','restart-joined-line',lambda d,b:struct.pack_into('<I',b,ac_offset(d,2)+8,0))
 changed('lines','restart-declaration-missing',lambda d,b:d.pop('extensionsRequired'))
 changed('points','wrong-feature-id',lambda d,b:struct.pack_into('<H',b,ac_offset(d,1),1))
 changed('points','wrong-table-association',lambda d,b:d['meshes'][0]['primitives'][0]['extensions']['EXT_mesh_features']['featureIds'][0].update(propertyTable=1))
 changed('lines','wrong-finite-position',lambda d,b:struct.pack_into('<f',b,ac_offset(d,0)+12,3.0))
 changed('points','wrong-position-minmax',lambda d,b:d['accessors'][0].update(max=[3,1,0]))
 changed('hole','wrong-loop-vertex-order',lambda d,b:struct.pack_into('<I',b,ac_offset(d,4)+4,3))
 changed('multiple','wrong-triangle-offset-association',lambda d,b:struct.pack_into('<I',b,ac_offset(d,3)+4,3))
 changed('hole','triangle-omission-degenerate',lambda d,b:struct.pack_into('<I',b,ac_offset(d,2)+4,0))
 write_json(w/'manifest.json',{'status':'AUTHORED_CONTROLS_ONLY_NO_TARGET_EXECUTION',
  'author_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'cases':records})
 print(json.dumps({'manifest':str(w/'manifest.json'),'positive':len(cases),'negative':len(records)-len(cases)}))
if __name__=='__main__':main()
