#!/usr/bin/env python3
"""Artifact-bound public Cesium coarse/fine proxy and exact-triangle queries."""
import argparse
import functools
import hashlib
import http.server
import json
import math
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import threading
import zipfile

import f1d1_oracle as oracle


def file_sha256(path):
    result=hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda:stream.read(1024*1024),b''):result.update(block)
    return result.hexdigest()


def plan(source):
    truth,names=oracle.leaf_oracle.expected_source(source)
    regions=sorted({key[:3] for key in truth}); targets=[]
    for region in regions:
        keys=sorted(key for key in truth if key[:3]==region)
        key=keys[len(keys)//3]
        positions=truth[key]['positions']
        center=[sum(p[c] for p in positions)/3 for c in range(3)]
        proxy={column:[identity[c] for identity in keys] for c,column in enumerate(oracle.ARRAY_KEYS)}
        proxy.update(source_node_name=names[region[0]][1],source_node_name_present=names[region[0]][0])
        targets.append({'world':[center[0],-center[2],center[1]],'proxy_region':proxy,'source_triangle':dict(zip(oracle.KEYS,key))})
    center=sum(t['world'][0] for t in targets)/len(targets)
    def camera(distance):
        length=math.hypot(distance,distance)
        return {'destination':[center,-distance,distance],'direction':[0,distance/length,-distance/length],'up':[0,distance/length,distance/length]}
    return {'targets':targets,'camera':{'coarse':camera(400),'fine':camera(30)}}


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--source',type=Path,required=True);p.add_argument('--archive',type=Path,required=True)
    p.add_argument('--cesium-dir',type=Path,required=True);p.add_argument('--node-modules',type=Path,required=True)
    p.add_argument('--chromium',type=Path,required=True);p.add_argument('--json-output',type=Path)
    p.add_argument('--synthetic',action='store_true',help='Label independently authored synthetic artifact as consumer feasibility only')
    p.add_argument('--leaf-limit',type=int,default=16);p.add_argument('--triangle-limit',type=int,default=48);p.add_argument('--max-error',type=float,default=8)
    a=p.parse_args();source=a.source.read_bytes();runtime=a.cesium_dir.resolve(strict=True)
    with zipfile.ZipFile(a.archive) as stream:members={n:stream.read(n) for n in stream.namelist() if n!='@3dtilesIndex1@'}
    checked=oracle.inspect_members(source,members,a.leaf_limit,a.triangle_limit,a.max_error)
    with tempfile.TemporaryDirectory(prefix='f1d1-viewer-') as temporary:
        root=Path(temporary)
        for name,data in members.items():
            target=root/'good'/name;target.parent.mkdir(parents=True,exist_ok=True);target.write_bytes(data)
        (root/'plan.json').write_text(json.dumps(plan(source)))
        (root/'index.html').write_text("""<!doctype html><html><head><script>window.CESIUM_BASE_URL='/cesium/';</script>
<script src='/cesium/Cesium.js'></script><link rel='stylesheet' href='/cesium/Widgets/widgets.css'>
<style>html,body,#viewer{width:100%;height:100%;margin:0}</style></head><body><div id='viewer'></div><script>
window.viewer=new Cesium.Viewer('viewer',{globe:false,baseLayer:false,skyBox:false,skyAtmosphere:false,animation:false,timeline:false,geocoder:false,homeButton:false,sceneModePicker:false,baseLayerPicker:false,navigationHelpButton:false,fullscreenButton:false});
</script></body></html>""")
        class Handler(http.server.SimpleHTTPRequestHandler):
            def translate_path(self,path):
                if path.startswith('/cesium/'):
                    translated=super().translate_path(path[7:]);return str(runtime/Path(translated).relative_to(root))
                return super().translate_path(path)
            def log_message(self,*unused):pass
        server=http.server.ThreadingHTTPServer(('127.0.0.1',0),functools.partial(Handler,directory=str(root)))
        thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
        try:
            env=dict(os.environ,NODE_PATH=str(a.node_modules.resolve(strict=True)),CHROMIUM=str(a.chromium.resolve(strict=True)))
            with subprocess.Popen(['node',str(Path(__file__).parent/'fixtures/f1d1_viewer.cjs'),f'http://127.0.0.1:{server.server_port}/'],
                env=env,text=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE,start_new_session=True) as child:
                try:stdout,stderr=child.communicate(timeout=120)
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid,signal.SIGKILL);child.communicate();raise
                oracle.require(child.returncode==0,'public viewer '+stdout+stderr)
                browser=json.loads(stdout)
        finally:server.shutdown();server.server_close();thread.join()
    paths=(Path(__file__),Path(__file__).with_name('f1d1_oracle.py'),Path(__file__).with_name('f1c2_oracle.py'),Path(__file__).parent/'fixtures/f1d1_viewer.cjs',Path(__file__).with_name('f1d2_certificate.py'))
    result={'mode':'independent-synthetic-consumer-feasibility' if a.synthetic else 'candidate-published-archive','source_sha256':oracle.digest(source),'archive_sha256':oracle.digest(a.archive.read_bytes()),
        'artifact':checked,'browser':browser,'driver_sha256':{str(path.relative_to(Path(__file__).parent)):oracle.digest(path.read_bytes()) for path in paths},
        'cesium_js_sha256':oracle.digest((runtime/'Cesium.js').read_bytes()),
        'invocation':[sys.executable,*sys.argv],
        'browser_executable':str(a.chromium.resolve(strict=True)),
        'browser_executable_sha256':file_sha256(a.chromium.resolve(strict=True)),
        'packages':{name:{'version':json.loads(path.read_text())['version'],'package_json_sha256':file_sha256(path)} for name,path in (
            ('cesium',runtime.parents[1]/'package.json'),('playwright',a.node_modules.resolve(strict=True)/'playwright/package.json'))},
        'scope':'Finite local planar cameras, unchanged screen-space-error threshold, public tileVisible traversal and getProperty arrays/exact tuples. No textured appearance, coincident selection, world-distance or general topology acceptance.'}
    text=json.dumps(result,indent=2,allow_nan=False)+'\n'
    if a.json_output:a.json_output.parent.mkdir(parents=True,exist_ok=True);a.json_output.write_text(text)
    else:print(text,end='')


if __name__=='__main__':main()
