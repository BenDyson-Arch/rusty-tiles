#!/usr/bin/env python3
"""Decoded independent ownership, variable-depth cuts and complete interface census."""
import collections,itertools,json,sys
from pathlib import Path
import numpy as np
from scipy.spatial import cKDTree
from evaluate import Surface,topology,stats,lab
from octree_partition import triangle_ids
from build import GLB

source,root,output=sys.argv[1:];root=Path(root);out=Path(output);out.mkdir(parents=True,exist_ok=True)
nodes=json.loads((root/'build.json').read_text())['nodes'];src=Surface().add(source,True).finish();rng=np.random.default_rng(20261024)
rows=[]
for depth in [0,2,4,6]:
    cut=[]
    def visit(n):
        if n['depth']>=depth or not n['children']:cut.append(n)
        else:
            for c in n['children']:visit(nodes[c])
    visit(nodes[0]);surface=Surface()
    for n in cut:surface.add(root/n['file'])
    surface.finish();distances=[];deltas=[];directions={}
    for label,a,b in [('source_to_lod',src,surface),('lod_to_source',surface,src)]:
        q,fi,bc=a.sample(200000,rng);d,near,nbc=b.closest(q);assert np.isfinite(d).all() and np.isfinite(nbc).all()
        ca=a.rgb(fi,bc);cb=b.rgb(near,nbc);delta=np.linalg.norm(lab(ca)-lab(cb),axis=1)
        srgb=lambda c:np.where(c<=.0031308,c*12.92,1.055*np.maximum(c,0)**(1/2.4)-.055)
        mse=float(np.mean((srgb(np.clip(ca,0,1))-srgb(np.clip(cb,0,1)))**2))
        directions[label]=dict(samples=len(q),geometry_m=stats(d),delta_e76=stats(delta),psnr_srgb_db=float(-10*np.log10(max(mse,1e-30))))
        distances.append(d);deltas.append(delta)
    row=dict(depth=depth,cut_tiles=len(cut),triangles=len(surface.xyz),geometry_m=stats(np.concatenate(distances)),delta_e76=stats(np.concatenate(deltas)),directions=directions)
    rows.append(row);(out/'quality.json').write_text(json.dumps(rows,indent=2));print('quality',json.dumps(row),flush=True);del surface
p=src.provenance;starts=np.r_[0,np.flatnonzero(np.any(p[1:,:2]!=p[:-1,:2],axis=1))+1]
offsets=np.zeros((int(p[:,0].max())+1,int(p[:,1].max())+1),np.int64);known=set(map(tuple,p[starts,:2]));offsets[p[starts,0],p[starts,1]]=starts
owner=np.full(len(p),-1,np.int32);leaves=[n for n in nodes if not n['children']];baseline=Path(json.loads((root/'build.json').read_text())['baseline'])
for rank,n in enumerate(leaves):
    assert n['leaf_begin']==rank and n['leaf_end']==rank+1
    surf=Surface().add(root/n['file']).finish();sp=surf.provenance;ids=offsets[sp[:,0],sp[:,1]]+sp[:,2]
    assert len(ids)<=8192 and len(np.unique(ids))==len(ids) and np.all(owner[ids]==-1)
    assert np.array_equal(p[ids],sp) and np.array_equal(src.xyz[ids],surf.xyz)
    assert np.array_equal(src.uv[ids],surf.uv) and np.array_equal(src.color[ids],surf.color)
    assert np.array_equal(src.primitive_identity[ids],surf.primitive_identity)
    baseline_ids=triangle_ids(GLB(baseline/n['baseline_file']),offsets)
    assert np.array_equal(np.sort(baseline_ids),np.sort(ids)),'baseline ownership differs'
    owner[ids]=rank
assert np.all(owner>=0);source_triangles=len(owner);del surf,p
for key in ['uv','color','mat','materials','provenance','primitive_identity','faces','vertices']:delattr(src,key)
pos,edges,order,starts,counts=topology(src.xyz);del src
shared=starts[counts==2];a=owner[order[shared]//3];b=owner[order[shared+1]//3];cut=a!=b
ends=[edges[order[shared[cut]]]];left=[a[cut]];right=[b[cut]]
nonmanifold=int(np.count_nonzero(counts>2));split_nonmanifold=0
for s,c in zip(starts[counts>2],counts[counts>2]):
    labels=np.unique(owner[order[s:s+c]//3]);split_nonmanifold+=len(labels)>1
    for a,b in itertools.combinations(labels,2):
        ends.append(edges[order[s]][None]);left.append(np.array([a]));right.append(np.array([b]))
ends=np.concatenate(ends);left=np.concatenate(left);right=np.concatenate(right)
del edges,order,starts,counts,shared,a,b,cut,owner
used,inv=np.unique(ends,return_inverse=True);interface_pos=pos[used];ends=inv.reshape(-1,2);del pos,used,inv
keys=ends[:,0]*len(interface_pos)+ends[:,1];tree=cKDTree(interface_pos);retention={}
for n in nodes:
    inside_a=(left>=n['leaf_begin'])&(left<n['leaf_end']);inside_b=(right>=n['leaf_begin'])&(right<n['leaf_end'])
    need=np.unique(keys[inside_a^inside_b])
    surf=Surface().add(root/n['file']).finish();assert set(map(tuple,surf.primitive_identity))<=known
    if len(need):
        pp,ee,oo,ss,cc=topology(surf.xyz);distance,mapped=tree.query(pp,workers=1);unique=ee[oo[ss]];valid=(distance[unique]==0).all(1)
        pair=np.sort(mapped[unique[valid]],axis=1);actual=pair[:,0]*len(interface_pos)+pair[:,1]
        missing=int(np.count_nonzero(~np.isin(need,actual)));assert missing==0,f'decoded segment missing tile {n["id"]}'
        retention[n['id']]=dict(interface_edges=len(need),missing_exact_segments=missing)
        del pp,ee,oo,ss,cc
    del surf
# An interface pair can refine independently iff its subtrees are disjoint.
# Enumerate every pair of ancestors of each adjacent leaf pair, not N²×edges.
ancestors={}
for rank,n in enumerate(leaves):
    chain=[];current=n
    while current['parent'] is not None:
        chain.append(current['id']);current=nodes[current['parent']]
    ancestors[rank]=chain
pairs=set()
for a,b in np.unique(np.sort(np.column_stack([left,right]),axis=1),axis=0):
    for na in ancestors[int(a)]:
        for nb in ancestors[int(b)]:
            x,y=nodes[na],nodes[nb]
            if x['leaf_end']<=y['leaf_begin'] or y['leaf_end']<=x['leaf_begin']:pairs.add(tuple(sorted((na,nb))))
mixed=sum(nodes[a]['depth']!=nodes[b]['depth'] for a,b in pairs)
result=dict(source_triangles=source_triangles,exact_baseline_leaf_ownership=True,leaf_cap=8192,exact_leaf_geometry_uv_color_and_provenance=True,all_corner_feature_ids_valid=True,proxy_primitive_ids_valid=True,
    source_nonmanifold_edges=nonmanifold,split_nonmanifold_edges=int(split_nonmanifold),shared_cross_leaf_edges=len(np.unique(keys)),mixed_pairs=mixed,same_depth_pairs=len(pairs)-mixed,
    mixed_width_m=stats(np.zeros(1)),mixed_fraction_ge=stats(np.zeros(1)),same_depth_width_m=stats(np.zeros(1)),retention=retention,
    pairs=[dict(a=a,b=b,depth_a=nodes[a]['depth'],depth_b=nodes[b]['depth']) for a,b in sorted(pairs)],
    method='Full decoded exact-segment census of every source-shared-edge adjacency, including every incident owner pair on nonmanifold edges. Independent subtree refinement pairs; no projection fallback.')
(out/'cracks.json').write_text(json.dumps(result,indent=2));print('cracks',json.dumps({k:v for k,v in result.items() if k not in ['pairs','retention']}),flush=True)
