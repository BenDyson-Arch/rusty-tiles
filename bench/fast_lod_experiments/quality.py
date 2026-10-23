"""Independent area-weighted surface probes, 200k samples in each direction/level.
Colour: unlit base colour, bilinear decoded L0 in linear light, evaluated at nearest
source points. No claim about lighting, normal maps or viewer-selected mip levels.
"""
import json,sys,pathlib,subprocess,math,os
import numpy as np
from PIL import Image
S=pathlib.Path('/home/bend/scratch/rusty-tiles-scratch/lod-bakeoff/astra-keep-h17');E=pathlib.Path('/home/bend/.cache/rusty-tiles-evidence/fast-lod-exp-20261024')
name=sys.argv[1];D=S/name/'decoded';N=int(os.getenv('QUALITY_SAMPLES','200000'))
source=D if name=='source' else S/('source-crop3-decoded' if 'crop3' in name else 'source-decoded')
def load(d):
 rows=json.loads((d/'index.json').read_text());arr=[];groups=[]
 for row in rows:
  a=np.fromfile(d/f"{row['id']}.tri.f64",np.float64).reshape(-1,27);off=sum(len(v) for v in arr);arr.append(a)
  for g in row['surfaces']:groups.append(dict(g,first=off+g['first']))
 return rows,np.concatenate(arr),groups
sr,src,sg=load(source);rows,dst,dg=load(D);tree=json.loads((D/'tree.json').read_text())
linear=lambda x:np.where(x<=0.04045,x/12.92,((x+0.055)/1.055)**2.4)
srgb=lambda x:np.where(x<=0.0031308,x*12.92,1.055*np.maximum(x,0)**(1/2.4)-0.055)
cache={}
def rgba(d,im):
 key=(str(d),im['file'])
 if key not in cache:
  cache[key]=np.fromfile(d/im['file'],np.uint8).reshape(im['height'],im['width'],4) if im['raw'] else np.array(Image.open(d/im['file']).convert('RGBA'))
 return cache[key]
def colors(d,a,groups,idx,b):
 uv=(a[idx,9:15].reshape(-1,3,2)*b[:,:,None]).sum(1)
 col=(a[idx,15:27].reshape(-1,3,4)*b[:,:,None]).sum(1)[:,:3]
 for g in groups:
  mask=(idx>=g['first'])&(idx<g['first']+g['count'])
  if not mask.any() or g['image'] is None:continue
  im=rgba(d,g['image']);h,w=im.shape[:2];v=uv[mask].copy();s=g['sampler']
  for k,wrap in [(0,s.get('wrapS',10497)),(1,s.get('wrapT',10497))]:
   if wrap==10497:v[:,k]%=1
   elif wrap==33648:v[:,k]=1-np.abs(v[:,k]%2-1)
   else:v[:,k]=np.clip(v[:,k],0,1)
  xy=v*np.array([w,h])-.5;lo=np.floor(xy).astype(np.int64);f=xy-lo;out=np.zeros((mask.sum(),3))
  for dx,dy,wt in [(0,0,(1-f[:,0])*(1-f[:,1])),(1,0,f[:,0]*(1-f[:,1])),(0,1,(1-f[:,0])*f[:,1]),(1,1,f[:,0]*f[:,1])]:
   x=lo[:,0]+dx;y=lo[:,1]+dy
   x=x%w if s.get('wrapS',10497)==10497 else x.clip(0,w-1);y=y%h if s.get('wrapT',10497)==10497 else y.clip(0,h-1)
   out+=linear(im[y,x,:3]/255.)*wt[:,None]
  col[mask]*=out
 return srgb(col.clip(0,1))*255
rng=np.random.default_rng(20261024)
def sample(a):
 p=a[:,:9].reshape(-1,3,3);area=np.linalg.norm(np.cross(p[:,1]-p[:,0],p[:,2]-p[:,0]),axis=1);idx=rng.choice(len(a),N,p=area/area.sum());r=rng.random((N,2));sq=np.sqrt(r[:,0]);b=np.c_[1-sq,sq*(1-r[:,1]),sq*r[:,1]];return idx,b,(p[idx]*b[:,:,None]).sum(1)
def nearest(a,q,tag):
 a[:,:9].copy().tofile(D/'quality-tri.f64');q.tofile(D/'quality-q.f64')
 subprocess.run([str(E/'nearest'),str(D/'quality-tri.f64'),str(D/'quality-q.f64'),str(D/'quality-result.f64')],check=True)
 return np.fromfile(D/'quality-result.f64',np.float64).reshape(-1,5)
def stats(d):return dict(mean=float(d.mean()),p95=float(np.quantile(d,.95)),max=float(d.max()))
def psnr(a,b):
 mse=float(np.mean((a-b)**2));return dict(psnr_db=10*math.log10(255**2/max(mse,1e-30)),mae_byte=float(np.abs(a-b).mean()),max_byte=float(np.abs(a-b).max()))
starts={};off=0
for r in rows:starts[r['file']]=(off,off+r['triangles']);off+=r['triangles']
children={v['parent'] for v in tree.values()};leaves=set(tree)-children
if os.getenv('COMPARE_BASELINE'):
 base=S/'baseline-tex3'/'decoded';br,ba,bg=load(base)
 assert [r['file'] for r in br]==[r['file'] for r in rows]
 assert np.array_equal(ba[:,:9],dst[:,:9]), 'changed decoded geometry'
 comparisons=[]
 for depth in sorted({v['depth'] for v in tree.values()}):
  files=[f for f,v in tree.items() if v['depth']==depth and f not in leaves]
  if not files:continue
  ids=np.concatenate([np.arange(*starts[f]) for f in files]);ix,b,_=sample(dst[ids]);ix=ids[ix]
  c=colors(D,dst,dg,ix,b);ref=colors(base,ba,bg,ix,b)
  comparisons.append(dict(depth=depth,samples=N,**psnr(c,ref)))
 import hashlib
 equal_leaves=sum((S/name/'view'/f).read_bytes()==(S/'baseline-tex3'/'view'/f).read_bytes() for f in leaves)
 (E/name/'baseline-comparison.json').write_text(json.dumps(dict(decoded_positions_equal=True,leaf_glbs_equal=equal_leaves,total_leaves=len(leaves),proxy_colour=comparisons),indent=2))
 sys.exit(0)
si,sb,sq=sample(src);sc=colors(source,src,sg,si,sb)
result=[]
for depth in sorted({v['depth'] for v in tree.values()}):
 frontier=[f for f,v in tree.items() if v['depth']==depth or (f in leaves and v['depth']<depth)]
 ids=np.concatenate([np.arange(*starts[f]) for f in frontier]);level=dst[ids]
 # Both directions cover the entire depth frontier, including shallower leaves.
 fwd=nearest(level,sq,'forward');pi,pb,pq=sample(level);back=nearest(src,pq,'reverse')
 proxy=np.array([False]*len(dst))
 for f in frontier:
  if f not in leaves:proxy[slice(*starts[f])]=True
 select=proxy[ids[pi]]
 tc=colors(D,dst,dg,ids[pi[select]],pb[select]);ref=colors(source,src,sg,back[select,1].astype(int),back[select,2:5])
 row=dict(depth=depth,samples_per_direction=N,tiles_at_depth=sum(v['depth']==depth for v in tree.values()),triangles_at_depth=sum(starts[f][1]-starts[f][0] for f,v in tree.items() if v['depth']==depth),frontier_triangles=len(level),bytes_at_depth=sum((S/name/'view'/f).stat().st_size for f,v in tree.items() if v['depth']==depth),source_to_frontier=stats(fwd[:,0]),frontier_to_source=stats(back[:,0]),proxy_color_samples=int(select.sum()),proxy_color_vs_source=psnr(tc,ref) if len(tc) else None)
 result.append(row);(E/name/'quality.json').write_text(json.dumps(result,indent=2));print(row,flush=True)
for f in D.glob('quality-*.f64'):f.unlink()
