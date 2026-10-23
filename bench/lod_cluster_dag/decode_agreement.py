#!/usr/bin/env python3
"""Compare native and independent WASM decoding, including metadata bytes."""
import json
from pathlib import Path
import sys
from build import GLB

original,decoded,report=map(Path,sys.argv[1:]);count=0;size=0
for file in original.glob('content/*.glb'):
    a=GLB(file);b=GLB(decoded/'content'/file.name)
    assert len(a.j['bufferViews'])==len(b.j['bufferViews'])
    for i in range(len(a.j['bufferViews'])):
        av,bv=a.view(i),b.view(i)
        assert av==bv,(file,i)
        count+=1;size+=len(av)
data=dict(buffer_views=count,decoded_bytes=size,all_bytes_equal=True)
report.write_text(json.dumps(data,indent=2));print(data)
