"""Serve the P-IMPL fixtures and run probe.cjs once per variant, one browser at a time.

Usage: taskset -c 0,1 nice -n 10 python3 -I run.py FIXTURE_DIR OUT.json [VARIANT ...]
"""
import hashlib, http.server, json, os, pathlib, subprocess, sys, threading

CACHE = pathlib.Path.home() / '.cache'
CESIUM = CACHE / 'rusty-tiles-117-browser-cache/node_modules/cesium/Build/Cesium'
NODE_MODULES = CACHE / 'rusty-tiles-117-browser-cache/node_modules'
CHROMIUM = CACHE / 'rusty-tiles-137-playwright/chromium-1243/chrome-linux64/chrome'
HERE = pathlib.Path(__file__).resolve().parent
PAGE = b"""<!doctype html><html><head><meta charset="utf-8"><script>window.CESIUM_BASE_URL='/cesium/';</script>
<script src="/cesium/Cesium.js"></script><link rel="stylesheet" href="/cesium/Widgets/widgets.css">
<style>html,body,#view{width:100%;height:100%;margin:0;overflow:hidden}</style></head>
<body><div id="view"></div></body></html>"""


def serve(fixtures, log, other):
    class Handler(http.server.SimpleHTTPRequestHandler):
        extensions_map = {**http.server.SimpleHTTPRequestHandler.extensions_map, '.wasm': 'application/wasm',
                          '.subtree': 'application/octet-stream', '.glb': 'model/gltf-binary'}

        def translate_path(self, path):
            path = path.split('?')[0]
            if path.startswith('/cesium/'):
                return str(CESIUM / path[len('/cesium/'):])
            if path.startswith('/fx/'):
                return str(fixtures / path[len('/fx/'):])
            return '/nonexistent'

        def do_GET(self):
            if self.path in ('/', '/index.html'):
                self.send_response(200); self.send_header('Content-Type', 'text/html'); self.end_headers()
                self.wfile.write(PAGE); return
            if self.path.startswith('/fx/'):
                log.append(self.path)
            elif not self.path.startswith('/cesium/'):
                other.append(self.path)
            super().do_GET()

        def log_message(self, *unused):
            pass
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def tree_digest(directory):
    h = hashlib.sha256()
    for p in sorted(directory.rglob('*')):
        if p.is_file():
            h.update(str(p.relative_to(directory)).encode() + b'\0' + hashlib.sha256(p.read_bytes()).digest())
    return h.hexdigest()


def main():
    fixtures = pathlib.Path(sys.argv[1]).resolve()
    out = pathlib.Path(sys.argv[2])
    variants = sys.argv[3:] or ['E', 'A', 'S', 'X', 'F', 'D', 'N']
    spec_views = json.loads((fixtures / 'views.json').read_text())
    log, other = [], []
    server = serve(fixtures, log, other)
    env = dict(os.environ, NODE_PATH=str(NODE_MODULES), CHROMIUM=str(CHROMIUM))
    results = {}
    try:
        for v in variants:
            views = []
            for view in spec_views['views']:
                x = view['leaf'][1]
                uri = spec_views['leafUris'][v].format(x=x, p=x // 2, s=x % 2)
                views.append({**view, 'leafUri': uri})
            spec = {'url': f'/fx/{v}/tileset.json', 'height': spec_views['height'], 'views': views, 'stepMs': 60000}
            start = len(log)
            proc = subprocess.run(['node', str(HERE / 'probe.cjs'), f'http://127.0.0.1:{server.server_port}/', json.dumps(spec),
                                   str(out.parent / f'shot-{v}')], env=env, capture_output=True, text=True, timeout=900)
            try:
                result = json.loads(proc.stdout.strip().splitlines()[-1])
            except Exception:
                result = {'error': proc.stdout[-4000:] + proc.stderr[-4000:]}
            result['serverRequests'] = len(log) - start
            result['nonFixtureNonCesiumPaths'] = sorted(set(other))
            result['fixtureTreeSha256'] = tree_digest(fixtures / v)
            results[v] = result
            ok = [w for w in result.get('views', [])]
            print(v, 'failures', result.get('failures'), 'errors', result.get('errors'), file=sys.stderr)
            for w in ok:
                print(f"  {w['name']:<12} leafSelected={w['leafSelected']!s:<5} pickedLeaf={w['pickedLeaf']!s:<5} "
                      f"coverage={w['coverage']:.2f} leafColor={w['leafColorFraction']:.2f} picked={w['pickedUrl']} "
                      f"selected={w['selected']}", file=sys.stderr)
    finally:
        server.shutdown()
    receipt = {'cesium': str(CESIUM), 'chromium': str(CHROMIUM), 'probe': hashlib.sha256((HERE / 'probe.cjs').read_bytes()).hexdigest(),
               'runner': hashlib.sha256((HERE / 'run.py').read_bytes()).hexdigest(),
               'generator': hashlib.sha256((HERE / 'make_fixtures.py').read_bytes()).hexdigest(),
               'cpuAffinity': sorted(os.sched_getaffinity(0)), 'nice': os.getpriority(os.PRIO_PROCESS, 0)}
    out.write_text(json.dumps({'receipt': receipt, 'results': results}, indent=1))


if __name__ == '__main__':
    main()
