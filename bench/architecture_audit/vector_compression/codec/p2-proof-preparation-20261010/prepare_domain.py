#!/usr/bin/env python3
"""Independent raw-role/association sensitivity witnesses, not a producer or codec."""
from pathlib import Path
import hashlib,json,struct
HERE=Path(__file__).resolve().parent
sha=lambda b:hashlib.sha256(b).hexdigest()

def generate():
    root=HERE/'fixtures/generated-domain';root.mkdir(exist_ok=False);rows=[]
    def emit(name,role,streams,meaning,associations,proof,mutation):
        target=root/name;target.mkdir()
        files={}
        for key,data in streams.items():
            (target/(key+'.bin')).write_bytes(data);files[key+'.bin']=dict(bytes=len(data),sha256=sha(data))
        raw=json.dumps(associations,separators=(',',':'),ensure_ascii=False).encode();(target/'associations.json').write_bytes(raw)
        files['associations.json']=dict(bytes=len(raw),sha256=sha(raw))
        rows.append(dict(name=name,role=role,path=name,files=files,independentExpectedMeaning=meaning,associations=associations,proofScope=proof,sensitiveMutation=mutation))
    f32=[0x00000000,0x80000000,0x3f800000,0xbf800000,0x3f000000,0x40000000,0x40400000,0x40800000,0xc0800000,0x3e800000,0x3f800000,0x00000000]
    emit('positions-f32','mandatory generated unquantized',{'positions':b''.join(struct.pack('<I',v)for v in f32)},dict(bits=f32,count=4,stride=12),dict(POSITION=0,componentType=5126,type='VEC3',count=4,nodeRaw='{"translation":[17,19,23],"scale":[1,1,1]}'),'Independent explicit bit words, whole byte preservation including negative0; geometry/source CRS correctness outside scope','change signed-zero word or reorder positions')
    coords=[[0,65535,32768],[1,32767,65534],[65535,0,0],[30000,40000,50000]]
    quant=b''.join(struct.pack('<3H',*c)+b'\xa5\x3c'for c in coords)
    emit('positions-normalized-u16','mandatory generated quantized',{'positions':quant},dict(raw=coords,count=4,stride=8,paddingHex='a53c',normalizedDenominator=65535),dict(POSITION=0,componentType=5123,type='VEC3',normalized=True,byteStride=8,extension='KHR_mesh_quantization',nodeRaw='{"translation":[17,19,23],"scale":[100,200,300]}'),'Nonzero padding and authored dequantization node preserved; does not prove source quantizer error','drop/zero padding or change node scale/normalization')
    ids=[0,1,65535]
    emit('feature-ids-padded-u16','mandatory generated IDs <=65536 rows',{'ids':b''.join(struct.pack('<H',v)+b'\xb6\x4d'for v in ids)},dict(ids=ids,stride=4,paddingHex='b64d'),dict(_FEATURE_ID_0=1,propertyTable=0,featureCount=65536,componentType=5123,attribute=0),'Boundary raw encoding and exact feature/table associations; actual65536-row producer remains future','change ID padding/featureCount/propertyTable')
    ids=[0,1,65536,16777216]
    emit('feature-ids-f32','mandatory generated IDs >65536 up to16777217 rows',{'ids':struct.pack('<4f',*ids)},dict(ids=ids,stride=4),dict(_FEATURE_ID_0=1,propertyTable=0,featureCount=16777217,componentType=5126,attribute=0),'Exact representable f32 IDs; actual65537-row producer path and upper rejection unexecuted','change componentType/propertyTable or one ID byte')
    line=[0,1,0xffffffff,2,3]
    emit('line-restart','mandatory POINTS/LINE_STRIP stream semantics',{'indices':struct.pack('<5I',*line)},dict(indices=line,segments=[[0,1],[2,3]]),dict(mode=3,indices=2,restartExtension='KHR_mesh_primitive_restart',restartIndex=0xffffffff),'Order/restart raw bytes, not TRIANGLES reorder codec; point mode0 remains complete generated-corpus gate','remove/move restart or change mode')
    exterior=[0,1,2,3,0];hole=[4,5,6,7,4];loops=exterior+[0xffffffff]+hole
    emit('polygon-loops-offsets','mandatory polygon exterior/holes/multiple polygon raw streams',{'loops':struct.pack('<11I',*loops),'offsets':struct.pack('<3I',0,6,11)},dict(loops=loops,offsets=[0,6,11],exterior=exterior,hole=hole),dict(extension='EXT_mesh_polygon',indices=2,polygonOffsetAccessor=3,featureTable=0),'Independent raw arrays/topology association witness; full actual authored multiple polygon/fill semantics require producer corpus/primary walk','alter polygon offset/restart/hole link')
    fillindices=[0,1,2,0,2,3]
    emit('fill-triangles-material','mandatory fill-only generated GLB',{'indices':struct.pack('<6I',*fillindices)},dict(indices=fillindices),dict(mode=4,materialRaw='{"doubleSided":true,"pbrMetallicRoughness":{"baseColorFactor":[0.1,0.2,0.3,0.4]},"extensions":{"KHR_materials_unlit":{}}}',propertyTable=0),'Raw TRIANGLES order and exact material/table associations; b3dm wrapping belongs outside codec','change factor/material/unlit/table link or index order')
    bools=[True,False,True,False,False,False,False,True,True,False,True]
    bits=bytearray(2)
    for i,v in enumerate(bools):bits[i//8]|=int(v)<<(i%8)
    emit('metadata-boolean-bitset','mandatory raw BOOLEAN',{'values':bytes(bits)},dict(rows=bools,count=11),dict(className='c',propertyName='enabled',valuesView=0,type='BOOLEAN',count=11),'Bit boundaries across two bytes, raw metadata stays raw','flip bit8 or count/table association')
    values=[-9223372036854775808,-9007199254740993,9007199254740993,9223372036854775807]
    emit('metadata-int64-nodata','mandatory raw signed64/noData',{'values':struct.pack('<4q',*values)},dict(raw=values,rows=[None,*values[1:]],noData=-9223372036854775808),dict(valuesView=0,componentType='INT64',noDataRaw='-9223372036854775808',className='c',count=4),'Exact integer extrema and noData association; current writer sentinel selection separately tested with real data','change >2^53 low bit or noData/class/count')
    bits=[0x8000000000000000,0x3fb999999999999a,0x400921fb54442d18,0xc059000000000000]
    emit('metadata-float64','mandatory raw FLOAT64',{'values':struct.pack('<4Q',*bits)},dict(bits=bits),dict(valuesView=0,componentType='FLOAT64',count=4),'Exact IEEE64 bits including negative0, no narrowing through f32','truncate64-bit value or zero signed0')
    strings=['','é🦉','plain','[9007199254740993,null,"x",[1,2]]']
    encoded=[s.encode()for s in strings];offsets=[0]
    for b in encoded:offsets.append(offsets[-1]+len(b))
    emit('metadata-utf8-array-string','mandatory STRING and list_fields=json arrays',{'values':b''.join(encoded),'stringOffsets':struct.pack('<5I',*offsets)},dict(rows=strings,stringOffsets=offsets,listMeaning=[9007199254740993,None,'x',[1,2]]),dict(valuesView=0,stringOffsetsView=1,stringOffsetType='UINT32',className='c',count=4,list_fields='json'),'Unicode/empty strings and exact ordered heterogeneous list-as-STRING, not native structural array authoring','change UTF8/string offset/null/order/exact integer token')
    vals=[0,9007199254740993,18446744073709551615]
    emit('external-uint64','independent external raw domain, not current writer authoring',{'values':struct.pack('<3Q',*vals)},dict(rows=vals),dict(valuesView=0,componentType='UINT64',count=3),'Independent raw preservation extension; signed writer does not emit this domain','change high/low64 bit or componentType')
    emit('external-structural-array-shared','independent fixed/variable structural arrays/shared metadata',{'values':struct.pack('<4q',1,2,3,9007199254740993),'arrayOffsets':struct.pack('<4I',0,2,2,4)},dict(variableRows=[[1,2],[],[3,9007199254740993]],fixedRows=[[1,2],[3,9007199254740993]]),dict(valuesView=0,arrayOffsetsView=1,arrayOffsetType='UINT32',variableCount=3,fixedArrayCount=2,sharedValuesView=0),'Native array preservation is independent external fixture; list-string current source stays mandatory separately','swap values view or change offsets/shared reference')
    emit('external-interleaved-matrix-partial','independent core shared/interleaved/padded matrices',{'interleaved':bytes(range(48)),'mat3-u8-occupied11':bytes([1,2,3,0xa5,4,5,6,0x3c,7,8,9])},dict(interleavedStride=24,positionOffset=0,otherOffset=12,counts=[2,2],mat3ColumnStride=4,mat3Occupied=11,mat3EffectiveTight=12),dict(view0=0,sharedRefs=[0,1],view1=1,matrixType='MAT3',componentType=5121,count=1),'Physical views once; partial final matrix padding remains Raw after valid window; bytes preserve nonzero padding','repack count from last accessor, compress11-byte partial view, or zero column padding')
    emit('opaque-source-json','mandatory raw JSON preservation',{},dict(raw='{"n":9007199254740993,"u":18446744073709551615,"x":1e400,"m":{"$serde_json::private::Number":"1e400"}}'),dict(extrasRaw='{"n":9007199254740993,"u":18446744073709551615,"x":1e400,"m":{"$serde_json::private::Number":"1e400"}}'),'Exact raw token/subtree witness; custom key whitespace normalization outside raw subtrees is allowed','Value roundtrip/marker interpretation/token normalization')
    (root/'manifest.json').write_text(json.dumps(dict(status='Independent authored raw roles/associations, oracle sensitivity only; neither codec nor actual producer/domain acceptance',cases=rows,mandatoryActualCorpusGates=['portable/native default and raised caps','quantized/unquantized point/line/polygon exterior+holes/multiple polygons/fill-only','actual65536/65537 feature-ID branch and exact upper representation refusal','signed64/float64/boolean/UTF8/null/list_fields=json source field columns','shared/interleaved/partialmatrix raw disposition','canonical compressed+raw metadata identity/repeated invocation and all-raw identity','absentparent packedSCALAR vs physical12; explicitparent mismatch; real range negative','legacy metadata envelope disposition plus selected-framing producer bridge','tablePicking pinned browser consumer and unchanged feature/schema/class/table/topology/node associations']),indent=2)+'\n')
if __name__=='__main__':generate()
