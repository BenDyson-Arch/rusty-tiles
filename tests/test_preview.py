"""Exercise the installed native HTTP server, with no interpreter on PATH."""
import concurrent.futures
import contextlib
import http.client
import json
import os
import pathlib
import select
import subprocess
import tempfile
import unittest
from urllib.parse import urlsplit

BIN = os.environ.get('RUSTY_TILES_BIN')
MANIFESTS = {'point-cloud':'tileset.json', 'mesh':'tileset.json', 'annotations':'tileset.json',
             'imagery':'tilejson.json', 'terrain':'layer.json'}

@contextlib.contextmanager
def server(cesium, layers):
    args = [BIN, 'preview', '--json', '--port', '0', '--cesium', str(cesium)]
    for name, path in layers.items():
        args += [f'--{name}', str(path)]
    process = subprocess.Popen(args, env=dict(os.environ, PATH=''), stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, text=True)
    try:
        if not select.select([process.stdout], [], [], 10)[0]:
            raise AssertionError('preview startup timed out')
        ready = json.loads(process.stdout.readline())
        if not ready.get('ok'):
            raise AssertionError(ready)
        yield urlsplit(ready['url']), ready
    finally:
        process.terminate()
        try:
            process.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.communicate()

def request(address, path, method='GET'):
    connection = http.client.HTTPConnection(address.hostname, address.port, timeout=10)
    try:
        connection.request(method, path)
        response = connection.getresponse()
        return response.status, dict(response.getheaders()), response.read()
    finally:
        connection.close()

@unittest.skipUnless(BIN, 'set RUSTY_TILES_BIN for native preview acceptance')
class PreviewTests(unittest.TestCase):
    def fixture(self, root):
        cesium = root/'runtime'; cesium.mkdir()
        (cesium/'Cesium.js').write_text('// invented runtime')
        (cesium/'worker.wasm').write_bytes(b'\x00asm')
        layers = {}
        for name, manifest in MANIFESTS.items():
            path = root/name; path.mkdir(); layers[name] = path
            (path/manifest).write_text('{"fixture":true}')
        return cesium, layers

    def test_all_five_routes_embedded_page_mime_and_no_cache(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp); cesium, layers = self.fixture(root)
            (layers['mesh']/'tile.glb').write_bytes(b'glTF')
            (layers['terrain']/'0.terrain').write_bytes(b'terrain')
            with server(cesium, layers) as (address, ready):
                self.assertEqual(address.hostname, '127.0.0.1')
                status, headers, body = request(address, '/config.json?cache=fixture')
                self.assertEqual(status, 200)
                config = json.loads(body)
                self.assertEqual(config, {name:f'/{name}/{manifest}' for name, manifest in MANIFESTS.items()})
                self.assertEqual(config, ready['layers'])
                for path, content_type in [('/', 'text/html; charset=utf-8'), ('/index.html','text/html; charset=utf-8'),
                        ('/config.json','application/json'), ('/cesium/Cesium.js','text/javascript'),
                        ('/cesium/worker.wasm','application/wasm'), ('/mesh/tile.glb','model/gltf-binary'),
                        ('/terrain/0.terrain','application/vnd.quantized-mesh')]:
                    status, headers, body = request(address, path)
                    self.assertEqual(status, 200, path)
                    self.assertEqual(headers['Cache-Control'], 'no-cache')
                    self.assertEqual(headers['Content-Type'], content_type)
                    head_status, head_headers, head_body = request(address, path, 'HEAD')
                    self.assertEqual(head_status, 200)
                    self.assertEqual(head_body, b'')
                    self.assertEqual(int(head_headers['Content-Length']),len(body))
                self.assertIn(b'/cesium/Cesium.js',request(address,'/')[2])
                for path in config.values():
                    self.assertEqual(json.loads(request(address,path)[2]), {'fixture':True})
                self.assertEqual(request(address,'/config.json','POST')[0],405)
                with concurrent.futures.ThreadPoolExecutor(max_workers=16) as pool:
                    results = list(pool.map(lambda _:request(address,'/mesh/tile.glb'),range(64)))
                self.assertTrue(all(r[0]==200 and r[2]==b'glTF' for r in results))

    def test_only_selected_roots_no_listing_traversal_or_symlink_escape(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp); cesium, layers = self.fixture(root)
            secret = root/'private.txt';secret.write_text('not selected')
            (layers['mesh']/'escape.txt').symlink_to(secret)
            (layers['mesh']/'external').symlink_to(root, target_is_directory=True)
            (layers['mesh']/'valid file.bin').write_bytes(b'selected')
            with server(cesium, {'mesh':layers['mesh']}) as (address, _):
                for path in ['/mesh/', '/cesium/', '/terrain/layer.json', '/data/private.txt', '/private.txt',
                        '/mesh/../private.txt', '/mesh/%2e%2e/private.txt', '/mesh/%2e%2e%2fprivate.txt',
                        '/mesh/%2fetc/passwd','/mesh/%5c..%5cprivate.txt','/mesh/escape.txt',
                        '/mesh/external/private.txt','/mesh/%00private.txt']:
                    status, headers, body = request(address,path)
                    self.assertEqual(status,404,path)
                    self.assertEqual(headers['Cache-Control'],'no-cache')
                    self.assertNotIn(b'not selected',body)
                self.assertEqual(request(address,'/mesh/valid%20file.bin?x=1')[2],b'selected')

    def test_invalid_selections_and_no_implicit_data(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp); cesium, layers = self.fixture(root)
            empty = root/'empty';empty.mkdir()
            for extra in [[], ['--mesh',str(empty)], ['--terrain',str(root/'missing')]]:
                result = subprocess.run([BIN,'preview','--json','--cesium',str(cesium),*extra],capture_output=True,timeout=5)
                self.assertEqual(result.returncode,3,result.stderr)
                self.assertEqual(json.loads(result.stdout)['error']['code'],'data')
            (empty/'tileset.json').symlink_to(layers['mesh']/'tileset.json')
            result = subprocess.run([BIN,'preview','--json','--cesium',str(cesium),'--mesh',str(empty)],capture_output=True,timeout=5)
            self.assertEqual(result.returncode,3,result.stderr)

if __name__ == '__main__':
    unittest.main()
