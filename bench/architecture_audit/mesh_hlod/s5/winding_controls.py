#!/usr/bin/env python3
"""Permanent literal witnesses for the winding oracle's bounded proof scope."""
import json
from pathlib import Path
import sys
import tempfile

from oracle import Failure,require,area_vector,distance2
from spatial import Store
from winding import inspect


def probe(source,coarse):
    with tempfile.TemporaryDirectory() as temporary:
        store=Store(Path(temporary)/'w.sqlite')
        try:
            for i,xyz in enumerate(source):
                store.source_face(f'{i},0,0,0',{'xyz':xyz},i,actual={'xyz':xyz})
            store.validate_leaf_points(require)
            store.put_faces(0,({'xyz':xyz} for xyz in coarse))
            return inspect(store,[dict(id='0',children=[{}],rendered=True,lo=0,hi=len(source))],require)
        finally:
            store.close()


def run():
    triangle=[[0,0,0],[2,0,0],[0,2,0]]
    results=[]
    positive=probe([triangle],[triangle])
    assert positive['checkedBorders']==3 and positive['coincidentFaces']==1
    results.append(dict(name='root-border-aligned',ok=True))
    try:
        probe([triangle],[[triangle[0],triangle[2],triangle[1]]])
    except Failure as e:
        assert e.check=='interfaceWinding'
        results.append(dict(name='root-border-reversed',ok=False,check=e.check))
    else:raise AssertionError('reversed root border accepted')
    # Two nearby opposing sheets demonstrate why the nearest normal cannot
    # certify the orientation of a newly simplified triangle between them.
    folded=[[[.5,.5,1/64],[.5,1,1/64],[1,.5,1/64]],
            [[0,0,-1/32],[2,0,-1/32],[0,2,-1/32]]]
    centroid=[sum(p[k] for p in triangle)/3 for k in range(3)]
    nearest=min(folded,key=lambda f:distance2(centroid,f))
    assert sum(a*b for a,b in zip(area_vector(triangle),area_vector(nearest)))<0
    result=probe(folded,[triangle])
    assert result['uninspectedInteriorOrientation']==1 and result['checkedBorders']==0
    results.append(dict(name='opposing-nearest-normal-uninspected',ok=True,
                        uninspected=result['uninspectedInteriorOrientation']))
    tetra=[[[0,0,0],[0,1,0],[1,0,0]],[[0,0,0],[1,0,0],[0,0,1]],
           [[0,0,0],[0,0,1],[0,1,0]],[[1,0,0],[0,1,0],[0,0,1]]]
    result=probe(tetra,tetra)
    assert result['borderedEdges']==0 and result['coincidentFaces']==4
    flipped=[*tetra];flipped[0]=[tetra[0][0],tetra[0][2],tetra[0][1]]
    try:probe(tetra,flipped)
    except Failure as e:
        assert e.check=='interfaceWinding'
        results.append(dict(name='closed-source-coincident-reversed',ok=False,check=e.check))
    else:raise AssertionError('reversed exact closed-source face accepted')
    return results


if __name__=='__main__':
    print(json.dumps(run(),indent=2))
