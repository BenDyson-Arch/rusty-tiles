"""Read production leaf provenance and expanded hierarchy, without quantized geometry."""
import json,sys
from pathlib import Path
import numpy as np
from build import GLB
from baseline_tree import tree


def triangle_ids(g, offsets, counts=None):
    meta=g.j['extensions']['EXT_structural_metadata'];result=[]
    for mesh in g.j['meshes']:
        for pr in mesh['primitives']:
            fid=next(f for f in pr['extensions']['EXT_mesh_features']['featureIds'] if f.get('label')=='source_triangle')
            t=meta['propertyTables'][fid['propertyTable']];cls=meta['schema']['classes'][t['class']]
            cols={name:np.frombuffer(g.view(t['properties'][name]['values']),{'UINT32':'<u4','UINT64':'<u8'}[cls['properties'][name]['componentType']]).astype(np.int64) for name in ['mesh_index','primitive_index','triangle_index']}
            # Metadata tables may be referenced in a different triangle order.
            a=g.acc(pr['attributes']['_FEATURE_ID_'+str(fid['attribute'])]).reshape(-1)
            corners=a[g.acc(pr['indices']).reshape(-1,3)] if 'indices' in pr else a.reshape(-1,3)
            assert np.all(corners==corners[:,0,None]) and np.all(corners==np.floor(corners))
            rows=corners[:,0].astype(np.int64);assert rows.min()>=0 and rows.max()<t['count']
            mi=cols['mesh_index'][rows];pi=cols['primitive_index'][rows];ti=cols['triangle_index'][rows]
            assert np.all((mi>=0)&(mi<offsets.shape[0])) and np.all((pi>=0)&(pi<offsets.shape[1]))
            assert np.all(offsets[mi,pi]>=0) and np.all(ti>=0)
            if counts is not None:assert np.all(ti<counts[mi,pi])
            result.append(offsets[mi,pi]+ti)
    return np.concatenate(result)


def partition(g, baseline):
    baseline=Path(baseline);tt=tree(str(baseline));children={f:[] for f in tt}
    roots=[]
    for f,n in tt.items():
        if n['parent'] is None:roots.append(f)
        else:children[n['parent']].append(f)
    assert len(roots)==1
    offsets=np.full((len(g.j['meshes']),max(len(m['primitives']) for m in g.j['meshes'])),-1,np.int64);count=0
    for mi,m in enumerate(g.j['meshes']):
        for pi,p in enumerate(m['primitives']):offsets[mi,pi]=count;count+=g.j['accessors'][p['indices']]['count']//3
    counts=np.zeros_like(offsets)
    for mi,m in enumerate(g.j['meshes']):
        for pi,p in enumerate(m['primitives']):counts[mi,pi]=g.j['accessors'][p['indices']]['count']//3
    owner=np.full(count,-1,np.int32);nodes=[];rank=0
    def visit(file,parent):
        nonlocal rank
        nid=len(nodes);n=dict(id=nid,file=f'content/{nid}.glb',baseline_file=file,parent=parent,depth=tt[file]['depth'],children=[],leaf_begin=rank,baseline_box=tt[file]['box'])
        nodes.append(n)
        for f in children[file]:n['children'].append(visit(f,nid))
        if not n['children']:
            ids=triangle_ids(GLB(baseline/file),offsets,counts)
            assert 0<len(ids)<=8192 and len(np.unique(ids))==len(ids)
            assert np.all((ids>=0)&(ids<count)) and np.all(owner[ids]<0)
            owner[ids]=rank;rank+=1
        n['leaf_end']=rank
        return nid
    visit(roots[0],None);assert np.all(owner>=0)
    assert rank=={'tex3':156,'m2048':1959}[baseline.parent.name]
    return nodes,owner
