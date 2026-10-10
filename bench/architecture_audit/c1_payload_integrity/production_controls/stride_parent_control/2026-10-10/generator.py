#!/usr/bin/env python3
"""Generate one unmasked parent-agreement control from retained literal primary, never invoke target code."""
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
    arrays={}
    for name in ['encoded','expected']:
        literal=re.search(r'var '+name+r' = new Uint8Array\(\[([\s\S]*?)\]\);',section).group(1)
        arrays[name]=bytes(int(token.strip(),0) for token in literal.split(',') if token.strip())
    compressed,expected=arrays['encoded'],arrays['expected']
    assert len(compressed)==85 and sha(compressed)=='c1c311df6033a0900eb605c2ad3e5ca631e076f7cf120595fd41d4ba157eab6a'
    assert len(expected)==48 and sha(expected)=='820994b9ed8bf83e8c38f9f6ce0c6b0afb889ecb5e07f4e580add9ae462eab89'
    (directory/'golden-compressed.bin').write_bytes(compressed); (directory/'golden-expected.bin').write_bytes(expected)
    scalar_values=[struct.unpack_from('<f',expected,i*8)[0] for i in range(4)]
    assert all(math.isfinite(v) for v in scalar_values)
    # Hypothetical gate-removed background: SCALAR occupied4 <= stride8;
    # its offset0 +3*8 +4 =28 <= decoded view48. POSITION has NO bufferView.
    assert 0+3*8+4==28<=48 and 4<=8 and 8%4==0 and 4<=8<=252
    ext='EXT_meshopt_compression'
    doc=dict(asset=dict(version='2.0'),extensionsUsed=[ext],extensionsRequired=[ext],buffers=[dict(byteLength=85),dict(byteLength=48,extensions={ext:dict(fallback=True)})],bufferViews=[dict(buffer=1,byteOffset=0,byteLength=48,byteStride=8,extensions={ext:dict(buffer=0,byteOffset=0,byteLength=85,byteStride=12,count=4,mode='ATTRIBUTES',filter='NONE')})],accessors=[dict(componentType=5126,count=4,type='VEC3',min=[0,0,0],max=[0,0,0]),dict(bufferView=0,componentType=5126,count=4,type='SCALAR',byteOffset=0)],meshes=[dict(primitives=[dict(attributes=dict(POSITION=0),mode=0)])],nodes=[dict(mesh=0)],scenes=[dict(nodes=[0])],scene=0)
    assert 'bufferView' not in doc['accessors'][0]
    assert list(doc['meshes'][0]['primitives'][0]['attributes'].values())==[0]
    tileset=dict(asset=dict(version='1.1'),geometricError=0,root=dict(boundingVolume=dict(box=[0,0,0,10,0,0,0,10,0,0,0,10]),geometricError=0,content=dict(uri='tile.glb')))
    name='unmasked-parent-stride-disagreement'; case=directory/name; case.mkdir()
    members=[('tileset.json',packed(tileset)),('tile.glb',glb(doc,compressed))]
    for member,raw in members: (case/member).write_bytes(raw)
    raw=archive(members); (case/'input.3tz').write_bytes(raw); (case/'literal-payload.json').write_bytes(packed(doc))
    report=dict(ok=True,archive='$INPUT',tiles=1,contentReferences=1,entries=3,checks=CHECKS,notInspected=sorted(EXCLUSIONS),limits=LIMITS,payloads=[dict(uri='tile.glb',accessorsChecked=2,primitivesChecked=1,vertices=4)])
    error=dict(ok=False,exitCode=3,error=dict(code='invalid_input',message='meshopt and bufferView stride disagree'))
    row=dict(name=name,path=name+'/input.3tz',expectedCategory='invalid_input',expectedExitCode=3,expectedReport=None,expectedErrorReport=error,hypotheticalGateRemovedReport=report,rationale='Defined parent8 disagrees with physical extension12. All remaining C1 background gates admit: POSITION is viewless zero, only unused SCALAR references view with stride8 and end28 within48; finite values. Thus deleting parent agreement should admit rather than hitting ordinary POSITION stride rejection.',components=16,physicalViews=1,decodedPhysicalBytes=48,scalarWindow=dict(offset=0,effectiveStride=8,count=4,occupiedBytes=4,end=28,rawValues=scalar_values),sha256=sha(raw),bytes=len(raw),memberSha256={n:sha(b) for n,b in members},literalSha256=sha(packed(doc)))
    manifest=dict(schemaVersion=1,status='One independently authored unmasked parent-gate negative, predictions frozen before target execution',generatorSha256=sha(Path(__file__).read_bytes()),runnerSha256=sha((HERE/'runner.py').read_bytes()),referencePins=refs,cases=[row],decodedOracle=dict(method='Literal primary expected Uint8Array; independent little-endian struct float32 offsets0/8/16/24; no production encoder/decoder/numeric helper',compressedSha256=sha(compressed),expectedSha256=sha(expected),unusedScalarValues=scalar_values,viewlessPositionValues=[[0,0,0]]*4),staticCaps=dict(sourcePayloadBytes=len(members[1][1]),jsonBytes=len(packed(doc)),decodedPhysicalBytes=48,components=16,defaultDecodedCap=LIMITS['documentDecodedBytes'],defaultAccessorCap=LIMITS['accessorElements'],defaultAggregateComponents=LIMITS['totalPayloadElements']))
    (directory/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
if __name__=='__main__': generate()
