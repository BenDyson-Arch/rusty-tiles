#!/usr/bin/env python3
"""Tiny read-only payload-fact/scene/coherence design probe, no product imports.

Uses six retained final2 sources, not producer/CLI executions. Rational extrema
are the independent reference; outward intervals model the existing private
mesh source operations. No production decoder is copied into this audit.
"""
import argparse
import base64
import copy
from fractions import Fraction as F
import gzip
import hashlib
import itertools
import json
import math
from pathlib import Path
import struct
import subprocess
import zipfile

HERE=Path(__file__).resolve().parent
CODEC=Path('/home/bend/.cache/rusty-tiles-117-browser-cache/node_modules/meshoptimizer/meshopt_decoder_reference.js')
CODEC_SHA='207f64595be3d5d0c0d728a981673c3fee8b4d9e671341831d556485d96de8dc'
I=[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1]
PASSIVE={'EXT_mesh_features','EXT_structural_metadata','EXT_mesh_polygon','KHR_materials_unlit',
         'KHR_mesh_quantization','EXT_meshopt_compression','KHR_mesh_primitive_restart'}
BIT_LIMIT=127
OP_LIMIT=1000000


class Refusal(Exception):
    def __init__(self,kind,message):self.kind=kind;self.message=message;super().__init__(message)


def need(ok,message,kind='invalid_input'):
    if not ok:raise Refusal(kind,message)


def sha(data):return hashlib.sha256(data).hexdigest()


def bounded(value):
    x=F(value)
    need(max(abs(x.numerator).bit_length(),x.denominator.bit_length())<=BIT_LIMIT,'rational width','unsupported')
    return x


class Arithmetic:
    def __init__(self):self.ops=0;self.max_bits=0;self.max_intermediate_bits=0
    def take(self,x):
        x=bounded(x);self.max_bits=max(self.max_bits,abs(x.numerator).bit_length(),x.denominator.bit_length());return x
    def op(self,a,b,multiply=False):
        a=self.take(a);b=self.take(b);self.ops+=1
        need(self.ops<=OP_LIMIT,'rational work','resource_limit')
        # Admit before allocating unreduced cross-products, including carry.
        an=abs(a.numerator).bit_length();ad=a.denominator.bit_length()
        bn=abs(b.numerator).bit_length();bd=b.denominator.bit_length()
        width=max(an+bn,ad+bd) if multiply else max(max(an+bd,bn+ad)+1,ad+bd)
        self.max_intermediate_bits=max(self.max_intermediate_bits,width)
        need(width<=BIT_LIMIT,'rational intermediate width','unsupported')
        return self.take(a*b if multiply else a+b)
    def compare(self,a,b):
        a=self.take(a);b=self.take(b);self.ops+=1
        need(self.ops<=OP_LIMIT,'rational work','resource_limit')
        width=max(abs(a.numerator).bit_length()+b.denominator.bit_length(),abs(b.numerator).bit_length()+a.denominator.bit_length())
        self.max_intermediate_bits=max(self.max_intermediate_bits,width)
        need(width<=BIT_LIMIT,'comparison intermediate width','unsupported')
        return -1 if a<b else (1 if a>b else 0)


def envelope(data):
    wrapper=None
    if data[:4]==b'b3dm':
        need(len(data)>=28,'short b3dm');version,length,fj,fb,bj,bb=struct.unpack_from('<6I',data,4)
        need(version==1 and length==len(data),'b3dm framing')
        table=json.loads(data[28:28+fj]);wrapper={'table':table,'feature_binary':fb,'batch_json':bj,'batch_binary':bb}
        need(table=={'BATCH_LENGTH':0} and fb==bj==bb==0,'nontrivial b3dm feature/batch/RTC profile','unsupported')
        at=28+fj;need(at%8==0,'b3dm GLB alignment');data=data[at:]
        # Source wrappers have no opaque trailing payload after the inner GLB.
    need(data[:4]==b'glTF' and struct.unpack_from('<II',data,4)==(2,len(data)),'GLB framing')
    chunks=[];at=12
    while at<len(data):
        n,k=struct.unpack_from('<I4s',data,at);need(n%4==0 and at+8+n<=len(data),'GLB chunk');chunks.append((k,data[at+8:at+8+n]));at+=8+n
    need(chunks[0][0]==b'JSON','JSON first')
    return json.loads(chunks[0][1]),next((d for k,d in chunks if k==b'BIN\0'),b''),wrapper


def encode(d,binary):
    j=json.dumps(d,separators=(',',':')).encode();j+=b' '*(-len(j)%4);binary+=b'\0'*(-len(binary)%4)
    return struct.pack('<4sII',b'glTF',2,28+len(j)+len(binary))+struct.pack('<I4s',len(j),b'JSON')+j+struct.pack('<I4s',len(binary),b'BIN\0')+binary


def facts(data,files,parent):
    d,bin,wrapper=envelope(data)
    used=set(d.get('extensionsUsed',[]));need(used<=PASSIVE,'unknown spatial extension','unsupported')
    for key in ('animations','skins'):
        need(not d.get(key),'dynamic scene profile','unsupported')
    buffers=[files[str(Path(parent)/b['uri'])] if 'uri' in b else bin for b in d.get('buffers',[])]
    views=[];pending=[];indices=[]
    for index,v in enumerate(d.get('bufferViews',[])):
        if 'EXT_meshopt_compression' in v.get('extensions',{}):
            e=v['extensions']['EXT_meshopt_compression'];need(e.get('filter','NONE')=='NONE','meshopt filter','unsupported')
            at=e.get('byteOffset',0);raw=buffers[e['buffer']][at:at+e['byteLength']]
            need(len(raw)==e['byteLength'] and e['count']*e['byteStride']==v['byteLength'],'meshopt range')
            pending.append({'count':e['count'],'stride':e['byteStride'],'mode':e['mode'],'data':base64.b64encode(raw).decode()});indices.append(index);views.append(None)
        else:
            at=v.get('byteOffset',0);raw=buffers[v['buffer']][at:at+v['byteLength']];need(len(raw)==v['byteLength'],'view range');views.append(raw)
    if pending:
        result=subprocess.run(['nice','-n','10','node',str(HERE/'meshopt.mjs'),str(CODEC)],input=json.dumps(pending),text=True,capture_output=True,timeout=20)
        need(result.returncode==0,'reference meshopt '+result.stderr)
        for index,raw in zip(indices,json.loads(result.stdout)):views[index]=base64.b64decode(raw)
    def values(index,position=False):
        a=d['accessors'][index];need('sparse' not in a,'sparse accessor','unsupported')
        code={5121:'B',5123:'H',5125:'I',5126:'f'}[a['componentType']]
        components=3 if a['type']=='VEC3' else 1;need(components==3 if position else a['type']=='SCALAR','accessor type')
        if position:need((a['componentType']==5126 and not a.get('normalized',False)) or (a['componentType']==5123 and a.get('normalized',False) and 'KHR_mesh_quantization' in used),'POSITION profile','unsupported')
        v=d['bufferViews'][a['bufferView']];length=struct.calcsize('<'+code)*components;stride=v.get('byteStride',length);at=a.get('byteOffset',0)
        need(a['count']>0 and at+(a['count']-1)*stride+length<=len(views[a['bufferView']]),'accessor range')
        rows=[struct.unpack_from('<'+code*components,views[a['bufferView']],at+i*stride) for i in range(a['count'])]
        need(all(math.isfinite(x) for row in rows for x in row),'finite POSITION')
        rawlo=[min(r[c] for r in rows) for c in range(components)];rawhi=[max(r[c] for r in rows) for c in range(components)]
        if position:
            for key,actual in [('min',rawlo),('max',rawhi)]:need(a.get(key)==actual,'declared '+key+' differs from decoded POSITION')
        den=65535 if a.get('normalized',False) else 1
        return [[F(x)/den for x in r] for r in rows],rawlo,rawhi
    bindings=[]
    for mi,mesh in enumerate(d.get('meshes',[])):
        need(not mesh.get('weights'),'morph weights','unsupported')
        for pi,p in enumerate(mesh['primitives']):
            need(not p.get('targets'),'morph targets','unsupported');rows,rawlo,rawhi=values(p['attributes']['POSITION'],True)
            mode=p.get('mode',4);need(mode in (0,1,3,4),'primitive mode','unsupported')
            if 'indices' in p:
                idx,_,_=values(p['indices']);indices=[int(v[0]) for v in idx]
                reserved={5121:255,5123:65535,5125:4294967295}[d['accessors'][p['indices']]['componentType']]
                if reserved in indices:
                    need(mode==3 and 'KHR_mesh_primitive_restart' in used and 'KHR_mesh_primitive_restart' in d.get('extensionsRequired',[]),'restart profile')
                    segments=[];segment=0
                    for value in indices:
                        if value==reserved:segments.append(segment);segment=0
                        else:segment+=1
                    segments.append(segment);need(min(segments)>=2,'restart segment','unsupported')
                need(all(0<=x<len(rows) or x==reserved for x in indices),'index outside POSITION')
                referenced=[rows[x] for x in indices if x!=reserved]
            else:referenced=rows
            need(referenced,'no referenced POSITION','unsupported')
            lo=[min(r[c] for r in referenced) for c in range(3)];hi=[max(r[c] for r in referenced) for c in range(3)]
            bindings.append({'mesh':mi,'primitive':pi,'accessor':p['attributes']['POSITION'],'indices':p.get('indices'),
                             'mode':mode,'count':len(rows),'referenced_count':len(referenced),'lo':lo,'hi':hi,'rows':referenced})
    return {'document':d,'bindings':bindings,'wrapper':wrapper}


def selected_envelopes(f,arithmetic):
    d=f['document'];need('scene' in d,'no explicit default scene','unsupported')
    scene=d['scene'];need(type(scene)==int and 0<=scene<len(d.get('scenes',[])),'selected scene')
    nodes=d.get('nodes',[]);parents={};roots=d['scenes'][scene].get('nodes',[])
    for index,n in enumerate(nodes):
        for child in n.get('children',[]):
            need(type(child)==int and 0<=child<len(nodes),'child reference');need(child not in parents,'multiple node parents');parents[child]=index
    need(len(roots)==len(set(roots)),'duplicate selected root');need(all(r not in parents for r in roots),'selected child as root')
    # The glTF graph is a forest, including unselected nodes. Scene selection
    # controls evaluation, not whether invalid graph references are admitted.
    for index in range(len(nodes)):
        cursor=index;chain=set()
        while cursor in parents:
            need(cursor not in chain,'node cycle');chain.add(cursor);cursor=parents[cursor]
            need(len(chain)<=31,'scene depth','unsupported')
    result=[];seen=set()
    def visit(index,scale,translation,stack):
        need(type(index)==int and 0<=index<len(nodes),'node reference');need(index not in stack,'node cycle');need(len(stack)<31,'scene depth','unsupported')
        n=nodes[index];need('skin' not in n and not n.get('weights'),'dynamic node','unsupported')
        need(not n.get('extensions'),'node spatial extension','unsupported')
        if 'matrix' in n:
            need(not any(k in n for k in ('translation','rotation','scale')),'matrix and TRS')
            m=n['matrix'];need(len(m)==16 and m[3]==m[7]==m[11]==0 and m[15]==1,'affine node matrix')
            need(all(m[i]==0 for i in (1,2,4,6,8,9)),'rotated/sheared node matrix','unsupported');s=[m[0],m[5],m[10]];t=m[12:15]
        else:
            need(n.get('rotation',[0,0,0,1])==[0,0,0,1],'rotation profile','unsupported');s=n.get('scale',[1,1,1]);t=n.get('translation',[0,0,0])
        need(len(s)==len(t)==3 and all(math.isfinite(v) for v in s+t),'finite TS');need(all(v>0 for v in s),'positive scale profile','unsupported')
        translated=[arithmetic.op(translation[a],arithmetic.op(scale[a],t[a],True)) for a in range(3)]
        scaled=[arithmetic.op(scale[a],s[a],True) for a in range(3)]
        if 'mesh' in n:
            need(type(n['mesh'])==int and 0<=n['mesh']<len(d.get('meshes',[])),'mesh reference')
            for b in f['bindings']:
                if b['mesh']!=n['mesh']:continue
                lo=[arithmetic.op(translated[a],arithmetic.op(scaled[a],b['lo'][a],True)) for a in range(3)]
                hi=[arithmetic.op(translated[a],arithmetic.op(scaled[a],b['hi'][a],True)) for a in range(3)]
                transformed=[]
                for row in b['rows']:
                    p=[arithmetic.op(translated[a],arithmetic.op(scaled[a],row[a],True)) for a in range(3)]
                    transformed.append([p[0],-p[2],p[1]])
                actual_lo=[min(row[a] for row in transformed) for a in range(3)]
                actual_hi=[max(row[a] for row in transformed) for a in range(3)]
                need(actual_lo==[lo[0],-hi[2],lo[1]] and actual_hi==[hi[0],-lo[2],hi[1]],'extrema/reference point disagreement')
                result.append({'node':index,'binding':b,'lo':actual_lo,'hi':actual_hi})
        seen.add(index)
        for child in n.get('children',[]):visit(child,scaled,translated,stack+[index])
    for root in roots:visit(root,[F(1)]*3,[F(0)]*3,[])
    need(result,'selected scene has no supported content','unsupported')
    return result


def contain(lo,hi,box,translation,arithmetic):
    need(len(box)==12 and all(math.isfinite(v) for v in box),'finite box')
    need(all(box[i]==0 for i in (4,5,6,8,9,10)) and all(box[i]>0 for i in (3,7,11)),'axis aligned positive box','unsupported')
    for a in range(3):
        low=arithmetic.op(box[a],-F(box[3+4*a]));high=arithmetic.op(box[a],F(box[3+4*a]))
        p=arithmetic.op(lo[a],translation[a]);q=arithmetic.op(hi[a],translation[a])
        need(arithmetic.compare(low,p)<=0 and arithmetic.compare(p,q)<=0 and arithmetic.compare(q,high)<=0,'decoded content escapes own/ancestor box')


def check_tree(files,arithmetic):
    d=json.loads(files['tileset.json']);checked=0;bindings=0;wrappers=[]
    def visit(n,ancestors,path,offset):
        nonlocal checked,bindings
        current=[(n['boundingVolume']['box'],offset,path)]+ancestors
        for h in n.get('contents',[n['content']] if 'content' in n else []):
            f=facts(files[h['uri']],files,str(Path(h['uri']).parent));envelopes=selected_envelopes(f,arithmetic)
            if f['wrapper']:wrappers.append(f['wrapper'])
            for e in envelopes:
                bindings+=1
                for box,ancestor_offset,_ in current:
                    delta=[arithmetic.op(offset[a],-ancestor_offset[a]) for a in range(3)]
                    contain(e['lo'],e['hi'],box,delta,arithmetic);checked+=1
        for i,child in enumerate(n.get('children',[])):
            m=child.get('transform',I);need(len(m)==16 and m[:12]==I[:12] and m[15]==1,'child translation profile','unsupported')
            child_offset=[arithmetic.op(offset[a],m[12+a]) for a in range(3)]
            visit(child,current,path+[i],child_offset)
    # Root global matrix is intentionally not used: coherence is checked in root
    # local/ancestor frames. Its unchanged bytes/meaning have a separate owner.
    visit(d['root'],[],[],[F(0)]*3)
    return {'own_ancestor_checks':checked,'selected_primitive_instances':bindings,'b3dm_wrappers':wrappers,
            'rational_operations':arithmetic.ops,'max_reduced_integer_bits':arithmetic.max_bits,
            'max_unreduced_intermediate_bits':arithmetic.max_intermediate_bits}


def interval_op(a,b,multiply=False):
    v=[x*y for x in a for y in b] if multiply else [a[0]+b[0],a[1]+b[1]]
    return math.nextafter(min(v),-math.inf),math.nextafter(max(v),math.inf)


def arithmetic_controls():
    results=[]
    for label,value,scale,shift,boxhi in [
        ('dyadic-boundary',F(1),F(1),F(0),F(1)),
        ('normalized-third-cancels',F(21845,65535),F(3),F(0),F(1)),
        ('outside-by-exact-half-ulp',F(1),F(1),F(0),F(.5)+F(math.nextafter(.5,-math.inf)))] :
        a=Arithmetic();exact=a.op(a.op(value,scale,True),shift)
        init=(math.nextafter(float(value),-math.inf),math.nextafter(float(value),math.inf)) if value.denominator not in (1,2) else (float(value),float(value))
        interval=interval_op(interval_op(init,(float(scale),float(scale)),True),(float(shift),float(shift)))
        exact_result='contained' if a.compare(exact,boxhi)<=0 else 'invalid_input'
        interval_result='contained' if F(interval[1])<=boxhi else ('invalid_input' if F(interval[0])>boxhi else 'unsupported_cannot_prove')
        need(interval_result=='unsupported_cannot_prove','boundary interval premise')
        results.append({'label':label,'exact':str(exact),'box_high_exact':str(boxhi),'exact_result':exact_result,
                        'outward_interval':[repr(v) for v in interval],'interval_result':interval_result,
                        'ordinary_float_box_high':repr(float(boxhi)),'ordinary_float_would_contain':float(exact)<=float(boxhi)})
    try:Arithmetic().take(F(2)**1000);need(False,'width control insensitive')
    except Refusal as e:need(e.kind=='unsupported','width kind');results.append({'label':'finite-over-width','result':e.kind})
    try:Arithmetic().op(F(1,2**70),F(1,2**70));need(False,'intermediate width control insensitive')
    except Refusal as e:need(e.kind=='unsupported','intermediate width kind');results.append({'label':'unreduced-intermediate-over-width-despite-small-result','result':e.kind})
    try:a=Arithmetic();a.ops=OP_LIMIT;a.op(F(1),F(0));need(False,'work control insensitive')
    except Refusal as e:need(e.kind=='resource_limit','work kind');results.append({'label':'at-work-ceiling-then-one-more','result':e.kind})
    return results


def main():
    p=argparse.ArgumentParser();p.add_argument('--artifacts',type=Path,required=True);p.add_argument('--work',type=Path,required=True)
    args=p.parse_args();work=args.work.resolve();need(str(work).startswith('/tmp/rusty-tiles-a2-probe-artifacts'),'work namespace');work.mkdir(parents=True,exist_ok=False)
    need(sha(CODEC.read_bytes())==CODEC_SHA,'reference codec identity')
    root=HERE.parents[3];retained=json.loads(gzip.decompress((root/'bench/architecture_audit/implicit_rewrite/a2_real_sources'/('native-raw.json.gz' if 'native' in args.artifacts.name else 'portable-raw.json.gz')).read_bytes()))
    labels=['point-flat-rounded','vector-flat-rounded','vector-quantized-compressed','vector-fragmented-line','fragmented-array-leaf','shared-relative-resources']
    sources={};observations=[]
    for label in labels:
        path=args.artifacts/(label+'.3tz');pin=next(c for c in retained['cases'] if c['label']==label);need(sha(path.read_bytes())==pin['source_sha256'],'source archive identity')
        with zipfile.ZipFile(path) as z:files={n:z.read(n) for n in z.namelist() if n!='@3dtilesIndex1@'}
        need({n:sha(v) for n,v in files.items()}==pin['source_members'],'source member identities')
        sources[label]=files;observations.append({'label':label,'source_sha256':pin['source_sha256'],**check_tree(files,Arithmetic())})
    base=sources['vector-flat-rounded'];doc=json.loads(base['tileset.json']);leaf=doc['root']['children'][0];uri=leaf['content']['uri'];gd,gb,_=envelope(base[uri]);controls=[]
    def controlled(label,files,expected='invalid_input'):
        try:check_tree(files,Arithmetic());result='admitted';message=None
        except Refusal as e:result=e.kind;message=e.message
        need(result==expected,'insensitive '+label+' '+str(result));controls.append({'label':label,'result':result,'reason':message})
    def edited_document(label,edit,expected='invalid_input'):
        f=copy.deepcopy(base);d=copy.deepcopy(gd);edit(d);f[uri]=encode(d,gb);controlled(label,f,expected)
    changed=bytearray(gb);a=gd['accessors'][gd['meshes'][0]['primitives'][0]['attributes']['POSITION']];v=gd['bufferViews'][a['bufferView']];struct.pack_into('<f',changed,v.get('byteOffset',0)+a.get('byteOffset',0),500.)
    d=copy.deepcopy(gd);position=d['accessors'][d['meshes'][0]['primitives'][0]['attributes']['POSITION']]
    stride=v.get('byteStride',12);at=v.get('byteOffset',0)+a.get('byteOffset',0)
    rows=[struct.unpack_from('<3f',changed,at+i*stride) for i in range(a['count'])]
    position['min']=[min(row[c] for row in rows) for c in range(3)];position['max']=[max(row[c] for row in rows) for c in range(3)]
    f=copy.deepcopy(base);f[uri]=encode(d,bytes(changed));controlled('wrong-POSITION-with-correct-minmax',f)
    # The same valid POSITION outlier becomes unreferenced. Whole-accessor bounds
    # cannot certify rendered geometry; primitive reference bounds can.
    index=d['accessors'][d['meshes'][0]['primitives'][0]['indices']];iv=d['bufferViews'][index['bufferView']]
    unreferenced=bytearray(changed);struct.pack_into('<I',unreferenced,iv.get('byteOffset',0)+index.get('byteOffset',0),1)
    f=copy.deepcopy(base);f[uri]=encode(d,bytes(unreferenced));controlled('unreferenced-POSITION-outlier',f,'admitted')
    d=copy.deepcopy(gd);extra=gb+struct.pack('<3f',500,500,500);newview=len(d['bufferViews']);newaccessor=len(d['accessors'])
    d['buffers'][0]['byteLength']=len(extra);d['bufferViews'].append({'buffer':0,'byteOffset':len(gb),'byteLength':12})
    d['accessors'].append({'bufferView':newview,'componentType':5126,'count':1,'type':'VEC3','min':[500,500,500],'max':[500,500,500]})
    d['meshes'].append({'primitives':[{'attributes':{'POSITION':newaccessor},'mode':0}]});f=copy.deepcopy(base);f[uri]=encode(d,extra);controlled('unreferenced-mesh-outlier',f,'admitted')
    edited_document('wrong-minmax',lambda d:d['accessors'][0]['max'].__setitem__(0,500.))
    edited_document('wrong-node-translation',lambda d:d['nodes'][0].__setitem__('translation',[1000,0,0]))
    edited_document('unsupported-rotation',lambda d:d['nodes'][0].__setitem__('rotation',[0,0,1,0]),'unsupported')
    edited_document('unsupported-skin-index-zero',lambda d:d['nodes'][0].__setitem__('skin',0),'unsupported')
    edited_document('invalid-mesh-reference',lambda d:d['nodes'][0].__setitem__('mesh',999))
    edited_document('unsupported-sparse',lambda d:d['accessors'][0].__setitem__('sparse',{}),'unsupported')
    edited_document('unsupported-no-default-scene',lambda d:d.pop('scene'),'unsupported')
    edited_document('unsupported-unknown-spatial-extension',lambda d:d.setdefault('extensionsUsed',[]).append('VENDOR_displacement'),'unsupported')
    edited_document('invalid-selected-child-root',lambda d:(d['nodes'].append({'children':[0]}),d['scenes'][0].__setitem__('nodes',[0])))
    edited_document('invalid-unselected-node-cycle',lambda d:d['nodes'].append({'children':[len(d['nodes'])]}))
    def nested(d):
        d['nodes'][0]={'children':[1],'scale':[.5,.5,.5],'translation':[1/8192,1/8192,1/8192]}
        d['nodes'].append({'mesh':0,'scale':[1,.5,.5],'translation':[1/8192,0,0]})
    edited_document('positive-nested-nonuniform-TS',nested,'admitted')
    edited_document('positive-diagonal-matrix',lambda d:d['nodes'][0].__setitem__('matrix',[.5,0,0,0,0,.25,0,0,0,0,.25,0,3/16384,1/8192,1/8192,1]),'admitted')
    # A valid extra unselected scene must not change default scene geometry.
    d=copy.deepcopy(gd);d['nodes'].append({'mesh':0,'translation':[1000,0,0]});d['scenes'].append({'nodes':[1]});f=copy.deepcopy(base);f[uri]=encode(d,gb)
    check_tree(f,Arithmetic());controls.append({'label':'unselected-scene-is-not-rendered','result':'admitted'})
    d['scene']=1;f[uri]=encode(d,gb);controlled('wrong-selected-scene',f)
    f=copy.deepcopy(base);m=json.loads(f['tileset.json']);m['root']['children'][0]['boundingVolume']['box'][3]=0.000001;f['tileset.json']=json.dumps(m).encode();controlled('authoritative-own-box-too-small',f)
    f=copy.deepcopy(base);m=json.loads(f['tileset.json']);m['root']['children'][0]['transform'][12]+=1000;f['tileset.json']=json.dumps(m).encode();controlled('translated-child-escapes-ancestor',f)
    f=copy.deepcopy(base);m=json.loads(f['tileset.json']);m['root']['transform']=[0,1,0,0,-1,0,0,0,0,0,1,0,6000000,2000,3000,1];f['tileset.json']=json.dumps(m).encode();check_tree(f,Arithmetic());controls.append({'label':'changed-global-frame-is-separate-preservation-obligation','result':'local_coherence_unchanged','source_global_matrix_equal':m['root']['transform']==doc['root']['transform']})
    q=sources['vector-quantized-compressed'];m=json.loads(q['tileset.json']);quri=m['root']['children'][0]['content']['uri'];d,b,_=envelope(q[quri]);d['nodes'][0]['scale'][0]*=100000;f=copy.deepcopy(q);f[quri]=encode(d,b);controlled('wrong-quantized-nonuniform-scale',f)
    array=sources['fragmented-array-leaf'];wrapper_uri=next(k for k,v in array.items() if v[:4]==b'b3dm');innerd,innerb,_=envelope(array[wrapper_uri]);inner=encode(innerd,innerb)
    for label,table in [('unsupported-b3dm-RTC',{'BATCH_LENGTH':0,'RTC_CENTER':[1,2,3]}),('unsupported-b3dm-batch',{'BATCH_LENGTH':1})]:
        tablebytes=json.dumps(table,separators=(',',':')).encode();tablebytes+=b' '*(-(28+len(tablebytes))%8)
        f=copy.deepcopy(array);f[wrapper_uri]=struct.pack('<4s6I',b'b3dm',1,28+len(tablebytes)+len(inner),len(tablebytes),0,0,0)+tablebytes+inner;controlled(label,f,'unsupported')
    arithmetic=arithmetic_controls()
    inputs={name:sha((root/name).read_bytes()) for name in ('src/validate/payload.rs','src/validate.rs','src/validate/json.rs','src/validate/types.rs','src/mesh_archive/source/original_bounds.rs','src/mesh_archive/approximation/certificate.rs','Cargo.toml','Cargo.lock')}
    receipt={'baseline':'e4d90897518d2b569fb5e99b876581523e4c418c (coordinator supplied)','classification':'read-only independent model/fact controls; no product execution/acceptance',
             'probe_sha256':sha(Path(__file__).read_bytes()),'wrapper_sha256':sha((HERE/'meshopt.mjs').read_bytes()),'codec_sha256':CODEC_SHA,
             'retained_real_probe_sha256':retained['pins']['probe_sha256'],'inspected_source_sha256':inputs,'observations':observations,'controls':controls,'arithmetic_controls':arithmetic,
             'limits_model':{'rational_numerator_denominator_and_intermediate_bits':BIT_LIMIT,'rational_operation_count':OP_LIMIT,'selected_scene_depth':31},
             'method':{'producer_executions':0,'converter_executions':0,'cargo_builds':0,'codec_priority':10,'codec_timeout_seconds':20,'process_concurrency':1}}
    (work/'receipt.json').write_text(json.dumps(receipt,indent=2,sort_keys=True)+'\n');print(json.dumps({'observations':len(observations),'controls':len(controls),'arithmetic_controls':len(arithmetic),'probe_sha256':receipt['probe_sha256']}))


if __name__=='__main__':main()
