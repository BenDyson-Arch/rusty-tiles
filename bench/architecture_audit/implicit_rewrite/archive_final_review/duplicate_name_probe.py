#!/usr/bin/env python3
"""Nonauthor, independent physical-cardinality control; no product/fixture imports."""
import argparse
import binascii
import hashlib
import json
from pathlib import Path
import struct
import subprocess

HERE = Path(__file__).resolve().parent

def generate(hidden_duplicate):
    manifest = json.dumps({"asset":{"version":"1.1"},"geometricError":0,
        "root":{"boundingVolume":{"box":[0,0,0,1,0,0,0,1,0,0,0,1]},
        "geometricError":0,"refine":"REPLACE"}},separators=(",",":")).encode()
    members = []
    if hidden_duplicate:
        members.append(("tileset.json", b"hidden record is not a manifest"))
    members.append(("tileset.json", manifest))
    body = bytearray()
    records = []
    def local(name, data):
        name = name.encode()
        offset = len(body)
        crc = binascii.crc32(data) & 0xffffffff
        body.extend(struct.pack("<4s5H3I2H", b"PK\x03\x04",20,0,0,0,0,crc,len(data),len(data),len(name),0))
        body.extend(name)
        body.extend(data)
        records.append((name,data,offset,crc))
    for name, data in members:
        local(name,data)
    selected = records[-1]
    index = hashlib.md5(selected[0]).digest() + struct.pack("<Q",selected[2])
    local("@3dtilesIndex1@",index)
    central_start = len(body)
    for name,data,offset,crc in records:
        body.extend(struct.pack("<4s6H3I5H2I",b"PK\x01\x02",20,20,0,0,0,0,crc,len(data),len(data),len(name),0,0,0,0,0,offset))
        body.extend(name)
    central_size = len(body) - central_start
    body.extend(struct.pack("<4s4H2IH",b"PK\x05\x06",0,0,len(records),len(records),central_size,central_start,0))
    return bytes(body), {"physical_central_records":len(records),"non_index_records":len(records)-1,
        "index_bytes":len(index),"required_physical_index_bytes":24*(len(records)-1),
        "local_starts":[r[2] for r in records],"central_start":central_start,
        "all_physical_extents_disjoint":True}

def main():
    p=argparse.ArgumentParser()
    p.add_argument("--binary",type=Path,required=True)
    p.add_argument("--binary-sha256",required=True)
    p.add_argument("--receipt",required=True)
    args=p.parse_args()
    binary=args.binary.resolve()
    digest=hashlib.sha256(binary.read_bytes()).hexdigest()
    assert digest==args.binary_sha256
    records=[]
    for duplicate in (False,True):
        label="disjoint-duplicate-name-hidden-index-row" if duplicate else "single-manifest-control"
        raw,facts=generate(duplicate)
        archive=HERE/(label+".3tz")
        archive.write_bytes(raw)
        command=["nice","-n","10",str(binary),"validate","--json",str(archive)]
        result=subprocess.run(command,capture_output=True,text=True,timeout=10)
        records.append({"label":label,"archive_sha256":hashlib.sha256(raw).hexdigest(),
            "archive_bytes":len(raw),"facts":facts,"command":command,"returncode":result.returncode,
            "stdout":result.stdout,"stderr":result.stderr})
    receipt={"classification":"independent tiny actual CLI physical-cardinality control; no generic ZIP or allocation claim",
        "binary_path":str(binary),"binary_sha256":digest,"driver_sha256":hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "records":records}
    target=HERE/args.receipt
    assert target.parent==HERE and not target.exists()
    target.write_text(json.dumps(receipt,indent=2)+"\n")
    print(json.dumps({"receipt":str(target),"returncodes":[r["returncode"] for r in records]}))

if __name__=="__main__":main()
