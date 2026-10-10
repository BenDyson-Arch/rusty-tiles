#!/usr/bin/env python3
"""Independent byte-offset/slack oracle; selftests are model-only, never consumer conformance."""
import argparse,json,struct
from pathlib import Path
HERE=Path(__file__).resolve().parent
ROOT=HERE/'fixtures/serializer'
def check(raw,metadata):
    assert len(raw)>=28
    magic,v,total=struct.unpack_from('<4sII',raw);assert (magic,v,total)==(b'glTF',2,len(raw))
    j,typ=struct.unpack_from('<I4s',raw,12);assert typ==b'JSON' and j%4==0
    end=20+j;assert end+8<=len(raw)
    b,typ=struct.unpack_from('<I4s',raw,end);assert typ==b'BIN\0' and end+8+b==len(raw) and b%4==0
    text=raw[20:end].decode();doc,n=json.JSONDecoder(parse_float=str).raw_decode(text)
    actual_j=len(text[:n].encode());assert raw[20+actual_j:end]==b' '*(j-actual_j)
    declared=doc['buffers'][0]['byteLength'];assert isinstance(declared,int) and 0<=declared<=b
    slack=b-declared;assert 0<=slack<=(7 if metadata else 3)
    assert raw[end+8+declared:]==b'\0'*slack
    if metadata:assert end%8==0 and (end+8)%8==0 and len(raw)%8==0 and b%8==0
    return dict(jsonBytes=actual_j,jsonChunkBytes=j,binOrigin=end+8,declaredBinBytes=declared,binChunkBytes=b,zeroSlack=slack,total=len(raw))
def core_frame(text,binary):
    raw=text+b' '*(-len(text)%4);data=binary+b'\0'*(-len(binary)%4);chunks=struct.pack('<I4s',len(raw),b'JSON')+raw+struct.pack('<I4s',len(data),b'BIN\0')+data
    return struct.pack('<4sII',b'glTF',2,12+len(chunks))+chunks
def rejects(raw,metadata):
    try:check(raw,metadata)
    except (AssertionError,ValueError,UnicodeError,struct.error,KeyError):return True
    return False
if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--candidate',type=Path);p.add_argument('--metadata',action='store_true');args=p.parse_args()
    if args.candidate:print(json.dumps(check(args.candidate.read_bytes(),args.metadata)));raise SystemExit(0)
    cases=json.loads((ROOT/'manifest.json').read_text())['cases'];residues=set();records=[];four_fail=0
    for c in cases:
        directory=ROOT/c['path'];raw=(directory/'expected.glb').read_bytes();facts=check(raw,c['metadata']);faults=[]
        # Incorrect envelope length is always independently malformed.
        broken=bytearray(raw);struct.pack_into('<I',broken,8,len(raw)+4);assert rejects(broken,c['metadata']);faults.append('wrong_envelope_length')
        j=facts['jsonChunkBytes'];origin=facts['binOrigin']
        if j>facts['jsonBytes']:
            broken=bytearray(raw);broken[20+facts['jsonBytes']]=9;assert rejects(broken,c['metadata']);faults.append('JSON_non_space_padding')
        if facts['zeroSlack']:
            broken=bytearray(raw);broken[-1]=1;assert rejects(broken,c['metadata']);faults.append('BIN_nonzero_slack')
        broken=bytearray(raw);extra=8 if c['metadata'] else 4;broken.extend(b'\0'*extra);struct.pack_into('<I',broken,8,len(broken));struct.pack_into('<I',broken,origin-8,facts['binChunkBytes']+extra);assert rejects(broken,c['metadata']);faults.append('BIN_excess_zero_slack')
        if c['metadata']:
            residues.add((c['jsonResidue'],c['binResidue']))
            four=core_frame((directory/'expected.json').read_bytes(),(directory/'binary.bin').read_bytes())
            if rejects(four,True):four_fail+=1;faults.append('wrong_four_byte_metadata_framing')
        records.append(dict(name=c['name'],facts=facts,sensitiveFaults=faults))
    assert len(residues)==64 and four_fail>0
    print(json.dumps(dict(evidence='EXECUTED_INTEGER_AND_BYTE_MODEL_ORACLE_ONLY; not actual serializer/consumer/codec acceptance',metadataResidues=len(residues),fourByteFramingSensitiveCases=four_fail,cases=records),indent=2))
