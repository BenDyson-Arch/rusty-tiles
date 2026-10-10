#!/usr/bin/env python3
"""Independent lit source-versus-output core-PBR viewer with sensitive controls."""
import argparse
import functools
import hashlib
import http.server
import json
import os
from pathlib import Path
import signal
import struct
import subprocess
import tempfile
import threading
import zipfile

import f1b3_oracle as oracle


CONTROLS=('wrong-uv','wrong-slot','wrong-w','no-mr','no-normal','no-occlusion','no-emissive','wrong-color','wrong-scale','wrong-strength')


def mutate(data,control):
    def change(doc,binary):
        for material in doc.get('materials',[]):
            pbr=material.get('pbrMetallicRoughness',{})
            if control=='wrong-uv':
                for info in oracle.texture.texture_infos(material):info['texCoord']=0
            elif control=='wrong-slot' and 'emissiveTexture' in material:
                material['emissiveTexture']['index']=(material['emissiveTexture']['index']+1)%len(doc['textures'])
            elif control=='no-mr':pbr.pop('metallicRoughnessTexture',None)
            elif control=='no-normal':material.pop('normalTexture',None)
            elif control=='no-occlusion':material.pop('occlusionTexture',None)
            elif control=='no-emissive':material.pop('emissiveTexture',None)
            elif control=='wrong-scale' and 'normalTexture' in material:material['normalTexture']['scale']=0
            elif control=='wrong-strength' and 'occlusionTexture' in material:material['occlusionTexture']['strength']=0
        for mesh in doc.get('meshes',[]):
            for primitive in mesh['primitives']:
                name='TANGENT' if control=='wrong-w' else 'COLOR_0' if control=='wrong-color' else None
                if name not in primitive['attributes']:continue
                a=doc['accessors'][primitive['attributes'][name]];v=doc['bufferViews'][a['bufferView']]
                for i in range(a['count']):
                    start=v.get('byteOffset',0)+a.get('byteOffset',0)+i*v.get('byteStride',16)
                    if name=='TANGENT':struct.pack_into('<f',binary,start+12,-struct.unpack_from('<f',binary,start+12)[0])
                    else:struct.pack_into('<4f',binary,start,1,1,1,1)
    return oracle.texture.mutated_source(data,change)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',required=True,type=Path);p.add_argument('--three-dir',required=True,type=Path)
    p.add_argument('--node-modules',required=True,type=Path);p.add_argument('--chromium',required=True,type=Path)
    p.add_argument('--json-output',type=Path);args=p.parse_args()
    binary=args.binary.resolve(strict=True);runtime=args.three_dir.resolve(strict=True)
    oracle.require(json.loads((runtime/'package.json').read_text())['version']=='0.180.0','pinned Three0.180.0 required')
    with tempfile.TemporaryDirectory(prefix='f1b3-lit-viewer-') as temporary:
        root=Path(temporary);source=root/'reference'/'source.glb';source.parent.mkdir();source.write_bytes(oracle.viewer_fixture())
        archive=root/'candidate.3tz'
        completed=subprocess.run([str(binary),'--json','mesh-local-to-3tz','-i',str(source),'-o',str(archive),'--leaf-triangles','2'],text=True,capture_output=True,timeout=60)
        oracle.require(completed.returncode==0,'lit fixture conversion: '+completed.stdout+completed.stderr)
        inspected=oracle.inspect(source,archive,2)
        with zipfile.ZipFile(archive) as z:members={n:z.read(n) for n in z.namelist() if n!='@3dtilesIndex1@'}
        for prefix in ('mesh',*CONTROLS):
            for name,data in members.items():
                if name.endswith('.glb') and prefix!='mesh':data=mutate(data,prefix)
                path=root/prefix/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(data)
        html="""<!doctype html><html><head><style>html,body{width:100%;height:100%;margin:0}</style><script type='importmap'>{"imports":{"three":"/three/build/three.module.js","three/addons/":"/three/examples/jsm/"}}</script></head><body><script type='module'>import * as THREE from 'three';import {GLTFLoader} from 'three/addons/loaders/GLTFLoader.js';window.THREE=THREE;window.GLTFLoader=GLTFLoader;</script></body></html>"""
        (root/'index.html').write_text(html)
        class Handler(http.server.SimpleHTTPRequestHandler):
            def translate_path(self,path):
                if path.startswith('/three/'):
                    translated=super().translate_path(path[6:]);return str(runtime/Path(translated).relative_to(root))
                return super().translate_path(path)
            def log_message(self,*unused):pass
        server=http.server.ThreadingHTTPServer(('127.0.0.1',0),functools.partial(Handler,directory=str(root)))
        thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
        try:
            env=dict(os.environ,NODE_PATH=str(args.node_modules.resolve()),CHROMIUM=str(args.chromium.resolve()))
            with subprocess.Popen(['node',str(Path(__file__).parent/'fixtures/f1b3_viewer.cjs'),f'http://127.0.0.1:{server.server_port}/',str(inspected['leaves'])],text=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=env,start_new_session=True) as child:
                try:stdout,stderr=child.communicate(timeout=120)
                except subprocess.TimeoutExpired:os.killpg(child.pid,signal.SIGKILL);child.communicate();raise
                oracle.require(child.returncode==0,'lit viewer proof: '+stdout+stderr)
                browser=json.loads(stdout)
        finally:server.shutdown();server.server_close();thread.join()
        evidence={'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'three_version':'0.180.0',
                  'authored_driver_sha256':{str(path.relative_to(Path(__file__).parent)):
                      hashlib.sha256(path.read_bytes()).hexdigest() for path in
                      (Path(__file__),Path(__file__).parent/'fixtures/f1b3_viewer.cjs',
                       Path(__file__).parent/'f1b3_oracle.py',Path(__file__).parent/'f1b_oracle.py',
                       Path(__file__).parent/'f1b2_oracle.py')},
                  'three_module_sha256':hashlib.sha256((runtime/'build/three.module.js').read_bytes()).hexdigest(),
                  'gltf_loader_sha256':hashlib.sha256((runtime/'examples/jsm/loaders/GLTFLoader.js').read_bytes()).hexdigest(),
                  'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'archive':inspected,'browser':browser}
    encoded=json.dumps(evidence,indent=2)+'\n'
    if args.json_output:args.json_output.parent.mkdir(parents=True,exist_ok=True);args.json_output.write_text(encoded)
    else:print(encoded,end='')


if __name__=='__main__':main()
