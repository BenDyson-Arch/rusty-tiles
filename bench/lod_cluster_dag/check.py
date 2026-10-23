#!/usr/bin/env python3
"""Focused structural and independent numeric controls; no production claims."""
import json
from pathlib import Path
import sys
import numpy as np
import point_cloud_utils as pcu
from build import GLB

root=Path(sys.argv[1]);src=GLB(sys.argv[2]);report=Path(sys.argv[3])
tree=json.loads((root/'tileset.json').read_text());assert tree['asset']['version']=='1.1'
counts=dict(tiles=0,leaves=0,exact_source_attributes=0,contained_vertices=0)
def bounds(box):
    c=np.array(box[:3]);h=np.array([box[3],box[7],box[11]]);return c-h,c+h
def walk(n,parent=None):
    assert n['refine']=='REPLACE';lo,hi=bounds(n['boundingVolume']['box'])
    if parent:
        plo,phi=bounds(parent['boundingVolume']['box']);assert np.all(lo>=plo-1e-10) and np.all(hi<=phi+1e-10)
        assert n['geometricError']<=parent['geometricError']
    g=GLB(root/n['content']['uri']);counts['tiles']+=1
    if not n.get('children'):counts['leaves']+=1
    for p in g.j['meshes'][0]['primitives']:
        xyz=g.acc(p['attributes']['POSITION']).astype(np.float64)[:,[0,2,1]]*[1,-1,1]
        assert np.all(xyz>=lo-1e-10) and np.all(xyz<=hi+1e-10);counts['contained_vertices']+=len(xyz)
        if n.get('children'):continue
        fid=next(f for f in p['extensions']['EXT_mesh_features']['featureIds'] if f['label']=='source_triangle')
        table=g.j['extensions']['EXT_structural_metadata']['propertyTables'][fid['propertyTable']]
        cols={k:np.frombuffer(g.view(v['values']),'<u4') for k,v in table['properties'].items()}
        mi=int(cols['mesh_index'][0]);pi=int(cols['primitive_index'][0]);sp=src.j['meshes'][mi]['primitives'][pi]
        assert np.all(cols['mesh_index']==mi) and np.all(cols['primitive_index']==pi)
        original=src.acc(sp['indices']).reshape(-1,3)[cols['triangle_index']].reshape(-1)
        for name,ai in sp['attributes'].items():
            assert np.array_equal(src.acc(ai)[original],g.acc(p['attributes'][name])), name
            counts['exact_source_attributes']+=1
    for c in n.get('children',[]):walk(c,n)
walk(tree['root'])
# Independent analytic nearest-segment reference, including PCU's degenerate-triangle API.
rng=np.random.default_rng(7);v=rng.random((32,3));e=np.arange(32).reshape(-1,2)
f=np.column_stack([e,e[:,1]]).astype(np.int32);q=rng.random((100,3));_,fi,_=pcu.closest_points_on_mesh(q,v,f)
a=v[e[:,0]];ab=v[e[:,1]]-a;t=np.clip(((q[:,None]-a[None])*ab[None]).sum(2)/(ab*ab).sum(1),0,1)
p=a[None]+t[:,:,None]*ab[None];d=np.linalg.norm(p-q[:,None],axis=2)
err=float(np.max(np.abs(d[np.arange(len(q)),fi]-d.min(1))));assert err<1e-12
counts['analytic_segment_control_max_error']=err
report.write_text(json.dumps(counts,indent=2));print(counts)
