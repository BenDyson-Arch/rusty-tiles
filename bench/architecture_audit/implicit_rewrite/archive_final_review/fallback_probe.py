#!/usr/bin/env python3
"""Independent selected-EOCD control using this review lane's own tiny builder."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
from duplicate_name_probe import generate
from identity_override_probe import crc, local, tlv

HERE=Path(__file__).resolve().parent

def archive(extra):
    name="opaque.bin"
    nested_start=30+len(name)
    inner,facts=generate(False)
    inner=bytearray(inner)
    index_start=facts["local_starts"][-1]
    data_start=index_start+30+len("@3dtilesIndex1@")
    # The downstream codec validates absolute source offsets after ZipArchive
    # applies a nested archive prefix. Keep this control's index coherent there.
    struct.pack_into("<Q",inner,data_start+16,nested_start)
    index_crc=crc(inner[data_start:data_start+24])
    struct.pack_into("<I",inner,index_start+14,index_crc)
    index_central=facts["central_start"]+46+len("tileset.json")
    struct.pack_into("<I",inner,index_central+16,index_crc)
    inner=bytes(inner)
    body=local(name,inner)
    central_start=len(body)
    n=name.encode()
    body+=struct.pack("<4s6H3I5H2I",b"PK\x01\x02",20,20,0,0,0,0,crc(inner),len(inner),len(inner),len(n),len(extra),0,0,0,0,0)+n+extra
    size=len(body)-central_start
    body+=struct.pack("<4s4H2IH",b"PK\x05\x06",0,0,1,1,size,central_start,0)
    return body,{"selected_final_central_records":1,"selected_final_names":[name],"final_directory_offset":central_start,
        "nested_start":nested_start,"nested_central_records":2,"outer_extra_hex":extra.hex(),
        "extra_TLV_framing_valid":True,"outer_stored_extent_disjoint":True}

def main():
    p=argparse.ArgumentParser();p.add_argument("--binary",type=Path,required=True);p.add_argument("--binary-sha256",required=True);p.add_argument("--receipt",required=True);args=p.parse_args()
    binary=args.binary.resolve();digest=hashlib.sha256(binary.read_bytes()).hexdigest();assert digest==args.binary_sha256
    records=[]
    for label,extra in [("opaque-nested-ZIP-selected-outer",b""),("opaque-nested-ZIP-invalid-NTFS-fallback",tlv(0x000a,b"")),("opaque-nested-ZIP-invalid-timestamp-fallback",tlv(0x5455,b""))]:
        raw,facts=archive(extra);path=HERE/(label+".3tz");path.write_bytes(raw)
        command=["nice","-n","10",str(binary),"validate","--json",str(path)];result=subprocess.run(command,capture_output=True,text=True,timeout=10)
        records.append({"label":label,"archive_bytes":len(raw),"archive_sha256":hashlib.sha256(raw).hexdigest(),"facts":facts,"command":command,"returncode":result.returncode,"stdout":result.stdout,"stderr":result.stderr})
    receipt={"classification":"independent actual CLI selected-final-directory fallback control; no generic ZIP or allocation claim","binary_path":str(binary),"binary_sha256":digest,"driver_sha256":hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),"records":records}
    target=HERE/args.receipt;assert target.parent==HERE and not target.exists();target.write_text(json.dumps(receipt,indent=2)+"\n")
    print(json.dumps({"receipt":str(target),"returncodes":[r["returncode"] for r in records]}))
if __name__=="__main__":main()
