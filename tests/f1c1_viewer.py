#!/usr/bin/env python3
"""Actual unchanged Cesium tileset placement with independent fixed world targets.

Synthetic mode is consumer feasibility/self-sensitivity evidence, not conversion
acceptance. --binary inspects every original candidate archive before rendering.
"""
import argparse
import copy
import functools
import hashlib
import http.server
import json
import math
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import threading
import zipfile

import f1c1_oracle as oracle

CASES = (
    ('brisbane-tilted',{'anchor':[153.,-27.,130.],'orientation_xyzw':oracle.MIXED,'scene_offset':[11.,13.,17.]},1.),
    ('western-tilted',{'anchor':[-123.,48.,1000.],'orientation_xyzw':oracle.MIXED,'scene_offset':[-19.,23.,29.]},1.),
    ('pole-meridian',{'anchor':[137.,90.,100.],'orientation_xyzw':oracle.MIXED,'scene_offset':[11.,13.,17.]},1.),
    ('far-small-detail',{'anchor':[37.,48.,50000000.],'orientation_xyzw':oracle.MIXED,'scene_offset':[11.,13.,17.]},.01),
)


def normalized(p):
    length=math.sqrt(sum(x*x for x in p));return [x/length for x in p]


def plan_case(name,placement,scale):
    expectation,reference=oracle.placement_reference(placement)
    targets=[{'world':[float(x) for x in oracle.decimal_world(reference,(x*scale,y*scale,0.))], 'local_enu':[x*scale,y*scale,0.]} for x,y in ((-7.,-4.),(8.,-2.),(-1.,10.))]
    eye=(0.,-25.*scale,45.*scale);direction=(0.,27.,-45.)
    return {'name':name,'placement':placement,'scale':scale,'expectedRootTransform':expectation['root_transform'],'targets':targets,
            'camera':{'destination':[float(x) for x in oracle.decimal_world(reference,eye)],
                      'direction':normalized([float(x) for x in oracle.decimal_world(reference,direction,True)]),
                      'up':normalized([float(x) for x in oracle.decimal_world(reference,(0.,0.,1.),True)])},
            'farCamera':{'destination':[float(x) for x in oracle.decimal_world(reference,(0.,-250000.,450000.))]},
            'nearPlaneMetres':max(.0001,.01*scale)}


def synthetic_members(data,placement):
    """Independently author three single-triangle leaves, not converter output."""
    doc,payload=oracle.geometry.decode_glb(data);members={};boxes=[]
    for i,primitive in enumerate(doc['meshes'][0]['primitives']):
        source=copy.deepcopy(doc);source['meshes']=[{'primitives':[copy.deepcopy(primitive)]}]
        source['materials']=[source['materials'][primitive['material']]];source['meshes'][0]['primitives'][0]['material']=0
        # Accessor/view closure is not a placement gate; unused admitted storage
        # stays in this independent feasibility asset, while material closure is exact.
        members['t/'+str(i)+'.glb']=oracle.texture.encode_glb(source,payload)
        positions=oracle.texture.accessor(doc,payload,primitive['attributes']['POSITION']);points=[oracle.geometry.z_up(p) for p in positions]
        low=[min(p[c] for p in points) for c in range(3)];high=[max(p[c] for p in points) for c in range(3)]
        center=[(low[c]+high[c])/2 for c in range(3)];half=[max(center[c]-low[c],high[c]-center[c]) for c in range(3)]
        boxes.append(center+[half[0],0.,0.,0.,half[1],0.,0.,0.,half[2]])
    low=[min(box[c]-box[3+4*c] for box in boxes) for c in range(3)];high=[max(box[c]+box[3+4*c] for box in boxes) for c in range(3)]
    center=[(low[c]+high[c])/2 for c in range(3)];half=[max(center[c]-low[c],high[c]-center[c]) for c in range(3)]
    box=center+[half[0],0.,0.,0.,half[1],0.,0.,0.,half[2]];error=max(1.,math.sqrt(sum((2*h)**2 for h in half)))
    expectation,_=oracle.placement_reference(placement)
    tileset={'asset':{'version':'1.1'},'geometricError':error,'root':{'transform':expectation['root_transform'],'boundingVolume':{'box':box},'geometricError':error,'refine':'REPLACE',
             'children':[{'boundingVolume':{'box':b},'geometricError':0,'content':{'uri':'t/'+str(i)+'.glb'}} for i,b in enumerate(boxes)]}}
    members['tileset.json']=json.dumps(tileset).encode();return members


def control_members(members,placement,control):
    changed=copy.deepcopy(members);doc=json.loads(changed['tileset.json']);root=doc['root']
    if control=='missing-root':root.pop('transform')
    elif control=='zero-error':root['geometricError']=0
    elif control=='wrong-height':
        p=copy.deepcopy(placement);p['anchor'][2]+=150.;root['transform']=oracle.placement_reference(p)[0]['root_transform']
    elif control=='wrong-order':
        # Passive/inverse orientation is a plausible order/frame error. Fixed
        # world-camera targets remain the original active Hamilton expectation.
        p=copy.deepcopy(placement);p['orientation_xyzw']=[-v for v in p['orientation_xyzw'][:3]]+[p['orientation_xyzw'][3]]
        root['transform']=oracle.placement_reference(p)[0]['root_transform']
    elif control=='wrong-axis':
        transform=root['transform'];first=transform[4:7];second=transform[8:11]
        transform[4:7]=second;transform[8:11]=[-v for v in first]
    changed['tileset.json']=json.dumps(doc).encode();return changed


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    mode=parser.add_mutually_exclusive_group(required=True);mode.add_argument('--binary',type=Path);mode.add_argument('--synthetic',action='store_true')
    parser.add_argument('--cesium-dir',required=True,type=Path);parser.add_argument('--node-modules',required=True,type=Path);parser.add_argument('--chromium',required=True,type=Path)
    parser.add_argument('--json-output',type=Path);args=parser.parse_args();runtime=args.cesium_dir.resolve(strict=True)
    binary=args.binary.resolve(strict=True) if args.binary else None
    records=[];plans=[]
    with tempfile.TemporaryDirectory(prefix='f1c1-world-viewer-') as temporary:
        root=Path(temporary)
        for name,placement,scale in CASES:
            data=oracle.viewer_fixture()
            if scale!=1.:data=oracle.mutate_positions(data,lambda p,i:tuple(x*scale for x in p))
            source=root/'sources'/name/'source.glb';source.parent.mkdir(parents=True);source.write_bytes(data)
            if binary:
                archive=root/(name+'.3tz');command=[str(binary),'--json','mesh-local-to-3tz','-i',str(source),'-o',str(archive),'--leaf-triangles','1',*oracle.placement_cli_args(placement)]
                completed=subprocess.run(command,text=True,capture_output=True,timeout=60)
                oracle.require(completed.returncode==0,'viewer conversion '+completed.stdout+completed.stderr)
                inspected=oracle.inspect(source,archive,1,placement)
                oracle.require(json.loads(completed.stdout)['meshReport']==inspected['report'],'viewer CLI/raw report parity')
                with zipfile.ZipFile(archive) as stream:members={n:stream.read(n) for n in stream.namelist() if n!='@3dtilesIndex1@'}
                records.append({'name':name,'archive':inspected})
            else:
                members=synthetic_members(data,placement);oracle.read_world(members)
                records.append({'name':name,'synthetic_source_sha256':oracle.texture.digest(data)})
            for prefix in ('mesh','wrong-axis','wrong-order','wrong-height','missing-root','zero-error'):
                output=members if prefix=='mesh' else control_members(members,placement,prefix)
                for member,payload in output.items():
                    path=root/name/prefix/member;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(payload)
            plans.append(plan_case(name,placement,scale))
        (root/'plan.json').write_text(json.dumps(plans))
        html=b'''<!doctype html><html><head><script>window.CESIUM_BASE_URL='/cesium/';</script><script src='/cesium/Cesium.js'></script><link rel='stylesheet' href='/cesium/Widgets/widgets.css'><style>html,body,#viewer{width:100%;height:100%;margin:0}</style></head><body><div id='viewer'></div><script>window.viewer=new Cesium.Viewer('viewer',{globe:false,baseLayer:false,skyBox:false,skyAtmosphere:false,animation:false,timeline:false,geocoder:false,homeButton:false,sceneModePicker:false,baseLayerPicker:false,navigationHelpButton:false,fullscreenButton:false});</script></body></html>'''
        (root/'index.html').write_bytes(html)
        class Handler(http.server.SimpleHTTPRequestHandler):
            def translate_path(self,path):
                if path.startswith('/cesium/'):
                    translated=super().translate_path(path[7:]);return str(runtime/Path(translated).relative_to(root))
                return super().translate_path(path)
            def log_message(self,*unused):pass
        server=http.server.ThreadingHTTPServer(('127.0.0.1',0),functools.partial(Handler,directory=str(root)))
        thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
        try:
            env=dict(os.environ,NODE_PATH=str(args.node_modules.resolve()),CHROMIUM=str(args.chromium.resolve()))
            command=['node',str(Path(__file__).parent/'fixtures/f1c1_viewer.cjs'),f'http://127.0.0.1:{server.server_port}/']
            with subprocess.Popen(command,text=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=env,start_new_session=True) as child:
                try:stdout,stderr=child.communicate(timeout=180)
                except subprocess.TimeoutExpired:os.killpg(child.pid,signal.SIGKILL);child.communicate();raise
                oracle.require(child.returncode==0,'placement consumer '+stdout+stderr);browser=json.loads(stdout)
        finally:server.shutdown();server.server_close();thread.join()
    evidence={'mode':'candidate-archive' if binary else 'independent-synthetic-consumer-probe',
              'binary_sha256':oracle.texture.digest(binary.read_bytes()) if binary else None,'records':records,'browser':browser,
              'driver_sha256':{p.name:oracle.texture.digest(p.read_bytes()) for p in (Path(__file__),Path(__file__).parent/'fixtures/f1c1_viewer.cjs',Path(__file__).parent/'f1c1_oracle.py')},
              'cesium_js_sha256':oracle.texture.digest((runtime/'Cesium.js').read_bytes()),
              'scope':'Pinned Cesium/Chromium, fixed independent world cameras/targets and specified scales. Measured depth values include raster sampling. Separate offline Decimal/byte proof owns numeric accuracy; Three owns PBR appearance.'}
    encoded=json.dumps(evidence,indent=2,allow_nan=False)+'\n'
    if args.json_output:args.json_output.parent.mkdir(parents=True,exist_ok=True);args.json_output.write_text(encoded)
    else:print(encoded,end='')


if __name__=='__main__':main()
