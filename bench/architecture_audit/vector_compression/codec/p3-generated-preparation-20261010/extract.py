#!/usr/bin/env python3
"""Independent literal GLB framing/view/accessor/property association reader.

Python stdlib ZIP is transport only, CRC checked. No C1 or production oracle.
Compressed views require separately pinned consumer-decoded bytes. This reader
never repairs container alignment or claims source/world geometry accuracy.
"""
import argparse,base64,binascii,hashlib,json,struct,zipfile
from pathlib import Path
def sha(b):return hashlib.sha256(b).hexdigest()
def dump(p,d):p.write_text(json.dumps(d,indent=2,ensure_ascii=False)+'\n')
def raw_associations(raw):
 # Preserve authored value lexemes, independently of semantic JSON equality.
 text=raw.decode('utf8');decoder=json.JSONDecoder();spans={}
 def space(i):
  while i<len(text) and text[i] in ' \t\r\n':i+=1
  return i
 def walk(i,path,depth=0):
  assert depth<=64;i=space(i);_,end=decoder.raw_decode(text,i)
  if (path==('extensions','EXT_structural_metadata') or path[:1]==('nodes',)
      or (len(path)>=5 and path[0]=='meshes' and path[2]=='primitives' and path[4]=='extensions')
      or path==('materials',) or path==('extras',)):
   spans['/'.join(map(str,path))]=text[i:end]
  if text[i]=='{':
   p=space(i+1)
   while text[p]!='}':
    key,q=decoder.raw_decode(text,p);q=space(q);assert text[q]==':'
    p=space(walk(q+1,path+(key,),depth+1))
    if text[p]==',':p=space(p+1)
    else:assert text[p]=='}'
  elif text[i]=='[':
   p=space(i+1);index=0
   while text[p]!=']':
    p=space(walk(p,path+(index,),depth+1));index+=1
    if text[p]==',':p=space(p+1)
    else:assert text[p]==']'
  return end
 walk(0,());return spans
def accessor(doc,views,i):
 a=doc['accessors'][i];v=doc['bufferViews'][a['bufferView']];raw=views[a['bufferView']]
 fmt={5121:'B',5123:'H',5125:'I',5126:'f'}[a['componentType']]
 n={'SCALAR':1,'VEC3':3}[a['type']];width=struct.calcsize(fmt)*n
 stride=v.get('byteStride',width);off=a.get('byteOffset',0)
 assert off+(a['count']-1)*stride+width<=len(raw)
 result=[list(struct.unpack_from('<'+fmt*n,raw,off+j*stride)) for j in range(a['count'])]
 if n==1:result=[x[0] for x in result]
 return result
def semantic(doc,views):
 meta=doc['extensions']['EXT_structural_metadata'];rows=[];properties=[]
 for table in meta['propertyTables']:
  defs=meta['schema']['classes'][table['class']]['properties'];count=table['count'];cols={}
  for key,p in table['properties'].items():
   definition=defs[key];raw=views[p['values']]
   if definition['type']=='BOOLEAN':
    assert len(raw)==(count+7)//8
    values=[bool(raw[i//8]&(1<<(i%8))) for i in range(count)]
    assert not count%8 or raw[-1]>>(count%8)==0
   elif definition['type']=='STRING':
    assert p.get('stringOffsetType','UINT32')=='UINT32'
    offs=views[p['stringOffsets']];assert len(offs)==4*(count+1)
    offsets=list(struct.unpack('<'+'I'*(count+1),offs));assert offsets[0]==0 and offsets[-1]==len(raw)
    assert offsets==sorted(offsets)
    values=[raw[a:b].decode('utf8') for a,b in zip(offsets,offsets[1:])]
   else:
    fmt={'INT64':'q','FLOAT64':'d'}[definition['componentType']]
    assert len(raw)==count*8;values=list(struct.unpack('<'+fmt*count,raw))
   decoded=[None if x==definition.get('noData',object()) else x for x in values]
   cols[key]=decoded;properties.append({'property':key,'definition':definition,'association':p,
    'raw_values_sha256':sha(raw),'raw_values_bytes':len(raw),'physical_values':values,'logical_values':decoded})
  rows.extend({k:v[i] for k,v in cols.items()} for i in range(count))
 return rows,properties
def associations(doc):
 # These are associations to preserve, including exact integers; exclude only
 # placement/meshopt bookkeeping that the proposed codec actually owns.
 d=json.loads(json.dumps(doc));d.pop('buffers',None)
 for v in d.get('bufferViews',[]):
  v.pop('buffer',None);v.pop('byteOffset',None)
  e=v.get('extensions',{});e.pop('EXT_meshopt_compression',None)
  if not e:v.pop('extensions',None)
 for k in ['extensionsUsed','extensionsRequired']:
  if k in d:
   d[k]=[x for x in d[k] if x!='EXT_meshopt_compression']
   if not d[k]:d.pop(k)
 return d
def inspect(raw,folder,decoded,expected):
 wrapper=None
 if raw[:4]==b'b3dm':
  version,total,fj,fb,bj,bb=struct.unpack_from('<6I',raw,4);assert version==1 and total==len(raw)
  at=28+fj+fb+bj+bb;wrapper={'feature_json_raw_b64':base64.b64encode(raw[28:28+fj]).decode(),
   'feature_binary_bytes':fb,'batch_json_bytes':bj,'batch_binary_bytes':bb,'inner_offset':at}
  feature=json.loads(raw[28:28+fj]);assert feature.get('BATCH_LENGTH')==0 and 'RTC_CENTER' not in feature and fb==bj==bb==0
  raw=raw[at:]
 assert struct.unpack_from('<4sII',raw)==(b'glTF',2,len(raw))
 jl,jtype=struct.unpack_from('<I4s',raw,12);assert jtype==b'JSON' and jl%4==0
 jraw=raw[20:20+jl];doc=json.loads(jraw);ba=28+jl
 bl,btype=struct.unpack_from('<I4s',raw,20+jl);assert btype==b'BIN\0' and ba+bl==len(raw) and bl%4==0
 binary=raw[ba:];folder.mkdir();(folder/'source.glb').write_bytes(raw);(folder/'document.raw.json').write_bytes(jraw)
 (folder/'binary.bin').write_bytes(binary);dump(folder/'document.json',doc)
 framed={'json_chunk_length':jl,'json_chunk_end_absolute':20+jl,'bin_start_absolute':ba,
  'bin_chunk_length':bl,'end_absolute':len(raw),'bin_slack_vs_buffer0':bl-doc['buffers'][0]['byteLength'],
  'absolute_metadata8_json_boundary':(20+jl)%8==0,'absolute_metadata8_bin_start':ba%8==0,
  'absolute_metadata8_end':len(raw)%8==0,'source_sha256':sha(raw),'wrapper':wrapper}
 views=[];view_records=[];missing=[]
 for i,v in enumerate(doc.get('bufferViews',[])):
  ext=v.get('extensions',{}).get('EXT_meshopt_compression');target=folder/('view-%04d.bin'%i)
  if ext:
   assert ext['buffer']==0 and ext['mode']=='ATTRIBUTES' and ext.get('filter','NONE')=='NONE'
   encoded=binary[ext.get('byteOffset',0):ext.get('byteOffset',0)+ext['byteLength']]
   assert len(encoded)==ext['byteLength'];(folder/('encoded-%04d.bin'%i)).write_bytes(encoded)
   if decoded and (decoded/target.name).is_file():value=(decoded/target.name).read_bytes();assert len(value)==v['byteLength']
   else:value=None;missing.append(i)
  else:
   assert v['buffer']==0;start=v.get('byteOffset',0);value=binary[start:start+v['byteLength']];assert len(value)==v['byteLength']
  if value is not None:target.write_bytes(value)
  views.append(value);view_records.append({'index':i,'association':v,'logical_bytes':v['byteLength'],
   'decoded_sha256':sha(value) if value is not None else None,'compressed':ext is not None})
 report={'framing_observation':framed,'views':view_records,'associations':associations(doc),
         'raw_association_values':raw_associations(jraw),'missing_consumer_decoded_views':missing,
         'status':'decoded-semantic-checks-pending' if missing else 'extracted-not-complete-format-certification'}
 if not missing:
  rows,properties=semantic(doc,views);report.update(rows=rows,properties=properties)
  for prop in properties:
   kind=expected['property_kinds'].get(prop['property'])
   if kind:
    definition=prop['definition'];actual=definition['type'] if definition['type'] in ['BOOLEAN','STRING'] else definition['componentType']
    assert actual==kind,('authored schema kind changed',prop['property'],actual,kind)
  for row in rows:
   sid=row['_source_id'];assert sid in expected['source_rows'],('unknown source identity',sid)
   want=expected['source_rows'][sid]
   for key,value in want.items():
    got=row[key]
    if key in expected['list_properties_are_JSON_strings']:assert json.loads(got)==value,(key,got,value)
    else:assert got==value,(sid,key,got,value)
  report['authored_metadata_rows_checked']=len(rows)
  primitives=[]
  for mesh in doc.get('meshes',[]):
   for p in mesh['primitives']:
    position=doc['accessors'][p['attributes']['POSITION']];pv=doc['bufferViews'][position['bufferView']]
    ids=accessor(doc,views,p['attributes']['_FEATURE_ID_0']);assert all(int(x)==x and 0<=x<len(rows) for x in ids)
    ids_ac=doc['accessors'][p['attributes']['_FEATURE_ID_0']];iv=doc['bufferViews'][ids_ac['bufferView']]
    assert ids_ac['componentType']==(5126 if expected['expected_point_id_row_count'] else 5123)
    if expected['expected_point_id_row_count']:
     assert len(rows)==len(ids)==expected['expected_point_id_row_count'] and set(ids)==set(range(len(rows)))
    if ids_ac['componentType']==5123:
     assert iv['byteStride']==4;assert all(views[ids_ac['bufferView']][i*4+2:i*4+4]==b'\0\0' for i in range(len(ids)))
    if position['componentType']==5123:
     assert position['normalized'] and pv['byteStride']==8
     assert all(views[position['bufferView']][i*8+6:i*8+8]==b'\0\0' for i in range(position['count']))
    else:assert position['componentType']==5126 and pv.get('byteStride',12)==12
    f=p['extensions']['EXT_mesh_features']['featureIds'][0];assert f['featureCount']==len(rows) and f['propertyTable']==0 and f['attribute']==0
    indices=accessor(doc,views,p['indices']);restart=0xffffffff
    assert all(x==restart or x<position['count'] for x in indices)
    if restart in indices:
     assert p['mode']==3 and 'KHR_mesh_primitive_restart' in doc.get('extensionsRequired',[])
    polygon=p.get('extensions',{}).get('EXT_mesh_polygon');streams={}
    if polygon:
     assert p['mode']==4
     for key in ['indicesOffsets','loopIndices','loopIndicesOffsets']:streams[key]=accessor(doc,views,polygon[key])
     assert len(streams['indicesOffsets'])==len(streams['loopIndicesOffsets'])==polygon['count']
     assert all(x==restart or x<position['count'] for x in streams['loopIndices'])
    primitives.append({'mode':p['mode'],'position_accessor':position,'id_accessor':ids_ac,
     'ids_sha256':sha(json.dumps(ids,separators=(',',':')).encode()),'id_min':min(ids),'id_max':max(ids),
     'indices':indices,'polygon':polygon,'polygon_streams':streams,
     'material':doc.get('materials',[None])[p.get('material',0)] if 'material' in p else None})
  report['primitives']=primitives
 dump(folder/'extraction.json',report);return report
def main():
 p=argparse.ArgumentParser();p.add_argument('--archive',type=Path,required=True);p.add_argument('--out',type=Path,required=True)
 p.add_argument('--expected',type=Path,required=True);p.add_argument('--decoded-root',type=Path);a=p.parse_args();a.out.mkdir()
 expected=json.loads(a.expected.read_bytes());raw=a.archive.read_bytes();records=[];seen=set();modes=set();has_restart=False;has_polygon=False;pending=False
 with zipfile.ZipFile(a.archive) as z:
  infos=z.infolist();assert len(infos)==len({x.filename for x in infos})
  for n,info in enumerate(infos):
   data=z.read(info);assert binascii.crc32(data)&0xffffffff==info.CRC
   if data[:4] not in [b'glTF',b'b3dm']:continue
   folder=a.out/('payload-%04d'%n);decoded=a.decoded_root/folder.name if a.decoded_root else None
   report=inspect(data,folder,decoded,expected);records.append({'member':info.filename,'member_sha256':sha(data),'folder':folder.name,
     'framing':report['framing_observation'],'metadata_checked':report.get('authored_metadata_rows_checked',0)})
   pending|=bool(report['missing_consumer_decoded_views'])
   seen.update(row['_source_id'] for row in report.get('rows',[]))
   for primitive in report.get('primitives',[]):
    modes.add(primitive['mode']);has_restart|=0xffffffff in primitive['indices'];has_polygon|=primitive['polygon'] is not None
 assert records
 if expected['requires_fill_b3dm']:assert any(x['framing']['wrapper'] is not None for x in records),'actual fill branch not reached'
 if not pending:
  assert seen==set(expected['source_rows']),('authored source identity coverage changed',len(seen),len(expected['source_rows']))
  assert modes==set(expected['expected_primitive_modes']),('expected current generated modes not reached',modes)
  assert not expected['requires_line_restart'] or has_restart,'genuine restart branch not reached'
  assert not expected['requires_polygon_extension'] or has_polygon,'genuine polygon extension branch not reached'
 dump(a.out/'receipt.json',{'extractor_sha256':sha(Path(__file__).read_bytes()),'archive_sha256':sha(raw),
  'expected_sha256':sha(a.expected.read_bytes()),'scope':'raw bytes and source-typed metadata/association checks; geometry coordinates and quantization accuracy not certified',
  'complete_source_identity_and_mode_coverage_checked':not pending,'records':records})
 print(json.dumps({'payloads':len(records),'out':str(a.out)}))
if __name__=='__main__':main()
