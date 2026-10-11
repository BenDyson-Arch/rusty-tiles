"""Deep-view Cesium A/B on real producer output: today's layout against the forest model.

For each case NAME=DIR:MAPPING (MAPPING is 'today' or 'forest' in VIEWS.json), serve DIR and run
probe.cjs over the same close top-down leaf views, then translate selected/picked content URIs
into global tile keys so both layouts can be compared tile-for-tile. One browser at a time.

Usage: taskset -c 0,1 nice -n 10 python3 -I run_real.py VIEWS.json OUT.json NAME=DIR:MAPPING ...
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


def serve(mounts):
    class Handler(http.server.SimpleHTTPRequestHandler):
        extensions_map = {**http.server.SimpleHTTPRequestHandler.extensions_map, '.wasm': 'application/wasm',
                          '.subtree': 'application/octet-stream', '.glb': 'model/gltf-binary'}

        def translate_path(self, path):
            path = path.split('?')[0]
            if path.startswith('/cesium/'):
                return str(CESIUM / path[len('/cesium/'):])
            if path.startswith('/fx/'):
                name, _, rest = path[len('/fx/'):].partition('/')
                return str(mounts[name] / rest)
            return '/nonexistent'

        def do_GET(self):
            if self.path in ('/', '/index.html'):
                self.send_response(200); self.send_header('Content-Type', 'text/html'); self.end_headers()
                self.wfile.write(PAGE); return
            super().do_GET()

        def log_message(self, *unused):
            pass
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def main():
    views_doc = json.loads(pathlib.Path(sys.argv[1]).read_text())
    out = pathlib.Path(sys.argv[2])
    cases = []
    for item in sys.argv[3:]:
        name, _, rest = item.partition('=')
        directory, _, mapping = rest.rpartition(':')
        cases.append((name, pathlib.Path(directory).resolve(), mapping))
    server = serve({name: d for name, d, _ in cases})
    env = dict(os.environ, NODE_PATH=str(NODE_MODULES), CHROMIUM=str(CHROMIUM))
    results = {}
    try:
        for name, directory, mapping in cases:
            uri_to_key = views_doc[mapping]
            key_to_uri = {}
            for u, k in uri_to_key.items():
                key_to_uri.setdefault(k, u)
            views = [{**v, 'leafUri': key_to_uri.get(v['key'])} for v in views_doc['views']]
            spec = {'url': f'/fx/{name}/tileset.json', 'height': 100.0, 'views': views, 'stepMs': 90000, 'maximumScreenSpaceError': float(os.environ.get('PIMPL_MSSE', '16')),
                    'deterministic': os.environ.get('PIMPL_DETERMINISTIC') == '1', 'stableFrames': int(os.environ.get('PIMPL_STABLE', '8')),
                    'cullWithChildrenBounds': os.environ.get('PIMPL_CHILD_BOUNDS', '1') == '1'}
            proc = subprocess.run(['node', str(HERE / 'probe.cjs'), f'http://127.0.0.1:{server.server_port}/', json.dumps(spec),
                                   str(out.parent / f'shot-real-{name}')], env=env, capture_output=True, text=True, timeout=1800)
            try:
                result = json.loads(proc.stdout.strip().splitlines()[-1])
            except Exception:
                results[name] = {'error': proc.stdout[-4000:] + proc.stderr[-4000:]}
                continue
            key = lambda u: uri_to_key.get(u, u)
            for v in result.get('views', []):
                v['selectedKeys'] = sorted(key(u) for u in v['selected'] if u)
                v['pickedKey'] = key(v['pickedUrl']) if v['pickedUrl'] else None
                v['newRequestCount'] = len(v.pop('newRequests'))
            result['overview']['selectedKeys'] = sorted(key(u) for u in result['overview']['selected'] if u)
            result['overview']['requestCount'] = len(result['overview'].pop('requests'))
            result['servedDirectory'] = str(directory)
            results[name] = result
            print(name, 'failures', result.get('failures'), 'errors', result.get('errors'), 'requests', result.get('requests'),
                  'overview', result['overview']['requestCount'], round(result['overview']['seconds'], 2), file=sys.stderr)
            for v in result.get('views', []):
                print(f"  {v['name']:<24} leafSelected={v['leafSelected']!s:<5} pickedLeaf={v['pickedLeaf']!s:<5} cov={v['coverage']:.2f} "
                      f"newReq={v['newRequestCount']:<3} sec={v['seconds']:.2f} sel={len(v['selectedKeys'])}", file=sys.stderr)
    finally:
        server.shutdown()
    receipt = {'probe': hashlib.sha256((HERE / 'probe.cjs').read_bytes()).hexdigest(),
               'runner': hashlib.sha256((HERE / 'run_real.py').read_bytes()).hexdigest(),
               'views': hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest(),
               'maximumScreenSpaceError': float(os.environ.get('PIMPL_MSSE', '16')),
               'deterministic': os.environ.get('PIMPL_DETERMINISTIC') == '1', 'stableFrames': int(os.environ.get('PIMPL_STABLE', '8')),
               'cullWithChildrenBounds': os.environ.get('PIMPL_CHILD_BOUNDS', '1') == '1',
               'cpuAffinity': sorted(os.sched_getaffinity(0)), 'nice': os.getpriority(os.PRIO_PROCESS, 0)}
    names = list(results)
    comparison = {}
    if len(names) >= 2:
        base = results[names[0]]
        for other in names[1:]:
            r = results[other]
            if 'views' not in r or 'views' not in base:
                continue
            comparison[f'{names[0]}~{other}'] = {
                'overviewSameSelection': base['overview']['selectedKeys'] == r['overview']['selectedKeys'],
                'viewsSameSelection': sum(a['selectedKeys'] == b['selectedKeys'] for a, b in zip(base['views'], r['views'])),
                'viewsSamePick': sum(a['pickedKey'] == b['pickedKey'] for a, b in zip(base['views'], r['views'])),
                'views': len(r['views'])}
    out.write_text(json.dumps({'receipt': receipt, 'comparison': comparison, 'results': results}, indent=1))
    print(json.dumps(comparison), file=sys.stderr)


if __name__ == '__main__':
    main()
