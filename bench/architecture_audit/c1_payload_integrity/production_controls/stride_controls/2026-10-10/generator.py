#!/usr/bin/env python3
"""Generate additive stride controls from retained literal primary, never invoke target code."""
import copy, hashlib, io, json, math, re, struct, zipfile
from pathlib import Path
HERE=Path(__file__).resolve().parent
PRIMARY=HERE/'primary'
def sha(raw): return hashlib.sha256(raw).hexdigest()
def packed(obj): return json.dumps(obj,separators=(',',':'),allow_nan=False).encode()
def glb(doc,binary):
    raw=packed(doc); raw+=b' '*(-len(raw)%4)
    padded=binary+b'\0'*(-len(binary)%4)
    chunks=struct.pack('<I4s',len(raw),b'JSON')+raw+struct.pack('<I4s',len(padded),b'BIN\0')+padded
    return struct.pack('<4sII',b'glTF',2,12+len(chunks))+chunks
def archive(members):
    stream=io.BytesIO(); records=[]
    with zipfile.ZipFile(stream,'w',compression=zipfile.ZIP_STORED) as target:
        for name,raw in members:
            records.append((hashlib.md5(name.encode()).digest(),stream.tell()))
            target.writestr(zipfile.ZipInfo(name,(1980,1,1,0,0,0)),raw)
        records.sort(key=lambda row:struct.unpack('<QQ',row[0]))
        target.writestr(zipfile.ZipInfo('@3dtilesIndex1@',(1980,1,1,0,0,0)),b''.join(d+struct.pack('<Q',o) for d,o in records))
    raw=stream.getvalue()
    with zipfile.ZipFile(io.BytesIO(raw)) as target:
        assert target.testzip() is None
        for name,data in members: assert target.read(name)==data
    return raw
CHECKS=['archiveIndex','archiveCrc','archiveStoredRecordLayout','contentHashes','tilesetSchema','payloadEnvelope','bufferRanges','accessorValues','primitiveIndices','hierarchyBounds','geometricErrorOrder','resourceReferences','recordedBudgets']
EXCLUSIONS=['decodedContentBounds','geometricErrorAccuracy','materialAndImageSemantics','metadata and feature semantics','metadataSemantics','scene transforms, skins, animations, morph targets and rendered bounds']
LIMITS=dict(jsonBytes=8388608,jsonDepth=64,memberBytes=67108864,archiveStoredBytes=1073741824,archiveEntries=65536,accessorElements=4000000,documentDecodedBytes=67108864,hierarchyVisits=65536,hierarchyDepth=128,references=262144,totalPayloadElements=16000000,totalBytesRead=2147483648,sourceArchiveBytes=1073741824,centralDirectoryBytes=16777216,documentItems=65536)
def generate():
    directory=HERE/'fixtures'; directory.mkdir(exist_ok=False)
    refs=json.loads((PRIMARY/'pins.json').read_text())
    for ref in refs:
        raw=(PRIMARY/ref['file']).read_bytes(); assert sha(raw)==ref['sha256'] and len(raw)==ref['bytes']
    source=(PRIMARY/'meshopt_decoder.test.js').read_text()
    section=source.split('decodeVertexBuffer: function () {',1)[1].split('decodeVertexBuffer_More:',1)[0]
    literal={}
    for name in ['encoded','expected']:
        text=re.search(r'var '+name+r' = new Uint8Array\(\[([\s\S]*?)\]\);',section).group(1)
        literal[name]=bytes(int(token.strip(),0) for token in text.split(',') if token.strip())
    compressed=literal['encoded']; expected=literal['expected']
    assert len(compressed)==85 and sha(compressed)=='c1c311df6033a0900eb605c2ad3e5ca631e076f7cf120595fd41d4ba157eab6a'
    assert len(expected)==48 and sha(expected)=='820994b9ed8bf83e8c38f9f6ce0c6b0afb889ecb5e07f4e580add9ae462eab89'
    (directory/'golden-compressed.bin').write_bytes(compressed); (directory/'golden-expected.bin').write_bytes(expected)
    scalars=struct.unpack('<12f',expected); assert all(math.isfinite(v) for v in scalars)
    positions=[scalars[i:i+3] for i in range(0,12,3)]
    minima=[min(p[i] for p in positions) for i in range(3)]
    maxima=[max(p[i] for p in positions) for i in range(3)]
    ext='EXT_meshopt_compression'
    base=dict(asset=dict(version='2.0'),extensionsUsed=[ext],extensionsRequired=[ext],buffers=[dict(byteLength=85),dict(byteLength=48,extensions={ext:dict(fallback=True)})],bufferViews=[dict(buffer=1,byteOffset=0,byteLength=48,extensions={ext:dict(buffer=0,byteOffset=0,byteLength=85,byteStride=12,count=4,mode='ATTRIBUTES',filter='NONE')})],accessors=[dict(bufferView=0,componentType=5126,count=4,type='VEC3',min=minima,max=maxima)],meshes=[dict(primitives=[dict(attributes=dict(POSITION=0),mode=0)])],nodes=[dict(mesh=0)],scenes=[dict(nodes=[0])],scene=0)
    definitions=[('absent-parent-baseline',None,None,0,'admitted'),('unused-packed-scalar',None,12,0,'admitted'),('unused-packed-scalar-offset',None,11,4,'admitted'),('explicit-parent-stride',12,4,0,'admitted'),('scalar-real-range',None,13,0,'invalid_input'),('wrong-parent-stride',8,None,0,'invalid_input')]
    rationales={
        'absent-parent-baseline':'One POSITION vertex accessor; missing parent stride gives tightly packed VEC3. Extension physical stride12/count4 supplies48 bytes.',
        'unused-packed-scalar':'Unused SCALAR is not a second vertex attribute: core permits tightly packed scalar stride4 while extension physical stride12 produces the same48-byte view.',
        'unused-packed-scalar-offset':'Unused SCALAR offset4/count11 fits exactly48 bytes, testing a nonzero tight accessor window independently of the physical stream.',
        'explicit-parent-stride':'Defined parentstride12 must equal extensionstride12; both accessor windows use12, scalar count4 occupies40 bytes.',
        'scalar-real-range':'Absent parent stride: scalar count13 requires52 bytes and genuinely exceeds decoded view48.',
        'wrong-parent-stride':'Defined parentstride8 disagrees with extensionstride12; extension validity requires equality regardless of accessor use.'}
    tileset=dict(asset=dict(version='1.1'),geometricError=0,root=dict(boundingVolume=dict(box=[0,0,0,10,0,0,0,10,0,0,0,10]),geometricError=0,content=dict(uri='tile.glb')))
    rows=[]; rust=[]
    for name,parent,count,offset,category in definitions:
        doc=copy.deepcopy(base)
        if parent is not None: doc['bufferViews'][0]['byteStride']=parent
        if count is not None: doc['accessors'].append(dict(bufferView=0,componentType=5126,count=count,type='SCALAR',byteOffset=offset))
        case=directory/name; case.mkdir(); data=glb(doc,compressed)
        members=[('tileset.json',packed(tileset)),('tile.glb',data)]
        raw=archive(members)
        for member,value in members: (case/member).write_bytes(value)
        text=packed(doc); (case/'literal-payload.json').write_bytes(text); (case/'input.3tz').write_bytes(raw)
        components=12+(count or 0)
        report=dict(ok=True,archive='$INPUT',tiles=1,contentReferences=1,entries=3,checks=CHECKS,notInspected=sorted(EXCLUSIONS),limits=LIMITS,payloads=[dict(uri='tile.glb',accessorsChecked=len(doc['accessors']),primitivesChecked=1,vertices=4)])
        window=None if count is None else dict(offset=offset,effectiveStride=parent or 4,count=count,occupiedBytes=4,end=offset+(count-1)*(parent or 4)+4,rawValues=[struct.unpack_from('<f',expected,offset+i*(parent or 4))[0] for i in range(count)] if offset+(count-1)*(parent or 4)+4<=48 else None)
        rows.append(dict(name=name,path=name+'/input.3tz',expectedCategory=category,expectedExitCode=0 if category=='admitted' else 3,rationale=rationales[name],expectedReport=report if category=='admitted' else None,components=components,decodedPhysicalBytes=48,physicalViews=1,scalarWindow=window,sha256=sha(raw),bytes=len(raw),memberSha256={k:sha(v) for k,v in members},literalSha256=sha(text)))
        rust.append('const '+name.upper().replace('-','_')+': &str = r###"'+text.decode()+'"###;')
    manifest=dict(schemaVersion=1,status='Predictions frozen before target execution; not execution evidence',generatorSha256=sha(Path(__file__).read_bytes()),runnerSha256=sha((HERE/'runner.py').read_bytes()),referencePins=refs,decodedOracle=dict(method='Literal primary expected Uint8Array bytes, independently struct.unpack little-endian float32; no project encoder/decoder/helpers',compressedSha256=sha(compressed),expectedSha256=sha(expected),positions=positions,positionMin=minima,positionMax=maxima,scalarValues=scalars),cases=rows,privateCaps=[dict(case='unused-packed-scalar',accessorComponents=24,remainingComponents=24,decodedBytes=48,expected='admitted',components=24),dict(case='unused-packed-scalar',accessorComponents=23,remainingComponents=24,decodedBytes=48,expected='resource_limit',resolverCalls=0),dict(case='unused-packed-scalar',accessorComponents=24,remainingComponents=23,decodedBytes=48,expected='resource_limit',resolverCalls=0)])
    (directory/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    (HERE/'compiled_stride_controls.rs').write_text(RUST_PREAMBLE+'\n'.join(rust)+'\nconst COMPRESSED: &[u8] = &'+repr(list(compressed))+';\n'+RUST_TESTS)
RUST_PREAMBLE='''// Independently authored additive controls; no repository fixture or target encoder dependency.
// Literal primary: zeux/meshoptimizer@3d8e9b8a2a2b9a5becfc6fbc512307b04207ab5d,
// js/meshopt_decoder.test.js decodeVertexBuffer expected/encoded arrays (85 -> 48 bytes).
// Integrate as a cfg(test) crate module; generated file is self-contained for CI.
use crate::content_integrity::{payload::{self,PayloadKind,PayloadInspection},PayloadLimits,JsonLimits,PayloadError,FormatError};
fn frame(text: &str) -> Vec<u8> {
    let mut json=text.as_bytes().to_vec(); while json.len()%4!=0 { json.push(b' '); }
    let mut bin=COMPRESSED.to_vec(); while bin.len()%4!=0 { bin.push(0); }
    let mut bytes=b"glTF".to_vec(); bytes.extend_from_slice(&2u32.to_le_bytes());
    bytes.extend_from_slice(&(28u32+json.len() as u32+bin.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&(json.len() as u32).to_le_bytes()); bytes.extend_from_slice(b"JSON"); bytes.extend_from_slice(&json);
    bytes.extend_from_slice(&(bin.len() as u32).to_le_bytes()); bytes.extend_from_slice(b"BIN\\0"); bytes.extend_from_slice(&bin); bytes
}
fn inspect(text:&str, cap:u64, remaining:u64, decoded:usize)->Result<PayloadInspection,PayloadError<&'static str>> {
    let mut calls=0;
    let result=payload::inspect(PayloadKind::Glb,&frame(text),PayloadLimits {member_bytes:4096, decoded_bytes:decoded, accessor_components:cap,json:JsonLimits{bytes:4096,depth:64,value_nodes:256}},remaining,|_,_| {calls+=1; Err("unexpected resolver")});
    assert_eq!(calls,0,"all backing is embedded or decoded placeholder; resolver must not run"); result
}
'''
RUST_TESTS='''
#[test]
fn independent_primary_stride_views_and_accessor_windows() {
    for (text,accessors,components) in [(ABSENT_PARENT_BASELINE,1,12),(UNUSED_PACKED_SCALAR,2,24),(UNUSED_PACKED_SCALAR_OFFSET,2,23),(EXPLICIT_PARENT_STRIDE,2,16)] {
        let facts=inspect(text,24,24,48).expect("primary-consistent layout must admit with one48-byte physical decode");
        assert_eq!(facts.accessors_checked,accessors); assert_eq!(facts.primitives_checked,1);
        assert_eq!(facts.vertices,4); assert_eq!(facts.elements_checked,components);
    }
    for text in [SCALAR_REAL_RANGE,WRONG_PARENT_STRIDE] {
        assert!(matches!(inspect(text,64,64,48),Err(PayloadError::Format(FormatError::InvalidInput(_)))));
    }
}
#[test]
fn independent_unused_scalar_exact_component_and_physical_decode_caps() {
    let facts=inspect(UNUSED_PACKED_SCALAR,24,24,48).expect("12 POSITION plus12 unused SCALAR components fit24; shared physical view is charged once48 bytes");
    assert_eq!(facts.elements_checked,24);
    for (cap,remaining,decoded) in [(23,24,48),(24,23,48),(24,24,47)] {
        assert!(matches!(inspect(UNUSED_PACKED_SCALAR,cap,remaining,decoded),Err(PayloadError::Format(FormatError::ResourceLimit(_)))));
    }
}
'''
if __name__=='__main__': generate()
