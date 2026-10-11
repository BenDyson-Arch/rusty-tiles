"""Load rusty-tiles outputs in pinned headless Cesium and gate on runtime failures.

Usage: python3 bench/perf_lane/cesium.py --work DIR --out JSON NAME=PATH...
PATH is a .3tz archive (unpacked under --work, as the archive viewers do), a
3D Tiles directory with tileset.json, or a raster directory with tilejson.json.
Each case gets a fresh Chromium at 1024x768 with software rendering; the local
server binds 127.0.0.1 and every other request or WebSocket is refused and counted
as a failure. --timeout bounds each asynchronous browser step; the case budget is
that times the step count plus a fixed browser start allowance.
"""
import argparse, fractions, hashlib, http.server, json, math, os, pathlib, shutil, signal, subprocess, sys
import threading, urllib.parse, zipfile

HERE = pathlib.Path(__file__).resolve().parent
# Defaults: environment overrides, else the local caches used for the recorded runs.
CACHE = pathlib.Path.home() / '.cache'
DEFAULTS = dict(cesium=os.environ.get('CESIUM_DIR', CACHE / 'rusty-tiles-117-browser-cache/node_modules/cesium/Build/Cesium'),
                node_modules=os.environ.get('NODE_PATH', CACHE / 'rusty-tiles-117-browser-cache/node_modules'),
                chromium=os.environ.get('CHROMIUM', CACHE / 'rusty-tiles-137-playwright/chromium-1243/chrome-linux64/chrome'))
STARTUP_SECONDS = 60  # browser launch, before the first bounded step
PAGE = b"""<!doctype html><html><head><meta charset="utf-8"><script>window.CESIUM_BASE_URL='/cesium/';</script>
<script src="/cesium/Cesium.js"></script><link rel="stylesheet" href="/cesium/Widgets/widgets.css">
<style>html,body,#view{width:100%;height:100%;margin:0;overflow:hidden}</style></head>
<body><div id="view"></div></body></html>"""
CATEGORIES = {'.json': 'manifest', '.subtree': 'subtree', '.glb': 'content', '.b3dm': 'content', '.pnts': 'content',
              '.i3dm': 'content', '.cmpt': 'content', '.png': 'imagery', '.jpg': 'imagery', '.webp': 'imagery'}
# Gate: absolute failures always; with --baseline, >25% growth of these measures.
RATIO, FLOORS = 1.25, {'requests': 2, 'requestBytes': 64 * 1024, 'secondsToLoaded': 1.0}


def tree(path):
    files = sorted(p for p in path.rglob('*') if p.is_file())
    digest = hashlib.sha256()
    for p in files:
        digest.update(str(p.relative_to(path)).encode() + b'\0' + hashlib.sha256(p.read_bytes()).digest())
    return dict(files=len(files), bytes=sum(p.stat().st_size for p in files), treeSha256=digest.hexdigest())


def mount(name, path, work):
    """Return (served directory, kind, identity) for an archive or output directory."""
    if path.is_file():
        target = work / 'unpacked' / name
        shutil.rmtree(target, ignore_errors=True)
        with zipfile.ZipFile(path) as archive:
            for member in archive.infolist():
                if member.is_dir() or member.filename == '@3dtilesIndex1@': continue
                destination = (target / member.filename).resolve()
                if not destination.is_relative_to(target.resolve()): raise SystemExit(f'{path}: unsafe member')
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(archive.read(member))
        return target, 'tileset', dict(archive=str(path), bytes=path.stat().st_size,
                                       sha256=hashlib.sha256(path.read_bytes()).hexdigest())
    kind = 'raster' if (path / 'tilejson.json').exists() else 'tileset'
    identity = dict(directory=str(path), **tree(path))
    if kind == 'raster': identity['pngColorTypes'] = color_types((path / 'tiles').rglob('*.png'))
    return path, kind, identity


def color_types(paths):
    """Count PNG IHDR colour types: 0 gray, 2 RGB, 3 palette, 4 gray+alpha, 6 RGBA."""
    types = {}
    for png in paths:
        with open(png, 'rb') as stream: name = {0: 'gray', 2: 'RGB', 3: 'palette', 4: 'grayAlpha', 6: 'RGBA'}[stream.read(26)[25]]
        types[name] = types.get(name, 0) + 1
    return types


def serve(mounts, cesium, log):
    class Handler(http.server.SimpleHTTPRequestHandler):
        extensions_map = {**http.server.SimpleHTTPRequestHandler.extensions_map, '.wasm': 'application/wasm',
                          '.glb': 'model/gltf-binary', '.json': 'application/json'}

        def translate_path(self, path):
            parts = urllib.parse.unquote(urllib.parse.urlsplit(path).path).lstrip('/').split('/')
            if parts[0] == 'cesium': base, rest = cesium, parts[1:]
            elif parts[0] == 'data' and len(parts) > 1 and parts[1] in mounts: base, rest = mounts[parts[1]], parts[2:]
            else: return '/nonexistent'
            # Lexical containment: '..' cannot escape, while mounts may contain symlinked tile trees.
            target = os.path.normpath(base.joinpath(*rest))
            return target if pathlib.Path(target).is_relative_to(base) else '/nonexistent'

        def do_GET(self):
            if self.path == '/':
                self.send_response(200); self.send_header('Content-Type', 'text/html')
                self.send_header('Content-Length', str(len(PAGE))); self.end_headers(); self.wfile.write(PAGE); return
            self.sent, self.status = 0, None
            super().do_GET()
            log.append(dict(path=urllib.parse.urlsplit(self.path).path, status=self.status, bytes=self.sent))

        def send_response(self, code, message=None):
            self.status = code; super().send_response(code, message)

        def copyfile(self, source, destination):
            while block := source.read(1 << 16):
                destination.write(block); self.sent += len(block)

        def log_message(self, *unused): pass

    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def requests(entries, name, directory):
    prefix, summary = f'/data/{name}/', dict(runtimeRequests=0)
    paths = [e for e in entries if e['path'].startswith(prefix)]
    summary['runtimeRequests'] = sum(e['path'].startswith('/cesium/') for e in entries)
    for category in ('manifest', 'subtree', 'content', 'imagery', 'other'):
        chosen = [e for e in paths if CATEGORIES.get(pathlib.PurePath(e['path']).suffix.lower(), 'other') == category]
        summary[category] = dict(requests=len(chosen), bytes=sum(e['bytes'] for e in chosen))
    summary['requests'] = len(paths)
    summary['requestBytes'] = sum(e['bytes'] for e in paths)
    summary['duplicateRequests'] = len(paths) - len({e['path'] for e in paths})
    summary['serverErrors'] = [e for e in entries if (e['status'] or 0) >= 400]
    summary['imageryColorTypes'] = color_types(directory / e['path'][len(prefix):] for e in paths
                                               if e['path'].endswith('.png') and e['status'] == 200)
    return summary


def run_case(command, environment, budget):
    """Run one browser case; a budget overrun or unreadable output fails the case, not the run."""
    child = subprocess.Popen(command, env=environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                             start_new_session=True)
    try:
        stdout, stderr = child.communicate(timeout=budget)
    except subprocess.TimeoutExpired:
        os.killpg(child.pid, signal.SIGKILL)
        stdout, stderr = child.communicate()
        return dict(loaded=False, timedOut=True, driverError=stderr[-4000:])
    try:
        if child.returncode == 0: return json.loads(stdout)
    except json.JSONDecodeError:
        pass
    return dict(loaded=False, driverError=(stderr or stdout)[-4000:])


def problems(case, baseline):
    found = []
    if case.get('timedOut'): found.append('case budget exceeded')
    if not case.get('loaded'): found.append('load failed or timed out')
    found += [f"view {v['name']} bare-globe reference did not settle" for v in case.get('views', [])
              if not v.get('backgroundSettled', True)]
    for key in ('failures', 'failedRequests', 'externalRequests'):
        if case.get(key): found.append(f'{key}: {len(case[key])}')
    if case.get('console', {}).get('errors'): found.append(f"console errors: {len(case['console']['errors'])}")
    if case['requests']['serverErrors']: found.append('server errors')
    found += [f"probe {p['name']} expected {p['expect']}" for p in case.get('probes', []) if not p['pass']]
    if baseline:
        for metric, floor in FLOORS.items():
            pick = lambda c: c['requests'][metric] if metric in c['requests'] else c.get(metric)
            old, new = pick(baseline), pick(case)
            if old is not None and new is not None and new > old * RATIO and new - old > floor:
                found.append(f'{metric} {old:.6g} -> {new:.6g} (>{RATIO:g}x)')
    return found


def inset_degrees(text, maxzoom):
    """--inset: one or four (W,S,E,N) values, in degrees or as 'Npx' finest-zoom pixels (e.g. 1/1024px)."""
    pixel = 360 / 2 ** maxzoom / 256
    values = [float(fractions.Fraction(v[:-2])) * pixel if v.endswith('px') else float(v) for v in text.split(',')]
    return values * 4 if len(values) == 1 else values


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('cases', nargs='+', help='NAME=PATH')
    parser.add_argument('--work', type=pathlib.Path, required=True)
    parser.add_argument('--out', type=pathlib.Path, required=True)
    parser.add_argument('--hole', help='W,S,E,N degrees that must render transparent in raster cases')
    parser.add_argument('--baseline', type=pathlib.Path, help='earlier cesium.py JSON to compare by case name')
    parser.add_argument('--timeout', type=float, default=120, help='seconds per asynchronous browser step')
    parser.add_argument('--inset', help='shrink the raster provider rectangle: DEG, W,S,E,N or Npx (finest-zoom '
                        'pixels, e.g. 1/1024px). A workaround/probe that changes what is tested; reported as a warning')
    parser.add_argument('--cpus', default='0,1', help='CPU affinity (AGENTS.md: two CPUs)')
    parser.add_argument('--corner-views', action='store_true', help='add raster cameras on each bounds corner, '
                        'four finest-zoom tiles across, after the overview')
    for key, value in DEFAULTS.items(): parser.add_argument('--' + key.replace('_', '-'), type=pathlib.Path, default=value)
    args = parser.parse_args()
    os.sched_setaffinity(0, {int(c) for c in args.cpus.split(',')}); os.setpriority(os.PRIO_PROCESS, 0, 10)
    if args.inset: print(f'cesium: WARNING --inset {args.inset} changes the raster rectangle under test', file=sys.stderr)
    work = args.work.resolve(); shutil.rmtree(work / 'screenshots', ignore_errors=True)
    (work / 'screenshots').mkdir(parents=True)
    baseline = {c['name']: c for c in json.loads(args.baseline.read_text())['cases']} if args.baseline else {}
    mounts, plans = {}, []
    for item in args.cases:
        name, _, path = item.partition('=')
        directory, kind, identity = mount(name, pathlib.Path(path).resolve(), work)
        mounts[name] = directory; plans.append((name, kind, identity, directory))
    log = []
    server = serve(mounts, args.cesium.resolve(), log)
    environment = dict(os.environ, NODE_PATH=str(args.node_modules), CHROMIUM=str(args.chromium))
    results = []
    try:
        for name, kind, identity, directory in plans:
            spec = dict(kind=kind, url=f'/data/{name}/' + ('' if kind == 'raster' else 'tileset.json'),
                        stepMs=int(args.timeout * 1000))
            if kind == 'raster' and args.inset:
                spec['insetDegrees'] = inset_degrees(args.inset, json.loads((directory / 'tilejson.json').read_text())['maxzoom'])
            if kind == 'raster' and args.hole:
                hole = [float(v) for v in args.hole.split(',')]
                bounds = json.loads((directory / 'tilejson.json').read_text())['bounds']
                pad = [(hole[2] - hole[0]) * .1, (hole[3] - hole[1]) * .1]
                spec['probes'] = [dict(name='inside-hole', rect=hole, expect='background'),
                                  dict(name='outside-hole', rect=bounds, expect='content',
                                       exclude=[hole[0] - pad[0], hole[1] - pad[1], hole[2] + pad[0], hole[3] + pad[1]])]
                # Near maximum zoom across the hole's west edge: opaque tiles beside hole-boundary tiles.
                dx, middle = (hole[2] - hole[0]) * .1, (hole[1] + hole[3]) / 2
                dy = dx * .75 * math.cos(math.radians(middle))
                spec['views'] = [dict(name='hole-edge', rect=[hole[0] - dx, middle - dy, hole[0] + dx, middle + dy], probes=[
                    dict(name='west-of-edge', rect=[hole[0] - dx * .9, middle - dy * .5, hole[0] - dx * .15, middle + dy * .5],
                         expect='content'),
                    dict(name='east-of-edge', rect=[hole[0] + dx * .15, middle - dy * .5, hole[0] + dx * .9, middle + dy * .5],
                         expect='background')])]
            shot = work / 'screenshots' / f'{name}.png'
            start = len(log)
            if kind == 'raster' and args.corner_views:
                metadata = json.loads((directory / 'tilejson.json').read_text())
                (w, s, e, n), half = metadata['bounds'], 2 * 360 / 2 ** metadata['maxzoom']
                spec['views'] = spec.get('views', []) + [
                    dict(name=f'corner-{label}', rect=[x - half, y - half * math.cos(math.radians(y)) * .75,
                                                      x + half, y + half * math.cos(math.radians(y)) * .75])
                    for label, x, y in (('nw', w, n), ('ne', e, n), ('sw', w, s), ('se', e, s))]
            # Bounded steps: page load, Cesium global, screenshot, then fromUrl + settle, or the TileJSON fetch,
            # two settles per view and a final settle.
            steps = 3 + (2 + 2 * (1 + len(spec.get('views', []))) if kind == 'raster' else 2)
            budget = steps * args.timeout + STARTUP_SECONDS
            print(f'cesium: {name} ({kind}, budget {budget:g} s)', file=sys.stderr, flush=True)
            case = run_case(['node', str(HERE / 'cesium.cjs'), f'http://127.0.0.1:{server.server_port}/', json.dumps(spec), str(shot)],
                            environment, budget)
            case.update(name=name, kind=kind, output=identity, spec=spec, budgetSeconds=budget, screenshot=str(shot),
                        requests=requests(log[start:], name, directory))
            if spec.get('insetDegrees'): case['warnings'] = [f"raster rectangle inset {spec['insetDegrees']} degrees: not the exact bounds"]
            shots = [shot] + [shot.with_name(f"{name}-{view['name']}.png") for view in case.get('views', [])]
            case['captureSha256'] = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in shots if p.exists()}
            case['problems'] = problems(case, baseline.get(name))
            results.append(case)
            print(json.dumps(dict(name=name, loaded=case.get('loaded'), seconds=case.get('secondsToLoaded'),
                                  requests=case['requests']['requests'], bytes=case['requests']['requestBytes'],
                                  problems=case['problems'])), file=sys.stderr, flush=True)
    finally:
        server.shutdown(); server.server_close()
    report = dict(gate=dict(rule='fail on load failure, page/console errors, failed or external requests, failed render '
                                 'probes; with --baseline also on >25% more requests/bytes or load time beyond floors',
                            ratio=RATIO, floors=FLOORS),
                  environment=dict(cesium=json.loads((args.cesium.resolve().parents[1] / 'package.json').read_text())['version'],
                                   playwright=json.loads((args.node_modules / 'playwright/package.json').read_text())['version'],
                                   chromium=str(args.chromium), viewport=[1024, 768], cpuAffinity=sorted(os.sched_getaffinity(0)),
                                   baseline=str(args.baseline) if args.baseline else None, rasterInset=args.inset,
                                   stepTimeoutSeconds=args.timeout,
                                   scriptSha256={name: hashlib.sha256((HERE / name).read_bytes()).hexdigest()
                                                 for name in ('cesium.py', 'cesium.cjs')}),
                  cases=results)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2) + '\n')
    sys.exit(1 if any(case['problems'] for case in results) else 0)


if __name__ == '__main__':
    main()
