#!/usr/bin/env python3
"""Physical bytes per depth, and the supplied simulator plus image dependencies."""
import collections
import io,struct
from PIL import Image
import json
import os
from pathlib import Path
import subprocess
import sys
from build import GLB

root=Path(sys.argv[1]);out=Path(sys.argv[2]);out.mkdir(exist_ok=True,parents=True)
sys.path.insert(0,'/home/bend/.cache/rusty-tiles-evidence/leaf-gaps-20261024')
from tile_tree import tree
tt=tree(str(root));levels=collections.defaultdict(lambda:dict(tiles=0,triangles=0,glb_bytes=0,images=set(),ge=[],embedded_texels=0))
for file,n in tt.items():
    g=GLB(root/file);lv=levels[n['depth']];lv['tiles']+=1;lv['glb_bytes']+=(root/file).stat().st_size;lv['ge'].append(n['ge'])
    for mesh in g.j['meshes']:
        for p in mesh['primitives']:
            lv['triangles']+=g.j['accessors'][p.get('indices',p['attributes']['POSITION'])]['count']//3
    lv['images'].update(((root/file).parent/im['uri']).resolve() for im in g.j.get('images',[]) if 'uri' in im)
    for im in g.j.get('images',[]):
        if 'bufferView' not in im:continue
        data=g.view(im['bufferView'])
        if data.startswith(b'\xabKTX 20'):w,h=struct.unpack_from('<II',data,20)
        else:w,h=Image.open(io.BytesIO(data)).size
        lv['embedded_texels']+=w*h
for lv in levels.values():
    images=lv.pop('images');lv['image_bytes']=sum(p.stat().st_size for p in images)
    lv['texels']=lv.pop('embedded_texels')+sum(Image.open(p).width*Image.open(p).height for p in images)
    lv['total_bytes']=lv['glb_bytes']+lv['image_bytes'];lv['ge_min']=min(lv['ge']);lv['ge_max']=max(lv.pop('ge'))
views={}
for name,w,h,r in [('far',1600,1000,3),('wide',2560,1600,3),('near',2560,1600,.75)]:
    env=dict(os.environ,SIM_W=str(w),SIM_H=str(h),SIM_RANGE=str(r))
    data=json.loads(subprocess.check_output([sys.executable,'/tmp/simview/sim.py',str(root),'--json'],env=env))
    (out/f'sim-{name}.json').write_text(json.dumps(data,indent=2));row=data[0];images=set()
    for file in row['names']:
        if not file.endswith('.glb'):continue
        g=GLB(root/file);images.update((root/file).parent/im['uri'] for im in g.j.get('images',[]) if 'uri' in im)
    row['external_image_bytes']=sum(p.stat().st_size for p in images)
    row['external_image_requests']=len(images)
    row['root_document_bytes']=(root/'tileset.json').stat().st_size
    row['inclusive_bytes']=row['bytes']+row['external_image_bytes']+row['root_document_bytes']
    views[name]=row
result=dict(levels=levels,first_views=views,total_output_bytes=sum(p.stat().st_size for p in root.rglob('*') if p.is_file() and p.name!='build.json' and '.run' not in p.relative_to(root).parts))
(out/'inventory.json').write_text(json.dumps(result,indent=2))
print(json.dumps(dict(levels=levels,first_view_bytes={k:v['inclusive_bytes'] for k,v in views.items()})))
