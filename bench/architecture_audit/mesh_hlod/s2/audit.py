"""Independent decoded cut audit; numpy + point_cloud_utils, seed differs from producer.
Usage: python audit.py DECODED_DIR OUT_JSON
Run decode_geometry.mjs first. No producer nearest/sampling/decoder code is used.
"""
import hashlib, json, sys
from pathlib import Path
import numpy as np
import point_cloud_utils as pcu
root=Path(sys.argv[1]);rows=json.loads((root/'index.json').read_text())
def mesh(row): return np.fromfile(root/row['file'],dtype='<f8').reshape(-1,3,3)
def finish(xyz):
    xyz=np.ascontiguousarray(xyz);v=xyz.reshape(-1,3);f=np.arange(len(v),dtype=np.int32).reshape(-1,3)
    return xyz,v,f
def sample(xyz,rng):
    area=np.linalg.norm(np.cross(xyz[:,1]-xyz[:,0],xyz[:,2]-xyz[:,0]),axis=1)
    ids=rng.choice(len(area),200000,p=area/area.sum());r=rng.random((200000,2));u=np.sqrt(r[:,0]);w=np.column_stack([1-u,u*(1-r[:,1]),u*r[:,1]])
    return (xyz[ids]*w[:,:,None]).sum(1)
def stats(d):return dict(samples=len(d),mean=float(d.mean()),p95=float(np.quantile(d,.95)),p99=float(np.quantile(d,.99)),max=float(d.max()))
def select_cut(rows, depth):
    # First content at/below the requested cell depth, carrying terminal leaves.
    # Sparse unary cells have no content: refinement must descend to the next tile.
    candidates=sorted((r for r in rows if r['depth']>=depth or r['leaf']),
                      key=lambda r:(r['depth'],r['first']))
    cut=[]
    for row in candidates:
        if not any(r['first']<=row['first'] and row['end']<=r['end'] for r in cut):
            cut.append(row)
    return sorted(cut,key=lambda r:r['first'])
# Sensitive independent controls: unary holes descend; shallow leaves carry forward.
probe=[dict(first=0,end=8,depth=0,leaf=False),dict(first=0,end=4,depth=1,leaf=False),
       dict(first=0,end=2,depth=4,leaf=True),dict(first=2,end=4,depth=4,leaf=True),
       dict(first=4,end=8,depth=1,leaf=True)]
assert [r['depth'] for r in select_cut(probe,0)]==[0]
assert [r['depth'] for r in select_cut(probe,2)]==[4,4,1]
assert all(0 < r["triangles"] <= 8192 for r in rows[1:])
source=finish(mesh(rows[0]));assert np.isfinite(source[0]).all()
source_sha256=hashlib.sha256(source[0].tobytes()).hexdigest();result=[];rng=np.random.default_rng(20261025)
for depth in [0,2,4,6]:
    cut=select_cut(rows[1:],depth);assert cut[0]['first']==0
    assert all(a['end']==b['first'] for a,b in zip(cut,cut[1:]));assert cut[-1]['end']==len(source[0])
    proxy=finish(np.concatenate([mesh(r) for r in cut]));directions={};combined=[]
    for label,a,b in [('source_to_proxy',source,proxy),('proxy_to_source',proxy,source)]:
        q=sample(a[0],rng);d,_,_=pcu.closest_points_on_mesh(np.ascontiguousarray(q),b[1],b[2]);assert np.isfinite(d).all();directions[label]=stats(d);combined.append(d)
    published=max(r.get('geometricError',r.get('geometryError',0)) for r in cut)
    row=dict(depth=depth,tiles=len(cut),triangles=len(proxy[0]),published_max=published,geometry_m=stats(np.concatenate(combined)),directions=directions)
    row['max_exceeds_published']=row['geometry_m']['max']>published
    result.append(row);Path(sys.argv[2]).write_text(json.dumps(dict(seed=20261025,source_geometry_sha256=source_sha256,decoded_members=len(rows)-1,max_member_triangles=max(r["triangles"] for r in rows[1:]),cuts=result),indent=2));print(json.dumps(row),flush=True)
