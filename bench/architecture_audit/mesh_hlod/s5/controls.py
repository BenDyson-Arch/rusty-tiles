#!/usr/bin/env python3
"""Literal analytic bytes, source expectations and named negative controls; no producer imports."""
import copy
import base64
import hashlib
import json
from pathlib import Path
import struct
import sys
from oracle import audit, Failure
from continuity import check as continuity

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

def transformed(p, trs=False):
    # outer translation [8,4,2], inner column-major matrix: rotate 90 deg
    # about Z, with nonuniform/reflected scale [-2,4,1].
    x,y,z=p
    return [8+2*x,4-4*y,2+z] if trs else [8-4*y,4-2*x,2+z]

def glb(face_ids, quantized=False, wrong_id=False, moved=False, bad_metadata=False, trs=False, normalized=False, leaf=True):
    binary=bytearray(); views=[]; accessors=[]
    def view(data):
        while len(binary)%8: binary.append(0)
        views.append(dict(buffer=0,byteOffset=len(binary),byteLength=len(data)));binary.extend(data)
        return len(views)-1
    def accessor(data,ct,shape,count,**extra):
        accessors.append(dict(bufferView=view(data),componentType=ct,type=shape,count=count,**extra))
        return len(accessors)-1
    vertices=[list(p) for i in face_ids for p in POINTS[i]]
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
    sets=[];attrs={'POSITION':pos}
    classes={};tables=[]
    for si,label in enumerate(['source_primitive','source_triangle'] if leaf else ['source_primitive']):
        ids=[j for j,_ in enumerate(face_ids) for _ in range(3)]
        if wrong_id: ids[0]=999
        attrs[f'_FEATURE_ID_{si}']=accessor(struct.pack('<'+'f'*len(ids),*ids),5126,'SCALAR',len(ids))
        schema={};props={}
        columns={'node_index':[0]*len(face_ids),'mesh_index':[0]*len(face_ids),'primitive_index':face_ids,
                 'expanded_ordinal':face_ids,'material_index':[i%2 for i in face_ids]}
        if si:
            columns.update(triangle_index=[0]*len(face_ids),vertex_0=[0]*len(face_ids),vertex_1=[1]*len(face_ids),vertex_2=[2]*len(face_ids))
        for key,values in columns.items():
            data=struct.pack('<'+'I'*len(values),*values)
            schema[key]=dict(type='SCALAR',componentType='UINT32',required=True)
            props[key]=dict(values=view(data))
        if bad_metadata: props['node_index']['values']=999
        classes[label]=dict(properties=schema)
        tables.append(dict(count=len(face_ids),**{'class':label},properties=props))
        sets.append(dict(featureCount=len(face_ids),attribute=si,propertyTable=si,label=label))
    primitives=[]
    for rank,identity in enumerate(face_ids):
        indices=accessor(struct.pack('<III',rank*3,rank*3+1,rank*3+2),5125,'SCALAR',3)
        primitives.append(dict(attributes=attrs,indices=indices,material=identity%2,extensions={'EXT_mesh_features':{'featureIds':sets}}))
    doc=dict(asset={'version':'2.0'},scene=0,scenes=[{'nodes':[0]}],nodes=[{'translation':[8,4,2],'children':[1]},
             {'matrix':[0,-2,0,0,-4,0,0,0,0,0,1,0,0,0,0,1],'mesh':0}],
             meshes=[{'primitives':primitives}],materials=MATERIALS,buffers=[{'byteLength':len(binary)}],bufferViews=views,accessors=accessors,
             extensionsUsed=['EXT_mesh_features','EXT_structural_metadata','KHR_materials_unlit'],
             extensions={'EXT_structural_metadata':{'schema':{'classes':classes},'propertyTables':tables}})
    if trs:doc['nodes'][1]={'rotation':[0,0,1,0],'scale':[-2,4,1],'mesh':0}
    if normalized:
        doc['nodes'][1].pop('mesh');doc['nodes'][1]['children']=[2];doc['nodes'].append({'scale':[32767]*3,'mesh':0})
    if quantized or normalized: doc['extensionsUsed'].append('KHR_mesh_quantization');doc['extensionsRequired']=['KHR_mesh_quantization']
    data=json.dumps(doc,separators=(',',':')).encode();data+=b' '*(-len(data)%4);binary+=b'\0'*(-len(binary)%4)
    return b'glTF'+struct.pack('<III',2,28+len(data)+len(binary),len(data))+b'JSON'+data+struct.pack('<I',len(binary))+b'BIN\0'+binary

def generate(root):
    root=Path(root);root.mkdir(parents=True,exist_ok=True)
    expected={'triangles':{f'0,0,{i},0':dict(xyz=[transformed(p) for p in face],vertices=[0,1,2],material=MATERIALS[i%2],expanded_ordinal=i,material_index=i%2) for i,face in enumerate(POINTS)}}
    expected_trs=copy.deepcopy(expected)
    for i,face in enumerate(POINTS):expected_trs['triangles'][f'0,0,{i},0']['xyz']=[transformed(p,True) for p in face]
    (root/'source-trs.json').write_text(json.dumps(expected_trs,sort_keys=True,indent=2)+'\n')
    (root/'source.json').write_text(json.dumps(expected,sort_keys=True,indent=2)+'\n')
    def tile(uri,g,children=None):
        d=dict(boundingVolume={'box':[0,0,0,32,0,0,0,32,0,0,0,32]},geometricError=g,refine='REPLACE',content={'uri':uri})
        if children:d['children']=children
        return d
    left=tile('left.glb',2,[tile('leaf0.glb',0),tile('leaf1.glb',0)])
    right=tile('right.glb',2,[tile('middle.glb',1,[tile('leaf2.glb',0),tile('leaf3.glb',0)])])
    doc=dict(asset={'version':'1.1'},geometricError=4,root=tile('root.glb',4,[left,right]))
    groups={'root.glb':[0,1,2,3],'left.glb':[0,1],'right.glb':[2,3],'middle.glb':[2,3],**{f'leaf{i}.glb':[i] for i in range(4)}}
    cases=[('valid',None,None),('quantized',None,None),('normalized',None,None),('trs',None,None),('numeric-lexemes',None,None),('opaque-extras',None,None),('wrong-feature','featureIdentity','hlodFeatureIds'),
           ('wrong-source','sourceIdentity',None),('wrong-representative','representativeIdentity',None),('wrong-representative-contents','representativeIdentity',None),('missing-leaf','leafCoverage',None),('duplicate-leaf','leafCoverage',None),
           ('moved-interface','interfaceSegments',None),('moved-corner','interfacePoints',None),('empty-intermediate','replaceContentContinuity',None),
           ('negative-g','geometricErrorFinite','tileset schema'),('nan-g','geometricErrorFinite','JSON'),
           ('nonmonotone-g','geometricErrorOrder','geometricError increases'),('bad-bounds',None,'decoded content'),
           ('stale-hash',None,'content checksum mismatch'),('malformed-codec',None,'meshopt'),('bad-metadata','featureIdentity','hlodFeatureIds'),('optional-property','featureIdentity','hlodFeatureIds'),('bad-property-type','featureIdentity','hlodFeatureIds'),('wrong-table-class','featureIdentity','hlodFeatureIds'),('missing-feature-set','sourceIdentity','hlodFeatureIds'),('correlated-metadata','sourceIdentity',None),('active-morph','codecStructure',None),('infinite-root-g','geometricErrorFinite','JSON'),('normalized-identity','featureIdentity','hlodFeatureIds'),('array-identity','featureIdentity','hlodFeatureIds'),('offset-identity','featureIdentity','hlodFeatureIds'),('null-identity','featureIdentity','hlodFeatureIds'),('gpu-instancing','codecStructure',None),('source-triangle-only','featureIdentity','hlodFeatureIds'),('local-gltf-identity',None,'hlodFeatureIds'),('coarse-material-class','representativeMaterialClass',None),('uri-buffer-shadow','codecStructure','hlodFeatureIds'),('wrong-bin-chunk','codecStructure','buffer has no resource')]
    manifest=[]
    for name,oracle_check,archive_check in cases:
        out=root/name;out.mkdir(exist_ok=True);d=copy.deepcopy(doc)
        for uri,ids in groups.items():
            data=glb(ids,quantized=name=='quantized',wrong_id=name=='wrong-feature' and uri=='root.glb',moved=(name=='moved-interface' and uri=='left.glb') or (name=='moved-corner' and uri=='right.glb'),bad_metadata=name=='bad-metadata' and uri=='root.glb',trs=name=='trs',normalized=name=='normalized',leaf=uri.startswith('leaf'))
            (out/uri).write_bytes(data)
        if name in ('numeric-lexemes','opaque-extras','optional-property','correlated-metadata','active-morph','bad-property-type','wrong-table-class','missing-feature-set','normalized-identity','array-identity','offset-identity','null-identity','gpu-instancing','source-triangle-only','coarse-material-class','uri-buffer-shadow'):
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
                if name=='correlated-metadata':
                    for table in j['extensions']['EXT_structural_metadata']['propertyTables']:
                        for key in ('expanded_ordinal','material_index'):
                            v=j['bufferViews'][table['properties'][key]['values']]
                            for i in range(table['count']):struct.pack_into('<I',binary,v['byteOffset']+i*4,12345)
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
        if name=='local-gltf-identity':
            p=out/'root.glb';b=p.read_bytes();jl=int.from_bytes(b[12:16],'little');j=json.loads(b[20:20+jl]);j['buffers'][0]['uri']='root.bin';(out/'root.bin').write_bytes(b[28+jl:]);(out/'root.gltf').write_text(json.dumps(j));p.unlink();d['root']['content']['uri']='root.gltf'
        if name=='wrong-representative-contents':d['root']['contents']=[d['root'].pop('content')]
        if name=='missing-leaf':d['root']['children'][0]['children'].pop()
        if name=='duplicate-leaf':
            duplicate=copy.deepcopy(d['root']['children'][0]['children'][0]);duplicate['extras']={'duplicate':True};d['root']['children'][0]['children'].append(duplicate)
        if name=='empty-intermediate':
            d['root']['children'][1].pop('content')
            (out/'right.glb').unlink()
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
        manifest.append(dict(name=name,oracleCheck=oracle_check,archiveCheck=archive_check,source='source-trs.json' if name=='trs' else 'source.json'))
    (root/'cases.json').write_text(json.dumps(manifest,indent=2)+'\n')
    return manifest

if __name__=='__main__':
    root=Path(sys.argv[1]);cases=generate(root);results=[]
    tree=json.loads((root/'valid'/'tileset.json').read_text())
    leading=copy.deepcopy(tree);leading['root'].pop('content')
    assert continuity(leading,lambda _:None)['ok']
    additive=copy.deepcopy(tree);additive['root']['refine']='ADD';additive['root']['children'][0].pop('content')
    assert continuity(additive,lambda _:None)['ok']
    external=copy.deepcopy(tree);external['root']['children']=[{'content':{'uri':'external.json'}}]
    assert continuity(external,lambda _:tree)['ok']
    for c in cases:
        if not c['oracleCheck'] and c['name'] not in ('valid','quantized','normalized','trs','numeric-lexemes','opaque-extras'):continue
        try:
            result=audit(root/c['name'],root/c['source'])
            assert c['oracleCheck'] is None, c
        except Failure as e:
            assert e.check==c['oracleCheck'], (c,e)
            result=dict(ok=False,check=e.check)
        results.append(dict(name=c['name'],**result))
    print(json.dumps(results,indent=2))
