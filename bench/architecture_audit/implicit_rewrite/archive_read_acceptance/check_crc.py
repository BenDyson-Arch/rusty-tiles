#!/usr/bin/env python3
"""Supplementary independently read central-declared stored-range CRC facts.

This is not an admission reader: it deliberately does not choose descriptors,
apply a name/overlap policy, decode glTF or interpret payload magic. It proves
that unsupported nesting controls are CRC coherent, including both overlapping
data ranges, without depending on the fixture writer's CRC calculations.
"""
import argparse
import binascii
import hashlib
import json
from pathlib import Path
import struct


def sha(data):return hashlib.sha256(data).hexdigest()


def range_facts(raw):
    n,cbytes,cat=struct.unpack_from('<HII',raw,len(raw)-12)
    if n==0xffff or cbytes==0xffffffff or cat==0xffffffff:
        zoff=struct.unpack_from('<Q',raw,len(raw)-34)[0]
        n,cbytes,cat=struct.unpack_from('<QQQ',raw,zoff+32)
    at=cat;rows=[]
    for _ in range(n):
        assert raw[at:at+4]==b'PK\x01\x02'
        expected_crc,csize,size,nlen,xlen,clen=struct.unpack_from('<IIIHHH',raw,at+16)
        offset=struct.unpack_from('<I',raw,at+42)[0];name=raw[at+46:at+46+nlen].decode()
        if size==0xffffffff or csize==0xffffffff or offset==0xffffffff:
            extra=raw[at+46+nlen:at+46+nlen+xlen];cursor=0;zip64=None
            while cursor<len(extra):
                tag,length=struct.unpack_from('<HH',extra,cursor);value=extra[cursor+4:cursor+4+length];cursor+=4+length
                if tag==1:zip64=value
            if zip64 is None:return {'readable':False,'reason':'central sentinel has no ZIP64 values'}
            cursor=0;values=[]
            for value in (size,csize,offset):
                if value==0xffffffff:value=struct.unpack_from('<Q',zip64,cursor)[0];cursor+=8
                values.append(value)
            size,csize,offset=values
        assert raw[offset:offset+4]==b'PK\x03\x04'
        local_name,local_extra=struct.unpack_from('<HH',raw,offset+26);start=offset+30+local_name+local_extra;end=start+size
        if end>cat:return {'readable':False,'reason':'central-declared data range enters central directory'}
        data=raw[start:end];actual=binascii.crc32(data)&0xffffffff
        rows.append({'raw_name':name,'local_offset':offset,'data_range':[start,end],'crc_declared':expected_crc,'crc_actual':actual,'crc_equal':actual==expected_crc})
        at+=46+nlen+xlen+clen
    assert at==cat+cbytes
    return {'readable':True,'all_declared_payload_crc_equal':all(r['crc_equal'] for r in rows),'rows':rows}


def main():
    p=argparse.ArgumentParser();p.add_argument('receipts',nargs='+',type=Path);p.add_argument('--output',type=Path,required=True);a=p.parse_args();out=[]
    for receipt in a.receipts:
        d=json.loads(receipt.read_bytes())
        for r in d['records']:
            raw=Path(r['archive_path']).read_bytes();assert len(raw)==r['archive_bytes'] and sha(raw)==r['archive_sha256']
            facts=range_facts(raw)
            if r['label'].startswith('physical-overlap-'):assert facts['readable'] and facts['all_declared_payload_crc_equal']
            if r['label']=='index-CRC-mismatch':assert facts['readable'] and not facts['all_declared_payload_crc_equal']
            out.append({'receipt_sha256':sha(receipt.read_bytes()),'label':r['label'],'archive_sha256':r['archive_sha256'],**facts})
    result={'driver_sha256':sha(Path(__file__).read_bytes()),'classification':'supplementary raw stored-range CRC facts; no admission/descriptor/semantic interpretation','records':out}
    a.output.write_text(json.dumps(result,indent=2,sort_keys=True)+'\n');print(json.dumps({'records':len(out),'CRC_coherent_nested_records':sum(r['label'].startswith('physical-overlap-') and r['all_declared_payload_crc_equal'] for r in out)}))


if __name__=='__main__':main()
