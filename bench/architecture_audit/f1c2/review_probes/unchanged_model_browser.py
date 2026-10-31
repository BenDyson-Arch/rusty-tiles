#!/usr/bin/env python3
"""Nonauthor unchanged W1 browser proof using independent alias/texture fixtures."""
import argparse
import copy
from decimal import Decimal, localcontext
import functools
import hashlib
import http.server
import json
import math
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import threading
import zipfile
import zlib


def digest(data):
    return hashlib.sha256(data).hexdigest()


def image():
    def chunk(kind, value):
        return struct.pack('>I', len(value))+kind+value+struct.pack('>I', zlib.crc32(kind+value))
    return (b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR', struct.pack('>IIBBBBB', 1, 1, 8, 6, 0, 0, 0))
            +chunk(b'IDAT', zlib.compress(bytes([0, 220, 80, 40, 255])))+chunk(b'IEND', b''))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--node-modules', required=True, type=Path)
    parser.add_argument('--chromium', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--source-artifacts', type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    browser_assets = args.node_modules.resolve(strict=True)/'cesium/Build/Cesium'
    source = {'asset': {'version': '2.0'}, 'buffers': [{'uri': 'geometry%20%CE%B2.bin', 'byteLength': 60},
                {'uri': 'alias%25.bin', 'byteLength': 60}],
        'bufferViews': [{'buffer': buffer, 'byteOffset': offset, 'byteLength': length}
                        for buffer in range(2) for offset, length in [(0, 36), (36, 24)]],
        'accessors': [accessor for buffer in range(2) for accessor in [
            {'bufferView': buffer*2, 'componentType': 5126, 'count': 3, 'type': 'VEC3',
             'min': [-1, 0, -1], 'max': [1, 0, 1]},
            {'bufferView': buffer*2+1, 'componentType': 5126, 'count': 3, 'type': 'VEC2'}]],
        'images': [{'uri': 'texture%20%25%20%CE%B2.png'}],
        'textures': [{'source': 0}], 'materials': [{'doubleSided': True, 'pbrMetallicRoughness':
            {'baseColorTexture': {'index': 0}, 'metallicFactor': 0, 'roughnessFactor': 1}}],
        'meshes': [{'primitives': [{'attributes': {'POSITION': index*2, 'TEXCOORD_0': index*2+1},
                                   'material': 0}]} for index in range(2)],
        'nodes': [{'mesh': 0, 'translation': [.1, 0, 0]}, {'mesh': 1, 'translation': [3.1, 0, 0]}],
        'scenes': [{'nodes': [1, 0]}], 'scene': 0}
    source_bytes = json.dumps(source, indent=2).encode()
    payload = struct.pack('<9f', -1, 0, -1, 1, 0, -1, 0, 0, 1)+struct.pack('<6f', 0, 0, 1, 0, .5, 1)
    texture = image()
    binary_hash = digest(binary.read_bytes())
    source_identity = None
    if args.source_artifacts:
        source_receipt_bytes = args.source_artifacts.read_bytes()
        source_receipt = json.loads(source_receipt_bytes)
        source_root = Path(__file__).resolve().parents[4]
        assert source_receipt['artifacts']['portable']['sha256'] == binary_hash
        assert all(digest((source_root/name).read_bytes()) == expected
                   for name, expected in source_receipt['production_inputs'].items())
        assert digest(json.dumps(source_receipt['production_inputs'], sort_keys=True, separators=(',', ':')).encode()) == source_receipt['production_sha256']
        source_identity = {'receipt_sha256': digest(source_receipt_bytes),
            'production_sha256': source_receipt['production_sha256'],
            'input_count': len(source_receipt['production_inputs'])}
    conversions = []
    with tempfile.TemporaryDirectory(prefix='w1-browser-nonauthor-') as temporary:
        root = Path(temporary)
        input_path = root/'source.gltf'
        input_path.write_bytes(source_bytes)
        (root/'geometry β.bin').write_bytes(payload)
        os.link(root/'geometry β.bin', root/'alias%.bin')
        (root/'texture % β.png').write_bytes(texture)
        plans = []
        for placement in ('local', 'wgs84'):
            archive_path = root/(placement+'.3tz')
            arguments = [str(binary), '--json', 'glb-to-3tz', '-i', str(input_path), '-o', str(archive_path)]
            if placement == 'wgs84':
                arguments += ['--anchor', '0', '0', '10']
            called = subprocess.run(arguments, capture_output=True, text=True, timeout=90)
            assert called.returncode == 0, (called.stdout, called.stderr)
            with zipfile.ZipFile(archive_path) as archive:
                members = {name: archive.read(name) for name in archive.namelist()}
            assert members['model/source.gltf'] == source_bytes
            assert members['model/geometry β.bin'] == members['model/alias%.bin'] == payload
            assert members['model/texture % β.png'] == texture
            manifest = json.loads(members['tileset.json'])
            assert manifest['root']['geometricError'] == 0
            assert manifest['root']['refine'] == 'REPLACE'
            with localcontext() as context:
                context.prec = 100
                box = manifest['root']['boundingVolume']['box']
                axes = [[Decimal.from_float(float(box[start+offset])) for offset in range(3)] for start in (3, 6, 9)]
                # Maximal box-corner separation: the largest of the eight signed axis sums.
                squared = [sum(sum(sign[axis]*axes[axis][component] for axis in range(3))**2 for component in range(3))
                           for sign in ((a,b,c) for a in (-1,1) for b in (-1,1) for c in (-1,1))]
                omission_minimum = max(Decimal(1), 2*max(squared).sqrt())
                emitted_error = Decimal.from_float(float(manifest['geometricError']))
                assert emitted_error >= omission_minimum, (emitted_error, omission_minimum)
                assert emitted_error <= omission_minimum*(1+Decimal('1e-12')), (emitted_error, omission_minimum)
            variant_members = {}
            for variant in ('good', 'wrong-root', 'missing-alias', 'zero-omission-error'):
                changed = dict(members)
                if variant == 'wrong-root':
                    wrong = copy.deepcopy(manifest)
                    wrong['root']['transform'][13 if placement == 'wgs84' else 12] += 40
                    changed['tileset.json'] = json.dumps(wrong).encode()
                elif variant == 'missing-alias':
                    del changed['model/alias%.bin']
                elif variant == 'zero-omission-error':
                    zero = copy.deepcopy(manifest)
                    zero['geometricError'] = 0
                    changed['tileset.json'] = json.dumps(zero).encode()
                variant_members[variant] = {name: digest(data) for name, data in changed.items()}
                for name, data in changed.items():
                    target = root/placement/variant/name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes(data)
            length = math.hypot(12, 20)
            if placement == 'local':
                camera = {'position': [1.6, -12, 20], 'direction': [0, 12/length, -20/length],
                          'up': [0, 20/length, 12/length]}
                targets = [[.1, 1/3, 0], [3.1, 1/3, 0]]
            else:
                camera = {'position': [6378147+20, 1.6, -12], 'direction': [-20/length, 0, 12/length],
                          'up': [12/length, 0, 20/length]}
                targets = [[6378147, .1, 1/3], [6378147, 3.1, 1/3]]
            plans.append({'placement': placement, 'camera': camera, 'targets': targets})
            conversions.append({'placement': placement, 'archive_sha256': digest(archive_path.read_bytes()),
                                'members': {name: digest(data) for name, data in members.items()},
                                'variant_members': variant_members, 'omission_error_metres': manifest['geometricError'],
                                'decimal100_minimum_omission_error_metres': str(omission_minimum)})
        (root/'plan.json').write_text(json.dumps(plans))
        (root/'index.html').write_text("""<!doctype html><html><head><script>window.CESIUM_BASE_URL='/cesium/';</script>
<script src='/cesium/Cesium.js'></script><link rel='stylesheet' href='/cesium/Widgets/widgets.css'>
<style>html,body,#viewer{width:100%;height:100%;margin:0}</style></head><body><div id='viewer'></div><script>
window.viewer=new Cesium.Viewer('viewer',{globe:false,baseLayer:false,skyBox:false,skyAtmosphere:false,animation:false,timeline:false,geocoder:false,homeButton:false,sceneModePicker:false,baseLayerPicker:false,navigationHelpButton:false,fullscreenButton:false});
</script></body></html>""")

        class Handler(http.server.SimpleHTTPRequestHandler):
            def translate_path(self, path):
                if path.startswith('/cesium/'):
                    return str(browser_assets/Path(super().translate_path(path[7:])).relative_to(root))
                return super().translate_path(path)

            def log_message(self, *unused):
                pass

        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), functools.partial(Handler, directory=str(root)))
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            driver = Path(__file__).with_name('unchanged_model_browser.cjs')
            run = subprocess.run(['node', str(driver), f'http://127.0.0.1:{server.server_port}/'],
                env={**os.environ, 'NODE_PATH': str(args.node_modules.resolve()), 'CHROMIUM': str(args.chromium.resolve())},
                capture_output=True, text=True, timeout=160)
            if run.returncode != 0:
                failure = {'accepted': False, 'binary_sha256': binary_hash, 'source_sha256': digest(source_bytes),
                    'conversions': conversions, 'browser': json.loads(run.stdout) if run.stdout else None,
                    'error': run.stderr, 'probes': {p.name: digest(p.read_bytes()) for p in (Path(__file__), driver)},
                    'cesium_js_sha256': digest((browser_assets/'Cesium.js').read_bytes()),
                    'limits': 'failed ordinary consumer execution; not candidate acceptance; deadlines unchanged'}
                args.output.with_name(args.output.stem+'-failed-'+binary_hash[:12]+'-'+digest(driver.read_bytes())[:12]+'.json').write_text(json.dumps(failure, indent=2)+'\n')
            assert run.returncode == 0, run.stderr
            browser = json.loads(run.stdout)
        finally:
            server.shutdown()
            server.server_close()
            thread.join()
    assert digest(binary.read_bytes()) == binary_hash
    if args.source_artifacts:
        assert args.source_artifacts.read_bytes() == source_receipt_bytes
        assert all(digest((source_root/name).read_bytes()) == expected
                   for name, expected in source_receipt['production_inputs'].items())
    result = {'source_identity': source_identity, 'binary_sha256': binary_hash, 'source_sha256': digest(source_bytes), 'conversions': conversions,
        'browser': browser, 'probes': {p.name: digest(p.read_bytes()) for p in (Path(__file__), driver)},
        'cesium_js_sha256': digest((browser_assets/'Cesium.js').read_bytes()),
        'chromium_sha256': digest(args.chromium.read_bytes()),
        'consumer_packages': {name: {'package_json_sha256': digest((args.node_modules/name/'package.json').read_bytes()),
            'version': json.loads((args.node_modules/name/'package.json').read_text())['version']} for name in ('cesium', 'playwright')},
        'limits': ['finite authored static core-PBR two-triangle model; direct unchanged resources; no GPU bounds precision or new feature metadata claim']}
    args.output.write_text(json.dumps(result, indent=2)+'\n')


if __name__ == '__main__':
    main()
