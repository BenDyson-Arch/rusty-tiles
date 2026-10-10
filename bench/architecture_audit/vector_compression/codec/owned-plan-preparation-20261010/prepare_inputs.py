"""Author-only bounded fixture/pin preparation. This never executes a target or model.
Existing frozen fixtures/oracles are referenced unchanged; new numeric fixtures use finite ints.
stdlib JSONDecoder extracts targeted lexical fields, not a production JSON utility.
"""
from pathlib import Path
import json,struct,hashlib
HERE=Path(__file__).resolve().parent
OLD=Path('/tmp/rusty-tiles-next-vector-compression-audit/codec/p2-proof-preparation-20261010')
SER=OLD/'fixtures/serializer'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def dump(p,x):p.write_text(json.dumps(x,indent=2)+'\n')
D=json.JSONDecoder()
def fields(t):
    i=1; out=[]
    while True:
        while i<len(t) and t[i].isspace():i+=1
        if t[i]=='}':return out
        begin=i; key,i=D.raw_decode(t,i); lex=t[begin:i]
        while t[i].isspace():i+=1
        assert t[i]==':'; i+=1
        while t[i].isspace():i+=1
        begin=i; _,i=D.raw_decode(t,i); out.append((key,lex,t[begin:i]))
        while t[i].isspace():i+=1
        if t[i]=='}':return out
        assert t[i]==',';i+=1

def elements(t):
    i=1;out=[]
    while True:
        while t[i].isspace():i+=1
        if t[i]==']':return out
        begin=i;_,i=D.raw_decode(t,i);out.append(t[begin:i])
        while t[i].isspace():i+=1
        if t[i]==']':return out
        assert t[i]==',';i+=1

def nodes(x):
    return 1+sum(map(nodes,x.values())) if isinstance(x,dict) else 1+sum(map(nodes,x)) if isinstance(x,list) else 1

def stats(p):
    raw=(p/'input.json').read_text();obj=json.loads(raw);k=nodes(obj);root=fields(raw)
    if 'buffers' not in obj:return dict(J=len(raw.encode()),K=k,V=0,A=0)
    buf=fields(elements(next(v for key,_,v in root if key=='buffers'))[0])
    viewraw=elements(next(v for key,_,v in root if key=='bufferViews'))
    allfields=root+buf+[f for v in viewraw for f in fields(v)]
    own=[len(key.encode()) for key,lex,_ in allfields if '\\' in lex]
    v=len(viewraw);a=len(obj.get('accessors',[]));upper=min(k,4096)
    return dict(J=len(raw.encode()),K=k,V=v,A=a,arenaLen=len(allfields),borrowedKeys=len(allfields)-len(own),ownedKeyBytes=sum(own),ownedKeyLengths=own,arenaCapacity=k,viewCapacity=v,retainedRequested=40*k+96*v+sum(own),planWork=3*len(raw.encode())+8+40*k+96*upper+16*max(upper,1))

EDIT=dict(binBytes=5,fallbackBytes=48,byteOffset=0,byteLength=85,byteStride=12,count=4,metadata=False)
def glb(js,bin,metadata=False):
    j=((20+len(js)+7)//8*8-20) if metadata else (len(js)+3)//4*4
    u=(len(bin)+(7 if metadata else 3))//(8 if metadata else 4)*(8 if metadata else 4)
    return struct.pack('<4sIIII',b'glTF',2,28+j+u,j,0x4e4f534a)+js+b' '*(j-len(js))+struct.pack('<I4s',u,b'BIN\0')+bin+b'\0'*(u-len(bin))

def base(n=1):return dict(asset={'version':'2.0'},buffers=[{'byteLength':48}],bufferViews=[{'buffer':0,'byteOffset':0,'byteLength':48} for _ in range(n)],accessors=[])
def make(name,obj,raw_override=None,unsupported=False,shared=None):
    p=HERE/'fixtures'/name;p.mkdir(parents=True,exist_ok=True)
    source=raw_override if raw_override is not None else json.dumps(obj,separators=(',',':'),ensure_ascii=False)
    (p/'input.json').write_text(source);dump(p/'edits.json',EDIT);(p/'binary.bin').write_bytes(bytes([1,2,3,4,5]))
    if unsupported:expected=b''
    else:
        # Independently authored fixed fixture transformation. No source classifier/native stream.
        out=json.loads(source);out['buffers'][0]['byteLength']=5
        out['buffers'].append({'byteLength':48,'extensions':{'EXT_meshopt_compression':{'fallback':True}}})
        for view in out['bufferViews']:
            view['buffer']=1;view['byteOffset']=0
            view['extensions']={'EXT_meshopt_compression':dict(buffer=0,byteOffset=0,byteLength=85,byteStride=12,count=4,mode='ATTRIBUTES',filter='NONE')}
        for key in ('extensionsUsed','extensionsRequired'):
            if key not in out:out[key]=[]
            if 'EXT_meshopt_compression' not in out[key]:out[key].append('EXT_meshopt_compression')
        encoded=json.dumps(out,separators=(',',':'),ensure_ascii=False).encode();(p/'expected.json').write_bytes(encoded);expected=glb(encoded,(p/'binary.bin').read_bytes())
    (p/'expected.glb').write_bytes(expected)
    if shared is not None:dump(p/'shared_refs.json',shared)
    return p

rows=[]
selected=['opaque-token-and-marker','escaped-keys','high-J-low-K','long-root-key-late-escape','low-J-high-K','escaped-key-cardinality','deep-chain-wide-sibling','high-view-cardinality','declarations-already-present','declarations-missing','declarations-other-required','metadata-j0-u0','metadata-j7-u7']
for name in selected:
    p=SER/name; assert p.is_dir(),name
    rows.append(dict(name=name,path=str(p),origin='original frozen P2, bytes unchanged',expected='encode',facts=stats(p)))
# Targeted later object key scratch/owned-copy witnesses, with one literal late escape.
for position in ('buffer','view'):
    obj=base();target=obj['buffers'][0] if position=='buffer' else obj['bufferViews'][0]
    target['z'*65536+'a']=0
    raw=json.dumps(obj,separators=(',',':')).replace('z'*65536+'a','z'*65536+'\\u0061')
    p=make('long-'+position+'-key-late-escape',obj,raw)
    rows.append(dict(name=p.name,path=str(p),origin='new independent finite fixture',expected='encode',facts=stats(p)))
p=make('tiny-no-views-huge-configured-limit',base(0));rows.append(dict(name=p.name,path=str(p),origin='new',expected='encode',facts=stats(p)))
obj=base();obj['accessors']=[dict(bufferView=0,count=1,componentType=5121,type='SCALAR') for _ in range(4)]
p=make('shared-descriptor-four-references',obj,shared=[0,0,0,0]);rows.append(dict(name=p.name,path=str(p),origin='new fixture reference seam only',expected='encode',facts=stats(p)))
p=make('distinct-overlapping-physical-views',base(2));rows.append(dict(name=p.name,path=str(p),origin='new fixture view-index ownership seam only',expected='encode',facts=stats(p)))
for name in ('short','long-late-escape'):
    obj=base();decl='UNKNOWN' if name=='short' else 'z'*65536+'a';obj['extensionsUsed']=[decl]
    raw=json.dumps(obj,separators=(',',':'))
    if name!='short':raw=raw.replace('z'*65536+'a','z'*65536+'\\u0061')
    p=make('unknown-declaration-'+name,obj,raw,unsupported=True)
    rows.append(dict(name=p.name,path=str(p),origin='new prepare refusal',expected='Unsupported',facts=stats(p)))
for m in (1,3,4,7,8,14,15):
    obj={f'k{i}':0 for i in range(m)};p=make('parser-table-m'+str(m),obj,unsupported=True)
    rows.append(dict(name=p.name,path=str(p),origin='new accepted C1 parser only',expected='admission',tableKeys=m,facts=stats(p)))
manifest={'status':'STATIC_ONLY authored expectations; target not compiled/executed','target':'x86_64-unknown-linux-gnu rustc1.98.0 matched std','expectedTypeSizes':{'RawField':40,'PreparedView':96,'ViewEdit':32,'RawRef':16,'Span':16,'Declarations':352,'Facts':64},'cases':rows}
dump(HERE/'fixtures.json',manifest)
commands=[]
def command(row,label,work='18446744073709551615',fault='none',role='encode',nodeslimit=65536):
    commands.append(dict(label=label,case=row['name'],args=[row['path'],str(work),fault,role,str(nodeslimit)]))
for row in rows:
    if row['expected']=='admission':
        command(row,row['name']+'-exact-nodes',role='admission',nodeslimit=row['facts']['K'])
        command(row,row['name']+'-one-below-nodes',role='admission',nodeslimit=row['facts']['K']-1)
    elif row['expected']=='Unsupported':command(row,row['name'])
    else:command(row,row['name'])
by={r['name']:r for r in rows}
for name in ('long-root-key-late-escape','tiny-no-views-huge-configured-limit','high-view-cardinality','deep-chain-wide-sibling'):
    row=by[name];command(row,name+'-exact-work',work=row['facts']['planWork']);command(row,name+'-one-below-work',work=row['facts']['planWork']-1)
for fault in ('arena','views','refs','owned_key','edits','candidate'):
    command(by['long-root-key-late-escape'],'forced-'+fault,fault=fault)
for role in ('raw','identity'):command(by['opaque-token-and-marker'],'fixture-selected-'+role,role=role)
# Deep + wide existing witness also refuses one node below, after some real parser work.
row=by['deep-chain-wide-sibling'];command(row,'pending-overlap-one-below',role='admission',nodeslimit=row['facts']['K']-1)
commands.append(dict(label='checked-capacity-controls',args=['--checked-capacity-controls']))
dump(HERE/'commands.json',{'status':'ROOT_ONLY future execution after source freeze review','compile':{'rustc':'/home/bend/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/rustc','args':['--edition=2021',str(HERE/'owned_plan_probe.rs'),'-o','<FRESH_OUTPUT>/owned_plan_probe','-L','dependency=/tmp/rusty-tiles-codec-p2-executions/artifact-binding-traced-link/deps','--extern','serde=/tmp/rusty-tiles-codec-p2-executions/artifact-binding-traced-link/deps/libserde-aa8e5dbbc29a15a8.rlib','--extern','serde_json=/tmp/rusty-tiles-codec-p2-executions/artifact-binding-traced-link/deps/libserde_json-60507217a1b3d3c8.rlib']},'runPrefix':'nice10, first two currently allowed affinity CPUs, root serial coordinator; no author execution','commands':commands})
inputs=[Path('/tmp/rusty-tiles-codec-p2-executions/exact-artifact-receipt.json'),Path('/tmp/rusty-tiles-codec-p2-executions/accepted-c1-gate.json'),Path('/tmp/rusty-tiles-codec-p2-allocation-review/prototype-plan.md'),Path('/tmp/rusty-tiles-vector-codec-foundation/AGENTS.md'),Path('/tmp/rusty-tiles-vector-codec-foundation/docs/architecture/vector-compression-foundation.md'),OLD/'inputs/source/src/content_integrity/json.rs',OLD/'inputs/dependencies/serde_json-1.0.151/src/de.rs',OLD/'inputs/dependencies/serde_json-1.0.151/src/read.rs',OLD/'inputs/dependencies/serde_json-1.0.151/src/raw.rs',OLD/'inputs/toolchain/lib/rustlib/src/rust/library/alloc/src/raw_vec/mod.rs']
for row in rows:
    p=Path(row['path'])
    inputs.extend(f for f in p.iterdir() if f.is_file())
inputs=sorted(set(inputs))
dump(HERE/'input-pin.json',{'status':'immutable file pins, historical P2 input bytes retained','acceptedMerge':'f5401dd120441b4f9aac5ad229dbfffe03fe4428','files':{str(p):dict(sha256=sha(p),bytes=p.stat().st_size) for p in inputs}})
