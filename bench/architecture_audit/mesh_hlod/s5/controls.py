#!/usr/bin/env python3
"""Literal analytic bytes, source expectations and named negative controls; no producer imports."""
import copy
import base64
import hashlib
import json
from pathlib import Path
import runpy
import struct
import sys

POINTS = [
    [(0,0,0),(2,0,0),(0,2,0)],
    [(2,0,0),(2,2,0),(0,2,0)],
    [(2,0,0),(4,0,0),(2,-2,0)],
    [(1,0,0),(1,-1,0),(0,-1,0)],
]
# Two different materials and nested transforms. Powers of two remain exactly
# representable under the u16/f32 deliveries, rotation and reflected scale.
MATERIALS=[{'pbrMetallicRoughness':{'baseColorFactor':[.3,.5,.7,1],'metallicFactor':.2},'doubleSided':True},
           {'pbrMetallicRoughness':{'baseColorFactor':[.8,.2,.4,1]},'alphaMode':'MASK','alphaCutoff':.4,'extensions':{'KHR_materials_unlit':{}}}]
NORMALS=[(1,0,0),(0,1,0),(0,0,1)]
UVS=[(0,0),(1,0),(0,1)]
SPLIT_POINTS=copy.deepcopy(POINTS)
SPLIT_POINTS[3]=[(0,0,0),(1,0,0),(0,-1,0)]

def transformed(p, trs=False):
    # outer translation [8,4,2], inner column-major matrix: rotate 90 deg
    # about Z, with nonuniform/reflected scale [-2,4,1].
    x,y,z=p
    return [8+2*x,4-4*y,2+z] if trs else [8-4*y,4-2*x,2+z]

def glb(face_ids, quantized=False, wrong_id=False, moved=False, bad_metadata=False, trs=False, normalized=False, leaf=True, compressed=False, points=POINTS, detour=False, winding=False, swap_uv=False, swap_normal=False, triangle_wrong_in_range=False, octahedral=False):
    binary=bytearray(); views=[]; accessors=[]
    def view(data):
        while len(binary)%8: binary.append(0)
        views.append(dict(buffer=0,byteOffset=len(binary),byteLength=len(data)));binary.extend(data)
        return len(views)-1
    def accessor(data,ct,shape,count,**extra):
        accessors.append(dict(bufferView=view(data),componentType=ct,type=shape,count=count,**extra))
        return len(accessors)-1
    vertices=[list(p) for i in face_ids for p in points[i]]
    if detour:vertices.append([.5,-.5,0])
    if moved: vertices[0][0]+=.125
    flat=[c for p in vertices for c in p]
    if quantized or normalized:
        # Signed shorts with 8-byte vertex stride. The normalized variant
        # dequantizes through a nested node scale, independently of the census.
        pos=accessor(b''.join(struct.pack('<hhhxx',*p) for p in vertices),5122,'VEC3',len(vertices),min=[min(p[k] for p in vertices) for k in range(3)],max=[max(p[k] for p in vertices) for k in range(3)])
        views[accessors[pos]['bufferView']]['byteStride']=8
        if normalized:accessors[pos]['normalized']=True
    else:
        pos=accessor(struct.pack('<'+'f'*len(flat),*flat),5126,'VEC3',len(vertices),min=[min(p[k] for p in vertices) for k in range(3)],max=[max(p[k] for p in vertices) for k in range(3)])
    normals=[list(n) for _ in face_ids for n in NORMALS]
    uvs=[list(uv) for _ in face_ids for uv in UVS]
    if detour:normals.append(list(NORMALS[0]));uvs.append([.5,0])
    if swap_normal:normals[0],normals[1]=normals[1],normals[0]
    if swap_uv:uvs[0],uvs[1]=uvs[1],uvs[0]
    if octahedral:
        # Literal Appendix B octahedral codes for X, Y, Z, at K=8 precision.
        codes=[(127,0,127,0),(0,127,127,0),(0,0,127,0)]*len(face_ids)
        normal_accessor=accessor(b''.join(struct.pack('<4h',*n) for n in codes),5122,'VEC3',len(codes),normalized=True)
        views[accessors[normal_accessor]['bufferView']]['byteStride']=8
    else:
        normal_accessor=accessor(struct.pack('<'+'f'*len(normals)*3,*(c for n in normals for c in n)),5126,'VEC3',len(normals))
    sets=[];attrs={'POSITION':pos,'NORMAL':normal_accessor,
        'TEXCOORD_0':accessor(struct.pack('<'+'f'*len(uvs)*2,*(c for uv in uvs for c in uv)),5126,'VEC2',len(uvs))}
    classes={};tables=[]
    for si,label in enumerate(['source_primitive','source_triangle'] if leaf else ['source_primitive']):
        ids=[j for j,_ in enumerate(face_ids) for _ in range(3)]
        if detour:ids.append(face_ids.index(3))
        if triangle_wrong_in_range and si:ids[:3]=[1]*3
        if wrong_id: ids[0]=999
        attrs[f'_FEATURE_ID_{si}']=accessor(struct.pack('<'+'f'*len(ids),*ids),5126,'SCALAR',len(ids))
        schema={};props={}
        table_ids=face_ids+[1] if triangle_wrong_in_range and si else face_ids
        columns={'node_index':[0]*len(table_ids),'mesh_index':[0]*len(table_ids),'primitive_index':table_ids,
                 'expanded_ordinal':table_ids,'material_index':[i%2 for i in table_ids]}
        if si:
            columns.update(triangle_index=[0]*len(table_ids),vertex_0=[0]*len(table_ids),vertex_1=[1]*len(table_ids),vertex_2=[2]*len(table_ids))
        for key,values in columns.items():
            wide=key in ('expanded_ordinal','triangle_index','vertex_0','vertex_1','vertex_2')
            data=struct.pack('<'+('Q' if wide else 'I')*len(values),*values)
            schema[key]=dict(type='SCALAR',componentType='UINT64' if wide else 'UINT32',required=True)
            props[key]=dict(values=view(data))
        if bad_metadata: props['node_index']['values']=999
        classes[label]=dict(properties=schema)
        tables.append(dict(count=len(table_ids),**{'class':label},properties=props))
        sets.append(dict(featureCount=len(table_ids),attribute=si,propertyTable=si,label=label))
    primitives=[]
    for rank,identity in enumerate(face_ids):
        corners=[rank*3,rank*3+1,rank*3+2]
        if winding and rank==0:corners[1],corners[2]=corners[2],corners[1]
        if detour and identity==3:
            a,t,d=corners;e=len(vertices)-1;corners=[a,e,d,e,t,d]
        indices=accessor(struct.pack('<'+'I'*len(corners),*corners),5125,'SCALAR',len(corners))
        primitives.append(dict(attributes=attrs,indices=indices,material=identity%2,extensions={'EXT_mesh_features':{'featureIds':sets}}))
    doc=dict(asset={'version':'2.0'},scene=0,scenes=[{'nodes':[0]}],nodes=[{'translation':[8,4,2],'children':[1]},
             {'matrix':[0,-2,0,0,-4,0,0,0,0,0,1,0,0,0,0,1],'mesh':0}],
             meshes=[{'primitives':primitives}],materials=MATERIALS,buffers=[{'byteLength':len(binary)}],bufferViews=views,accessors=accessors,
             extensionsUsed=['EXT_mesh_features','EXT_structural_metadata','KHR_materials_unlit'],
             extensions={'EXT_structural_metadata':{'schema':{'classes':classes},'propertyTables':tables}})
    if trs:doc['nodes'][1]={'rotation':[0,0,1,0],'scale':[-2,4,1],'mesh':0}
    if normalized:
        doc['nodes'][1].pop('mesh');doc['nodes'][1]['children']=[2];doc['nodes'].append({'scale':[32767]*3,'mesh':0})
    if quantized or normalized or octahedral: doc['extensionsUsed'].append('KHR_mesh_quantization');doc['extensionsRequired']=['KHR_mesh_quantization']
    if compressed:
        # Reuse the retained independent Khronos-spec encoder, never meshopt's
        # producer encoder. Every delivered view uses the virtual fallback.
        encode=runpy.run_path(str(Path(__file__).resolve().parents[4]/'tests/ext_meshopt_spec.py'))['encode_attributes']
        encoded=bytearray();fallback=0
        for vi,v in enumerate(views):
            raw=binary[v['byteOffset']:v['byteOffset']+v['byteLength']]
            stride=v.get('byteStride',4);stream=encode(raw,len(raw)//stride,stride)
            while len(encoded)%4:encoded.append(0)
            filter_name='OCTAHEDRAL' if octahedral and vi==accessors[attrs['NORMAL']]['bufferView'] else 'NONE'
            v['extensions']={'EXT_meshopt_compression':dict(buffer=0,byteOffset=len(encoded),byteLength=len(stream),count=len(raw)//stride,byteStride=stride,mode='ATTRIBUTES',filter=filter_name)}
            v['buffer']=1;v['byteOffset']=fallback;fallback+=len(raw);encoded.extend(stream)
        binary=encoded
        doc['buffers']=[{'byteLength':len(binary)},{'byteLength':fallback,'extensions':{'EXT_meshopt_compression':{'fallback':True}}}]
        doc['extensionsUsed'].append('EXT_meshopt_compression');doc.setdefault('extensionsRequired',[]).append('EXT_meshopt_compression')
    data=json.dumps(doc,separators=(',',':')).encode();data+=b' '*(-len(data)%4);binary+=b'\0'*(-len(binary)%4)
    return b'glTF'+struct.pack('<III',2,28+len(data)+len(binary),len(data))+b'JSON'+data+struct.pack('<I',len(binary))+b'BIN\0'+binary

def generate(root):
    root=Path(root);root.mkdir(parents=True,exist_ok=True)
    expected={'triangles':{f'0,0,{i},0':dict(xyz=[transformed(p) for p in face],normals=[[0,-1,0],[-1,0,0],[0,0,1]],uvs=UVS,vertices=[0,1,2],material=MATERIALS[i%2],expanded_ordinal=i,material_index=i%2) for i,face in enumerate(POINTS)}}
    expected_trs=copy.deepcopy(expected)
    for i,face in enumerate(POINTS):
        expected_trs['triangles'][f'0,0,{i},0'].update(xyz=[transformed(p,True) for p in face],normals=[[1,0,0],[0,-1,0],[0,0,1]])
    (root/'source-trs.json').write_text(json.dumps(expected_trs,sort_keys=True,indent=2)+'\n')
    expected_wide=copy.deepcopy(expected);expected_wide['triangles']['0,0,0,0']['expanded_ordinal']=2**53
    (root/'source-wide.json').write_text(json.dumps(expected_wide,sort_keys=True,indent=2)+'\n')
    (root/'source.json').write_text(json.dumps(expected,sort_keys=True,indent=2)+'\n')
    expected_split=copy.deepcopy(expected)
    expected_split['triangles']['0,0,3,0']['xyz']=[transformed(p) for p in SPLIT_POINTS[3]]
    (root/'source-split.json').write_text(json.dumps(expected_split,sort_keys=True,indent=2)+'\n')
    def tile(uri,g,children=None):
        d=dict(boundingVolume={'box':[0,0,0,32,0,0,0,32,0,0,0,32]},geometricError=g,refine='REPLACE',content={'uri':uri})
        if children:d['children']=children
        return d
    left=tile('left.glb',2,[tile('leaf0.glb',0),tile('leaf1.glb',0)])
    right=tile('right.glb',2,[tile('middle.glb',1,[tile('leaf2.glb',0),tile('leaf3.glb',0)])])
    doc=dict(asset={'version':'1.1'},geometricError=4,root=tile('root.glb',4,[left,right]))
    groups={'root.glb':[0,1,2,3],'left.glb':[0,1],'right.glb':[2,3],'middle.glb':[2,3],**{f'leaf{i}.glb':[i] for i in range(4)}}
    cases=[('valid',None,None),('quantized',None,None),('normalized',None,None),('trs',None,None),('numeric-lexemes',None,None),('opaque-extras',None,None),('wide-identity',None,None),('meshopt-fallback',None,None),('adjacent-uint64-leaf','sourceIdentity',None),('adjacent-uint64-representative','representativeIdentity',None),('wrong-feature','featureIdentity','hlodFeatureIds'),
           ('wrong-source','sourceIdentity',None),('wrong-representative','representativeIdentity',None),('wrong-representative-contents','representativeIdentity',None),('missing-leaf','leafCoverage',None),('duplicate-leaf','leafCoverage',None),
           ('moved-interface','interfaceSegments',None),('moved-corner','interfacePoints',None),('empty-intermediate','replaceContentContinuity','replaceContentContinuity'),
           ('negative-g','geometricErrorFinite','tileset schema'),('nan-g','geometricErrorFinite','JSON'),
           ('nonmonotone-g','geometricErrorOrder','geometricError increases'),('bad-bounds',None,'decoded content'),
           ('stale-hash',None,'content checksum mismatch'),('malformed-codec',None,'meshopt'),('bad-metadata','featureIdentity','hlodFeatureIds'),('optional-property','featureIdentity','hlodFeatureIds'),('bad-property-type','featureIdentity','hlodFeatureIds'),('wrong-table-class','featureIdentity','hlodFeatureIds'),('missing-feature-set','sourceIdentity','hlodFeatureIds'),('correlated-metadata','sourceIdentity',None),('active-morph','codecStructure',None),('infinite-root-g','geometricErrorFinite','JSON'),('normalized-identity','featureIdentity','hlodFeatureIds'),('array-identity','featureIdentity','hlodFeatureIds'),('offset-identity','featureIdentity','hlodFeatureIds'),('null-identity','featureIdentity','hlodFeatureIds'),('gpu-instancing','codecStructure',None),('source-triangle-only','featureIdentity','hlodFeatureIds'),('local-gltf-identity',None,'hlodFeatureIds'),('coarse-material-class','representativeMaterialClass',None),('uri-buffer-shadow','codecStructure','hlodFeatureIds'),('wrong-bin-chunk','codecStructure','buffer has no resource')]
    cases += [('split-side',None,None),('split-side-detour','interfaceSegments','interfaceSegments'),
              ('coarse-winding','interfaceWinding','interfaceWinding'),('swapped-uv','sourceTexcoords',None),
              ('swapped-normal','sourceNormals',None),('swapped-uv-normal-accessors','codecStructure','NORMAL accessor type'),
              ('partial-identity-strip','featureIdentity','hlodFeatureIds'),
              ('wrong-in-range-triangle','sourceIdentity','sourceIdentity'),('coarse-source-triangle','featureIdentity','hlodFeatureIds'),
              ('implicit-empty-intermediate','replaceContentContinuity','replaceContentContinuity'),
              ('repeated-coarse-cell',None,None),('meshopt-octahedral',None,None)]
    manifest=[]
    for name,oracle_check,archive_check in cases:
        out=root/name;out.mkdir(exist_ok=True);d=copy.deepcopy(doc)
        for uri,ids in groups.items():
            data=glb(ids,quantized=name=='quantized',wrong_id=name=='wrong-feature' and uri=='root.glb',moved=(name=='moved-interface' and uri=='left.glb') or (name=='moved-corner' and uri=='right.glb'),bad_metadata=name=='bad-metadata' and uri=='root.glb',trs=name=='trs',normalized=name=='normalized',leaf=uri.startswith('leaf') or (name=='coarse-source-triangle' and uri=='root.glb'),compressed=name in ('meshopt-fallback','meshopt-octahedral'),octahedral=name=='meshopt-octahedral',
                points=SPLIT_POINTS if name in ('split-side','split-side-detour') else POINTS,
                detour=name=='split-side-detour' and uri=='right.glb',winding=name=='coarse-winding' and uri=='root.glb',
                swap_uv=name=='swapped-uv' and uri=='leaf0.glb',swap_normal=name=='swapped-normal' and uri=='leaf0.glb',
                triangle_wrong_in_range=name=='wrong-in-range-triangle' and uri=='leaf0.glb')
            (out/uri).write_bytes(data)
        if name in ('wide-identity','adjacent-uint64-leaf','adjacent-uint64-representative','numeric-lexemes','opaque-extras','optional-property','correlated-metadata','active-morph','bad-property-type','wrong-table-class','missing-feature-set','normalized-identity','array-identity','offset-identity','null-identity','gpu-instancing','source-triangle-only','coarse-material-class','uri-buffer-shadow'):
            for uri in groups:
                p=out/uri;b=p.read_bytes();jl=int.from_bytes(b[12:16],'little');j=json.loads(b[20:20+jl]);binary=bytearray(b[28+jl:])
                if name=='numeric-lexemes':
                    def numeric(v):
                        if type(v) is int:return float(v)
                        if isinstance(v,list):return [numeric(x) for x in v]
                        if isinstance(v,dict):return {k:numeric(x) for k,x in v.items()}
                        return v
                    j=numeric(j)
                if name=='opaque-extras':j['extras']={'opaque':'BIG_NUMBER'}
                if name=='optional-property':
                    meta=j['extensions']['EXT_structural_metadata'];meta['schema']['classes']['source_primitive']['properties']['optional']={'type':'SCALAR','componentType':'UINT32','required':False};meta['propertyTables'][0]['properties']['optional']={'values':999}
                if name=='bad-property-type':j['extensions']['EXT_structural_metadata']['schema']['classes']['source_primitive']['properties']['node_index']['type']='VEC2'
                if name=='wrong-table-class':j['extensions']['EXT_structural_metadata']['propertyTables'][0]['class']='bogus'
                if name=='missing-feature-set' and uri.startswith('leaf'):
                    for primitive in j['meshes'][0]['primitives']:primitive['extensions']['EXT_mesh_features']['featureIds'].pop()
                if name in ('wide-identity','adjacent-uint64-leaf','adjacent-uint64-representative'):
                    meta=j['extensions']['EXT_structural_metadata']
                    for table in meta['propertyTables']:
                        v=j['bufferViews'][table['properties']['expanded_ordinal']['values']]
                        for i,source_id in enumerate(groups[uri]):
                            if source_id==0:
                                corrupt=(name=='adjacent-uint64-leaf' and uri=='leaf0.glb') or (name=='adjacent-uint64-representative' and uri=='root.glb')
                                struct.pack_into('<Q',binary,v['byteOffset']+i*8,2**53+int(corrupt))
                if name=='correlated-metadata':
                    for table in j['extensions']['EXT_structural_metadata']['propertyTables']:
                        for key in ('expanded_ordinal','material_index'):
                            v=j['bufferViews'][table['properties'][key]['values']]
                            for i in range(table['count']):
                                wide=key=='expanded_ordinal';struct.pack_into('<Q' if wide else '<I',binary,v['byteOffset']+i*(8 if wide else 4),12345)
                if name in ('normalized-identity','array-identity','offset-identity'):
                    schema=j['extensions']['EXT_structural_metadata']['schema']['classes']['source_primitive']['properties']['primitive_index']
                    if name=='normalized-identity':schema['normalized']=True
                    if name=='array-identity':schema.update(array=True,count=2)
                    if name=='offset-identity':schema['offset']=100
                if name=='null-identity':
                    for primitive in j['meshes'][0]['primitives']:
                        for feature in primitive['extensions']['EXT_mesh_features']['featureIds']:feature['nullFeatureId']=0
                if name=='uri-buffer-shadow':
                    resolved=bytearray(binary)
                    for primitive in j['meshes'][0]['primitives']:
                        a=j['accessors'][primitive['attributes']['_FEATURE_ID_0']];v=j['bufferViews'][a['bufferView']]
                        for i in range(a['count']):struct.pack_into('<f',resolved,v['byteOffset']+i*4,999)
                    j['buffers'][0]['uri']='data:application/octet-stream;base64,'+base64.b64encode(resolved).decode()
                if name=='source-triangle-only' and uri=='leaf0.glb':
                    meta=j['extensions']['EXT_structural_metadata'];meta['schema']['classes']['renamed']=meta['schema']['classes'].pop('source_primitive');meta['propertyTables'][0]['class']='renamed'
                    for primitive in j['meshes'][0]['primitives']:primitive['extensions']['EXT_mesh_features']['featureIds'][0]['label']='renamed'
                    a=j['accessors'][j['meshes'][0]['primitives'][0]['attributes']['_FEATURE_ID_1']];v=j['bufferViews'][a['bufferView']]
                    for i in range(a['count']):struct.pack_into('<f',binary,v['byteOffset']+i*4,999)
                if name=='coarse-material-class' and not uri.startswith('leaf'):
                    for primitive in j['meshes'][0]['primitives']:primitive['material']=1-primitive['material']
                if name=='gpu-instancing':
                    j['nodes'][0]['extensions']={'EXT_mesh_gpu_instancing':{'attributes':{'TRANSLATION':0}}}
                    j['extensionsUsed'].append('EXT_mesh_gpu_instancing')
                if name=='active-morph':
                    for mesh in j['meshes']:
                        mesh['weights']=[1]
                        for primitive in mesh['primitives']:primitive['targets']=[{'POSITION':0}]
                data=json.dumps(j).replace('"BIG_NUMBER"','1e400').encode();data+=b' '*(-len(data)%4)
                p.write_bytes(b'glTF'+struct.pack('<III',2,28+len(data)+len(binary),len(data))+b'JSON'+data+struct.pack('<I',len(binary))+b'BIN\0'+binary)
        if name in ('wrong-source','wrong-representative','wrong-representative-contents'):
            # In-range feature ID mutation only: same coordinates, wrong row.
            p=out/('leaf0.glb' if name=='wrong-source' else 'root.glb');data=p.read_bytes();jl=int.from_bytes(data[12:16],'little');j=json.loads(data[20:20+jl]);t=j['extensions']['EXT_structural_metadata']['propertyTables'][0];v=j['bufferViews'][t['properties']['primitive_index']['values']];b=bytearray(data);struct.pack_into('<I',b,28+jl+v['byteOffset'],1);p.write_bytes(b)
        if name=='wrong-bin-chunk':
            p=out/'root.glb';b=bytearray(p.read_bytes());jl=int.from_bytes(b[12:16],'little');b[24+jl:28+jl]=b'SHAD';p.write_bytes(b)
        if name in ('partial-identity-strip','swapped-uv-normal-accessors'):
            p=out/('root.glb' if name=='partial-identity-strip' else 'leaf0.glb')
            b=p.read_bytes();jl=int.from_bytes(b[12:16],'little');j=json.loads(b[20:20+jl]);binary=b[28+jl:]
            if name=='partial-identity-strip':
                j.pop('extensions')
                j['extensionsUsed']=[e for e in j['extensionsUsed'] if e not in ('EXT_mesh_features','EXT_structural_metadata')]
                for primitive in j['meshes'][0]['primitives']:
                    primitive.pop('extensions')
                    primitive['attributes']={k:v for k,v in primitive['attributes'].items() if not k.startswith('_FEATURE_ID_')}
            else:
                for primitive in j['meshes'][0]['primitives']:
                    attrs=primitive['attributes'];attrs['NORMAL'],attrs['TEXCOORD_0']=attrs['TEXCOORD_0'],attrs['NORMAL']
            data=json.dumps(j).encode();data+=b' '*(-len(data)%4)
            p.write_bytes(b'glTF'+struct.pack('<III',2,28+len(data)+len(binary),len(data))+b'JSON'+data+struct.pack('<I',len(binary))+b'BIN\0'+binary)
        if name=='local-gltf-identity':
            p=out/'root.glb';b=p.read_bytes();jl=int.from_bytes(b[12:16],'little');j=json.loads(b[20:20+jl]);j['buffers'][0]['uri']='root.bin';(out/'root.bin').write_bytes(b[28+jl:]);(out/'root.gltf').write_text(json.dumps(j));p.unlink();d['root']['content']['uri']='root.gltf'
        if name=='wrong-representative-contents':d['root']['contents']=[d['root'].pop('content')]
        if name=='missing-leaf':d['root']['children'][0]['children'].pop()
        if name=='duplicate-leaf':
            duplicate=copy.deepcopy(d['root']['children'][0]['children'][0]);duplicate['extras']={'duplicate':True};d['root']['children'][0]['children'].append(duplicate)
        if name=='empty-intermediate':
            d['root']['children'][1].pop('content')
            (out/'right.glb').unlink()
        if name=='repeated-coarse-cell':
            d['root']=tile('root.glb',4,[d['root']])
        if name=='implicit-empty-intermediate':
            # Three available cells in one quadtree branch: content at levels
            # zero and two, an available empty level-one cell between them.
            # Level offsets are 0, 1 and 5; Morton index zero at every level.
            for uri in groups:(out/uri).unlink()
            (out/'0-0-0.glb').write_bytes(glb([0,1,2,3],leaf=False))
            (out/'2-0-0.glb').write_bytes(glb([0,1,2,3],leaf=True))
            d['root']=tile('{level}-{x}-{y}.glb',4)
            d['root']['implicitTiling']=dict(subdivisionScheme='QUADTREE',subtreeLevels=3,availableLevels=3,subtrees={'uri':'{level}-{x}-{y}.subtree'})
            subtree=dict(buffers=[{'byteLength':11}],bufferViews=[dict(buffer=0,byteOffset=0,byteLength=3),dict(buffer=0,byteOffset=8,byteLength=3)],
                tileAvailability={'bitstream':0},contentAvailability=[{'bitstream':1}],childSubtreeAvailability={'constant':0})
            data=json.dumps(subtree,separators=(',',':')).encode();data+=b' '*(-len(data)%8)
            binary=bytes([0x23])+bytes(7)+bytes([0x21])+bytes(7)
            (out/'0-0-0.subtree').write_bytes(b'subt'+struct.pack('<IQQ',1,len(data),len(binary))+data+binary)
        if name=='infinite-root-g':d['geometricError']=float('inf')
        if name=='negative-g':d['root']['geometricError']=-1
        if name=='nan-g':d['root']['geometricError']=float('nan')
        if name=='nonmonotone-g':d['root']['children'][0]['geometricError']=5
        if name=='bad-bounds':
            def shrink(node):
                node['boundingVolume']['box']=[0,0,0,1,0,0,0,1,0,0,0,1]
                for child in node.get('children',[]):shrink(child)
            shrink(d['root'])
        if name=='stale-hash':
            p=out/'root.glb';new='0'*64+'.glb';p.rename(out/new);d['root']['content']['uri']=new
        if name=='malformed-codec':
            # Add a malformed EXT_meshopt stream to a POSITION buffer view.
            p=out/'root.glb';b=p.read_bytes();jl=int.from_bytes(b[12:16],'little');j=json.loads(b[20:20+jl]);j['bufferViews'][0]['extensions']={'EXT_meshopt_compression':dict(buffer=0,byteOffset=0,byteLength=1,count=12,byteStride=12,mode='ATTRIBUTES')};j['extensionsUsed'].append('EXT_meshopt_compression');j['extensionsRequired']=['EXT_meshopt_compression'];data=json.dumps(j).encode();data+=b' '*(-len(data)%4);binary=b[28+jl:];p.write_bytes(b'glTF'+struct.pack('<III',2,28+len(data)+len(binary),len(data))+b'JSON'+data+struct.pack('<I',len(binary))+b'BIN\0'+binary)
        (out/'tileset.json').write_text(json.dumps(d))
        manifest.append(dict(name=name,oracleCheck=oracle_check,archiveCheck=archive_check,source='source-split.json' if name in ('split-side','split-side-detour') else 'source-trs.json' if name=='trs' else 'source-wide.json' if name in ('wide-identity','adjacent-uint64-leaf','adjacent-uint64-representative') else 'source.json'))
    (root/'cases.json').write_text(json.dumps(manifest,indent=2)+'\n')
    return manifest

if __name__=='__main__':
    from oracle import audit, Failure
    from continuity import check as continuity
    root=Path(sys.argv[1]);cases=generate(root);results=[]
    tree=json.loads((root/'valid'/'tileset.json').read_text())
    leading=copy.deepcopy(tree);leading['root'].pop('content')
    assert continuity(leading,lambda _:None)['ok']
    additive=copy.deepcopy(tree);additive['root']['refine']='ADD';additive['root']['children'][0].pop('content')
    assert continuity(additive,lambda _:None)['ok']
    external=copy.deepcopy(tree);external['root']['children']=[{'content':{'uri':'external.json'}}]
    assert continuity(external,lambda _:tree)['ok']
    for c in cases:
        if not c['oracleCheck'] and c['archiveCheck']:continue
        try:
            result=audit(root/c['name'],root/c['source'])
            assert c['oracleCheck'] is None, c
        except Failure as e:
            assert e.check==c['oracleCheck'], (c,e)
            result=dict(ok=False,check=e.check)
        results.append(dict(name=c['name'],**result))
    print(json.dumps(results,indent=2))
