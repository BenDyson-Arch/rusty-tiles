"""Independently split completed campaign archives into leaf/proxy texture bytes.
Usage: python3 archive_breakdown.py EVIDENCE_DIR SCRATCH_DIR
Uses the supplied simview logical tree to classify content, not filename guesses.
"""
import importlib.util, json, struct, sys, zipfile
from pathlib import Path
spec=importlib.util.spec_from_file_location("sim","/tmp/simview/sim.py")
sim=importlib.util.module_from_spec(spec); spec.loader.exec_module(sim)
root, scratch=map(Path,sys.argv[1:])
for result_path in sorted(root.glob("*/result.json")):
    archive=scratch/result_path.parent.name/"out.3tz"
    if not archive.exists(): archive=result_path.parent/"out.3tz"
    if not archive.exists(): continue
    logical,_,_,_=sim.parse_output(sim.Source(archive))
    leaves=set(); stack=[logical]
    while stack:
        node=stack.pop()
        if node.content and not node.children: leaves.add(node.content[0])
        stack.extend(node.children)
    totals={side:{"bytes":0,"base_texels":0,"pages":0,"codecs":{}} for side in ["leaf","proxy"]}
    with zipfile.ZipFile(archive) as z:
        for entry in z.infolist():
            if not entry.filename.endswith(".glb"): continue
            b=z.read(entry); jl=struct.unpack_from("<I",b,12)[0]; doc=json.loads(b[20:20+jl]); offset=28+jl
            side=totals["leaf" if entry.filename in leaves else "proxy"]
            for im in doc.get("images",[]):
                v=doc["bufferViews"][im["bufferView"]]; pos=offset+v.get("byteOffset",0)
                side["bytes"]+=v["byteLength"]; side["pages"]+=1
                if im["mimeType"]=="image/ktx2":
                    w,h=struct.unpack_from("<II",b,pos+20)
                    dfd=struct.unpack_from("<I",b,pos+48)[0]
                    codec={163:"etc1s",166:"uastc"}[b[pos+dfd+12]]
                else:
                    w,h=struct.unpack_from(">II",b,pos+16); codec="png"
                side["base_texels"]+=w*h
                side["codecs"][codec]=side["codecs"].get(codec,0)+1
    for side in totals.values():
        side["bytes_per_base_texel"]=side["bytes"]/side["base_texels"] if side["base_texels"] else 0
    result=json.loads(result_path.read_text()); result["texture_sides"]=totals
    result_path.write_text(json.dumps(result,indent=2)+"\n")
    print(result_path.parent.name,totals)
