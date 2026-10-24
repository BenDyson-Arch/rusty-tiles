#!/usr/bin/env python3
"""Permanent source-point lattice controls; literal bytes, no producer facts.

Fine-before-coarse leaves share a source point across distinct primitives and
corner references. Its canonical position uses the coarser incident grid. An
unrelated still-coarser leaf cannot enlarge that source point's error bound.
Run under the caller's resource scope, e.g. python3 lattice_controls.py SCRATCH.
"""
import json
from pathlib import Path
import struct
import sys

import controls
from oracle import audit,Failure


def payload(identity,step,code=0,*,leaf=True,shift=0):
    points=[[(code,0,0),(0,2,0),(0,0,2)],
                     [(0,0,0),(0,-2,0),(0,0,-2)],
                     [(100,0,0),(100,1,0),(100,0,1)]]
    raw=controls.glb([identity],quantized=True,points=points,leaf=leaf)
    length=struct.unpack_from('<I',raw,12)[0]
    doc=json.loads(raw[20:20+length]);binary=raw[28+length:]
    doc['nodes'][0]={'children':[1]}
    doc['nodes'][1]={'mesh':0,'scale':[step]*3}
    if shift:doc['nodes'][1]['translation']=[shift,0,0]
    encoded=json.dumps(doc,separators=(',',':')).encode();encoded+=b' '*(-len(encoded)%4)
    return b'glTF'+struct.pack('<III',2,28+len(encoded)+len(binary),len(encoded))+b'JSON'+encoded+struct.pack('<I',len(binary))+b'BIN\0'+binary


def run(root):
    root=Path(root);root.mkdir(parents=True,exist_ok=True)
    source={'triangles':{
        '0,0,0,0':dict(xyz=[[.0625,0,0],[0,.125,0],[0,0,.125]],vertices=[0,1,2],expanded_ordinal=0,material_index=0,material=controls.MATERIALS[0]),
        '0,0,1,0':dict(xyz=[[.0625,0,0],[0,-.25,0],[0,0,-.25]],vertices=[0,1,2],expanded_ordinal=1,material_index=1,material=controls.MATERIALS[1]),
        '0,0,2,0':dict(xyz=[[100,0,0],[100,1,0],[100,0,1]],vertices=[0,1,2],expanded_ordinal=2,material_index=0,material=controls.MATERIALS[0])}}
    source_path=root/'source.json';source_path.write_text(json.dumps(source))
    results=[]
    for name,order,code,want in (
        ('fine-before-coarse',[0,1,2],0,None),
        ('coarse-before-fine',[1,0,2],0,None),
        ('one-code-beyond-incident',[0,1,2],3,'leafGeometry'),
        ('within-bound-seam-disagreement',[0,1,2],1,'interfacePoints')):
        out=root/name;out.mkdir(exist_ok=True)
        for i,step in enumerate((.0625,.125,1.)):
            (out/f'{i}.glb').write_bytes(payload(i,step,code if i==0 else 0))
        doc=dict(asset={'version':'1.1'},geometricError=1,root=dict(geometricError=1,
            children=[dict(geometricError=0,content={'uri':f'{i}.glb'}) for i in order]))
        (out/'tileset.json').write_text(json.dumps(doc))
        try:
            result=audit(out,source_path)
            assert want is None,(name,result)
        except Failure as e:
            assert e.check==want,(name,want,e)
            result=dict(ok=False,check=e.check)
        results.append(dict(name=name,**result))
    for name,shift,want in (('coarse-canonical-aligned',0,None),
                            ('coarse-sub-code-gap',.015625,'interfacePoints')):
        out=root/name;out.mkdir(exist_ok=True)
        for i,step in enumerate((.0625,.125,1.)):
            (out/f'{i}.glb').write_bytes(payload(i,step))
        (out/'parent.glb').write_bytes(payload(0,.0625,leaf=False,shift=shift))
        leaves=[dict(geometricError=0,content={'uri':f'{i}.glb'}) for i in range(3)]
        doc=dict(asset={'version':'1.1'},geometricError=1,root=dict(geometricError=1,
            children=[dict(geometricError=.5,content={'uri':'parent.glb'},children=[leaves[0]]),*leaves[1:]]))
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
