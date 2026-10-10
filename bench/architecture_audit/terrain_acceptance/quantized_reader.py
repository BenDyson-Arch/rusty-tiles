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


def expected_sample(values, transform, longitude, latitude, nodata=None):
 """Literal source expectation: closed footprint and centre-clamped bilinear."""
 height=len(values); width=len(values[0]); west,dx,_,north,_,negative_dy=transform
 east=west+width*dx; south=north+height*negative_dy
 if not west<=longitude<=east or not south<=latitude<=north:
  return None
 x=min(width-1,max(0,(longitude-west)/dx-.5))
 y=min(height-1,max(0,(latitude-north)/negative_dy-.5))
 import math
 ix,iy=math.floor(x),math.floor(y); fx,fy=x-ix,y-iy
 contributors={}
 for cx,cy,weight in [(ix,iy,(1-fx)*(1-fy)),(min(ix+1,width-1),iy,fx*(1-fy)),(ix,min(iy+1,height-1),(1-fx)*fy),(min(ix+1,width-1),min(iy+1,height-1),fx*fy)]:
  if weight:contributors[cx,cy]=contributors.get((cx,cy),0)+weight
 answer=0.
 for (cx,cy),weight in contributors.items():
  value=values[cy][cx]
  if value is None or (nodata is not None and (value==nodata or math.isnan(nodata) and math.isnan(value))):return None
  if not math.isfinite(value):raise ValueError('invalid literal source')
  answer+=value*weight
 return answer


def self_test():
 assert expected_sample([[1.,2.],[3.,4.]],[0,1,0,2,0,-1],1,1)==2.5
 assert expected_sample([[1.,2.],[3.,4.]],[0,1,0,2,0,-1],0,2)==1.
 assert expected_sample([[1.,2.],[3.,4.]],[0,1,0,2,0,-1],-1e-8,2) is None
 assert expected_sample([[1.,None],[3.,4.]],[0,1,0,2,0,-1],.5,1.5)==1.
 assert expected_sample([[1.,None],[3.,4.]],[0,1,0,2,0,-1],1,1) is None

if __name__=='__main__':
 import argparse
 parser=argparse.ArgumentParser();parser.add_argument('tile',nargs='?');args=parser.parse_args()
 self_test()
 if args.tile:
  result=decode(pathlib.Path(args.tile).read_bytes())
  print(json.dumps({key:value if key in ('header','extensions') else len(value) for key,value in result.items()}))
 else:print('Independent sampler self-tests passed')
