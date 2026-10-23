"""Every non-overlapping adjacent legacy tile pair, using published geometry.
Sample both endpoints and midpoint of every shared original source edge. Project
anchors onto each tile's open border, measure to the other surface, symmetrically.
A finite probe, not a continuous maximum certificate.
"""
import pathlib,json,numpy as np,subprocess
S=pathlib.Path('/home/bend/scratch/rusty-tiles-scratch/lod-bakeoff/astra-keep-h17');w=S/'legacy-anchor-tex3';E=pathlib.Path('/home/bend/.cache/rusty-tiles-evidence/fast-lod-exp-20261024/legacy-anchor-tex3');D=w/'decoded';tmp=w/'pair-work';tmp.mkdir(exist_ok=True)
rows=json.load(open(D/'index.json'));tree=json.load(open(D/'tree.json'));src=np.fromfile(S/'source-decoded/0.tri.f64',np.float64).reshape(-1,27)[:,:9].copy().reshape(-1,3,3)
source_groups=json.load(open(S/'source-decoded/index.json'))[0]['surfaces'];bases={g['mesh']:g['first'] for g in source_groups}
ls=np.fromfile(w/'seams.leaf.src.u32',np.uint32).reshape(-1,6);lt=np.fromfile(w/'seams.leaf.tile.i32',np.int32);owner=np.zeros(len(src),int)
for mi,base in bases.items():mask=ls[:,0]==mi;owner[base+ls[mask,2]]=lt[mask]
_,W=np.unique(src.reshape(-1,3),axis=0,return_inverse=True);W=W.reshape(-1,3);A=W.ravel();B=W[:,[1,2,0]].ravel();keys=np.minimum(A,B).astype(np.uint64)*2**32+np.maximum(A,B).astype(np.uint64);o=np.argsort(keys);k=keys[o];st=np.r_[0,np.flatnonzero(np.diff(k))+1];count=np.diff(np.r_[st,len(k)]);st=st[count==2];ea=o[st];eb=o[st+1];mask=owner[ea//3]!=owner[eb//3];ea=ea[mask];eb=eb[mask]
chains={}
for row in rows:
 f=row['file'];chain=[]
 while f is not None:chain.append(f);f=tree[f]['parent']
 chains[row['file']]=chain
pairs={}
for i,(a,b) in enumerate(zip(owner[ea//3],owner[eb//3])):
 for fa in chains[rows[a]['file']]:
  for fb in chains[rows[b]['file']]:
   if fa in chains[fb] or fb in chains[fa]:continue
   key=tuple(sorted([fa,fb],key=lambda f:(tree[f]['depth'],f)));pairs.setdefault(key,[]).append(i)
meshes={r['file']:np.fromfile(D/f"{r['id']}.tri.f64",np.float64).reshape(-1,27)[:,:9].copy().reshape(-1,3,3) for r in rows}
paths={};borders={}
for r in rows:
 f=r['file'];p=meshes[f];fp=tmp/f"{r['id']}.surface";p.tofile(fp);paths[f]=fp
 _,v=np.unique(p.reshape(-1,3),axis=0,return_inverse=True);v=v.reshape(-1,3);a=v.ravel();b=v[:,[1,2,0]].ravel();kk=np.minimum(a,b).astype(np.uint64)*2**32+np.maximum(a,b).astype(np.uint64);_,ii,nn=np.unique(kk,return_index=True,return_counts=True);ii=ii[nn==1]
 tri=np.stack([p.reshape(-1,3)[ii],p[:,[1,2,0]].reshape(-1,3)[ii],p[:,[1,2,0]].reshape(-1,3)[ii]],axis=1);fp=tmp/f"{r['id']}.border";tri.tofile(fp);borders[f]=(fp,tri)
def nearest(path,q):
 q.tofile(tmp/'q');subprocess.run([str(E.parent/'nearest'),str(path),str(tmp/'q'),str(tmp/'r')],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,check=True);return np.fromfile(tmp/'r',np.float64).reshape(-1,5)
def project(f,q):
 p,t=borders[f];r=nearest(p,q);return (t[r[:,1].astype(int)]*r[:,2:,None]).sum(1)
res=[];groups={}
for (fa,fb),es in pairs.items():
 ix=ea[np.unique(es)];a=src.reshape(-1,3)[ix];b=src[:,[1,2,0]].reshape(-1,3)[ix];q=np.concatenate([a,b,(a+b)/2]);qa=project(fa,q);qb=project(fb,q);gap=np.maximum(nearest(paths[fb],qa)[:,0],nearest(paths[fa],qb)[:,0]);da,db=tree[fa]['depth'],tree[fb]['depth'];ge=tree[fa]['ge'];frac=gap/max(ge,1e-30)
 row=dict(a=fa,b=fb,depth_a=da,depth_b=db,samples=len(q),max_m=float(gap.max()),p99_m=float(np.quantile(gap,.99)),max_fraction_ge=float(frac.max()));res.append(row);groups.setdefault((da,db),[]).append((gap,frac));print(row,flush=True)
summary=[]
for (a,b),parts in groups.items():
 g=np.concatenate([g for g,f in parts]);f=np.concatenate([f for g,f in parts]);summary.append(dict(depth_a=a,depth_b=b,pairs=len(parts),samples=len(g),max_m=float(g.max()),p99_m=float(np.quantile(g,.99)),max_fraction_ge=float(f.max()),p99_fraction_ge=float(np.quantile(f,.99))))
(E/'crack-pairs.json').write_text(json.dumps(res,indent=2));(E/'crack-summary.json').write_text(json.dumps(summary,indent=2))
import shutil;shutil.rmtree(tmp)
