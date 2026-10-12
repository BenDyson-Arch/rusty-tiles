"""Load rusty-tiles outputs in pinned Cesium on the GPU and gate on runtime failures.

Usage: python3 bench/perf_lane/cesium.py --work DIR --out JSON NAME=PATH...
       python3 bench/perf_lane/cesium.py --check-renderer   (preflight only)
PATH is a .3tz archive (unpacked under --work, as the archive viewers do), a
3D Tiles directory with tileset.json, or a raster directory with tilejson.json.
By default each run opens its own browser context in a running Chromium reached
over CDP (--cdp-url, RUSTY_TILES_PERF_CDP, default http://localhost:9222), with the
cache disabled and the tab brought to the front; it closes only that context.
--launch starts a private headless Chromium with ANGLE on the GPU instead, and
--software-gl a SwiftShader one, whose results are labelled software and never
compared with GPU runs. The WebGL renderer must be a GPU unless --software-gl is
given. Browser runs hold ~/.cache/rusty-tiles-chromium.lock. The page is 1024x768;
the local server binds 127.0.0.1 and every other request or WebSocket is refused
and counted as a failure. --timeout bounds each asynchronous browser step; the case
budget is that times the step count plus a fixed browser start allowance.
"""
import argparse, contextlib, fcntl, fractions, hashlib, http.server, json, math, os, pathlib, re, shutil, signal
import subprocess, sys, threading, urllib.error, urllib.parse, urllib.request, zipfile

HERE = pathlib.Path(__file__).resolve().parent
# Defaults: environment overrides, else the local caches used for the recorded runs.
CACHE = pathlib.Path.home() / '.cache'
DEFAULTS = dict(cesium=os.environ.get('CESIUM_DIR', CACHE / 'rusty-tiles-117-browser-cache/node_modules/cesium/Build/Cesium'),
                node_modules=os.environ.get('NODE_PATH', CACHE / 'rusty-tiles-117-browser-cache/node_modules'),
                chromium=os.environ.get('CHROMIUM', CACHE / 'rusty-tiles-137-playwright/chromium-1243/chrome-linux64/chrome'))
STARTUP_SECONDS = 60  # browser launch, before the first bounded step
CDP_URL = os.environ.get('RUSTY_TILES_PERF_CDP', 'http://localhost:9222')
BROWSER_LOCK = CACHE / 'rusty-tiles-chromium.lock'  # shared with other agents driving the same browser
SETTLE_MS = 250  # finish rule: loaded and nothing pending, held for this much wall time
REFINED_SSE = 0  # the fully refined pass of every tileset case
SOFTWARE_RENDERERS = re.compile(r'swiftshader|llvmpipe|softpipe|lavapipe|software|basic render', re.I)
FINISH_RULE = ('loaded = tilesLoaded (raster: globe tilesLoaded and an empty tile queue), no pending or processing '
               'tile requests, no active Cesium requests and no in-flight XHR/fetch, held for {ms} ms of wall time; '
               'the time reported is when that state began')
ENVIRONMENT_EXIT = 3  # renderer, browser or endpoint problem: nothing was measured
PAGE = b"""<!doctype html><html><head><meta charset="utf-8"><script>window.CESIUM_BASE_URL='/cesium/';</script>
<script src="/cesium/Cesium.js"></script><link rel="stylesheet" href="/cesium/Widgets/widgets.css">
<style>html,body,#view{width:100%;height:100%;margin:0;overflow:hidden}</style></head>
<body><div id="view"></div></body></html>"""
CATEGORIES = {'.json': 'manifest', '.subtree': 'subtree', '.glb': 'content', '.b3dm': 'content', '.pnts': 'content',
              '.i3dm': 'content', '.cmpt': 'content', '.png': 'imagery', '.jpg': 'imagery', '.webp': 'imagery'}
# Gate: absolute failures always; with --baseline, >25% growth of these measures.
RATIO, FLOORS = 1.25, {'requests': 2, 'requestBytes': 64 * 1024, 'secondsToLoaded': 0.25}
TIMING = {'secondsToLoaded'}  # compared only between runs with the same renderer class


def renderer_class(renderer):
    """'gpu', 'software' (SwiftShader, llvmpipe, ...) or 'unknown' for a WebGL UNMASKED_RENDERER string."""
    if not renderer: return 'unknown'
    return 'software' if SOFTWARE_RENDERERS.search(renderer) else 'gpu'


def check_renderer_class(found, software_gl):
    """None when the renderer matches the requested mode, else the reason to stop."""
    expected = 'software' if software_gl else 'gpu'
    if renderer_class(found) == expected: return None
    if software_gl: return f'--software-gl was given but the WebGL renderer is {found!r}'
    return (f'WebGL renderer {found!r} is not a GPU; refusing to time software rendering. Fix the browser\'s GPU '
            'access, or pass --software-gl to run labelled software results that are never compared with GPU runs')


def visible(result):
    """The pass's tab was visible before and after the load (hidden in between is a recorded failure)."""
    seen = result.get('visibility') or {}
    return seen.get('before') == 'visible' and seen.get('after') == 'visible'


def die(message):
    print(f'cesium: {message}', file=sys.stderr)
    sys.exit(ENVIRONMENT_EXIT)


def cdp_version(url):
    """The DevTools /json/version record, or a clear stop when nothing listens there."""
    try:
        with urllib.request.urlopen(url.rstrip('/') + '/json/version', timeout=5) as response: return json.load(response)
    except (urllib.error.URLError, OSError, ValueError) as error:
        die(f'no Chromium DevTools endpoint at {url} ({error}). Start the GPU browser with '
            '`chromium --remote-debugging-port=9222 --user-data-dir=$HOME/.config/chromium-perf`, point --cdp-url or '
            'RUSTY_TILES_PERF_CDP at it, or pass --launch to start a private headless Chromium')


@contextlib.contextmanager
def browser_lock(path=BROWSER_LOCK):
    """flock(2) on the shared browser lock, the same lock as `flock ~/.cache/rusty-tiles-chromium.lock`."""
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, 'a') as handle:
        fcntl.flock(handle, fcntl.LOCK_EX)
        try: yield
        finally: fcntl.flock(handle, fcntl.LOCK_UN)


def browser_spec(args):
    if args.launch or args.software_gl:
        return dict(mode='launch', gl='swiftshader' if args.software_gl else args.angle, executablePath=str(args.chromium))
    return dict(mode='cdp', cdpUrl=args.cdp_url)


def preflight(args, environment):
    """Renderer and tab facts from a throwaway context; stops unless they match the requested mode."""
    spec = browser_spec(args)
    if spec['mode'] == 'cdp': cdp_version(spec['cdpUrl'])
    with browser_lock():
        child = subprocess.run(['node', str(HERE / 'cesium.cjs'), '--renderer', json.dumps(spec)], env=environment,
                               capture_output=True, text=True, timeout=STARTUP_SECONDS)
    if child.returncode != 0: die(f'browser preflight failed: {(child.stderr or child.stdout).strip()[-2000:]}')
    facts = json.loads(child.stdout)
    facts.update(rendererClass=renderer_class(facts.get('renderer')), browser=spec)
    if (reason := check_renderer_class(facts.get('renderer'), args.software_gl)): die(reason)
    if facts.get('visibility') != 'visible':
        die(f"the test tab is {facts.get('visibility')} after bringToFront; background tabs throttle frames")
    return facts


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
    """Run one browser case under the browser lock (waiting for it is not budgeted); a budget overrun or
    unreadable output fails the case, not the run. Killing the driver drops its CDP connection; Chromium
    then discards the driver's context, and the attached browser itself is never closed."""
    with browser_lock():
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


def problems(case, baseline, renderer=None):
    """Gate findings for one pass. renderer: {'renderer', 'rendererClass'} of the preflight; with a
    baseline, timings are compared only when both runs used the same renderer class."""
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
    if renderer and case.get('renderer') is not None and case['renderer'] != renderer['renderer']:
        found.append(f"renderer changed during the run: {case['renderer']!r}, preflight {renderer['renderer']!r}")
    if baseline:
        comparable = baseline.get('rendererClass', 'software') == (renderer or {}).get('rendererClass', 'software')
        for metric, floor in FLOORS.items():
            if metric in TIMING and not comparable: continue
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
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('cases', nargs='*', help='NAME=PATH')
    parser.add_argument('--work', type=pathlib.Path)
    parser.add_argument('--out', type=pathlib.Path)
    parser.add_argument('--hole', help='W,S,E,N degrees that must render transparent in raster cases')
    parser.add_argument('--baseline', type=pathlib.Path, help='earlier cesium.py JSON to compare by case name')
    parser.add_argument('--timeout', type=float, default=120, help='seconds per asynchronous browser step')
    parser.add_argument('--inset', help='shrink the raster provider rectangle: DEG, W,S,E,N or Npx (finest-zoom '
                        'pixels, e.g. 1/1024px). A workaround/probe that changes what is tested; reported as a warning')
    parser.add_argument('--cpus', default='0,1', help='CPU affinity of this driver (AGENTS.md: two CPUs)')
    parser.add_argument('--corner-views', action='store_true', help='add raster cameras on each bounds corner, '
                        'four finest-zoom tiles across, after the overview')
    parser.add_argument('--cdp-url', default=CDP_URL, help='DevTools endpoint of the running GPU Chromium '
                        '(default: RUSTY_TILES_PERF_CDP or http://localhost:9222)')
    parser.add_argument('--launch', action='store_true', help='start a private headless Chromium (--chromium) '
                        'with ANGLE on the GPU instead of attaching over CDP')
    parser.add_argument('--angle', default='gl', choices=['gl', 'gl-egl', 'vulkan'], help='ANGLE backend for --launch')
    parser.add_argument('--software-gl', action='store_true', help='SwiftShader in a private headless Chromium '
                        '(implies --launch); results are labelled software and never compared with GPU runs')
    parser.add_argument('--settle-ms', type=float, default=SETTLE_MS, help='finish-rule window in wall-clock ms')
    parser.add_argument('--no-refined', action='store_true', help='skip the fully refined (SSE 0) tileset pass')
    parser.add_argument('--request-diagnostic', action='store_true', help='also load every tileset pass with '
                        'RequestScheduler.throttleRequests=false and record the per-frame request cap\'s effect (not gated)')
    parser.add_argument('--check-renderer', action='store_true', help='print the preflight renderer facts and exit; '
                        f'exit {ENVIRONMENT_EXIT} unless the renderer matches the mode')
    parser.add_argument('--maximum-sse', type=float, default=16, help='3D Tiles maximumScreenSpaceError (default 16). '
                        'Any other value is a report-only probe, such as 1 for near-full refinement: baseline '
                        'comparisons are recorded as advisory and do not fail the run')
    for key, value in DEFAULTS.items(): parser.add_argument('--' + key.replace('_', '-'), type=pathlib.Path, default=value)
    args = parser.parse_args()
    os.sched_setaffinity(0, {int(c) for c in args.cpus.split(',')}); os.setpriority(os.PRIO_PROCESS, 0, 10)
    environment = dict(os.environ, NODE_PATH=str(args.node_modules), CHROMIUM=str(args.chromium))
    renderer = preflight(args, environment)
    print(f"cesium: renderer {renderer['renderer']!r} ({renderer['rendererClass']}), {renderer['browserVersion']}, "
          f"{renderer['browser']['mode']}", file=sys.stderr, flush=True)
    if args.check_renderer:
        print(json.dumps(renderer)); return
    if not (args.cases and args.work and args.out): parser.error('NAME=PATH cases, --work and --out are required')
    if args.inset: print(f'cesium: WARNING --inset {args.inset} changes the raster rectangle under test', file=sys.stderr)
    work = args.work.resolve(); shutil.rmtree(work / 'screenshots', ignore_errors=True)
    (work / 'screenshots').mkdir(parents=True)
    baseline_report = json.loads(args.baseline.read_text()) if args.baseline else {}
    baseline = {c['name']: c for c in baseline_report.get('cases', [])}
    for case in baseline.values():  # cases recorded before the renderer was captured ran on SwiftShader
        case.setdefault('rendererClass', baseline_report['environment'].get('rendererClass', 'software'))
        if case.get('refined'): case['refined'].setdefault('rendererClass', case['rendererClass'])
    timing_comparable = not args.baseline or baseline_report['environment'].get('rendererClass', 'software') == renderer['rendererClass']
    if not timing_comparable:
        print(f"cesium: --baseline used a {baseline_report['environment'].get('rendererClass', 'software')} renderer; "
              'its timings are not compared', file=sys.stderr)
    mounts, plans = {}, []
    for item in args.cases:
        name, _, path = item.partition('=')
        directory, kind, identity = mount(name, pathlib.Path(path).resolve(), work)
        mounts[name] = directory; plans.append((name, kind, identity, directory))
    log = []
    server = serve(mounts, args.cesium.resolve(), log)
    browser = browser_spec(args)
    base_spec = dict(stepMs=int(args.timeout * 1000), settleMs=args.settle_ms, browser=browser)
    results = []

    def load(name, spec, shot, budget, directory):
        """One browser pass; returns its result with server-side request counts and the renderer class."""
        start = len(log)
        result = run_case(['node', str(HERE / 'cesium.cjs'), f'http://127.0.0.1:{server.server_port}/', json.dumps(spec),
                           str(shot)], environment, budget)
        result.update(requests=requests(log[start:], name, directory), rendererClass=renderer_class(result.get('renderer')))
        return result

    def brief(result):
        keep = ('loaded', 'secondsToLoaded', 'secondsToSettled', 'secondsToFirstTile', 'framesToLoaded', 'peakActiveRequests',
                'scheduler', 'failures')
        return dict({k: result.get(k) for k in keep}, requests=result['requests']['requests'])

    try:
        for name, kind, identity, directory in plans:
            spec = dict(base_spec, kind=kind, url=f'/data/{name}/' + ('' if kind == 'raster' else 'tileset.json'))
            if kind == 'tileset': spec['maximumScreenSpaceError'] = args.maximum_sse
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
            case = load(name, spec, shot, budget, directory)
            case.update(name=name, kind=kind, output=identity, spec=spec, budgetSeconds=budget, screenshot=str(shot))
            if spec.get('insetDegrees'): case['warnings'] = [f"raster rectangle inset {spec['insetDegrees']} degrees: not the exact bounds"]
            shots = [shot] + [shot.with_name(f"{name}-{view['name']}.png") for view in case.get('views', [])]
            case['problems'] = problems(case, baseline.get(name) if args.maximum_sse == 16 else None, renderer)
            if args.maximum_sse != 16:  # report-only probe: baseline comparisons are advisory
                case['advisory'] = [p for p in problems(case, baseline.get(name), renderer) if p not in case['problems']]
            passes = [('default', spec, case)]
            if kind == 'tileset' and not args.no_refined:
                refined_spec = dict(spec, maximumScreenSpaceError=REFINED_SSE)
                refined_shot = shot.with_name(f'{name}-refined.png'); shots.append(refined_shot)
                refined = load(name, refined_spec, refined_shot, budget, directory)
                refined['spec'] = refined_spec
                refined['problems'] = problems(refined, (baseline.get(name) or {}).get('refined'), renderer)
                case['refined'] = refined
                case['problems'] += [f'refined (SSE {REFINED_SSE:g}): {p}' for p in refined['problems']]
                passes.append(('refined', refined_spec, refined))
            if kind == 'tileset' and args.request_diagnostic:  # viewer setting, recorded only
                case['requestCapDiagnostic'] = {}
                for label, pass_spec, throttled in passes:
                    free = load(name, dict(pass_spec, scheduler=dict(throttleRequests=False)),
                                shot.with_name(f'{name}-{label}-unthrottled.png'), budget, directory)
                    case['requestCapDiagnostic'][label] = dict(throttled=brief(throttled), unthrottled=brief(free))
            case['captureSha256'] = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in shots if p.exists()}
            results.append(case)
            print(json.dumps(dict(name=name, loaded=case.get('loaded'), seconds=case.get('secondsToLoaded'),
                                  firstTile=case.get('secondsToFirstTile'),
                                  refined=(case.get('refined') or {}).get('secondsToLoaded'),
                                  requests=case['requests']['requests'], bytes=case['requests']['requestBytes'],
                                  problems=case['problems'])), file=sys.stderr, flush=True)
    finally:
        server.shutdown(); server.server_close()
    first = next((c for c in results if c.get('scheduler')), {})
    report = dict(gate=dict(rule='fail on load failure, page/console errors, failed or external requests, failed render '
                                 'probes or a renderer change; with --baseline also on >25% more requests/bytes or load '
                                 'time beyond floors (load time only against the same renderer class)',
                            ratio=RATIO, floors=FLOORS),
                  environment=dict(cesium=json.loads((args.cesium.resolve().parents[1] / 'package.json').read_text())['version'],
                                   playwright=json.loads((args.node_modules / 'playwright/package.json').read_text())['version'],
                                   chromium=str(args.chromium) if browser['mode'] == 'launch' else None,
                                   browser=browser, browserVersion=renderer.get('browserVersion'),
                                   renderer=renderer.get('renderer'), vendor=renderer.get('vendor'),
                                   rendererClass=renderer['rendererClass'], webgl=renderer.get('webgl'),
                                   viewport=renderer.get('viewport'), window=renderer.get('window'),
                                   devicePixelRatio=renderer.get('devicePixelRatio'),
                                   tabVisible=all(visible(p) for c in results for p in (c, c.get('refined')) if p),
                                   cacheDisabled=True, browserLock=str(BROWSER_LOCK),
                                   finishRule=FINISH_RULE.format(ms=f'{args.settle_ms:g}'), settleMs=args.settle_ms,
                                   refinedMaximumScreenSpaceError=None if args.no_refined else REFINED_SSE,
                                   requestScheduler=dict(first.get('scheduler') or {}, note='Cesium defaults; a viewer setting'),
                                   cpuAffinity=sorted(os.sched_getaffinity(0)),
                                   baseline=str(args.baseline) if args.baseline else None,
                                   baselineTimingCompared=bool(args.baseline) and timing_comparable, rasterInset=args.inset,
                                   maximumScreenSpaceError=args.maximum_sse,
                                   stepTimeoutSeconds=args.timeout,
                                   scriptSha256={name: hashlib.sha256((HERE / name).read_bytes()).hexdigest()
                                                 for name in ('cesium.py', 'cesium.cjs')}),
                  cases=results)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2) + '\n')
    sys.exit(1 if any(case['problems'] for case in results) else 0)


if __name__ == '__main__':
    main()
