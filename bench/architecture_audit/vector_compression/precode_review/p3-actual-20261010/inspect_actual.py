"""Fresh reviewer inspection of frozen artifacts. No product/author imports or launches."""
import hashlib,json,math,posixpath,struct,zipfile,tarfile
from fractions import Fraction
from pathlib import Path

OUT=Path('/tmp/rusty-tiles-vector-p3-actual-review')
SMALL=Path('/tmp/rusty-tiles-vector-p3-inputs-small')
LARGE=Path('/tmp/rusty-tiles-vector-p3-large-ids-revision-root')
OBS=Path('/tmp/rusty-tiles-vector-p3-observations')
pins={}
def sha(b):return hashlib.sha256(b).hexdigest()
def read(p):
 p=Path(p);b=p.read_bytes();pins[str(p)]={'sha256':sha(b),'bytes':len(b)};return b
def jread(p):return json.loads(read(p))
def unpack(raw):
 wrapper=None
 if raw[:4]==b'b3dm':
  fields=struct.unpack_from('<4s6I',raw)
  magic,version,total,fj,fb,bj,bb=fields
  assert version==1 and total==len(raw) and fb==bj==bb==0
  offset=28+fj;feature=raw[28:offset]
  assert json.loads(feature)=={'BATCH_LENGTH':0} and offset%8==0
  wrapper={'magic':magic.decode(),'version':version,'feature_json_bytes':fj,'feature_binary_bytes':fb,
   'batch_json_bytes':bj,'batch_binary_bytes':bb,'feature_json_sha256':sha(feature),
   'feature_json_hex':feature.hex(),'inner_offset':offset}
  raw=raw[offset:]
 assert struct.unpack_from('<4sII',raw)==(b'glTF',2,len(raw))
 jl,jt=struct.unpack_from('<I4s',raw,12);assert jt==b'JSON' and jl%4==0
 bl,bt=struct.unpack_from('<I4s',raw,20+jl);assert bt==b'BIN\0' and bl%4==0 and 28+jl+bl==len(raw)
 return json.loads(raw[20:20+jl]),raw[28+jl:],wrapper,{'json_length':jl,'json_absolute_end':20+jl,'bin_start':28+jl,'glb_end':len(raw),
  'metadata8_all_three':all(x%8==0 for x in [20+jl,28+jl,len(raw)])}
def view(doc,binary,i):
 v=doc['bufferViews'][i];s=v.get('byteOffset',0);return binary[s:s+v['byteLength']]
def access(doc,views,i):
 a=doc['accessors'][i];fmt={5121:'B',5123:'H',5125:'I',5126:'f'}[a['componentType']]
 width={'SCALAR':1,'VEC3':3}[a['type']];stride=doc['bufferViews'][a['bufferView']].get('byteStride',struct.calcsize(fmt)*width)
 return [struct.unpack_from('<'+fmt*width,views[a['bufferView']],a.get('byteOffset',0)+stride*n) for n in range(a['count'])]
def selected_lexemes(raw):
 text=raw.decode('utf8');decoder=json.JSONDecoder();out={}
 def skip(i):
  while i<len(text) and text[i].isspace():i+=1
  return i
 def visit(i,path):
  i=skip(i);value,end=decoder.raw_decode(text,i)
  selected=(path in [('nodes',),('materials',),('extras',),('extensions','EXT_structural_metadata')]
   or len(path)==5 and path[0]=='meshes' and path[2]=='primitives' and path[4]=='extensions')
  if selected:out['/'.join(map(str,path))]=text[i:end]
  if isinstance(value,dict):
   cursor=skip(i+1)
   for key in value:
    actual,after=decoder.raw_decode(text,cursor);assert actual==key;after=skip(after);assert text[after]==':'
    cursor=skip(visit(after+1,path+(key,)));cursor=skip(cursor+1) if text[cursor]==',' else cursor
  elif isinstance(value,list):
   cursor=skip(i+1)
   for index in range(len(value)):
    cursor=skip(visit(cursor,path+(index,)));cursor=skip(cursor+1) if text[cursor]==',' else cursor
  return end
 visit(0,());return out
def metadata(doc,views):
 m=doc['extensions']['EXT_structural_metadata'];assert len(m['propertyTables'])==1
 t=m['propertyTables'][0];defs=m['schema']['classes'][t['class']]['properties'];cols={};details={}
 for name,p in t['properties'].items():
  d=defs[name];raw=views[p['values']];n=t['count']
  if d['type']=='STRING':
   assert p.get('stringOffsetType','UINT32')=='UINT32'
   offsets=struct.unpack('<'+'I'*(n+1),views[p['stringOffsets']]);assert offsets[0]==0 and offsets[-1]==len(raw) and list(offsets)==sorted(offsets)
   values=[raw[a:b].decode('utf8') for a,b in zip(offsets,offsets[1:])]
  elif d['type']=='BOOLEAN':
   assert len(raw)==(n+7)//8 and (not n%8 or raw[-1]>>(n%8)==0)
   values=[bool(raw[i//8]&(1<<(i%8))) for i in range(n)]
  else:
   fmt={'INT64':'q','FLOAT64':'d'}[d['componentType']];assert len(raw)==n*8
   values=list(struct.unpack('<'+fmt*n,raw))
  cols[name]=[None if 'noData' in d and x==d['noData'] else x for x in values]
  details[name]={'physical_first':values[0] if values else None,'type':d['type'],'component':d.get('componentType')}
 return [{k:v[i] for k,v in cols.items()} for i in range(t['count'])],details
def addresses(files):
 slots={};facts={}
 def tile(t,member,at):
  facts[at]={k:v for k,v in t.items() if k not in ['content','contents','children','extras']}
  if 'extras' in t:facts[at]['extras']={k:v for k,v in t['extras'].items() if k!='encodedBytes'}
  entries=[('content',t['content'])] if 'content' in t else [('contents/'+str(i),c) for i,c in enumerate(t.get('contents',[]))]
  for suffix,c in entries:
   key=at+'/'+suffix;facts[key]={k:v for k,v in c.items() if k!='uri'}
   name=posixpath.normpath(posixpath.join(posixpath.dirname(member),c['uri']))
   if files[name][:4] in [b'glTF',b'b3dm']:slots[key]=name
   else:document(name,key+'/external')
  for i,c in enumerate(t.get('children',[])):tile(c,member,at+'/children/'+str(i))
 def document(member,at):
  d=json.loads(files[member]);facts[at]={'geometricError':d.get('geometricError'),'gltfUpAxis':d.get('asset',{}).get('gltfUpAxis'),'version':d.get('asset',{}).get('version'),'extensionsUsed':d.get('extensionsUsed',[])}
  tile(d['root'],member,at+'/root')
 document('tileset.json','tileset');return slots,facts
def profile(doc,views):
 assert doc['scene']==0 and doc['scenes']==[{'nodes':[0]}] and len(doc['nodes'])==len(doc['meshes'])==1
 node=doc['nodes'][0];assert node['mesh']==0 and not any(k in node for k in ['matrix','rotation','children'])
 assert all(math.isfinite(x) and x>0 for x in node.get('scale',[1,1,1]))
 assert all(math.isfinite(x) for x in node.get('translation',[0,0,0]))
 assert not any('sparse' in a for a in doc.get('accessors',[]))
 rows,_=metadata(doc,views);sids=[r['_source_id'] for r in rows];assert len(sids)==len(set(sids))
 for p in doc['meshes'][0]['primitives']:
  xs=access(doc,views,p['attributes']['POSITION']);assert all(math.isfinite(x) for v in xs for x in v)
  a=doc['accessors'][p['attributes']['POSITION']]
  pv=doc['bufferViews'][a['bufferView']]
  if a['componentType']==5123:
   assert a['normalized'] and pv['byteStride']==8
   assert all(views[a['bufferView']][i*8+6:i*8+8]==b'\0\0' for i in range(a['count']))
  else:assert a['componentType']==5126 and not a.get('normalized',False) and pv.get('byteStride',12)==12
  for k,op in [('min',min),('max',max)]:
   if k in a:assert a[k]==[op(v[i] for v in xs) for i in range(3)]
  ids=[v[0] for v in access(doc,views,p['attributes']['_FEATURE_ID_0'])]
  ia=doc['accessors'][p['attributes']['_FEATURE_ID_0']];iv=doc['bufferViews'][ia['bufferView']]
  assert ia['type']=='SCALAR' and not ia.get('normalized',False) and iv.get('byteStride',4)==4
  if ia['componentType']==5123:assert all(views[ia['bufferView']][i*4+2:i*4+4]==b'\0\0' for i in range(len(ids)))
  else:assert ia['componentType']==5126
  assert all(int(i)==i and 0<=i<len(rows) for i in ids)
  f=p['extensions']['EXT_mesh_features']['featureIds'][0];assert f=={'featureCount':len(rows),'attribute':0,'propertyTable':0}
 return rows
def rational(x):return Fraction(x)
def area_vector(v):
 q=sum(x*x for x in v);a=math.isqrt(q.numerator);b=math.isqrt(q.denominator)
 assert a*a==q.numerator and b*b==q.denominator
 return Fraction(a,2*b)
def triangle(a,b,c):
 u=[b[i]-a[i] for i in range(3)];v=[c[i]-a[i] for i in range(3)]
 return area_vector([u[1]*v[2]-u[2]*v[1],u[2]*v[0]-u[0]*v[2],u[0]*v[1]-u[1]*v[0]])
def ring_area(ps):
 v=[Fraction(0)]*3
 for a,b in zip(ps,ps[1:]+ps[:1]):
  v[0]+=a[1]*b[2]-a[2]*b[1];v[1]+=a[2]*b[0]-a[0]*b[2];v[2]+=a[0]*b[1]-a[1]*b[0]
 return area_vector(v)
def segments(values):
 out=[[]]
 for v in values:
  if v==0xffffffff:out.append([])
  else:out[-1].append(v)
 # A polygon range ending at the next polygon offset may include the separator.
 if not out[-1]:out.pop()
 assert all(out);return out
def topology(payload,source,quantized):
 observed={};want={}
 for f in source['features']:
  sid=json.dumps(f['id'],ensure_ascii=False,separators=(',',':'));g=f['geometry'];c=g['coordinates'];k=g['type']
  expected={'points':0,'line_lengths':[],'polygon_sizes':[],'polygon_area':Fraction(0)}
  if k in ['Point','MultiPoint']:expected['points']=1 if k=='Point' else len(c)
  elif k in ['LineString','MultiLineString']:expected['line_lengths']=sorted(len(x) for x in ([c] if k=='LineString' else c))
  else:
   for rings in ([c] if k=='Polygon' else c):
    expected['polygon_sizes'].append([len(r)-1 for r in rings])
    areas=[ring_area([tuple(map(rational,p)) for p in r[:-1]]) for r in rings]
    expected['polygon_area']+=areas[0]-sum(areas[1:])
  want[sid]=expected;observed[sid]={'points':0,'line_lengths':[],'polygon_sizes':[],'triangle_area':Fraction(0)}
 for a in payload.values():
  d=a['doc'];vs=a['views'];node=d['nodes'][0];sids=[r['_source_id'] for r in a['rows']]
  for p in d['meshes'][0]['primitives']:
   pos=access(d,vs,p['attributes']['POSITION']);ac=d['accessors'][p['attributes']['POSITION']];den=65535 if ac.get('normalized') else 1
   ps=[tuple(rational(v[i])/den*rational(node.get('scale',[1,1,1])[i])+rational(node.get('translation',[0,0,0])[i]) for i in range(3)) for v in pos]
   ids=[int(x[0]) for x in access(d,vs,p['attributes']['_FEATURE_ID_0'])];ix=[x[0] for x in access(d,vs,p['indices'])]
   def owner(indices):
    owners={sids[ids[i]] for i in indices};assert len(owners)==1;return next(iter(owners))
   mode=p.get('mode',4)
   if mode==0:
    for i in ix:observed[owner([i])]['points']+=1
   elif mode==3:
    if 0xffffffff in ix:assert 'KHR_mesh_primitive_restart' in d.get('extensionsRequired',[])
    for line in segments(ix):observed[owner(line)]['line_lengths'].append(len(line))
   else:
    assert mode==4 and len(ix)%3==0
    for i in range(0,len(ix),3):observed[owner(ix[i:i+3])]['triangle_area']+=triangle(*(ps[n] for n in ix[i:i+3]))
    polygon=p.get('extensions',{}).get('EXT_mesh_polygon')
    if polygon:
     loops=[x[0] for x in access(d,vs,polygon['loopIndices'])];lo=[x[0] for x in access(d,vs,polygon['loopIndicesOffsets'])];io=[x[0] for x in access(d,vs,polygon['indicesOffsets'])]
     for i,start in enumerate(lo):
      rings=segments(loops[start:lo[i+1] if i+1<len(lo) else len(loops)]);sid=owner([v for r in rings for v in r]);observed[sid]['polygon_sizes'].append([len(r) for r in rings])
      tri=ix[io[i]:io[i+1] if i+1<len(io) else len(ix)];assert len(tri)%3==0 and owner(tri)==sid
      tri_area=sum(triangle(*(ps[v] for v in tri[n:n+3])) for n in range(0,len(tri),3))
      loop_area=ring_area([ps[v] for v in rings[0]])-sum(ring_area([ps[v] for v in r]) for r in rings[1:]);assert tri_area==loop_area
 for sid,w in want.items():
  o=observed[sid];assert w['points']==o['points']
  if w['line_lengths']:assert w['line_lengths']==sorted(o['line_lengths'])
  if o['polygon_sizes']:assert sorted(w['polygon_sizes'])==sorted(o['polygon_sizes'])
  if not quantized:assert w['polygon_area']==o['triangle_area']
 return {sid:{**o,'triangle_area':str(o['triangle_area'])} for sid,o in observed.items()}
def artifact(archive,folder):
 blob=read(archive)
 with zipfile.ZipFile(archive) as z:
  assert len(z.namelist())==len(set(z.namelist()));files={n:z.read(n) for n in z.namelist()}
 receipt=jread(folder/'receipt.json');assert receipt['archive_sha256']==sha(blob)
 payload={}
 for r in receipt['records']:
  p=folder/r['folder'];raw=files[r['member']];assert r['member_sha256']==sha(raw)
  d,b,w,f=unpack(raw);assert read(p/'source.glb')== (raw[w['inner_offset']:] if w else raw)
  views=[]
  for i,v in enumerate(d['bufferViews']):
   data=read(p/('view-%04d.bin'%i));assert len(data)==v['byteLength']
   if 'EXT_meshopt_compression' not in v.get('extensions',{}):assert data==view(d,b,i)
   views.append(data)
  inner=raw[w['inner_offset']:] if w else raw;jl=struct.unpack_from('<I',inner,12)[0]
  assert read(p/'document.raw.json')==inner[20:20+jl]
  payload[r['member']]={'doc':d,'views':views,'wrapper':w,'framing':f,'rows':profile(d,views),'raw_lexemes':selected_lexemes(inner[20:20+jl])}
 return files,payload
def association(doc):
 # Codec-owned relocation and EXT_meshopt bookkeeping only for this finite generated profile.
 d=json.loads(json.dumps(doc));d.pop('buffers')
 for v in d['bufferViews']:
  v.pop('buffer',None);v.pop('byteOffset',None)
  if 'extensions' in v:
   v['extensions'].pop('EXT_meshopt_compression',None)
   if not v['extensions']:v.pop('extensions')
 for k in ['extensionsUsed','extensionsRequired']:
  if k in d:
   d[k]=[s for s in d[k] if s!='EXT_meshopt_compression']
   if not d[k]:d.pop(k)
 return d
def pair(root,label,mode,variant):
 left=root/(label+'-'+mode+'-'+variant+'.3tz');right=root/(label+'-'+mode+'-'+('meshopt' if variant=='raw' else variant+'-meshopt')+'.3tz')
 lf,lp=artifact(left,left.with_suffix('.extracted'));rf,rp=artifact(right,right.with_suffix('.checked'))
 ls,la=addresses(lf);rs,ra=addresses(rf);assert la==ra and ls.keys()==rs.keys()
 assert set(ls.values())==set(lp) and set(rs.values())==set(rp)
 counts=0;wrappers=0;framing=[]
 for at in ls:
  a=lp[ls[at]];b=rp[rs[at]]
  assert association(a['doc'])==association(b['doc']) and a['views']==b['views'] and a['rows']==b['rows']
  assert a['raw_lexemes']==b['raw_lexemes']
  assert a['wrapper']==b['wrapper']
  counts+=len(a['views']);wrappers+=a['wrapper'] is not None
  framing.append({'address':at,'raw':a['framing'],'meshopt':b['framing'],'wrapper':a['wrapper']})
 expected=jread((Path('/tmp/rusty-tiles-vector-p3-inputs-large') if root==LARGE else root)/(label+'.expected.json'))
 covered=set()
 for payload in lp.values():
  meta=payload['doc']['extensions']['EXT_structural_metadata'];table=meta['propertyTables'][0];definitions=meta['schema']['classes'][table['class']]['properties']
  for name,kind in expected['property_kinds'].items():
   definition=definitions[name];actual=definition['type'] if definition['type'] in ['STRING','BOOLEAN'] else definition['componentType'];assert actual==kind
  for row in payload['rows']:
   sid=row['_source_id'];covered.add(sid);want=expected['source_rows'][sid]
   for k,v in want.items():assert (json.loads(row[k]) if k in expected['list_properties_are_JSON_strings'] else row[k])==v
 assert covered==set(expected['source_rows'])
 large=None
 if label.startswith('large'):
  assert len(lp)==1
  a=next(iter(lp.values()));p=a['doc']['meshes'][0]['primitives'];assert len(p)==1 and p[0]['mode']==0
  ac=a['doc']['accessors'][p[0]['attributes']['_FEATURE_ID_0']];ids=[x[0] for x in access(a['doc'],a['views'],p[0]['attributes']['_FEATURE_ID_0'])]
  assert ac['componentType']==5126 and ac['type']=='SCALAR' and len(ids)==len(a['rows'])==65537
  assert sorted(ids)==list(range(65537)) and covered=={str(i) for i in range(65537)}
  large={'rows':len(ids),'id_component':5126,'min':min(ids),'max':max(ids),'source_ids_complete':True,'one_leaf_point_primitive':True}
 consumer=jread(right.with_suffix('.extracted')/'consumer-receipt.json')
 assert consumer['decoderSha']=='fabafaaa29cbb1cfa367f20e896d7f6e95b86e575e2ea86ddb6f8cca1a87152d'
 assert sha(read(consumer['decoderPath']))==consumer['decoderSha']
 assert consumer['scriptSha256']=='09c996686f53935d2ecd9e049a795eef261ce574c004127268a064ca81fd6961'
 for r in consumer['records']:
  assert sha(read(right.with_suffix('.extracted')/r['payload']/('view-%04d.bin'%r['view'])))==r['sha256']
 topo=None
 if label in ['geometry','fragmented-fill']:
  source=jread(root/(label+'.geojson'));topo={'raw':topology(lp,source,variant=='quantized'),'decoded':topology(rp,source,variant=='quantized')}
 return {'label':label,'mode':mode,'variant':variant,'slots':len(ls),'views':counts,'fill_wrapper_slots':wrappers,'framing_and_wrapper':framing,'large_id':large,'independent_topology_metrics':topo}
def typed_controls():
 root=Path('/tmp/rusty-tiles-vector-p3-typed-controls-root');receipt=jread(root/'receipt.json')
 baseline=read(root/'baseline/source.glb');d,b,_,_=unpack(baseline);assert sha(baseline)==receipt['source_input_sha256']
 properties=d['extensions']['EXT_structural_metadata']['propertyTables'][0]['properties']
 records=[]
 for r in receipt['records']:
  name=r['control'];mut=read(root/(name+'.glb'));assert sha(mut)==r['input_sha256'];md,mb,_,_=unpack(mut)
  diff=[{'bin_offset':i,'before':a,'after':z} for i,(a,z) in enumerate(zip(b,mb)) if a!=z];assert len(b)==len(mb)
  detail={}
  if name!='wrong_property_table':assert md==d and len(diff)==1
  if name=='boolean_unused_tail_bit':
   raw=view(md,mb,properties['flag']['values']);assert raw==bytes([85,129])
   detail={'row_count':9,'tail_bits':raw[-1]>>1,'logical_boolean_values_unchanged':True,'owning_extractor_line':57}
  elif name=='feature_id_padding_bit':
   p=d['meshes'][0]['primitives'][0];a=d['accessors'][p['attributes']['_FEATURE_ID_0']];v=view(d,b,a['bufferView']);mv=view(md,mb,a['bufferView'])
   assert struct.unpack_from('<H',v)==struct.unpack_from('<H',mv)==(0,) and v[2:4]==b'\0\0' and mv[2:4]==b'\1\0'
   detail={'first_u16_id_unchanged':0,'first_padding_before_hex':v[2:4].hex(),'first_padding_after_hex':mv[2:4].hex(),'owning_extractor_line':145}
  elif name=='wrong_property_table':
   assert not diff;expect=json.loads(json.dumps(d));expect['meshes'][0]['primitives'][0]['extensions']['EXT_mesh_features']['featureIds'][0]['propertyTable']=1;assert md==expect
   detail={'table_inventory_count':1,'before_table':0,'after_table':1,'only_semantic_doc_change':True,'owning_extractor_line':150}
  elif name=='int64_low_bit':
   i=properties['integer']['values'];a=struct.unpack_from('<q',view(d,b,i))[0];z=struct.unpack_from('<q',view(md,mb,i))[0];assert (a,z)==(-9223372036854775808,-9223372036854775807);detail={'before':a,'after':z}
  elif name=='utf8_string_offset_shift':
   i=properties['text']['stringOffsets'];a=struct.unpack_from('<I',view(d,b,i),4)[0];z=struct.unpack_from('<I',view(md,mb,i),4)[0];assert (a,z)==(2,3)
   data=view(md,mb,properties['text']['values']);assert data[:2].decode('utf8')=='é'
   try:data[:3].decode('utf8')
   except UnicodeDecodeError:detail={'first_offset_before':a,'first_offset_after':z,'mutation_cuts_following_utf8_scalar':True}
   else:raise AssertionError('UTF8 mutation not sensitive')
  elif name=='list_large_integer_rounding':
   i=properties['list']['values'];old=view(d,b,i);new=view(md,mb,i);assert old.replace(b'9007199254740993',b'9007199254740992',1)==new;detail={'exact_integer_before':9007199254740993,'exact_integer_after':9007199254740992}
  elif name=='source_identity_changed':
   i=properties['_source_id']['values'];old=view(d,b,i);new=view(md,mb,i);assert old.replace(b'p0',b'q0',1)==new;detail={'identity_before':'p0','identity_after':'q0'}
  else:raise AssertionError(name)
  records.append({'control':name,'binary_diffs':diff,'independent_cause':detail,'recorded_cause':r['cause']})
 return records
def main():
 pairs=[]
 for mode in ['portable','native']:
  for label,variant in [('typed-metadata','raw'),('geometry','quantized'),('fragmented-fill','raw')]:pairs.append(pair(SMALL,label,mode,variant))
  pairs.append(pair(LARGE,'large-ids-65537',mode,'raw'))
 typed=typed_controls()
 literal=[]
 control_root=Path('/tmp/rusty-tiles-vector-p3-topology-controls-root')
 for name in ['points','lines','hole','multiple','fill']:
  d,b,w,f=unpack(read(control_root/(name+'.glb')));vs=[view(d,b,i) for i in range(len(d['bufferViews']))]
  a={'doc':d,'views':vs,'rows':profile(d,vs)}
  literal.append({'name':name,'independent_metrics':topology({'literal':a},jread(control_root/(name+'.source.geojson')),False)})
 mutations=[]
 def document_diff(a,b,path=''):
  if type(a)!=type(b):return [{'path':path,'before':a,'after':b}]
  if isinstance(a,dict):
   out=[]
   for k in sorted(a.keys()|b.keys()):
    if k not in a or k not in b:out.append({'path':path+'/'+k,'before':a.get(k,'<ABSENT>'),'after':b.get(k,'<ABSENT>')})
    else:out+=document_diff(a[k],b[k],path+'/'+k)
   return out
  if isinstance(a,list):
   if len(a)!=len(b):return [{'path':path,'before':a,'after':b}]
   return [r for i,(x,y) in enumerate(zip(a,b)) for r in document_diff(x,y,path+'/'+str(i))]
  return [] if a==b else [{'path':path,'before':a,'after':b}]
 owners={'restart-joined-line':'lines','restart-declaration-missing':'lines','wrong-feature-id':'points','wrong-table-association':'points','wrong-finite-position':'lines','wrong-position-minmax':'points','wrong-loop-vertex-order':'hole','wrong-triangle-offset-association':'multiple','triangle-omission-degenerate':'hole'}
 for name,owner in owners.items():
  d,b,_,_=unpack(read(control_root/(owner+'.glb')));md,mb,_,_=unpack(read(control_root/(name+'.glb')))
  mutations.append({'name':name,'baseline':owner,'same_semantic_document':d==md,'semantic_document_diffs':document_diff(d,md),'binary_diffs':[{'offset':i,'before':a,'after':z} for i,(a,z) in enumerate(zip(b,mb)) if a!=z]})
 summaries=jread(OBS/'topology-controls-summary.json');assert len(summaries)==14
 traces=[]
 for r in summaries:
  folder=OBS/('topology-control-'+r['name']);receipt=jread(folder/'receipt.json');stderr=read(folder/'stderr.log').decode();stdout=read(folder/'stdout.log').decode()
  assert receipt['exitCode']==r['exitCode'] and r['matchesExpected']
  traces.append({'name':r['name'],'exit_code':receipt['exitCode'],'last_stderr_lines':stderr.splitlines()[-5:]})
 for p in OBS.glob('*source-topology'):
  r=jread(p/'receipt.json');assert r['exitCode']==0;read(p/'stdout.log');read(p/'stderr.log')
 # Bind all completed phase receipts and hashes without executing their programs.
 for p in OBS.rglob('receipt.json'):
  r=jread(p)
  for n in ['stdout.log','stderr.log','result.json']:
   if (p.parent/n).is_file():read(p.parent/n)
  for n,h in r.get('outputs',r.get('outputHashes',{})).items():assert sha(read(p.parent/n))==h
 for p in Path('/tmp/rusty-tiles-vector-p3-observations-large-revised').rglob('receipt.json'):
  r=jread(p);assert r['before']==r['after'] and r['before']['sourceFiles']==97
  assert sha(read(r['product']['path']))==r['product']['sha256']
  binary=next(x for x in r['argv'] if '/stride-final-build/' in x);assert sha(read(binary))==r['before']['binarySha256']
  assert sha(read('/tmp/rusty-tiles-vector-p3-inputs-large/large-ids-65537.geojson'))==r['before']['inputSha256']
  assert sha(read('/tmp/rusty-tiles-vector-p3-inputs-large/large-ids-65537.expected.json'))==r['before']['expectedSha256']
  for n in ['stdout.log','stderr.log']:
   if (p.parent/n).is_file():read(p.parent/n)
  for n,h in r['outputHashes'].items():assert sha(read(p.parent/n))==h
 originals=[]
 for mode in ['raw','meshopt']:
  archive=Path('/tmp/rusty-tiles-vector-p3-inputs-large')/('large-ids-65537-portable-'+mode+'.3tz');read(archive)
  parts=[]
  with zipfile.ZipFile(archive) as z:
   for name in z.namelist():
    raw=z.read(name)
    if raw[:4] in [b'glTF',b'b3dm']:
     d,b,w,f=unpack(raw);table=d['extensions']['EXT_structural_metadata']['propertyTables'][0];p=d['meshes'][0]['primitives'][0]
     parts.append({'table_rows':table['count'],'id_component':d['accessors'][p['attributes']['_FEATURE_ID_0']]['componentType']})
  assert sorted(p['table_rows'] for p in parts)==[16384,16384,16384,16385] and all(p['id_component']==5123 for p in parts)
  originals.append({'variant':mode,'parts':parts})
 pre=jread('/tmp/rusty-tiles-vector-p3-preexecution-review/input-pin.json')
 for package,record in pre['packages'].items():
  assert sha(read(Path(package)/'preparation-pin.json'))==record['pin_sha256']
  for name,h in record['files_sha256'].items():assert sha(read(Path(package)/name))==h
 source_pin=jread(pre['accepted_source_pin']['path'])
 for name,h in source_pin['production_sha256'].items():assert sha(read(Path('/tmp/rusty-tiles-vector-codec-foundation')/name))==h
 retention=Path('/tmp/rusty-tiles-vector-codec-foundation/bench/architecture_audit/vector_compression/candidate_evidence/p3-existing-producer-20261010')
 index=jread(retention/'index.json');assert sha(read(retention/index['bundle']))==index['bundleSha256']
 blobs={}
 with tarfile.open(retention/index['bundle'],'r:gz') as t:
  for member in t.getmembers():
   assert member.isfile() and member.name not in blobs
   b=t.extractfile(member).read();assert member.name=='sha256/'+sha(b);blobs[member.name]=b
 assert len(blobs)==index['uniqueBlobs']==687 and len(index['records'])==2603
 assert len({r['retainedMember'] for r in index['records']})==2603
 for r in index['records']:
  b=blobs[r['blobMember']];assert len(b)==r['bytes'] and sha(b)==r['sha256'] and read(r['sourcePath'])==b
 storage={'index_sha256':sha(read(retention/'index.json')),'bundle_sha256':index['bundleSha256'],'indexed_actual_files':2603,'unique_blobs':687,'all_source_bytes_equal_full_retained_blobs':True,'semantic_acceptance_not_derived_from_storage':True}
 result={'status':'INDEPENDENT_ACTUAL_ARTIFACT_CHECKS_PASS_BOUNDED_BASELINE_ONLY','pairs':pairs,'typed_control_causes':typed,'literal_topology_metrics':literal,'topology_mutation_diffs':mutations,'topology_control_tracebacks':traces,'retained_original_large_id_partitions':originals,'all_97_production_hashes_unchanged':True,'lossless_retention_verification':storage,
  'limits':'No producer/decoder/compiler/author-oracle execution. Logical byte equality recomputed against root decoded artifacts; same upstream codec kernel lineage. No world/coverage/quantization accuracy or selected metadata8 admission approval.'}
 (OUT/'verification.json').write_text(json.dumps(result,indent=2)+'\n')
 (OUT/'input-pin.json').write_text(json.dumps({'files':pins,'file_count':len(pins)},indent=2)+'\n')
 print(json.dumps({'status':result['status'],'pairs':len(pairs),'views':sum(p['views'] for p in pairs),'typed_controls':len(typed),'pinned_files':len(pins)}))
if __name__=='__main__':main()
