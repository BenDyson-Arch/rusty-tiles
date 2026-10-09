"""Strict independent stdlib quantized-mesh decoder; no historical encoder imports."""
import struct,gzip,collections,pathlib,json

def decode(data):
 if data[:2]==b'\x1f\x8b': data=gzip.decompress(data)
 pos=0
 def read(fmt):
  nonlocal pos
  size=struct.calcsize('<'+fmt)
  if pos+size>len(data):raise ValueError('truncated payload')
  v=struct.unpack_from('<'+fmt,data,pos);pos+=size;return v
 header=read('3d2f7d');n,=read('I')
 if not 3<=n<=10_000_000:raise ValueError('vertex count')
 arrays=[]
 for _ in range(3):
  last=0;values=[]
  for _ in range(n):
   code,=read('H');last+=(code>>1)^-(code&1)
   if not 0<=last<=32767:raise ValueError('attribute range')
   values.append(last)
  arrays.append(values)
 width='H' if n<=65536 else 'I'
 if width=='I':pos+=(4-pos%4)%4
 count,=read('I');triangles=[];highest=0
 for _ in range(count):
  tri=[]
  for _ in range(3):
   code,=read(width);index=highest-code
   if not 0<=index<n:raise ValueError('index range')
   if code==0:highest+=1
   tri.append(index)
  triangles.append(tri)
 edges=[]
 for _ in range(4):
  count,=read('I');edge=[read(width)[0] for _ in range(count)]
  if any(i>=n for i in edge):raise ValueError('edge index')
  edges.append(edge)
 extensions={}
 while pos<len(data):
  ident,length=read('BI')
  if pos+length>len(data):raise ValueError('extension length')
  if ident in extensions:raise ValueError('duplicate extension')
  extensions[ident]=data[pos:pos+length];pos+=length
 vertices=list(zip(*arrays)); incidence=collections.Counter();area=0
 for tri in triangles:
  a,b,c=[vertices[i] for i in tri];cross=(b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])
  if cross<=0:raise ValueError('winding/degenerate')
  area+=cross
  for i in range(3):incidence[tuple(sorted((tri[i],tri[(i+1)%3])))]+=1
 if area!=2*32767**2:raise ValueError('coverage area')
 boundary=set()
 for side,ids in enumerate(edges):
  coordinate,value=(0,0) if side==0 else (1,0) if side==1 else (0,32767) if side==2 else (1,32767)
  if any(vertices[i][coordinate]!=value for i in ids):raise ValueError('edge position')
  if len(set(ids))!=len(ids):raise ValueError('duplicate edge vertex')
  ids=sorted(ids,key=lambda i:vertices[i][1-coordinate])
  if vertices[ids[0]][1-coordinate]!=0 or vertices[ids[-1]][1-coordinate]!=32767:raise ValueError('edge extent')
  boundary.update(tuple(sorted(pair)) for pair in zip(ids,ids[1:]))
 if any(count!=(1 if edge in boundary else 2) for edge,count in incidence.items()) or not boundary<=incidence.keys():raise ValueError('nonmanifold/boundary')
 return dict(header=header,vertices=vertices,triangles=triangles,edges=edges,extensions={i:len(v) for i,v in extensions.items()})
if __name__=='__main__':
 p=pathlib.Path('/home/bend/.cache/terrain-t1-evidence/constant-out/8/272/187.terrain');d=p.read_bytes();r=decode(d);print(json.dumps({k:v if k in ('header','extensions') else len(v) for k,v in r.items()}))
 for label,changed in [('truncate',d[:-1]),('bad_attribute',d[:92]+b'\xff\xff'+d[94:]),('bad_triangle',d[:1830]+b'\xff\xff'+d[1832:])]:
  try:decode(changed)
  except (ValueError,struct.error) as e:print(label,'rejected:',e)
  else:raise AssertionError(label+' accepted')
