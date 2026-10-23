"""Same spatial meshlet partition as build.py, without writing unused UV proxies."""
import numpy as np


def partition(g, mo):
    clusters=[];offset=0
    for mesh in g.j['meshes']:
        for pr in mesh['primitives']:
            assert pr.get('mode',4)==4
            pos=g.acc(pr['attributes']['POSITION']);idx=g.acc(pr['indices']).astype(np.uint32).reshape(-1,3)
            keys={tuple(sorted(map(int,t))):i for i,t in enumerate(idx)}
            assert len(keys)==len(idx),'duplicate triangles need multimap provenance'
            for c in mo.clusters(idx,pos):
                ids=np.array([keys[tuple(sorted(map(int,t)))]+offset for t in c],np.int64)
                clusters.append(dict(ids=ids,center=pos[c].mean((0,1))))
            offset+=len(idx)
    owner=np.full(offset,-1,np.int32);nodes=[];leaf_bit=0
    def split(cs,depth,parent):
        nonlocal leaf_bit
        nid=len(nodes);node=dict(id=nid,depth=depth,parent=parent,children=[],file=f'content/{nid}.glb');nodes.append(node)
        if depth<5 and len(cs)>1:
            centers=np.array([clusters[c]['center'] for c in cs]);axis=np.ptp(centers,0).argmax()
            order=np.argsort(centers[:,axis],kind='stable');mid=len(cs)//2
            node['children']=[split([cs[i] for i in order[:mid]],depth+1,nid),split([cs[i] for i in order[mid:]],depth+1,nid)]
            node['mask']=sum(nodes[c]['mask'] for c in node['children'])
        else:
            for c in cs:
                ids=clusters[c]['ids'];assert np.all(owner[ids]==-1);owner[ids]=nid
            node['mask']=1<<leaf_bit;leaf_bit+=1
        return nid
    split(list(range(len(clusters))),0,None)
    assert np.all(owner>=0) and leaf_bit<=64
    return nodes,owner
