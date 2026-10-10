#!/usr/bin/env python3
"""Independent finite source topology checks for the authored planar P3 corpus.

Reads literal GLB plus separately decoded logical views, not production/C1 or
the frozen extractor. Exact rational edge metrics and triangle area only for
unquantized axis-plane controls; quantization accuracy is explicitly outside
scope. Ring metrics admit reversal, so this is not a winding/coverage proof.
"""
import argparse,json,struct,math,hashlib
from pathlib import Path
from fractions import Fraction as F
R=0xffffffff
def exact(x):return F.from_float(x) if isinstance(x,float) else F(x)
def split(xs):
 out=[];part=[]
 for x in xs:
  if x==R:
   assert part,'empty restart segment';out.append(part);part=[]
  else:part.append(x)
 if part:out.append(part)
 return out
def distance(a,b):return sum((x-y)**2 for x,y in zip(a,b))
def edges(points,closed):
 pairs=list(zip(points,points[1:]));pairs+= [(points[-1],points[0])] if closed else []
 values=[distance(a,b) for a,b in pairs]
 if not closed:return min(tuple(values),tuple(reversed(values)))
 return min(tuple(v[i:]+v[:i]) for v in [values,list(reversed(values))] for i in range(len(v)))
def triangle_area(a,b,c):
 u=[b[i]-a[i] for i in range(3)];v=[c[i]-a[i] for i in range(3)]
 cross=[u[1]*v[2]-u[2]*v[1],u[2]*v[0]-u[0]*v[2],u[0]*v[1]-u[1]*v[0]]
 q=sum(x*x for x in cross);n=math.isqrt(q.numerator);d=math.isqrt(q.denominator)
 assert n*n==q.numerator and d*d==q.denominator,'outside exact rational plane-area profile'
 return F(n,d)/2
def ring_area(points):
 # A generated plane has a rational cross-product norm. Translation cancels
 # in the closed sum. This checks triangle/loop association, not world CRS.
 sums=[F(0)]*3
 for a,b in zip(points,points[1:]+points[:1]):
  values=[a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]]
  sums=[x+y for x,y in zip(sums,values)]
 return vector_area(sums)
def vector_area(values):
 q=sum(x*x for x in values);n=math.isqrt(q.numerator);d=math.isqrt(q.denominator)
 assert n*n==q.numerator and d*d==q.denominator,'outside rational planar area profile'
 return F(n,d)/2
def source_state(source):
 want={}
 for f in source['features']:
  sid=json.dumps(f['id'],separators=(',',':'),ensure_ascii=False);g=f['geometry'];kind=g['type'];c=g['coordinates']
  state={'points':0,'lines':[],'polygons':[],'area':F(0)}
  if kind=='Point':state['points']=1
  elif kind=='MultiPoint':state['points']=len(c)
  elif kind in ['LineString','MultiLineString']:
   state['lines']=[[tuple(exact(x) for x in p) for p in line] for line in ([c] if kind=='LineString' else c)]
  else:
   assert kind in ['Polygon','MultiPolygon'];polygons=[c] if kind=='Polygon' else c
   for rings in polygons:
    checked=[]
    for ring in rings:
     assert ring[0]==ring[-1];pts=[tuple(exact(x) for x in p) for p in ring[:-1]]
     assert all(p[2]==pts[0][2] for p in pts),'only authored constant-z source planes'
     checked.append(pts)
    state['polygons'].append(checked)
    areas=[abs(sum(a[0]*b[1]-a[1]*b[0] for a,b in zip(p,p[1:]+p[:1])))/2 for p in checked]
    state['area']+=areas[0]-sum(areas[1:])
  want[sid]=state
 return want
def read_payload(path,decoded=None):
 raw=path.read_bytes()
 if raw[:4]==b'b3dm':
  version,total,fj,fb,bj,bb=struct.unpack_from('<6I',raw,4);assert version==1 and total==len(raw)
  feature=json.loads(raw[28:28+fj]);assert feature=={'BATCH_LENGTH':0} and fb==bj==bb==0
  raw=raw[28+fj:]
 assert struct.unpack_from('<4sII',raw)==(b'glTF',2,len(raw))
 jlen,jtype=struct.unpack_from('<I4s',raw,12);assert jtype==b'JSON';doc=json.loads(raw[20:20+jlen])
 bl,btype=struct.unpack_from('<I4s',raw,20+jlen);assert btype==b'BIN\0' and 28+jlen+bl==len(raw)
 binary=raw[28+jlen:];views=[]
 for i,v in enumerate(doc['bufferViews']):
  if 'EXT_meshopt_compression' in v.get('extensions',{}):
   assert decoded is not None;value=(decoded/('view-%04d.bin'%i)).read_bytes()
  else:
   assert v['buffer']==0;s=v.get('byteOffset',0);value=binary[s:s+v['byteLength']]
  assert len(value)==v['byteLength'];views.append(value)
 def access(i):
  a=doc['accessors'][i];v=doc['bufferViews'][a['bufferView']];fmt={5123:'H',5125:'I',5126:'f'}[a['componentType']]
  width={'SCALAR':1,'VEC3':3}[a['type']];stride=v.get('byteStride',struct.calcsize(fmt)*width);off=a.get('byteOffset',0)
  assert a['count']<=1_000_000
  xs=[struct.unpack_from('<'+fmt*width,views[a['bufferView']],off+n*stride) for n in range(a['count'])]
  return [x[0] for x in xs] if width==1 else xs
 meta=doc['extensions']['EXT_structural_metadata'];tables=meta['propertyTables'];assert len(tables)==1
 table=tables[0];prop=table['properties']['_source_id'];values=views[prop['values']];offsets=views[prop['stringOffsets']]
 assert len(offsets)==4*(table['count']+1);offsets=struct.unpack('<'+'I'*(table['count']+1),offsets)
 assert offsets[0]==0 and offsets[-1]==len(values) and list(offsets)==sorted(offsets)
 ids=[values[a:b].decode('utf8') for a,b in zip(offsets,offsets[1:])]
 node=doc['nodes'][0];assert 'matrix' not in node and 'rotation' not in node
 scale=list(map(exact,node.get('scale',[1,1,1])));translation=list(map(exact,node.get('translation',[0,0,0])))
 return doc,access,ids,scale,translation
def check(payloads,source,quantized=False):
 want=source_state(source);seen=set();actual={k:{'points':0,'lines':[],'polygons':[],'area':F(0)} for k in want}
 for path,decoded in payloads:
  doc,access,sids,scale,translation=read_payload(path,decoded);assert set(sids)<=set(want);seen.update(sids)
  for mesh in doc['meshes']:
   for p in mesh['primitives']:
    ac=doc['accessors'][p['attributes']['POSITION']];positions=access(p['attributes']['POSITION'])
    assert bool(ac.get('normalized',False))==quantized
    # Min/max truth is an independent SOURCE-semantic check, not a claim that
    # a preservation-only codec must interpret or repair these opaque values.
    for key,op in [('min',min),('max',max)]:
     if key in ac:assert list(map(exact,ac[key]))==[op(exact(v[i]) for v in positions) for i in range(3)],'wrong POSITION min/max'
    divisor=65535 if ac.get('normalized') else 1
    points=[tuple(exact(v[i])/divisor*scale[i]+translation[i] for i in range(3)) for v in positions]
    fids=access(p['attributes']['_FEATURE_ID_0']);assert len(fids)==len(points)
    assert all(int(x)==x and 0<=x<len(sids) for x in fids)
    f=p['extensions']['EXT_mesh_features']['featureIds'][0]
    assert f['featureCount']==len(sids) and f['propertyTable']==0 and f['attribute']==0,'wrong feature-table association'
    indices=access(p['indices']);assert all(x==R or x<len(points) for x in indices)
    def owner(xs):
     owners={sids[int(fids[i])] for i in xs};assert len(owners)==1,'mixed source identities within topology';return next(iter(owners))
    mode=p.get('mode',4)
    if mode==0:
     assert R not in indices
     for i in indices:actual[owner([i])]['points']+=1
    elif mode==3:
     if R in indices:assert 'KHR_mesh_primitive_restart' in doc.get('extensionsRequired',[]),'restart required declaration missing'
     for segment in split(indices):
      assert len(segment)>=2;actual[owner(segment)]['lines'].append([points[i] for i in segment])
    else:
     assert mode==4 and R not in indices and len(indices)%3==0
     for i in range(0,len(indices),3):
      tri=indices[i:i+3];sid=owner(tri)
      if not quantized:actual[sid]['area']+=triangle_area(*(points[x] for x in tri))
     poly=p.get('extensions',{}).get('EXT_mesh_polygon')
     if poly:
      offsets=access(poly['loopIndicesOffsets']);loops=access(poly['loopIndices']);io=access(poly['indicesOffsets'])
      assert len(offsets)==len(io)==poly['count'] and offsets[0]==io[0]==0
      assert offsets==sorted(offsets) and io==sorted(io)
      for n,start in enumerate(offsets):
       rings=split(loops[start:offsets[n+1] if n+1<len(offsets) else len(loops)])
       assert rings and all(len(r)>=3 and len(set(r))==len(r) for r in rings)
       sid=owner([x for ring in rings for x in ring]);actual[sid]['polygons'].append([[points[x] for x in ring] for ring in rings])
       part=indices[io[n]:io[n+1] if n+1<len(io) else len(indices)]
       assert part and len(part)%3==0 and owner(part)==sid,'polygon triangle-offset association changed'
       tri_area=sum(triangle_area(*(points[x] for x in part[k:k+3])) for k in range(0,len(part),3))
       loop_area=ring_area([points[x] for x in rings[0]])-sum(ring_area([points[x] for x in ring]) for ring in rings[1:])
       assert tri_area==loop_area,'polygon triangle-offset area differs from its loop association'
 for sid,state in want.items():
  got=actual[sid];assert got['points']==state['points'],('point cardinality',sid)
  if state['lines']:
   assert sorted(map(len,got['lines']))==sorted(map(len,state['lines'])),('source line partition',sid)
   if not quantized:assert sorted(edges(x,False) for x in got['lines'])==sorted(edges(x,False) for x in state['lines']),('source line edge metric',sid)
  if state['polygons']:
   if got['polygons']:
    shape=lambda polys:sorted(tuple(map(len,p)) for p in polys)
    assert shape(got['polygons'])==shape(state['polygons']),('source polygon loop association',sid)
    if not quantized:
     signature=lambda polys:sorted(tuple(edges(r,True) for r in p) for p in polys)
     assert signature(got['polygons'])==signature(state['polygons']),('source polygon edge metric',sid)
   # Fill fragments intentionally expose triangles plus separate boundaries,
   # not polygon metadata. Count/area is not proof of union/no-overlap/winding.
   if not quantized:assert got['area']==state['area'],('source planar triangle area',sid,got['area'],state['area'])
 assert seen==set(want),'source identity coverage'
 return {'status':'finite_source_topology_checks_pass','source_ids':len(seen),
  'quantization_scope':'cardinality/associations only; no quantized coordinate accuracy' if quantized else 'exact rational metrics for authored local axis-plane corpus',
  'limits':'ring reversal admitted; no winding, triangle union/overlap/complete coverage, CRS/world-frame or full-extension conformance proof'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--source',type=Path,required=True);g=p.add_mutually_exclusive_group(required=True)
 g.add_argument('--input',type=Path);g.add_argument('--extracted',type=Path);p.add_argument('--quantized',action='store_true');a=p.parse_args()
 payloads=[(a.input,None)] if a.input else [(d/'source.glb',d) for d in a.extracted.glob('payload-*') if d.is_dir()]
 assert payloads;result=check(payloads,json.loads(a.source.read_bytes()),a.quantized)
 result['oracle_sha256']=hashlib.sha256(Path(__file__).read_bytes()).hexdigest();print(json.dumps(result))
if __name__=='__main__':main()
