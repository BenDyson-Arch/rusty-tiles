#!/usr/bin/env python3
"""Independent review inputs and explicit arithmetic; no production/oracle imports."""
import argparse, copy, hashlib, json, math, pathlib, struct, subprocess, zipfile, zlib

ROOT = pathlib.Path(__file__).parent
POS = [[0.,0.,0.], [1.,0.,0.], [0.,1.,0.]]
UV0 = [[0,0], [65535,0], [0,32768]]
UV1 = [[255,128], [64,255], [0,0]]
COL = [[255,0,128,17], [0,255,0,191], [64,128,255,255]]
NOR = [[0.,0.,1.]] * 3
TAN = [[1.,0.,0.,1.]] * 3

def png():
    def chunk(k,d): return struct.pack('>I',len(d))+k+d+struct.pack('>I',zlib.crc32(k+d))
    return b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',1,1,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(bytes([0,80,120,220,255])))+chunk(b'IEND',b'')

def fixture():
    payload=bytearray(); views=[]; acc=[]; attrs={}
    def add(name, rows, component=5126, normalized=False, bounds=False):
        while len(payload)%4: payload.append(0)
        offset=len(payload); width=len(rows[0]); fmt={5126:'f',5123:'H',5121:'B'}[component]
        stride=(width*struct.calcsize(fmt)+3)//4*4
        for row in rows:
            payload.extend(struct.pack('<'+fmt*width,*row));payload.extend(bytes(stride-width*struct.calcsize(fmt)))
        view={'buffer':0,'byteOffset':offset,'byteLength':len(rows)*stride,'target':34962}
        if stride!=width*struct.calcsize(fmt): view['byteStride']=stride
        views.append(view); a={'bufferView':len(views)-1,'componentType':component,'count':len(rows),'type':'VEC'+str(width)}
        if normalized: a['normalized']=True
        if bounds:
            a['min']=[min(row[c] for row in rows) for c in range(width)];a['max']=[max(row[c] for row in rows) for c in range(width)]
        acc.append(a);attrs[name]=len(acc)-1
    add('POSITION',POS,bounds=True);add('NORMAL',NOR);add('TANGENT',TAN,bounds=True)
    add('TEXCOORD_0',UV0,5123,True,True);add('TEXCOORD_1',UV1,5121,True,True);add('COLOR_0',COL,5121,True,True)
    primitive={'attributes':attrs,'material':0}
    material={'pbrMetallicRoughness':{'baseColorTexture':{'index':2,'texCoord':1},'metallicRoughnessTexture':{'index':0}},'normalTexture':{'index':1,'texCoord':1,'scale':-2.},'occlusionTexture':{'index':0,'texCoord':1,'strength':0.},'emissiveTexture':{'index':2},'emissiveFactor':[0.2,0.3,0.4]}
    doc={'asset':{'version':'2.0'},'buffers':[{'uri':'data.bin','byteLength':len(payload)}],'bufferViews':views,'accessors':acc,'meshes':[{'primitives':[primitive]}],'materials':[material,{}],'textures':[{'source':0,'sampler':1},{'source':0,'sampler':0},{'source':1}],'images':[{'uri':'a.png'},{'uri':'b.png'},{'uri':'unused.png'}],'samplers':[{'wrapS':33071},{'wrapT':33648},{}],'nodes':[{'mesh':0}],'scenes':[{'nodes':[0]}],'scene':0}
    return doc, bytes(payload)

def glb(raw):
    length,kind=struct.unpack_from('<II',raw,12); assert kind==0x4e4f534a
    doc=json.loads(raw[20:20+length]);n,k=struct.unpack_from('<II',raw,20+length);assert k==0x004e4942
    return doc,raw[28+length:28+length+n]

def channels(doc,bin):
    out={}
    for key,ai in doc['meshes'][0]['primitives'][0]['attributes'].items():
        a=doc['accessors'][ai];v=doc['bufferViews'][a['bufferView']];w=int(a['type'][3:]); assert a['componentType']==5126
        offset=v.get('byteOffset',0)+a.get('byteOffset',0);stride=v.get('byteStride',w*4)
        out[key]=[list(struct.unpack_from('<'+'f'*w,bin,offset+i*stride)) for i in range(a['count'])]
    return out

def check(expected,actual):
    assert expected.keys()==actual.keys(),(expected.keys(),actual.keys())
    for key,rows in expected.items():
        assert len(rows)==len(actual[key])
        for row,out in zip(rows,actual[key]):
            assert len(row)==len(out)
            assert all(abs(x-y)<=max(1.,abs(x))*2**-23 for x,y in zip(row,out)),(key,row,out)

def base_expected():
    return {'POSITION':POS,'NORMAL':NOR,'TANGENT':TAN,'TEXCOORD_0':[[x/65535 for x in row] for row in UV0],'TEXCOORD_1':[[x/255 for x in row] for row in UV1],'COLOR_0':[[x/255 for x in row] for row in COL]}

def run(binary):
    results=[]
    def case(name,modify=lambda d,b: (d,b),expected=None,kind=None,missing=False):
        directory=ROOT/name;directory.mkdir(exist_ok=True);doc,data=fixture();doc,data=modify(doc,data)
        (directory/'source.gltf').write_text(json.dumps(doc));(directory/'data.bin').write_bytes(data)
        for image in ['a.png','b.png','unused.png']:
            if not missing: (directory/image).write_bytes(png())
        preserved={name:(directory/name).read_bytes() for name in ['source.gltf','data.bin']+([] if missing else ['a.png','b.png','unused.png'])}
        target=directory/'result.3tz'
        if target.exists():target.unlink()
        p=subprocess.run([str(binary),'--json','mesh-local-to-3tz','-i',str(directory/'source.gltf'),'-o',str(target),'--leaf-triangles','1'],capture_output=True,text=True)
        assert all((directory/name).read_bytes()==content for name,content in preserved.items()),name
        (directory/'stdout.json').write_text(p.stdout);(directory/'stderr.txt').write_text(p.stderr)
        if kind:
            assert p.returncode!=0,(name,p.stdout,p.stderr)
            message=json.loads(p.stdout)
            assert message['error']['kind']=={'InvalidInput':'invalid_input','Unsupported':'unsupported'}[kind],(name,kind,message)
            assert not target.exists();assert not list(directory.glob('.mesh-*'))
        else:
            assert p.returncode==0,(name,p.stdout,p.stderr)
            with zipfile.ZipFile(target) as archive:
                leaves=[n for n in archive.namelist() if n.endswith('.glb')];assert len(leaves)==1
                out,blob=glb(archive.read(leaves[0]));check(expected or base_expected(),channels(out,blob))
                assert archive.read('textures/0.png')==png();assert archive.read('textures/1.png')==png()
                assert 'textures/2.png' not in archive.namelist()
                assert len(out['textures'])==3 and len(out['images'])==2 and len(out['samplers'])==2
                assert out['materials']==doc['materials'][:1],(name,out['materials'],doc['materials'])
                assert out['textures']==doc['textures'];assert out['samplers']==doc['samplers'][:2]
                report=json.loads(archive.read('conversion.json'));assert report['profile']=='f1b-core-pbr-gltf-v1'
        results.append({'case':name,'expected_kind':kind or 'success','pass':True})
    case('all-slots-normalized')
    def unnormalized_role(role):
        def modify(d,b):
            a=d['accessors'][d['meshes'][0]['primitives'][0]['attributes'][role]];a['normalized']=False;d['buffers'][0]['uri']='missing-before-I-O.bin';return d,b
        return modify
    case('color-uint-unnormalized-before-I-O',unnormalized_role('COLOR_0'),kind='InvalidInput',missing=True)
    case('uv-uint-unnormalized-before-I-O',unnormalized_role('TEXCOORD_0'),kind='InvalidInput',missing=True)
    def unused_generic(component,shape,normalized=False):
        def modify(d,b):
            size={5120:1,5121:1,5122:2,5123:2,5125:4,5126:4}[component]
            if shape.startswith('MAT'):
                columns=int(shape[-1]);column_stride=(columns*size+3)//4*4;stride=columns*column_stride;last=(columns-1)*column_stride+columns*size
            else:
                stride=last=(1 if shape=='SCALAR' else int(shape[-1]))*size
            payload=bytearray(b)
            while len(payload)%4:payload.append(0)
            offset=len(payload);length=2*stride+last;payload.extend(bytes(length))
            d['bufferViews'].append({'buffer':0,'byteOffset':offset,'byteLength':length})
            d['accessors'].append({'bufferView':len(d['bufferViews'])-1,'componentType':component,'type':shape,'count':3,'normalized':normalized})
            d['buffers'][0].update({'uri':'missing-before-I-O.bin','byteLength':len(payload)});return d,bytes(payload)
        return modify
    for label,component,shape,normalized,kind in [
        ('signed-byte',5120,'VEC3',True,'Unsupported'),
        ('signed-short',5122,'VEC4',False,'Unsupported'),
        ('scalar-float',5126,'SCALAR',False,'Unsupported'),
        ('matrix-float',5126,'MAT4',False,'Unsupported'),
        ('matrix-u8-padded',5121,'MAT2',False,'Unsupported'),
        ('matrix-u16-padded',5123,'MAT3',False,'Unsupported'),
        ('normalized-float',5126,'VEC4',True,'InvalidInput'),
        ('normalized-u32',5125,'VEC4',True,'InvalidInput')]:
        case('unused-'+label+'-before-I-O',unused_generic(component,shape,normalized),kind=kind,missing=True)
    def rgb(d,b):
        a=d['accessors'][d['meshes'][0]['primitives'][0]['attributes']['COLOR_0']];a['type']='VEC3';a['min']=a['min'][:3];a['max']=a['max'][:3]
        d['bufferViews'][a['bufferView']]['byteStride']=4;return d,b
    rgb_expected=copy.deepcopy(base_expected());rgb_expected['COLOR_0']=[row[:3]+[1.] for row in rgb_expected['COLOR_0']]
    case('normalized-rgb-alpha',rgb,rgb_expected)
    def replace_float_color(d,b,rows):
        payload=bytearray(b);offset=len(payload);payload.extend(b''.join(struct.pack('<ffff',*row) for row in rows))
        a=d['accessors'][d['meshes'][0]['primitives'][0]['attributes']['COLOR_0']];a['componentType']=5126;a.pop('normalized');a.pop('min');a.pop('max')
        d['bufferViews'][a['bufferView']]={'buffer':0,'byteOffset':offset,'byteLength':48,'target':34962};d['buffers'][0]['byteLength']=len(payload)
        return d,bytes(payload)
    def bad_color(d,b):return replace_float_color(d,b,[[1.01,0,0,1]]*3)
    case('finite-color-bounded-refusal',bad_color,kind='Unsupported')
    def nonfinite_color(d,b):return replace_float_color(d,b,[[float('nan'),0,0,1]]*3)
    case('nonfinite-color',nonfinite_color,kind='InvalidInput')
    def mixed_w(d,b):
        data=bytearray(b);a=d['accessors'][2];offset=d['bufferViews'][a['bufferView']]['byteOffset'];struct.pack_into('<f',data,offset+16+12,-1.);a['min'][3]=-1.;return d,bytes(data)
    case('mixed-triangle-handedness',mixed_w,kind='Unsupported')
    def reflect(d,b):d['nodes'][0]['scale']=[-2,2,2];return d,b
    reflected=copy.deepcopy(base_expected());order=[0,2,1]
    reflected={key:[rows[i] for i in order] for key,rows in reflected.items()}
    reflected['POSITION']=[[-2*x,2*y,2*z] for x,y,z in reflected['POSITION']]
    reflected['TANGENT']=[[-1.,0.,0.,-1.]]*3
    case('reflection-permutation',reflect,reflected)
    def nonconformal(d,b):d['nodes'][0]['scale']=[2,1,1];return d,b
    case('nonconformal',nonconformal,kind='Unsupported')
    def cancel_scales(d,b):d['nodes']=[{'scale':[2,1,1],'children':[1]},{'scale':[0.5,1,1],'mesh':0}];return d,b
    case('accumulated-conformal',cancel_scales)
    def nested_nonconformal(d,b):
        q=math.sqrt(0.5);d['nodes']=[{'scale':[2,1,1],'children':[1]},{'rotation':[0,0,q,q],'scale':[0.5,1,1],'mesh':0}];return d,b
    case('accumulated-nonconformal',nested_nonconformal,kind='Unsupported')
    def bad_fourth(d,b):d['accessors'][d['meshes'][0]['primitives'][0]['attributes']['COLOR_0']]['max'][3]=254;return d,b
    case('raw-fourth-extrema-mismatch',bad_fourth,kind='InvalidInput')
    def lost_uv(d,b):del d['meshes'][0]['primitives'][0]['attributes']['TEXCOORD_1'];return d,b
    case('bound-uv-missing',lost_uv,kind='InvalidInput')
    def unselected_lost_uv(d,b):
        p=copy.deepcopy(d['meshes'][0]['primitives'][0]);del p['attributes']['TEXCOORD_1'];d['meshes'].append({'primitives':[p]});return d,b
    case('unselected-bound-uv-missing',unselected_lost_uv,kind='InvalidInput')
    def shared_indices_short_positions(d,b):
        payload=bytearray(b);offset=len(payload);payload.extend(bytes([0,1,2,0]));d['bufferViews'].append({'buffer':0,'byteOffset':offset,'byteLength':3,'target':34963});d['accessors'].append({'bufferView':len(d['bufferViews'])-1,'componentType':5121,'count':3,'type':'SCALAR'});indices=len(d['accessors'])-1;d['meshes'][0]['primitives'][0]['indices']=indices
        offset=len(payload);payload.extend(struct.pack('<ffffff',0,0,0,1,0,0));d['bufferViews'].append({'buffer':0,'byteOffset':offset,'byteLength':24,'target':34962});d['accessors'].append({'bufferView':len(d['bufferViews'])-1,'componentType':5126,'count':2,'type':'VEC3','min':[0,0,0],'max':[1,0,0]});d['meshes'].append({'primitives':[{'attributes':{'POSITION':len(d['accessors'])-1},'indices':indices,'material':1}]});d['buffers'][0]['byteLength']=len(payload);return d,bytes(payload)
    case('shared-indices-unselected-short-position',shared_indices_short_positions,kind='InvalidInput')
    def semantic_role_reuse(d,b):
        data=bytearray(b);offset=d['bufferViews'][d['accessors'][1]['bufferView']]['byteOffset']
        for i in range(3):struct.pack_into('<fff',data,offset+i*12,-1,0,0)
        d['meshes'][0]['primitives'][0]['attributes']['COLOR_0']=1;return d,bytes(data)
    case('accessor-semantic-role-reuse',semantic_role_reuse,kind='Unsupported')
    def missing_tangent(d,b):del d['meshes'][0]['primitives'][0]['attributes']['TANGENT'];return d,b
    case('metadata-before-missing-image',missing_tangent,kind='Unsupported',missing=True)
    def bad_scalar(d,b):d['materials'][0]['occlusionTexture']['strength']=1.01;return d,b
    case('invalid-scalar-before-missing-image',bad_scalar,kind='InvalidInput',missing=True)
    def nonorthogonal(d,b):
        data=bytearray(b);view=d['bufferViews'][d['accessors'][1]['bufferView']];offset=view['byteOffset']
        for i in range(3):struct.pack_into('<fff',data,offset+12*i,0.6,0,0.8)
        return d,bytes(data)
    nonorth=copy.deepcopy(base_expected());nonorth['NORMAL']=[[0.6,0,0.8]]*3
    case('authored-nonorthogonal',nonorthogonal,nonorth)
    controls=[]
    for label,mutate in [('uv-swap',lambda c:c.update({'TEXCOORD_0':c['TEXCOORD_1'],'TEXCOORD_1':c['TEXCOORD_0']})),('tangent-handedness',lambda c:c.update({'TANGENT':[[1,0,0,-1]]*3})),('color-alpha',lambda c:c['COLOR_0'][0].__setitem__(3,1.0)),('lost-color',lambda c:c.pop('COLOR_0'))]:
        corrupted=copy.deepcopy(base_expected());mutate(corrupted)
        try:check(base_expected(),corrupted)
        except AssertionError:controls.append({'control':label,'detected':True})
        else:raise AssertionError('insensitive checker: '+label)
    original,_=fixture()
    for label,mutate in [('slot-swap',lambda m:m['pbrMetallicRoughness'].update({'baseColorTexture':m['pbrMetallicRoughness']['metallicRoughnessTexture'],'metallicRoughnessTexture':m['pbrMetallicRoughness']['baseColorTexture']})),('binding-uv',lambda m:m['normalTexture'].__setitem__('texCoord',0)),('normal-scalar',lambda m:m['normalTexture'].__setitem__('scale',2.)),('occlusion-scalar',lambda m:m['occlusionTexture'].__setitem__('strength',1.))]:
        changed=copy.deepcopy(original['materials'][0]);mutate(changed)
        try:assert changed==original['materials'][0]
        except AssertionError:controls.append({'control':label,'detected':True})
        else:raise AssertionError('insensitive binding checker: '+label)
    result={'binary':str(binary),'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'cases':results,'controls':controls}
    (ROOT/'runtime-results.json').write_text(json.dumps(result,indent=2));print(json.dumps(result,indent=2))

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('binary',type=pathlib.Path);run(parser.parse_args().binary)
