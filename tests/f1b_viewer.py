#!/usr/bin/env python3
"""Local pinned Cesium/Chromium F1b appearance with sensitive viewer controls.

Requires existing assets; no downloads. F1a's separate viewer supplies routing
acceptance. This runner verifies representative nearest/clamp base-color texels
and OPAQUE/MASK behavior with display-only unlit lighting.
"""
import argparse
import functools
import hashlib
import http.server
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import threading
import zipfile

import f1b_oracle as oracle


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--cesium-dir', required=True, type=Path)
    parser.add_argument('--node-modules', type=Path)
    parser.add_argument('--chromium', default=os.environ.get('CHROMIUM', '/usr/bin/chromium'))
    parser.add_argument('--json-output', type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    runtime = args.cesium_dir.resolve()
    if not (runtime / 'Cesium.js').is_file():
        parser.error('pinned existing Cesium runtime missing')
    with tempfile.TemporaryDirectory(prefix='f1b-viewer-') as temporary:
        root = Path(temporary)
        source = root / 'source.glb'
        source.write_bytes(oracle.viewer_fixture())
        archive = root / 'candidate.3tz'
        result = subprocess.run([str(binary), '--json', 'mesh-local-to-3tz', '-i', str(source), '-o', str(archive), '--leaf-triangles', '2'], text=True, capture_output=True, timeout=60)
        oracle.require(result.returncode == 0, 'appearance conversion: ' + result.stdout + result.stderr)
        inspected = oracle.inspect(source, archive, 2)
        oracle.require(inspected['leaves'] == 2, 'appearance fixture must produce two full-detail leaves')
        with zipfile.ZipFile(archive) as z:
            members = {name: z.read(name) for name in z.namelist() if name != '@3dtilesIndex1@'}
        for prefix in ('mesh', 'wrong-uv', 'wrong-alpha'):
            for name, data in members.items():
                if name.endswith('.glb') and prefix != 'mesh':
                    def mutate(doc, binary):
                        if prefix == 'wrong-alpha':
                            for material in doc.get('materials', []):
                                material['alphaMode'] = 'OPAQUE'
                        else:
                            for primitive in doc['meshes'][0]['primitives']:
                                item = doc['accessors'][primitive['attributes']['TEXCOORD_0']]
                                view = doc['bufferViews'][item['bufferView']]
                                import struct
                                for i in range(item['count']):
                                    offset = view.get('byteOffset', 0) + item.get('byteOffset', 0) + i * view.get('byteStride', 8)
                                    u = struct.unpack_from('<f', binary, offset)[0]
                                    struct.pack_into('<f', binary, offset, 1 - u)
                    data = oracle.mutated_source(data, mutate)
                path = root / prefix / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
        html = b'''<!doctype html><html><head><script>window.CESIUM_BASE_URL='/cesium/';</script><script src='/cesium/Cesium.js'></script><link rel='stylesheet' href='/cesium/Widgets/widgets.css'><style>html,body,#viewer{width:100%;height:100%;margin:0}</style></head><body><div id='viewer'></div><script>window.viewer=new Cesium.Viewer('viewer',{contextOptions:{webgl:{preserveDrawingBuffer:true}},globe:false,baseLayer:false,skyBox:false,skyAtmosphere:false,animation:false,timeline:false,geocoder:false,homeButton:false,sceneModePicker:false,baseLayerPicker:false,navigationHelpButton:false,fullscreenButton:false});</script></body></html>'''
        (root / 'index.html').write_bytes(html)
        class Handler(http.server.SimpleHTTPRequestHandler):
            def translate_path(self, path):
                if path.startswith('/cesium/'):
                    translated = super().translate_path(path[7:])
                    return str(runtime / Path(translated).relative_to(root))
                return super().translate_path(path)
            def log_message(self, *unused):
                pass
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), functools.partial(Handler, directory=str(root)))
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            env = dict(os.environ, CHROMIUM=args.chromium)
            if args.node_modules:
                env['NODE_PATH'] = str(args.node_modules.resolve())
            command = ['node', str(Path(__file__).parent / 'fixtures/f1b_viewer.cjs'), f'http://127.0.0.1:{server.server_port}/']
            with subprocess.Popen(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, start_new_session=True) as child:
                try:
                    stdout, stderr = child.communicate(timeout=90)
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid, signal.SIGKILL)
                    child.communicate()
                    raise
                oracle.require(child.returncode == 0, 'appearance viewer: ' + stdout + stderr)
                browser = json.loads(stdout)
        finally:
            server.shutdown()
            server.server_close()
            thread.join()
        evidence = {'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'cesium_js_sha256': hashlib.sha256((runtime / 'Cesium.js').read_bytes()).hexdigest(), 'archive': inspected, 'browser': browser}
    encoded = json.dumps(evidence, indent=2) + '\n'
    if args.json_output:
        args.json_output.parent.mkdir(parents=True, exist_ok=True)
        args.json_output.write_text(encoded)
    else:
        print(encoded, end='')


if __name__ == '__main__':
    main()
