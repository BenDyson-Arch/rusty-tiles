#!/usr/bin/env python3
"""Real Cesium primitive/triangle picking and sparse shared-table sensitivity.

--synthetic establishes consumer feasibility before candidate implementation.
--binary validates its original archive independently before browser picking.
Uses already provisioned browser assets only; no downloads or Cargo operation.
"""
import argparse
import copy
import functools
import http.server
import json
import math
import os
from pathlib import Path
import signal
import struct
import subprocess
import tempfile
import threading
import zipfile

import f1c2_oracle as oracle


def controls(members):
    changed = {}
    for label, which in (('primitive-row-swap', 0), ('triangle-row-swap', 1)):
        output = dict(members)
        for name, data in members.items():
            if not name.endswith('.glb'):
                continue
            doc, payload, _ = oracle.decode(data)
            raw = bytearray(payload[:doc['buffers'][0]['byteLength']])
            count = doc['extensions']['EXT_structural_metadata']['propertyTables'][which]['count']
            for p in doc['meshes'][0]['primitives']:
                a = doc['accessors'][p['attributes']['_FEATURE_ID_'+str(which)]]
                v = doc['bufferViews'][a['bufferView']]
                stride = v.get('byteStride', 4)
                start = v.get('byteOffset', 0)+a.get('byteOffset', 0)
                for i in range(a['count']):
                    original = struct.unpack_from('<f', raw, start+i*stride)[0]
                    # Rotate an entire primitive's row stream consistently;
                    # all three corners still share an in-range integer ID.
                    struct.pack_into('<f', raw, start+i*stride, (int(original)+1) % count)
            output[name] = oracle.glb(doc, raw)
        changed[label] = output
    return changed


def plan(source):
    truth, names = oracle.expected_source(source)
    targets = []
    for key, record in sorted(truth.items()):
        centroid = [sum(p[c] for p in record['positions'])/3 for c in range(3)]
        primitive = dict(zip(oracle.KEYS[:3], key[:3]))
        primitive.update(node_name_present=names[key[0]][0], node_name=names[key[0]][1])
        targets.append({'world': [centroid[0], -centroid[2], centroid[1]],
                        'source_primitive': primitive, 'source_triangle': dict(zip(oracle.KEYS, key))})
    center = sum(t['world'][0] for t in targets)/len(targets)
    length = math.hypot(18, 32)
    return {'targets': targets, 'camera': {'destination': [center, -18, 32],
            'direction': [0, 18/length, -32/length], 'up': [0, 32/length, 18/length]}, 'triangleCount': len(truth)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--synthetic', action='store_true')
    mode.add_argument('--binary', type=Path)
    parser.add_argument('--cesium-dir', required=True, type=Path)
    parser.add_argument('--node-modules', required=True, type=Path)
    parser.add_argument('--chromium', required=True, type=Path)
    parser.add_argument('--json-output', type=Path)
    parser.add_argument('--empty-names', action='store_true')
    args = parser.parse_args()
    runtime = args.cesium_dir.resolve(strict=True)
    binary = args.binary.resolve(strict=True) if args.binary else None
    source = oracle.fixture('empty-names' if args.empty_names else 'viewer')
    candidate = None
    with tempfile.TemporaryDirectory(prefix='f1c2-picking-') as temporary:
        root = Path(temporary)
        (root/'source.glb').write_bytes(source)
        if binary:
            archive = root/'candidate.3tz'
            completed = subprocess.run([str(binary), '--json', 'mesh-local-to-3tz', '-i', str(root/'source.glb'),
                '-o', str(archive), '--leaf-triangles', '100'], text=True, capture_output=True, timeout=60)
            oracle.require(completed.returncode == 0, 'viewer conversion '+completed.stdout+completed.stderr)
            with zipfile.ZipFile(archive) as stream:
                members = {n: stream.read(n) for n in stream.namelist() if n != '@3dtilesIndex1@'}
            candidate = oracle.inspect_members(source, members, 100)
            candidate['archive_sha256'] = oracle.digest(archive.read_bytes())
        else:
            members = oracle.synthetic_members(source)
            oracle.inspect_members(source, members, 100)
        streams = []
        for name, data in members.items():
            if not name.endswith('.glb'):
                continue
            doc, payload, _ = oracle.decode(data)
            for p in doc['meshes'][0]['primitives']:
                for t, feature in enumerate(p['extensions']['EXT_mesh_features']['featureIds']):
                    ids = oracle.accessor(doc, payload, p['attributes']['_FEATURE_ID_'+str(t)])
                    streams.append({'label': oracle.LABELS[t], 'featureCount': feature['featureCount'],
                        'tableCount': doc['extensions']['EXT_structural_metadata']['propertyTables'][t]['count'],
                        'usedRows': sorted(set(ids)), 'hasRowAboveFeatureCount': max(ids) >= feature['featureCount']})
        oracle.require(any(s['hasRowAboveFeatureCount'] for s in streams), 'shared-table sparse high-row control present')
        for label, output in dict(good=members, **controls(members)).items():
            for member, payload in output.items():
                target = root/label/member
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(payload)
        (root/'plan.json').write_text(json.dumps(plan(source)))
        (root/'index.html').write_text("""<!doctype html><html><head><script>window.CESIUM_BASE_URL='/cesium/';</script>
<script src='/cesium/Cesium.js'></script><link rel='stylesheet' href='/cesium/Widgets/widgets.css'>
<style>html,body,#viewer{width:100%;height:100%;margin:0}</style></head><body><div id='viewer'></div><script>
window.viewer=new Cesium.Viewer('viewer',{globe:false,baseLayer:false,skyBox:false,skyAtmosphere:false,animation:false,timeline:false,geocoder:false,homeButton:false,sceneModePicker:false,baseLayerPicker:false,navigationHelpButton:false,fullscreenButton:false});
</script></body></html>""")

        class Handler(http.server.SimpleHTTPRequestHandler):
            def translate_path(self, path):
                if path.startswith('/cesium/'):
                    translated = super().translate_path(path[7:])
                    return str(runtime/Path(translated).relative_to(root))
                return super().translate_path(path)

            def log_message(self, *unused):
                pass

        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), functools.partial(Handler, directory=str(root)))
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            env = dict(os.environ, NODE_PATH=str(args.node_modules.resolve(strict=True)), CHROMIUM=str(args.chromium.resolve(strict=True)))
            with subprocess.Popen(['node', str(Path(__file__).parent/'fixtures/f1c2_viewer.cjs'),
                    f'http://127.0.0.1:{server.server_port}/'], text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                    env=env, start_new_session=True) as child:
                try:
                    stdout, stderr = child.communicate(timeout=120)
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid, signal.SIGKILL)
                    child.communicate()
                    raise
                oracle.require(child.returncode == 0, 'picking consumer '+stdout+stderr)
                browser = json.loads(stdout)
        finally:
            server.shutdown()
            server.server_close()
            thread.join()
    evidence = {'mode': 'candidate-archive' if binary else 'independent-synthetic-consumer-feasibility',
                'binary_sha256': oracle.digest(binary.read_bytes()) if binary else None,
                'source_sha256': oracle.digest(source), 'candidate': candidate, 'streams': streams, 'browser': browser,
                'member_sha256': {name: oracle.digest(payload) for name, payload in members.items()},
                'driver_sha256': {p.name: oracle.digest(p.read_bytes()) for p in (Path(__file__), Path(__file__).parent/'f1c2_oracle.py',
                    Path(__file__).parent/'fixtures/f1c2_viewer.cjs')},
                'cesium_js_sha256': oracle.digest((runtime/'Cesium.js').read_bytes()),
                'scope': 'Actual primitive/default and triangle-label picks, sparse shared-table rows above featureCount-1, optional exact labels and sensitive ID swaps. Finite local cameras; coincident-surface selection is not uniquely observable.'}
    encoded = json.dumps(evidence, indent=2, allow_nan=False)+'\n'
    if args.json_output:
        args.json_output.parent.mkdir(parents=True, exist_ok=True)
        args.json_output.write_text(encoded)
    else:
        print(encoded, end='')


if __name__ == '__main__':
    main()
