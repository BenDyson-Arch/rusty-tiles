#!/usr/bin/env python3
"""Independent stdlib F1a fixture/archive oracle. No production loader or validator.

Spec references: glTF 2.0 §§3.5.3,3.7.4,4.4; 3D Tiles 1.1 §§6.7.1.6,
6.7.1.4 and grids. Profile-specific checks follow docs/architecture/f1a-contract.md.
Run --self-test for oracle sensitivity; --fixture PATH to write an analytic GLB;
--source INPUT --archive OUTPUT --leaf-limit N to inspect a real candidate.
"""
import argparse
import copy
import hashlib
import json
import math
from pathlib import Path
import struct
import subprocess
import tempfile
import zipfile


class OracleError(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise OracleError(message)


def finite(values):
    return all(isinstance(v, (int, float)) and math.isfinite(v) for v in values)


def identity():
    return [[float(i == j) for j in range(4)] for i in range(4)]


def matrix(values):
    require(len(values) == 16 and finite(values), 'invalid matrix')
    return [[values[4*j+i] for j in range(4)] for i in range(4)]


def product(a, b):
    return [[sum(a[i][k]*b[k][j] for k in range(4)) for j in range(4)] for i in range(4)]


def point(m, p):
    return tuple(sum(m[i][j]*p[j] for j in range(3))+m[i][3] for i in range(3))


def determinant(m):
    a,b,c=m[0][:3];d,e,f=m[1][:3];g,h,i=m[2][:3]
    return a*(e*i-f*h)-b*(d*i-f*g)+c*(d*h-e*g)


def normal(m, n):
    # Cofactor matrix / determinant is the inverse transpose. This implementation
    # is independent of the Rust converter, and checked against an analytic case.
    a,b,c=m[0][:3];d,e,f=m[1][:3];g,h,i=m[2][:3]
    co=[[e*i-f*h,f*g-d*i,d*h-e*g], [c*h-b*i,a*i-c*g,b*g-a*h], [b*f-c*e,c*d-a*f,a*e-b*d]]
    det=determinant(m);require(det != 0, 'singular normal transform')
    v=[sum(co[row][col]*n[col] for col in range(3))/det for row in range(3)]
    length=math.sqrt(sum(x*x for x in v));require(length > 0 and math.isfinite(length), 'invalid normal')
    return tuple(x/length for x in v)


def node_matrix(node):
    if 'matrix' in node:
        return matrix(node['matrix'])
    x,y,z,w=node.get('rotation',[0,0,0,1])
    length=math.sqrt(x*x+y*y+z*z+w*w)
    require(abs(length*length-1) <= 2.000001e-6,'non-unit quaternion')
    x,y,z,w=(v/length for v in (x,y,z,w))
    r=[[1-2*(y*y+z*z),2*(x*y-z*w),2*(x*z+y*w)],
       [2*(x*y+z*w),1-2*(x*x+z*z),2*(y*z-x*w)],
       [2*(x*z-y*w),2*(y*z+x*w),1-2*(x*x+y*y)]]
    m=identity();s=node.get('scale',[1,1,1]);t=node.get('translation',[0,0,0])
    for row in range(3):
        for col in range(3):m[row][col]=r[row][col]*s[col]
        m[row][3]=t[row]
    return m


def z_up(p):
    return (p[0],-p[2],p[1])


def encode_glb(doc, binary):
    doc=copy.deepcopy(doc);doc['buffers']=[{'byteLength':len(binary)}]
    raw=json.dumps(doc,separators=(',',':'),allow_nan=False).encode();raw+=b' '*(-len(raw)%4)
    binary+=b'\0'*(-len(binary)%4)
    return struct.pack('<4sII',b'glTF',2,28+len(raw)+len(binary))+struct.pack('<II',len(raw),0x4e4f534a)+raw+struct.pack('<II',len(binary),0x004e4942)+binary


def decode_glb(data):
    require(len(data)>=28,'truncated GLB')
    require(struct.unpack_from('<4sII',data)==(b'glTF',2,len(data)),'GLB header/length')
    chunks=[];offset=12
    while offset<len(data):
        require(offset+8<=len(data),'truncated chunk header')
        size,kind=struct.unpack_from('<II',data,offset);offset+=8
        require(size%4==0 and offset+size<=len(data),'GLB chunk range/alignment')
        chunks.append((kind,data[offset:offset+size]));offset+=size
    require(len(chunks)==2 and chunks[0][0]==0x4e4f534a and chunks[1][0]==0x004e4942,'F1a GLB chunks')
    doc=json.loads(chunks[0][1]);binary=chunks[1][1]
    require(doc.get('asset',{}).get('version')=='2.0','glTF version')
    buffers=doc.get('buffers',[])
    require(len(buffers)==1 and 'uri' not in buffers[0],'embedded buffer required')
    size=buffers[0]['byteLength'];require(0<=size<=len(binary)<=size+3,'declared BIN length')
    for view in doc.get('bufferViews',[]):
        offset=view.get('byteOffset',0);length=view['byteLength']
        require(view['buffer']==0 and 0<=offset and length>=0 and offset+length<=size,'bufferView range')
    return doc,binary[:size]


def accessor(doc,binary,index):
    require(isinstance(index,int) and 0<=index<len(doc.get('accessors',[])),'accessor reference')
    a=doc['accessors'][index];require('sparse' not in a,'sparse outside oracle profile')
    formats={5121:('B',1),5123:('H',2),5125:('I',4),5126:('f',4)}
    require(a['componentType'] in formats and a['type'] in ('SCALAR','VEC3'),'accessor encoding')
    code,size=formats[a['componentType']];width=1 if a['type']=='SCALAR' else 3
    require(not a.get('normalized',False),'normalized outside oracle profile')
    view=doc['bufferViews'][a['bufferView']];start=a.get('byteOffset',0);stride=view.get('byteStride',size*width)
    count=a['count'];require(isinstance(count,int) and count>0,'accessor count')
    require(start>=0 and start%size==0 and stride>=size*width and stride%size==0,'accessor stride/alignment')
    require(start+(count-1)*stride+size*width<=view['byteLength'],'accessor range')
    values=[struct.unpack_from('<'+code*width,binary,view.get('byteOffset',0)+start+i*stride) for i in range(count)]
    require(all(finite(v) for v in values),'nonfinite accessor')
    return [v[0] for v in values] if width==1 else values


def material(doc,index):
    m={} if index is None else doc['materials'][index];p=m.get('pbrMetallicRoughness',{})
    require(not any('Texture' in key for key in m) and not any('Texture' in key for key in p),'textured material')
    fields={key:copy.deepcopy(value) for key,value in m.items() if key!='name'}
    return {'present':index is not None,'fields':fields}


def scene_triangles(data, tile_transform=None):
    doc,binary=decode_glb(data);scenes=doc.get('scenes',[])
    selected=doc.get('scene',0);require(scenes and 0<=selected<len(scenes),'selected scene')
    if 'scene' not in doc:require(len(scenes)==1,'ambiguous scene')
    result=[];active=set();world=tile_transform or identity()
    def walk(index,parent):
        require(index not in active,'node cycle');active.add(index)
        node=doc['nodes'][index];m=product(parent,node_matrix(node));det=determinant(m)
        require(det != 0,'singular node transform')
        if 'mesh' in node:
            for primitive in doc['meshes'][node['mesh']]['primitives']:
                require(primitive.get('mode',4)==4,'not TRIANGLES')
                require(set(primitive['attributes'])<={'POSITION','NORMAL'},'unsupported output attribute')
                for semantic,accessor_index in primitive['attributes'].items():
                    a=doc['accessors'][accessor_index]
                    require(a['type']=='VEC3' and a['componentType']==5126,'f32 VEC3 attribute required')
                p=accessor(doc,binary,primitive['attributes']['POSITION'])
                n=accessor(doc,binary,primitive['attributes']['NORMAL']) if 'NORMAL' in primitive['attributes'] else None
                require(n is None or len(n)==len(p),'normal count')
                indices=accessor(doc,binary,primitive['indices']) if 'indices' in primitive else list(range(len(p)))
                require(len(indices)%3==0 and all(isinstance(i,int) and 0<=i<len(p) for i in indices),'triangle indices')
                mat=material(doc,primitive.get('material'))
                for start in range(0,len(indices),3):
                    ids=list(indices[start:start+3])
                    if det<0:ids[1],ids[2]=ids[2],ids[1]
                    positions=tuple(point(world,z_up(point(m,p[i]))) for i in ids)
                    normals=None if n is None else tuple(normal(world,z_up(normal(m,n[i]))) for i in ids)
                    result.append((positions,normals,mat))
        for child in node.get('children',[]):walk(child,m)
        active.remove(index)
    for root in scenes[selected]['nodes']:walk(root,identity())
    return result


def fixture(triangles=8, transformed=True, normals=True, indexed=True, variant='standard'):
    """Analytical slanted triangles, separated in X; reflected nonuniform matrix."""
    require(triangles>0,'fixture triangles')
    if variant=='normal-absent':normals=False
    if variant=='nonindexed':indexed=False
    positions=[];ns=[]
    for i in range(triangles):
        base=0 if variant=='coincident' else 4*i
        positions.extend([(base,0,0),(base,0,0),(base+1,0,0)] if variant=='degenerate' else [(base,0,0),(base+1,0,0),(base,1,1)])
        ns.extend([(0,-math.sqrt(0.5),math.sqrt(0.5))]*3)
    doc={'asset':{'version':'2.0'},'scene':0,'scenes':[{'nodes':[0]}], 'nodes':[{'mesh':0}],
         'meshes':[{'primitives':[{'attributes':{'POSITION':0},'material':0}]}],
         'materials':[{'pbrMetallicRoughness':{'baseColorFactor':[0.2,0.4,0.6,1], 'metallicFactor':0.25,'roughnessFactor':0.75},'doubleSided':True}],
         'accessors':[],'bufferViews':[]}
    binary=b''
    def add(values,code,kind,component):
        nonlocal binary
        index=len(doc['accessors']);width=1 if kind=='SCALAR' else 3
        raw=struct.pack('<'+code*(len(values)*width),*(v for row in values for v in row) if width>1 else values)
        doc['bufferViews'].append({'buffer':0,'byteOffset':len(binary),'byteLength':len(raw)})
        a={'bufferView':index,'componentType':component,'count':len(values),'type':kind}
        if index==0:a.update(min=[min(p[i] for p in values) for i in range(3)],max=[max(p[i] for p in values) for i in range(3)])
        doc['accessors'].append(a);binary+=raw;binary+=b'\0'*(-len(binary)%4)
        return index
    add(positions,'f','VEC3',5126)
    prim=doc['meshes'][0]['primitives'][0]
    if normals:prim['attributes']['NORMAL']=add(ns,'f','VEC3',5126)
    if indexed:prim['indices']=add(list(range(len(positions))),'H' if len(positions)<65536 else 'I','SCALAR',5123 if len(positions)<65536 else 5125)
    if variant=='default-material':del prim['material']
    if variant=='empty-material':doc['materials'][0]={}
    if transformed:doc['nodes'][0]['matrix']=[0,-2,0,0,-3,0,0,0,0,0,0.5,0,3,-5,7,1]
    if variant=='nested':
        doc['nodes'].append({'translation':[5,6,7],'children':[0]});doc['scenes'][0]['nodes']=[1]
    elif variant=='instanced':
        doc['nodes'].append({'mesh':0,'translation':[100,20,30]});doc['scenes'][0]['nodes'].append(1)
    elif variant=='scenes':
        doc['nodes'].append({'mesh':0,'translation':[100,20,30]});doc['scenes'].append({'nodes':[1]});doc['scene']=1
    elif variant=='interleaved':
        require(normals,'interleaved fixture requires normals')
        packed=b''.join(struct.pack('<6f',*p,*n) for p,n in zip(positions,ns))
        tail=binary[doc['bufferViews'][2]['byteOffset']:] if indexed else b''
        doc['bufferViews']=[{'buffer':0,'byteOffset':0,'byteLength':len(packed),'byteStride':24}]
        doc['accessors'][0]['bufferView']=0
        doc['accessors'][1].update(bufferView=0,byteOffset=12)
        if indexed:
            doc['bufferViews'].append({'buffer':0,'byteOffset':len(packed),'byteLength':doc['accessors'][2]['count']*(2 if len(positions)<65536 else 4)})
            doc['accessors'][2]['bufferView']=1
        binary=packed+tail
    return encode_glb(doc,binary)


def distance(a,b):
    return math.sqrt(sum((x-y)**2 for x,y in zip(a,b)))


def close_value(a,b,tolerance):
    if isinstance(a,dict):
        return isinstance(b,dict) and a.keys()==b.keys() and all(close_value(a[k],b[k],tolerance) for k in a)
    if isinstance(a,(tuple,list)):
        return isinstance(b,(tuple,list)) and len(a)==len(b) and all(close_value(x,y,tolerance) for x,y in zip(a,b))
    if isinstance(a,(str,bool)) or a is None:return a==b
    return isinstance(b,(int,float)) and abs(a-b)<=tolerance


def match_triangles(expected,actual,position_tolerance=None,normal_tolerance=2e-7):
    require(len(expected)==len(actual),'triangle multiplicity/count')
    pending=list(actual)
    for p,n,mat in expected:
        found=None
        for j,(q,k,m) in enumerate(pending):
            if not close_value(mat,m,1e-6) or ((n is None)!=(k is None)):continue
            for rotation in range(3):
                qp=q[rotation:]+q[:rotation];kn=None if k is None else k[rotation:]+k[:rotation]
                positions_match=all(all(abs(x-y)<= (position_tolerance if position_tolerance is not None else max(1e-6,abs(x)*2**-24))+1e-12 for x,y in zip(a,b)) for a,b in zip(p,qp))
                normals_match=n is None or all(all(abs(x-y)<=normal_tolerance+1e-12 for x,y in zip(a,b)) for a,b in zip(n,kn))
                if positions_match and normals_match:
                    found=j;break
            if found is not None:break
        require(found is not None,'oriented geometry/normal/material fidelity')
        pending.pop(found)


def box_contains(box,p,tolerance):
    # Profile output uses axis-aligned boxes in Tiles Z-up.
    require(len(box)==12 and finite(box),'box layout')
    require(all(box[i]==0 for i in (4,5,6,8,9,10)) and all(box[i]>=0 for i in (3,7,11)),'profile axis-aligned box')
    return all(abs(p[i]-box[i])<=box[3+4*i]+tolerance for i in range(3))


def check_index(z,raw):
    infos=z.infolist()
    require(infos[0].filename=='tileset.json' and infos[-1].filename=='@3dtilesIndex1@','3TZ member ordering')
    require(all(info.compress_type==zipfile.ZIP_STORED for info in infos),'stored ZIP required')
    index=z.read('@3dtilesIndex1@');require(len(index)==24*(len(infos)-1),'index length')
    expected={hashlib.md5(info.filename.encode()).digest():info for info in infos[:-1]};previous=None
    require(len(expected)==len(infos)-1,'MD5 collision in oracle corpus')
    for low,high,offset in struct.iter_unpack('<QQQ',index):
        require(previous is None or (low,high)>previous,'index hash order');previous=(low,high)
        digest=struct.pack('<QQ',low,high);require(digest in expected,'index hash membership')
        info=expected.pop(digest);require(offset==info.header_offset,'index header offset')
        require(raw[offset:offset+4]==b'PK\x03\x04','index local header')
        length,extra=struct.unpack_from('<HH',raw,offset+26)
        name=raw[offset+30:offset+30+length];require(name.decode()==info.filename,'index local name')
        payload=offset+30+length+extra;require(raw[payload:payload+info.file_size]==z.read(info.filename),'index payload')
    require(not expected,'index completeness')


def inspect(source,archive,leaf_limit,position_tolerance=None,normal_tolerance=2e-7):
    expected=scene_triangles(Path(source).read_bytes())
    with zipfile.ZipFile(archive) as z:
        require(z.testzip() is None,'ZIP CRC')
        names=z.namelist();require(len(names)==len(set(names)),'duplicate ZIP member')
        require(all(not n.startswith('/') and all(p not in ('','.','..') for p in n.split('/')) for n in names),'unsafe member')
        doc=json.loads(z.read('tileset.json'));root=doc['root'];children=root.get('children',[])
        require(doc['asset']['version']=='1.1' and root['refine']=='REPLACE','tileset profile')
        require('content' not in root and 'contents' not in root and children,'empty flat root')
        box=root['boundingVolume']['box'];ge=max(1.0,2*math.sqrt(sum(box[i]**2 for i in (3,7,11))))
        require(abs(root['geometricError']-ge)<=1e-10*max(1,ge),'root selection metric')
        require(abs(doc['geometricError']-ge)<=1e-10*max(1,ge),'tileset selection metric')
        root_m=matrix(root['transform']) if 'transform' in root else identity();actual=[];uris=[]
        require(root_m==identity(),'unexpected root transform')
        for child in children:
            require(not child.get('children') and 'implicitTiling' not in child,'explicit flat leaf')
            require(child['geometricError']==0,'leaf GE')
            uri=child['content']['uri'];require(uri not in uris,'repeated leaf content');uris.append(uri)
            cm=matrix(child['transform']) if 'transform' in child else identity();require(cm==identity(),'unexpected leaf transform');world=product(root_m,cm)
            child_box=child['boundingVolume']['box']
            for bits in range(8):
                corner=tuple(child_box[i]+(-1 if bits&(1<<i) else 1)*child_box[3+4*i] for i in range(3))
                require(box_contains(box,corner,1e-9),'leaf bounds outside root')
            triangles=scene_triangles(z.read(uri),world);require(0<len(triangles)<=leaf_limit,'leaf triangle limit')
            for p,_,_ in triangles:
                for v in p:
                    require(box_contains(box,v,1e-9),'decoded geometry outside root bounds')
            # Check local leaf box against axis-converted content before tile transforms.
            for p,_,_ in scene_triangles(z.read(uri)):
                for v in p:require(box_contains(child['boundingVolume']['box'],v,1e-9),'decoded geometry outside leaf bounds')
            actual.extend(triangles)
        require(set(names)=={'tileset.json','conversion.json','@3dtilesIndex1@',*uris},'exact accepted resource closure')
        report=json.loads(z.read('conversion.json'))
        expected_report={'schema_version':1,'profile':'f1a-local-static-glb-v1','coordinates':'local-gltf','source_bytes':Path(source).stat().st_size,'triangles':len(expected),'leaf_tiles':len(uris),'leaf_triangles':leaf_limit,'routing_geometric_error_metres':ge}
        require(close_value(expected_report,report,1e-10*max(1,ge)),'typed report facts/fields')
        check_index(z,Path(archive).read_bytes())
        match_triangles(expected,actual,position_tolerance,normal_tolerance)
        return {'source_triangles':len(expected),'leaves':len(uris),'leaf_triangles':len(actual),'root_geometric_error':ge,
                'members':sorted(names),'position_tolerance':position_tolerance if position_tolerance is not None else 'component max(1e-6m, abs(computed)*2^-24)' ,'normal_tolerance':normal_tolerance,
                'source_sha256':hashlib.sha256(Path(source).read_bytes()).hexdigest(), 'archive_sha256':hashlib.sha256(Path(archive).read_bytes()).hexdigest(),'report':report}


def write_control_archive(path,members):
    """Synthetic oracle controls only; uses stdlib ZIP, never a Rust writer."""
    with zipfile.ZipFile(path,'w',compression=zipfile.ZIP_STORED) as z:
        records=[]
        for name,payload in members.items():
            z.writestr(name,payload)
            low,high=struct.unpack('<QQ',hashlib.md5(name.encode()).digest())
            records.append((low,high,z.getinfo(name).header_offset))
        z.writestr('@3dtilesIndex1@',b''.join(struct.pack('<QQQ',*r) for r in sorted(records)))


def archive_controls():
    with tempfile.TemporaryDirectory(prefix='f1a-oracle-') as temporary:
        root=Path(temporary);source=root/'source.glb';source.write_bytes(fixture(2,transformed=False))
        box=[2.5,-0.5,0.5,2.5,0,0,0,0.5,0,0,0,0.5];ge=math.sqrt(27)
        manifest={'asset':{'version':'1.1'},'geometricError':ge,'root':{'boundingVolume':{'box':box},'geometricError':ge,'refine':'REPLACE','children':[{'boundingVolume':{'box':box},'geometricError':0,'content':{'uri':'t/0.glb'}}]}}
        report={'schema_version':1,'profile':'f1a-local-static-glb-v1','coordinates':'local-gltf','source_bytes':source.stat().st_size,'triangles':2,'leaf_tiles':1,'leaf_triangles':2,'routing_geometric_error_metres':ge}
        members={'tileset.json':json.dumps(manifest).encode(),'conversion.json':json.dumps(report).encode(),'t/0.glb':source.read_bytes()}
        archive=root/'control.3tz';write_control_archive(archive,members);inspect(source,archive,2)
        controls={}
        for name in ('orphan','wrong_root_metric','wrong_report','wrong_leaf_bounds','missing_leaf','invalid_payload'):
            changed=copy.deepcopy(members)
            if name=='orphan':changed['scratch.bin']=b'private'
            elif name=='wrong_root_metric':
                doc=json.loads(changed['tileset.json']);doc['root']['geometricError']=0;changed['tileset.json']=json.dumps(doc).encode()
            elif name=='wrong_report':
                doc=json.loads(changed['conversion.json']);doc['triangles']=9;changed['conversion.json']=json.dumps(doc).encode()
            elif name=='wrong_leaf_bounds':
                doc=json.loads(changed['tileset.json']);doc['root']['children'][0]['boundingVolume']['box'][0]=1000;changed['tileset.json']=json.dumps(doc).encode()
            elif name=='missing_leaf':del changed['t/0.glb']
            else:
                doc,binary=decode_glb(changed['t/0.glb']);doc['accessors'][0]['count']=300;changed['t/0.glb']=encode_glb(doc,binary)
            write_control_archive(archive,changed)
            try:inspect(source,archive,2)
            except (OracleError,KeyError):controls[name]='rejected'
            else:raise OracleError('undetected archive control: '+name)
        return controls


def self_test():
    data=fixture(2);actual=scene_triangles(data)
    require(close_value(actual[0][0],((3,-7,-5),(0,-7.5,-5),(3,-7,-7)),1e-12),'analytic reflected positions/winding')
    wanted=(1/math.sqrt(37),-6/math.sqrt(37),0)
    require(all(distance(n,wanted)<1e-12 for n in actual[0][1]),'analytic inverse-transpose normal')
    match_triangles(actual,actual,1e-4,2e-6)
    controls={}
    for name,mutate in (
        ('missing_triangle',lambda a:a.pop()),
        ('wrong_winding',lambda a:a.__setitem__(0,((a[0][0][0],a[0][0][2],a[0][0][1]),a[0][1],a[0][2]))),
        ('wrong_position',lambda a:a.__setitem__(0,(((999,0,0),*a[0][0][1:]),a[0][1],a[0][2]))),
        ('missing_normal',lambda a:a.__setitem__(0,(a[0][0],None,a[0][2]))),
        ('wrong_normal',lambda a:a.__setitem__(0,(a[0][0],((0,0,1),)*3,a[0][2]))),
        ('wrong_material',lambda a:a.__setitem__(0,(a[0][0],a[0][1],material({},None)))),
    ):
        corrupted=copy.deepcopy(actual);mutate(corrupted)
        try:match_triangles(actual,corrupted,1e-4,2e-6)
        except OracleError:controls[name]='rejected'
        else:raise OracleError('undetected control: '+name)
    doc,binary=decode_glb(data);doc['bufferViews'][0]['byteLength']=len(binary)+4096
    try:decode_glb(encode_glb(doc,binary))
    except OracleError:controls['invalid_buffer_range']='rejected'
    else:raise OracleError('undetected binary range')
    require(not box_contains([0,0,0,1,0,0,0,1,0,0,0,1],(1000,1000,1000),1e-4),'bounds negative control')
    controls['wrong_bounds']='rejected'
    variants={}
    for variant in ('coincident','degenerate','nested','instanced','scenes','interleaved','normal-absent','default-material','empty-material','nonindexed'):
        triangles=scene_triangles(fixture(2,variant=variant))
        require(len(triangles)==(4 if variant=='instanced' else 2),'variant count: '+variant)
        if variant=='nested':require(distance(triangles[0][0][0],(8,-14,1))<1e-12,'nested analytical position')
        if variant=='scenes':require(distance(triangles[0][0][0],(100,-30,20))<1e-12,'default scene selection analytical position')
        if variant=='normal-absent':require(all(n is None for _,n,_ in triangles),'missing normal semantics')
        if variant=='default-material':require(all(not m['present'] for _,_,m in triangles),'absent material semantics')
        if variant=='empty-material':require(all(m['present'] and m['fields']=={} for _,_,m in triangles),'empty material semantics')
        variants[variant]='passed'
    # Omitting a default-valued semantic field must be detected even when its
    # effective numeric default is unchanged. Names carry no preservation promise.
    declared=copy.deepcopy(actual);declared[0][2]['fields']['alphaMode']='OPAQUE'
    try:match_triangles(declared,actual)
    except OracleError:controls['default_valued_material_omission']='rejected'
    else:raise OracleError('material presence omission')
    return {'fixture_variants':variants,'analytic_transform_normal_winding':'passed','corruption_controls':controls,'archive_corruption_controls':archive_controls()}


def run_binary(binary):
    binary=Path(binary).resolve();cases=[]
    with tempfile.TemporaryDirectory(prefix='f1a-candidate-') as temporary:
        root=Path(temporary)
        for variant in ('standard','coincident','degenerate','nested','instanced','scenes','interleaved','normal-absent','default-material','empty-material','nonindexed'):
            source=root/(variant+'.glb');source.write_bytes(fixture(8,variant=variant))
            for limit in (1,3,1000):
                output=root/(variant+'-'+str(limit)+'.3tz')
                command=[str(binary),'--json','mesh-local-to-3tz','-i',str(source),'-o',str(output),'--leaf-triangles',str(limit)]
                completed=subprocess.run(command,text=True,capture_output=True,timeout=60)
                require(completed.returncode==0,'CLI '+variant+' limit '+str(limit)+': '+completed.stdout+' '+completed.stderr)
                summary=json.loads(completed.stdout);require(summary.get('ok') is True,'CLI success shape')
                evidence=inspect(source,output,limit)
                require(summary['meshReport']==evidence['report'],'CLI/archive typed report parity')
                require((limit>=evidence['source_triangles']) or evidence['leaves']>1,'forced multileaf not demonstrated')
                cases.append({'variant':variant,'leaf_limit':limit,'status':'passed','source_triangles':evidence['source_triangles'],'leaves':evidence['leaves'],'report':evidence['report']})
    return {'binary':str(binary),'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'cases':cases}


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,help='Run self-test and all fixture variants against this already-built CLI');p.add_argument('--self-test',action='store_true');p.add_argument('--fixture',type=Path)
    p.add_argument('--triangles',type=int,default=8);p.add_argument('--variant',choices=['standard','coincident','degenerate','nested','instanced','scenes','interleaved','normal-absent','default-material','empty-material','nonindexed'],default='standard');p.add_argument('--source',type=Path);p.add_argument('--archive',type=Path)
    p.add_argument('--leaf-limit',type=int);p.add_argument('--json-output',type=Path)
    args=p.parse_args();result={}
    if args.self_test or args.binary:result['self_test']=self_test()
    if args.binary:result['candidate']=run_binary(args.binary)
    if args.fixture:args.fixture.write_bytes(fixture(args.triangles,variant=args.variant));result['fixture']=str(args.fixture)
    if args.archive:
        p.error('--source and --leaf-limit are required with --archive') if not args.source or not args.leaf_limit else None
        result['inspection']=inspect(args.source,args.archive,args.leaf_limit)
    encoded=json.dumps(result,indent=2,allow_nan=False)+'\n'
    if args.json_output:args.json_output.write_text(encoded)
    else:print(encoded,end='')


if __name__=='__main__':main()
