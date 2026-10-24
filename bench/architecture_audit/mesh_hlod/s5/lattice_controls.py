#!/usr/bin/env python3
"""Permanent source-point lattice controls; literal bytes, no producer facts.

Fine-before-coarse leaves share a source point across distinct primitives and
corner references. Its canonical position uses the coarser incident grid. An
unrelated still-coarser leaf cannot enlarge that source point's error bound.
Run under the caller's resource scope, e.g. python3 lattice_controls.py SCRATCH.
"""
import copy
import json
from pathlib import Path
import struct
import sys

import controls
from oracle import audit,Failure


def payload(identity,step,code=0,*,leaf=True,shift=0,float_storage=False,source_exact=False,unshared_code=0):
    points=[[(code,0,0),(0,2+unshared_code,0),(0,0,2)],
                     [(0,0,0),(0,-2,0),(0,0,-2)],
                     [(100/step,0,0),(100/step,1/step,0),(100/step,0,1/step)]]
    if source_exact and identity in (0,1):
        points[identity][0]=(.0625/step,0,0)
    # FLOAT fallbacks retain the same quantization node, but do not declare an
    # integer grid of their own. A source-equal FLOAT delivery is also valid.
    literal=[[tuple(int(v) for v in p) for p in triangle] for triangle in points]
    raw=controls.glb([identity],quantized=True,points=literal,leaf=leaf)
    length=struct.unpack_from('<I',raw,12)[0]
    doc=json.loads(raw[20:20+length]);binary=bytearray(raw[28+length:])
    if float_storage:
        position=doc['accessors'][doc['meshes'][0]['primitives'][0]['attributes']['POSITION']]
        view=doc['bufferViews'][position['bufferView']]
        binary+=bytes(-len(binary)%4)
        view.update(byteOffset=len(binary),byteLength=36,byteStride=12)
        binary+=struct.pack('<9f',*(v for p in points[identity] for v in p))
        position['componentType']=5126
        position['min']=[min(p[k] for p in points[identity]) for k in range(3)]
        position['max']=[max(p[k] for p in points[identity]) for k in range(3)]
        doc['buffers'][0]['byteLength']=len(binary)
    doc['nodes'][0]={'children':[1]}
    doc['nodes'][1]={'mesh':0,'scale':[step]*3}
    if shift:doc['nodes'][1]['translation']=[shift,0,0]
    encoded=json.dumps(doc,separators=(',',':')).encode();encoded+=b' '*(-len(encoded)%4)
    return b'glTF'+struct.pack('<III',2,28+len(encoded)+len(binary),len(encoded))+b'JSON'+encoded+struct.pack('<I',len(binary))+b'BIN\0'+binary


def hierarchy(order,parent=False):
    """Observed ladder: unrelated depth1/.25, shared depth2/.125 + depth3/.0625."""
    fine=dict(geometricError=.25,children=[dict(geometricError=0,content={'uri':'0.glb'})])
    if parent:fine['content']={'uri':'parent.glb'}
    coarse=dict(geometricError=0,content={'uri':'1.glb'})
    branch=dict(geometricError=.5,children=[fine,coarse] if order[0]==0 else [coarse,fine])
    return dict(asset={'version':'1.1'},geometricError=1,
        root=dict(geometricError=1,children=[branch,dict(geometricError=0,content={'uri':'2.glb'})]))


def run(root):
    root=Path(root);root.mkdir(parents=True,exist_ok=True)
    source={'triangles':{
        '0,0,0,0':dict(xyz=[[.0625,0,0],[0,.125,0],[0,0,.125]],vertices=[0,1,2],expanded_ordinal=0,material_index=0,material=controls.MATERIALS[0]),
        '0,0,1,0':dict(xyz=[[.0625,0,0],[0,-.25,0],[0,0,-.25]],vertices=[0,1,2],expanded_ordinal=1,material_index=1,material=controls.MATERIALS[1]),
        '0,0,2,0':dict(xyz=[[100,0,0],[100,1,0],[100,0,1]],vertices=[0,1,2],expanded_ordinal=2,material_index=0,material=controls.MATERIALS[0])}}
    source_path=root/'source.json';source_path.write_text(json.dumps(source))
    unshared_source=copy.deepcopy(source)
    # 2.5 grid codes rounds ties-to-even to code 2. Delivering code 3 is
    # within the scalar half-step fidelity bound, but violates the known
    # canonical formula at an unshared point, without any raw contradiction.
    unshared_source['triangles']['0,0,0,0']['xyz'][1]=[0,.15625,0]
    unshared_path=root/'unshared-source.json';unshared_path.write_text(json.dumps(unshared_source))
    results=[]
    for name,order,code,want,float_ids,source_exact in (
        ('fine-before-coarse',[0,1,2],0,None,(),False),
        ('coarse-before-fine',[1,0,2],0,None,(),False),
        ('one-code-beyond-incident',[0,1,2],0,'leafGeometry',(),False),
        ('within-bound-seam-disagreement',[0,1,2],1,'interfacePoints',(),False),
        ('fine-float-fallback-known-incident-grid',[0,1,2],0,None,(0,),False),
        ('min-depth-float-fallback-without-grid',[0,1,2],0,'oracleDomain',(1,),False),
        ('all-float-source-capture',[0,1,2],0,None,(0,1,2),True)):
        out=root/name;out.mkdir(exist_ok=True)
        for i,step in enumerate((.0625,.125,.25)):
            (out/f'{i}.glb').write_bytes(payload(i,step,code if i==0 else 0,float_storage=i in float_ids,source_exact=source_exact,
                unshared_code=int(i==0 and name=='one-code-beyond-incident')))
        doc=hierarchy(order)
        (out/'tileset.json').write_text(json.dumps(doc))
        try:
            result=audit(out,unshared_path if name=='one-code-beyond-incident' else source_path)
            assert want is None,(name,result)
        except Failure as e:
            assert e.check==want,(name,want,e)
            if name in ('one-code-beyond-incident','min-depth-float-fallback-without-grid'):
                assert 'sourceCanonical' in e.detail,(name,e)
            result=dict(ok=False,check=e.check)
        results.append(dict(name=name,**result))
    for name,shift,want in (('coarse-canonical-aligned',0,None),
                            ('coarse-sub-code-gap',.015625,'interfacePoints')):
        out=root/name;out.mkdir(exist_ok=True)
        for i,step in enumerate((.0625,.125,.25)):
            (out/f'{i}.glb').write_bytes(payload(i,step))
        (out/'parent.glb').write_bytes(payload(0,.0625,leaf=False,shift=shift))
        doc=hierarchy([0,1,2],parent=True)
        (out/'tileset.json').write_text(json.dumps(doc))
        try:
            result=audit(out,source_path)
            assert want is None,(name,result)
        except Failure as e:
            assert e.check==want,(name,want,e)
            result=dict(ok=False,check=e.check)
        results.append(dict(name=name,**result))
    return results


if __name__=='__main__':
    print(json.dumps(run(sys.argv[1]),indent=2))
