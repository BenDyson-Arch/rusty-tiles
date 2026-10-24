#!/usr/bin/env python3
"""Literal exact-key D1 witnesses, independent of producer incidence or reports.

Only f32/+0 point keys and actual unordered triangle edges enter expectations.
Source-open edges are counted, never inferred as chords or T subsegments.
The above-cap witness has 8,193 coarse/source faces, two leaves (8,192 + 1),
and one source_primitive row; it does not exercise producer simplification.
"""
import argparse
import collections
import hashlib
import json
from pathlib import Path
import struct
import subprocess

import controls

A, B, C = [0,0,0], [2,0,0], [0,2,0]
D, E, F = [0,-2,0], [0,0,2], [0,0,-2]
T, DETOUR = [1,0,0], [.5,-.5,0]


def payload(triangles, *, leaf, first=0):
    """One core primitive and one primitive identity row, with literal bytes."""
    binary=bytearray();views=[];accessors=[]
    def view(data):
        binary.extend(bytes(-len(binary)%8))
        index=len(views)
        views.append(dict(buffer=0,byteOffset=len(binary),byteLength=len(data)))
        binary.extend(data)
        return index
    def acc(values,kind,width):
        values=list(values)
        data=struct.pack('<'+('f' if kind==5126 else 'I')*len(values),*values)
        accessors.append(dict(bufferView=view(data),componentType=kind,
            type={1:'SCALAR',2:'VEC2',3:'VEC3'}[width],count=len(values)//width))
        return len(accessors)-1
    count=len(triangles);vertices=[p for triangle in triangles for p in triangle]
    attrs=dict(POSITION=acc((x for p in vertices for x in p),5126,3),
        NORMAL=acc((x for _ in triangles for n in controls.NORMALS for x in n),5126,3),
        TEXCOORD_0=acc((x for _ in triangles for uv in controls.UVS for x in uv),5126,2),
        _FEATURE_ID_0=acc((0 for _ in vertices),5126,1))
    position=accessors[attrs['POSITION']]
    position.update(min=[min(p[k] for p in vertices) for k in range(3)],
                    max=[max(p[k] for p in vertices) for k in range(3)])
    indices=acc(range(len(vertices)),5125,1)
    classes={};tables=[];sets=[]
    for slot,label in enumerate(('source_primitive','source_triangle') if leaf else ('source_primitive',)):
        rows=count if slot else 1
        columns={key:[0]*rows for key in ('node_index','mesh_index','primitive_index','expanded_ordinal','material_index')}
        if slot:
            attrs['_FEATURE_ID_1']=acc((rank for rank in range(count) for _ in range(3)),5126,1)
            columns['triangle_index']=list(range(first,first+count))
            for corner in range(3):columns[f'vertex_{corner}']=[i*3+corner for i in range(first,first+count)]
        schema={};properties={}
        for key,values in columns.items():
            wide=key in ('expanded_ordinal','triangle_index','vertex_0','vertex_1','vertex_2')
            schema[key]=dict(type='SCALAR',componentType='UINT64' if wide else 'UINT32',required=True)
            properties[key]=dict(values=view(struct.pack('<'+('Q' if wide else 'I')*rows,*values)))
        classes[label]=dict(properties=schema)
        tables.append(dict(count=rows,**{'class':label},properties=properties))
        sets.append(dict(featureCount=rows,attribute=slot,propertyTable=slot,label=label))
    primitive=dict(attributes=attrs,indices=indices,material=0,
        extensions={'EXT_mesh_features':{'featureIds':sets}})
    doc=dict(asset={'version':'2.0'},scene=0,scenes=[{'nodes':[0]}],nodes=[{'mesh':0}],
        meshes=[{'primitives':[primitive]}],materials=[controls.MATERIALS[0]],
        accessors=accessors,bufferViews=views,buffers=[{'byteLength':len(binary)}],
        extensionsUsed=['EXT_mesh_features','EXT_structural_metadata'],
        extensions={'EXT_structural_metadata':{'schema':{'classes':classes},'propertyTables':tables}})
    text=json.dumps(doc,separators=(',',':')).encode();text+=b' '*(-len(text)%4)
    binary.extend(bytes(-len(binary)%4))
    return b'glTF'+struct.pack('<III',2,28+len(text)+len(binary),len(text))+b'JSON'+text+struct.pack('<I',len(binary))+b'BIN\0'+binary


def source_incidence(groups):
    """Small independent literal reference; never reads decoded oracle tables."""
    points=collections.defaultdict(set);edges=collections.Counter();owners=collections.defaultdict(set)
    for leaf,triangles in enumerate(groups):
        for triangle in triangles:
            captured=[struct.unpack('<3f',struct.pack('<3f',*p)) for p in triangle]
            keys=[struct.pack('<3f',*(0. if x==0 else x for x in p)) for p in captured]
            if len(set(keys))<3:continue
            for key in keys:points[key].add(leaf)
            for k in range(3):
                edge=tuple(sorted((keys[k],keys[(k+1)%3])))
                edges[edge]+=1;owners[edge].add(leaf)
    opened=[edge for edge,count in edges.items() if count==1]
    return dict(sharedPoints=sum(len(v)>1 for v in points.values()),
        sharedSegments=sum(len(v)>1 for v in owners.values()),sourceOpenEdges=len(opened),
        sourceOpenEdgesWithSharedEndpoints=sum(all(points[p]-owners[edge] for p in edge) for edge in opened))


def tile(uri=None,children=None):
    result=dict(boundingVolume={'box':[20000,0,0,40000,0,0,0,40000,0,0,0,40000]},geometricError=0,refine='REPLACE')
    if uri:result['content']={'uri':uri}
    if children:result['children']=children
    return result


def generate(root):
    root=Path(root);root.mkdir(parents=True,exist_ok=True)
    detour=[[A,DETOUR,C],[DETOUR,B,C]]
    witnesses=[
        ('exact-shared-edge',[[A,B,C]],[[B,A,D]],[[A,B,C]],None),
        ('welded-shared-segment-detour',[[A,B,C]],[[B,A,D]],detour,'interfaceSegments'),
        ('chord-not-shared',[[A,B,C]],[[A,D,E],[B,F,[3,0,-2]]],detour,None),
        ('incidence-one-t-exempt',[[A,B,C]],[[A,T,D],[T,B,D]],detour,None),
        ('nonmanifold-three-retained',[[A,B,C],[B,A,D]],[[A,B,E]],[[A,B,C],[B,A,D]],None),
        ('nonmanifold-three-detour',[[A,B,C],[B,A,D]],[[A,B,E]],[[A,D,C],[B,C,D]],'interfaceSegments'),
        ('canonical-degenerate-zero-items',[[[-0.,0,0],A,B]],[[A,B,B]],[[A,A,B]],None),
    ]
    cases=[]
    for name,left,right,coarse,check in witnesses:
        original=left+right;folder=root/name;folder.mkdir(exist_ok=True)
        files={'coarse.glb':payload(coarse,leaf=False),
               'left.glb':payload(left,leaf=True),'right.glb':payload(right,leaf=True,first=len(left))}
        document=dict(asset={'version':'1.1'},geometricError=0,
            root=tile(children=[tile('coarse.glb',[tile('left.glb')]),tile('right.glb')]))
        files['tileset.json']=json.dumps(document,separators=(',',':')).encode()
        for member,data in files.items():(folder/member).write_bytes(data)
        source=root/(name+'-source.json')
        source.write_text(json.dumps({'triangles':{f'0,0,0,{i}':dict(xyz=triangle,
            vertices=[i*3+k for k in range(3)],expanded_ordinal=0,material_index=0,
            material=controls.MATERIALS[0],normals=controls.NORMALS,uvs=controls.UVS)
            for i,triangle in enumerate(original)}},separators=(',',':'))+'\n')
        cases.append(dict(name=name,oracleCheck=check,archiveCheck=check,
            source=source.name,incidence=source_incidence([left,right])))
    # The admission limit concerns primitive-table rows, not triangle count.
    # Distinct separated triangles keep nearest-reference work bounded by the
    # R-tree rather than creating an 8,193-by-8,193 coincident query product.
    name='above-8192-positive';folder=root/name;folder.mkdir(exist_ok=True)
    original=[[[i*4,0,0],[i*4+1,0,0],[i*4,1,0]] for i in range(8193)]
    (folder/'coarse.glb').write_bytes(payload(original,leaf=False))
    (folder/'left.glb').write_bytes(payload(original[:8192],leaf=True))
    (folder/'right.glb').write_bytes(payload(original[8192:],leaf=True,first=8192))
    (folder/'tileset.json').write_text(json.dumps(dict(asset={'version':'1.1'},geometricError=0,
        root=tile('coarse.glb',[tile('left.glb'),tile('right.glb')])),separators=(',',':')))
    source=root/(name+'-source.json')
    source.write_text(json.dumps({'triangles':{f'0,0,0,{i}':dict(xyz=triangle,
        vertices=[i*3+k for k in range(3)],expanded_ordinal=0,material_index=0,
        material=controls.MATERIALS[0],normals=controls.NORMALS,uvs=controls.UVS)
        for i,triangle in enumerate(original)}},separators=(',',':'))+'\n')
    cases.append(dict(name=name,oracleCheck=None,archiveCheck=None,source=source.name,
        incidence=source_incidence([original[:8192],original[8192:]]),coarseTriangles=8193))
    (root/'cases.json').write_text(json.dumps(cases,indent=2)+'\n')
    return cases


def run(root,binary=None):
    from oracle import audit,Failure
    root=Path(root);cases=generate(root);results=[]
    if binary:
        binary=Path(binary).resolve(strict=True)
        digest=hashlib.sha256(binary.read_bytes()).hexdigest()
    for case in cases:
        folder=root/case['name']
        try:
            result=audit(folder,root/case['source'],scratch=root,work_limit=2_000_000)
            assert case['oracleCheck'] is None,(case,result)
            for key,value in case['incidence'].items():assert result[key]==value,(case['name'],key,result[key],value)
            if 'coarseTriangles' in case:
                assert result['representatives']==case['coarseTriangles'],result
                assert result['aboveCapCoarseTiles']==1 and result['maxCoarseTriangles']==8193,result
            verdict='pass'
        except Failure as error:
            assert error.check==case['oracleCheck'],(case,error)
            verdict=error.check
        if binary:
            archive=folder.with_suffix('.3tz')
            packed=subprocess.run([str(binary),'convert','-i',str(folder),'-o',str(archive),'--force','--json'],capture_output=True,timeout=30)
            assert packed.returncode==0,(case['name'],packed.stdout[-2000:],packed.stderr[-2000:])
            checked=subprocess.run([str(binary),'validate',str(archive),'--json'],capture_output=True,timeout=30)
            report=json.loads(checked.stdout)
            if case['archiveCheck']:
                assert checked.returncode==3 and case['archiveCheck'] in report['error']['message'],(case['name'],report)
            else:
                assert checked.returncode==0,(case['name'],report)
                census=report['interfaceCensus']
                for key,value in case['incidence'].items():assert census[key]==value,(case['name'],key,census[key],value)
                if 'coarseTriangles' in case:
                    assert census['stoppedAboveCap']==1 and census['maxAboveCapTriangles']==8193,(case['name'],census)
        results.append(dict(name=case['name'],oracle=verdict,sourceIncidence=case['incidence'],
            native=('pass' if case['archiveCheck'] is None else case['archiveCheck']) if binary else 'not run'))
    if binary:assert hashlib.sha256(binary.read_bytes()).hexdigest()==digest,'binary changed'
    return results


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('outputdir',type=Path);parser.add_argument('--binary',type=Path)
    args=parser.parse_args()
    print(json.dumps(run(args.outputdir,args.binary),indent=2))
