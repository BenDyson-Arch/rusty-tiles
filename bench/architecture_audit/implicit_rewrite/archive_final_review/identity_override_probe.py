#!/usr/bin/env python3
"""Independent tiny downstream-interpretation controls; no product imports."""
import argparse
import binascii
import hashlib
import json
from pathlib import Path
import struct
import subprocess

HERE = Path(__file__).resolve().parent
INDEX = "@3dtilesIndex1@"

def crc(data): return binascii.crc32(data) & 0xffffffff
def tlv(tag,data): return struct.pack("<HH",tag,len(data))+data
def manifest(content=None):
    root={"boundingVolume":{"box":[0,0,0,1,0,0,0,1,0,0,0,1]},"geometricError":0,"refine":"REPLACE"}
    if content is not None: root["content"]={"uri":content}
    return json.dumps({"asset":{"version":"1.1"},"geometricError":0,"root":root},separators=(",",":")).encode()
def local(name,data):
    n=name.encode()
    return struct.pack("<4s5H3I2H",b"PK\x03\x04",20,0,0,0,0,crc(data),len(data),len(data),len(n),0)+n+data
def finish(body,records,index_offsets=None):
    if index_offsets is None: index_offsets={n:o for n,d,o,x in records}
    rows=[(hashlib.md5(n.encode()).digest(),index_offsets[n]) for n,d,o,x in records]
    rows.sort(key=lambda row:struct.unpack("<QQ",row[0]))
    data=b"".join(h+struct.pack("<Q",o) for h,o in rows)
    records=records+[(INDEX,data,len(body),b"")]
    body+=local(INDEX,data)
    start=len(body)
    for name,data,offset,extra in records:
        n=name.encode()
        body+=struct.pack("<4s6H3I5H2I",b"PK\x01\x02",20,20,0,0,0,0,crc(data),len(data),len(data),len(n),len(extra),0,0,0,0,offset)+n+extra
    size=len(body)-start
    return body+struct.pack("<4s4H2IH",b"PK\x05\x06",0,0,len(records),len(records),size,start,0)
def unicode_alias():
    hidden_name="hidden.json"
    hidden_data=b"hidden input"
    extra=tlv(0x7075,b"\x01"+struct.pack("<I",crc(hidden_name.encode()))+b"tileset.json")
    body=local(hidden_name,hidden_data)
    good=manifest()
    records=[(hidden_name,hidden_data,0,extra),("tileset.json",good,len(body),b"")]
    body+=local("tileset.json",good)
    # Downstream Unicode alias causes hidden.json to disappear. A single index
    # record covers only the visible, later tileset.json record.
    h=hashlib.md5(b"tileset.json").digest()+struct.pack("<Q",records[-1][2])
    records.append((INDEX,h,len(body),b""));body+=local(INDEX,h);start=len(body)
    for name,data,offset,x in records:
        n=name.encode();body+=struct.pack("<4s6H3I5H2I",b"PK\x01\x02",20,20,0,0,0,0,crc(data),len(data),len(data),len(n),len(x),0,0,0,0,offset)+n+x
    return body+struct.pack("<4s4H2IH",b"PK\x05\x06",0,0,len(records),len(records),len(body)-start,start,0),{"physical_records":3,"raw_names_unique":True,"effective_name_alias":"hidden.json -> tileset.json","index_bytes":24,"required_physical_index_bytes":48}
def zip64_offset_override(enabled):
    child=manifest()
    records=[];body=b""
    for name,data in [("tileset.json",manifest("child.json")),("conversion.json",b'{"geometryReports":"carrier.bin"}')]:
        records.append((name,data,len(body),b""));body+=local(name,data)
    carrier_start=len(body)
    embedded_start=carrier_start+30+len("carrier.bin")+8
    carrier=b"prefix!!"+local("child.json",child)+b"suffix!!"
    records.append(("carrier.bin",carrier,carrier_start,b""));body+=local("carrier.bin",carrier)
    declared_child=len(body)
    extra=tlv(1,struct.pack("<QQQ",len(child),len(child),embedded_start)) if enabled else b""
    records.append(("child.json",child,declared_child,extra));body+=local("child.json",child)
    offsets={n:o for n,d,o,x in records}
    if enabled: offsets["child.json"]=embedded_start
    result=finish(body,records,offsets)
    return result,{"raw_central_record_extents_disjoint":True,"declared_child_local_offset":declared_child,
        "ZIP64_extra_offset":embedded_start if enabled else None,"central_size_sentinels":False,"central_offset_sentinel":False,
        "downstream_child_nested_inside_carrier":enabled}
def main():
    p=argparse.ArgumentParser();p.add_argument("--binary",type=Path,required=True);p.add_argument("--binary-sha256",required=True);p.add_argument("--receipt",required=True);args=p.parse_args()
    binary=args.binary.resolve();digest=hashlib.sha256(binary.read_bytes()).hexdigest();assert digest==args.binary_sha256
    cases=[("Unicode-extra-hidden-name",unicode_alias()),("ZIP64-no-sentinel-control",zip64_offset_override(False)),("ZIP64-no-sentinel-nested-offset",zip64_offset_override(True))]
    records=[]
    for label,(raw,facts) in cases:
        path=HERE/(label+".3tz");path.write_bytes(raw);command=["nice","-n","10",str(binary),"validate","--json",str(path)]
        result=subprocess.run(command,capture_output=True,text=True,timeout=10)
        records.append({"label":label,"archive_sha256":hashlib.sha256(raw).hexdigest(),"archive_bytes":len(raw),"facts":facts,"command":command,"returncode":result.returncode,"stdout":result.stdout,"stderr":result.stderr})
    receipt={"classification":"independent actual CLI identity-override controls; no generic format/allocation claim","binary_path":str(binary),"binary_sha256":digest,"driver_sha256":hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),"records":records}
    target=HERE/args.receipt;assert target.parent==HERE and not target.exists();target.write_text(json.dumps(receipt,indent=2)+"\n")
    print(json.dumps({"receipt":str(target),"returncodes":[r["returncode"] for r in records]}))
if __name__=="__main__": main()
