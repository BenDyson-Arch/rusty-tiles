#!/usr/bin/env python3
"""Author bounded independent P2 expectations; no target execution or production transform."""
from pathlib import Path
import hashlib,json,struct
HERE=Path(__file__).resolve().parent
sha=lambda data:hashlib.sha256(data).hexdigest()
def compact(v): return json.dumps(v,separators=(',',':'),ensure_ascii=False,allow_nan=False)
def nodes(v,depth=0):
    children=list(v.values()) if isinstance(v,dict) else v if isinstance(v,list) else []
    states=[nodes(c,depth+1) for c in children]
    return 1+sum(x[0] for x in states),max([depth]+[x[1] for x in states])
def frame(text,binary,metadata):
    raw=text.encode(); j=((20+len(raw)+7)//8)*8-20 if metadata else ((len(raw)+3)//4)*4
    u=((len(binary)+(7 if metadata else 3))//(8 if metadata else 4))*(8 if metadata else 4)
    chunks=struct.pack('<I4s',j,b'JSON')+raw+b' '*(j-len(raw))+struct.pack('<I4s',u,b'BIN\0')+binary+b'\0'*(u-len(binary))
    return struct.pack('<4sII',b'glTF',2,12+len(chunks))+chunks

def serializer(directory):
    rows=[]
    def emit(name,extras='{}',views=1,metadata=False,binary=b'\x91\x02\x03\x04\xa5',key='extras',pad='',max_width=False,used='default',required='default',kind='raw targeted serializer scaffold; not full codec/source semantic admission'):
        target=directory/name; target.mkdir()
        oldbuffer='{"byteLength":48,"name":"buffer\\nname","extras":'+extras+'}'
        oldview='{"buffer":0,"byteOffset":0,"byteLength":48,"target":34962,"extras":'+extras+',"odd\\nkey":9007199254740993}'
        used=['KHR_mesh_quantization'] if used=='default' else used
        required=[] if required=='default' else required
        declarations=(','+'"extensionsUsed":'+compact(used) if used is not None else '')+(','+'"extensionsRequired":'+compact(required) if required is not None else '')
        input_text='{"asset":{"version":"2.0"},"buffers":['+oldbuffer+'],"bufferViews":['+','.join([oldview]*views)+']'+declarations+',"'+key+'":'+extras+',"pad":'+compact(pad)+'}'
        edit=dict(binBytes=len(binary),fallbackBytes=(18446744073709551615 if max_width else 48*views),byteOffset=(18446744073709551615 if max_width else 0),byteLength=(18446744073709551615 if max_width else 85),byteStride=(18446744073709551615 if max_width else 12),count=(18446744073709551615 if max_width else 4),metadata=metadata)
        newbuffer='{"byteLength":'+str(len(binary))+',"name":"buffer\\nname","extras":'+extras+'}'
        fallback='{"byteLength":'+str(edit['fallbackBytes'])+',"extensions":{"EXT_meshopt_compression":{"fallback":true}}}'
        e='{"buffer":0,"byteOffset":'+str(edit['byteOffset'])+',"byteLength":'+str(edit['byteLength'])+',"byteStride":'+str(edit['byteStride'])+',"count":'+str(edit['count'])+',"mode":"ATTRIBUTES","filter":"NONE"}'
        newview='{"buffer":1,"byteOffset":'+str(edit['byteOffset'])+',"byteLength":48,"target":34962,"extras":'+extras+',"odd\\nkey":9007199254740993,"extensions":{"EXT_meshopt_compression":'+e+'}}'
        decodedkey=json.loads('"'+key+'"')
        newused=list(used or []);newrequired=list(required or [])
        if 'EXT_meshopt_compression' not in newused:newused.append('EXT_meshopt_compression')
        if 'EXT_meshopt_compression' not in newrequired:newrequired.append('EXT_meshopt_compression')
        declarations=(','+'"extensionsUsed":'+compact(newused) if used is not None else '')+(','+'"extensionsRequired":'+compact(newrequired) if required is not None else '')
        additions=(','+'"extensionsUsed":'+compact(newused) if used is None else '')+(','+'"extensionsRequired":'+compact(newrequired) if required is None else '')
        expected='{"asset":{"version":"2.0"},"buffers":['+newbuffer+','+fallback+'],"bufferViews":['+','.join([newview]*views)+']'+declarations+','+compact(decodedkey)+':'+extras+',"pad":'+compact(pad)+additions+'}'
        k,depth=nodes(json.loads(input_text,parse_int=str,parse_float=str))
        bound=len(input_text.encode())+512*views+1024
        assert len(expected.encode())<=bound
        glb=frame(expected,binary,metadata)
        for file,data in [('input.json',input_text.encode()),('edits.json',(compact(edit)+'\n').encode()),('expected.json',expected.encode()),('binary.bin',binary),('expected.glb',glb)]: (target/file).write_bytes(data)
        rows.append(dict(name=name,kind=kind,path=name,expected='admitted_json_and_byte_exact_scaffold_emission',inputBytes=len(input_text.encode()),nodes=k,depth=depth,views=views,expectedJsonBytes=len(expected.encode()),deltaBound=bound,metadata=metadata,jsonResidue=len(expected.encode())%8,binResidue=len(binary)%8,expectedTotal=len(glb),opaqueRaw=extras,files={p.name:sha(p.read_bytes()) for p in target.iterdir()}))
        return rows[-1]
    opaque='{ "big":9007199254740993,"u64":18446744073709551615,"huge":1e400,"fraction":123.4500e-2,"negativeZero":-0,"marker":{"$serde_json::private::Number":"1e400"},"array":[null,9007199254740993,"\\ud83d\\udc08"] }'
    emit('opaque-token-and-marker',opaque,key='ex\\u0074ras')
    emit('escaped-keys','{"line\\nkey":1e400,"\\u0061":9007199254740993,"unicode":"é🦉"}')
    emit('high-J-low-K','"'+'x'*32768+'"')
    emit('long-root-key-late-escape','{}',key='z'*65536+'\\u0061',kind='LowK late-escape scratch old+new growth witness; JSON/serializer only')
    emit('low-J-high-K','['+','.join(['0']*4096)+']')
    emit('escaped-key-cardinality','{'+','.join('"k\\u%04x":0'%i for i in range(256))+ '}')
    # View extras is at depth3;61 arrays place its terminal scalar at depth64.
    emit('deep-chain-wide-sibling','['*61+'0'+']'*61,views=32)
    emit('high-view-cardinality','{}',views=4096)
    emit('declarations-already-present',used=['KHR_mesh_quantization','EXT_meshopt_compression'],required=['EXT_meshopt_compression'])
    emit('declarations-missing',used=None,required=None)
    emit('declarations-other-required',required=['KHR_mesh_quantization'])
    emit('declarations-empty',used=[],required=[])
    emit('twenty-digit-dimensions',opaque,max_width=True,kind='synthetic20-digit serializer width stress; dimensional inconsistencies intentionally not codec-valid')
    for u in range(8):
        for j in range(8):
            # Set pad length using known compact-byte delta. Metadata adds no JSON field.
            base_j=rows[0]['expectedJsonBytes'] # unused baseline is intentionally not serializer truth
            name='metadata-j%d-u%d'%(j,u)
            trial='{"asset":{"version":"2.0"},"buffers":[{"byteLength":'+str(u)+',"name":"buffer\\nname","extras":{}},{"byteLength":48,"extensions":{"EXT_meshopt_compression":{"fallback":true}}}],"bufferViews":[{"buffer":1,"byteOffset":0,"byteLength":48,"target":34962,"extras":{},"odd\\nkey":9007199254740993,"extensions":{"EXT_meshopt_compression":{"buffer":0,"byteOffset":0,"byteLength":85,"byteStride":12,"count":4,"mode":"ATTRIBUTES","filter":"NONE"}}}],"extensionsUsed":["KHR_mesh_quantization","EXT_meshopt_compression"],"extensionsRequired":["EXT_meshopt_compression"],"extras":{},"pad":""}'
            pad='x'*((j-len(trial.encode()))%8)
            row=emit(name,metadata=True,binary=bytes(range(u)),pad=pad,kind='selected metadata framing residues; synthetic bookkeeping, not normative consumer execution')
            assert row['jsonResidue']==j and row['binResidue']==u
    emit('core-zero-slack',binary=b'',metadata=False)
    (directory/'manifest.json').write_text(json.dumps(dict(status='Frozen expected scaffold records; not target/allocator/format acceptance',cases=rows),indent=2)+'\n')

def streams(directory):
    rows=[]
    for stride in range(4,257,4):
        b=min(256,16*(8192//(16*stride)))
        for count in sorted(set([1,b-1,b,b+1,255,256,257])):
            n=(count+b-1)//b; h=(b+63)//64; bound=1+n*stride*(stride//4+h+b)+max(32,stride+stride//4)
            raw=bytes(((i*73+(i//stride)*19+0xa5)^(i>>3))&255 for i in range(count*stride))
            name='s%d-n%d'%(stride,count); (directory/(name+'.bin')).write_bytes(raw)
            rows.append(dict(name=name,path=name+'.bin',count=count,stride=stride,block=b,expectedBound=bound,bytes=len(raw),sha256=sha(raw),expected='nonzero_encoded_length_le_bound_guards_intact_and_byte_exact_same_kernel_decode',limits='bound agreement and byte preservation only; same-kernel decode is not independent conformance'))
    golden=(HERE/'inputs/primary/meshopt_decoder.test.js').read_text()
    import re
    section=golden.split('decodeVertexBuffer: function () {',1)[1].split('decodeVertexBuffer_More:',1)[0]
    for name in ['encoded','expected']:
        literal=re.search(r'var '+name+r' = new Uint8Array\(\[([\s\S]*?)\]\);',section).group(1)
        raw=bytes(int(x.strip(),0) for x in literal.split(',') if x.strip()); (directory/('golden-'+name+'.bin')).write_bytes(raw)
    (directory/'manifest.json').write_text(json.dumps(dict(status='Independent model bounds and raw expected bytes, all FFI execution future',cases=rows,golden=dict(count=4,stride=12,encodedSha256=sha((directory/'golden-encoded.bin').read_bytes()),expectedSha256=sha((directory/'golden-expected.bin').read_bytes())),faultExpectations=['encodedLength0/>B must be classified EncodingFailure by future result admission before truncate/use','corrupt golden header returns decode failure','header-only golden is proven too short; wrongcount3/5 outcomes must be characterized, decodable cases still fail48-byte declaration oracle','outputcapacity1 returns0 and leaves beyond-cap sentinel unchanged'],unsafeDomain='Only positive N, S positive/divisible4/<=256, exact N*S source and initialized destinations enter FFI'),indent=2)+'\n')

if __name__=='__main__':
    inputs=HERE/'fixtures'; inputs.mkdir(exist_ok=False)
    for name in ['serializer','streams']: (inputs/name).mkdir()
    serializer(inputs/'serializer'); streams(inputs/'streams')
