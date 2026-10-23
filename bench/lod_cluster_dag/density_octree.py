#!/usr/bin/env python3
"""Proxy-only texel census; baseline reported medians refer to explicit cells only."""
import collections,io,json,struct,sys
from pathlib import Path
from PIL import Image
from build import GLB
from baseline_tree import tree
candidate,baseline,output=map(Path,sys.argv[1:]);nodes=json.loads((candidate/'build.json').read_text())['nodes']
result={}
for name,root,tt in [('baseline',baseline,tree(baseline)),('candidate',candidate,{n['file']:dict(depth=n['depth'],parent=nodes[n['parent']]['file'] if n['parent'] is not None else None) for n in nodes})]:
    parents={n['parent'] for n in tt.values()};levels=collections.defaultdict(list)
    for file,n in tt.items():
        if file not in parents:continue
        g=GLB(root/file);texels=0;pages=0
        for im in g.j.get('images',[]):
            if 'uri' in im:w,h=Image.open((root/file).parent/im['uri']).size
            else:
                raw=g.view(im['bufferView'])
                if raw.startswith(b'\xabKTX 20'):w,h=struct.unpack_from('<II',raw,20)
                else:w,h=Image.open(io.BytesIO(raw)).size
            texels+=w*h;pages+=1
        levels[n['depth']].append(dict(file=file,texels=texels,pages=pages))
    result[name]={d:dict(tiles=len(rows),texels=sum(r['texels'] for r in rows),pages=sum(r['pages'] for r in rows),rows=rows) for d,rows in levels.items()}
Path(output).write_text(json.dumps(result,indent=2));print({k:{d:(r['tiles'],r['texels']) for d,r in v.items()} for k,v in result.items()})
