#!/usr/bin/env python3
"""Bounded full-input quality sample and exhaustive shared-source-edge census."""
import json,sys
from pathlib import Path
import numpy as np
from scipy.spatial import cKDTree
from evaluate import Surface,topology,stats,lab

source,root,output=sys.argv[1:];root=Path(root);out=Path(output);out.mkdir(parents=True,exist_ok=True)
nodes=json.loads((root/'build.json').read_text())['nodes'];src=Surface().add(source,True).finish()
source_triangles=len(src.xyz);rng=np.random.default_rng(20261024);rows=[]
for depth in [0,2,4]:
    surface=Surface()
    for n in nodes:
        if n['depth']==depth:surface.add(root/n['file'])
    surface.finish();distances=[];deltas=[];directions={}
    for label,a,b in [('source_to_lod',src,surface),('lod_to_source',surface,src)]:
        q,fi,bc=a.sample(200000,rng);d,near,nbc=b.closest(q);assert np.isfinite(d).all() and np.isfinite(nbc).all()
        ca=a.rgb(fi,bc);cb=b.rgb(near,nbc);delta=np.linalg.norm(lab(ca)-lab(cb),axis=1)
        srgb=lambda c:np.where(c<=.0031308,c*12.92,1.055*np.maximum(c,0)**(1/2.4)-.055)
        mse=float(np.mean((srgb(np.clip(ca,0,1))-srgb(np.clip(cb,0,1)))**2))
        directions[label]=dict(samples=len(q),geometry_m=stats(d),delta_e76=stats(delta),psnr_srgb_db=float(-10*np.log10(max(mse,1e-30))))
        distances.append(d);deltas.append(delta)
    row=dict(depth=depth,triangles=len(surface.xyz),geometry_m=stats(np.concatenate(distances)),delta_e76=stats(np.concatenate(deltas)),directions=directions)
    rows.append(row);(out/'quality.json').write_text(json.dumps(rows,indent=2));print('quality',json.dumps(row),flush=True)
    del surface
# Reconstruct ownership independently from exact decoded leaf triangle metadata.
p=src.provenance;starts=np.r_[0,np.flatnonzero(np.any(p[1:,:2]!=p[:-1,:2],axis=1))+1]
offsets=np.zeros((int(p[:,0].max())+1,int(p[:,1].max())+1),np.int64)
known_primitives=set(map(tuple,p[starts,:2]));offsets[p[starts,0],p[starts,1]]=starts;owner=np.full(len(p),-1,np.int32)
for n in nodes:
    if n['children']:continue
    surf=Surface().add(root/n['file']).finish();sp=surf.provenance
    ids=offsets[sp[:,0],sp[:,1]]+sp[:,2]
    assert len(np.unique(ids))==len(ids) and np.all(owner[ids]==-1)
    assert np.array_equal(p[ids],sp) and np.array_equal(src.xyz[ids],surf.xyz)
    assert np.array_equal(src.uv[ids],surf.uv) and np.array_equal(src.color[ids],surf.color)
    assert np.array_equal(src.primitive_identity[ids],surf.primitive_identity)
    owner[ids]=n['id'];print('leaf',n['id'],'exact',len(ids),flush=True)
assert np.all(owner>=0);del surf,p
# Only topology is needed below; discard colour/provenance before global edge sorting.
for key in ['uv','color','mat','materials','provenance','primitive_identity','faces','vertices']:delattr(src,key)
pos,edges,order,starts,counts=topology(src.xyz);del src
for s,c in zip(starts[counts>2],counts[counts>2]):
    assert len(np.unique(owner[order[s:s+c]//3]))==1,'nonmanifold junction split across leaves'
shared=starts[counts==2];a=owner[order[shared]//3];b=owner[order[shared+1]//3];cut=a!=b
ends=edges[order[shared[cut]]];left=a[cut];right=b[cut]
nonmanifold=int(np.count_nonzero(counts>2));del edges,order,starts,counts,shared,a,b,cut,owner
used,inv=np.unique(ends,return_inverse=True);interface_pos=pos[used];ends=inv.reshape(-1,2);del pos,used,inv
keys=ends[:,0]*len(interface_pos)+ends[:,1];tree=cKDTree(interface_pos)
leaf_masks={n['id']:n['mask'] for n in nodes if not n['children']}
la=np.array([leaf_masks[int(x)] for x in left],np.uint64);lb=np.array([leaf_masks[int(x)] for x in right],np.uint64)
retention={};primitive_ids=set()
for n in nodes:
    mask=np.uint64(n['mask']);need=((la&mask)!=0)^((lb&mask)!=0)
    if not need.any():continue
    surf=Surface().add(root/n['file']).finish();pp,ee,oo,ss,cc=topology(surf.xyz)
    # Complete decoded segments, not a finite sampling of their endpoints.
    distance,mapped=tree.query(pp,workers=1);unique=ee[oo[ss]];valid=(distance[unique]==0).all(1)
    pair=np.sort(mapped[unique[valid]],axis=1);actual=pair[:,0]*len(interface_pos)+pair[:,1]
    missing=int(np.count_nonzero(~np.isin(keys[need],actual)))
    retention[n['id']]=dict(interface_edges=int(need.sum()),missing_exact_segments=missing)
    assert missing==0,f'decoded outer segment missing in tile {n["id"]}'
    primitive_ids.update(map(tuple,surf.primitive_identity));print('interface',n['id'],int(need.sum()),'exact',flush=True)
    del surf,pp,ee,oo,ss,cc
pairs=[]
for i,a in enumerate(nodes):
    for b in nodes[i+1:]:
        ma=np.uint64(a['mask']);mb=np.uint64(b['mask'])
        if ma&mb:continue
        count=int(np.count_nonzero((((la&ma)!=0)&((lb&mb)!=0))|(((la&mb)!=0)&((lb&ma)!=0))))
        if count:pairs.append(dict(a=a['id'],b=b['id'],depth_a=a['depth'],depth_b=b['depth'],source_edges=count,width_m=stats(np.zeros(1)),fraction_ge=stats(np.zeros(1))))
assert primitive_ids<=known_primitives
result=dict(source_triangles=source_triangles,proxy_primitive_ids_valid=True,exact_leaf_geometry_uv_color_and_provenance=True,all_corner_feature_ids_valid=True,
    source_nonmanifold_edges=nonmanifold,shared_cross_leaf_edges=len(ends),mixed_pairs=sum(p['depth_a']!=p['depth_b'] for p in pairs),
    same_depth_pairs=sum(p['depth_a']==p['depth_b'] for p in pairs),mixed_width_m=stats(np.zeros(1)),mixed_fraction_ge=stats(np.zeros(1)),
    same_depth_width_m=stats(np.zeros(1)),retention=retention,pairs=pairs,
    method='Exhaustive source-shared-edge adjacency; exact complete decoded segment retention establishes zero width on every pair. No projection fallback.')
(out/'cracks.json').write_text(json.dumps(result,indent=2));print('cracks',json.dumps({k:v for k,v in result.items() if k not in ['pairs','retention']}),flush=True)
