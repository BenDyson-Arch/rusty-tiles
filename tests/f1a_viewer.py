#!/usr/bin/env python3
"""Pinned local Cesium/Chromium F1a routing acceptance; no network or downloads.

Requires already-installed Cesium1.146/Playwright1.63 and Chromium. This checks
rendered leaf selection at fixed camera/SSE settings, not geometric accuracy.
"""
import argparse
import functools
import hashlib
import http.server
import json
import os
from pathlib import Path
import subprocess
import signal
import tempfile
import threading

import f1a_oracle


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',required=True,type=Path)
    p.add_argument('--cesium-dir',required=True,type=Path,help='Pinned Cesium Build/Cesium directory')
    p.add_argument('--node-modules',type=Path,help='Directory containing playwright; otherwise NODE_PATH')
    p.add_argument('--chromium',default=os.environ.get('CHROMIUM','/usr/bin/chromium'))
    p.add_argument('--json-output',type=Path)
    args=p.parse_args();binary=args.binary.resolve();runtime=args.cesium_dir.resolve()
    if not (runtime/'Cesium.js').is_file():p.error('Cesium runtime missing')
    with tempfile.TemporaryDirectory(prefix='f1a-viewer-') as temporary:
        root=Path(temporary);source=root/'source.glb';source.write_bytes(f1a_oracle.fixture(12,transformed=False))
        archive=root/'candidate.3tz'
        result=subprocess.run([str(binary),'--json','mesh-local-to-3tz','-i',str(source),'-o',str(archive),'--leaf-triangles','3'],text=True,capture_output=True,timeout=60)
        f1a_oracle.require(result.returncode==0,'candidate conversion: '+result.stdout+result.stderr)
        inspected=f1a_oracle.inspect(source,archive,3)
        import zipfile
        with zipfile.ZipFile(archive) as z:
            for prefix in ('mesh','zero'):
                for name in z.namelist():
                    if name=='@3dtilesIndex1@':continue
                    path=root/prefix/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(z.read(name))
        manifest=json.loads((root/'zero/tileset.json').read_text());manifest['root']['geometricError']=0
        (root/'zero/tileset.json').write_text(json.dumps(manifest))
        html=b'''<!doctype html><html><head><script>window.CESIUM_BASE_URL='/cesium/';</script><script src='/cesium/Cesium.js'></script><link rel='stylesheet' href='/cesium/Widgets/widgets.css'><style>html,body,#viewer{width:100%;height:100%;margin:0}</style></head><body><div id='viewer'></div><script>window.viewer=new Cesium.Viewer('viewer',{globe:false,baseLayer:false,skyBox:false,skyAtmosphere:false,animation:false,timeline:false,geocoder:false,homeButton:false,sceneModePicker:false,baseLayerPicker:false,navigationHelpButton:false,fullscreenButton:false});</script></body></html>'''
        (root/'index.html').write_bytes(html)
        class Handler(http.server.SimpleHTTPRequestHandler):
            def translate_path(self,path):
                if path.startswith('/cesium/'):
                    # Static local runtime only, normalized by the base handler.
                    translated=super().translate_path(path[7:])
                    return str(runtime/Path(translated).relative_to(root))
                return super().translate_path(path)
            def log_message(self,*unused):pass
        server=http.server.ThreadingHTTPServer(('127.0.0.1',0),functools.partial(Handler,directory=str(root)))
        thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
        try:
            env=dict(os.environ,CHROMIUM=args.chromium)
            if args.node_modules:env['NODE_PATH']=str(args.node_modules.resolve())
            command=['node',str(Path(__file__).parent/'fixtures/f1a_viewer.cjs'),f'http://127.0.0.1:{server.server_port}/',str(inspected['leaves'])]
            # Timeout must reap the browser descendants as well as the Node runner.
            with subprocess.Popen(command,text=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE,
                                  env=env,start_new_session=True) as child:
                try:
                    stdout,stderr=child.communicate(timeout=90)
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid,signal.SIGKILL)
                    child.communicate()
                    raise
                completed=subprocess.CompletedProcess(command,child.returncode,stdout,stderr)
            f1a_oracle.require(completed.returncode==0,'viewer: '+completed.stdout+completed.stderr)
            browser=json.loads(completed.stdout)
        finally:
            server.shutdown();server.server_close();thread.join()
        evidence={'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'cesium_js_sha256':hashlib.sha256((runtime/'Cesium.js').read_bytes()).hexdigest(),'archive':inspected,'browser':browser,
                  'limits':'Defined local camera/SSE, pinned Cesium runtime and Chromium/SwiftShader on this platform. No universal client/view distance, GPU accuracy, source fidelity or CRS claim.'}
    encoded=json.dumps(evidence,indent=2)+'\n'
    if args.json_output:args.json_output.write_text(encoded)
    else:print(encoded,end='')


if __name__=='__main__':main()
