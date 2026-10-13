"""Game-style Cesium flythroughs of rusty-tiles outputs: deterministic camera paths, per-frame metrics, recipes.

Usage: python3 -I bench/perf_lane/flythrough.py --bin RUSTY_TILES --recipes FILE [--recipes FILE ...] \
         [--paths a,b] [--profiles native,cpu4x,lowcache] [--repeats 3] [--evidence DIR] [--commit-report FILE]

A recipe is a named conversion (CLI arguments) plus viewer options; a recipe file is one set of recipes on
one dataset, and only recipes of the same set are compared. Each output is converted once per distinct
argument list. Each run (recipe x path x profile x repeat) opens a fresh, cache-disabled context in the GPU
Chromium over CDP (cesium.py's browser handling, ~/.cache/rusty-tiles-chromium.lock held per run), flies a
path from flythrough.json frame by frame and records per-frame metrics (flythrough.cjs). Host load is
sampled around every run and noisy runs are flagged. Raw runs, screenshots, flythrough.json and an evidence
FLYTHROUGH.md (with images) go to --evidence; --commit-report writes the image-free copy for git.
Runs on a software renderer are refused unless --software-gl, and are labelled; profiles are never mixed.
"""
import argparse, copy, datetime, gzip, hashlib, json, math, os, pathlib, re, shutil, signal, statistics, subprocess, sys
import threading, time

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
import cesium, corpus, flythrough_report  # noqa: E402

CONFIG = HERE / 'flythrough.json'
POSE_KEYS = ('heading', 'pitch', 'range', 'east', 'north', 'up')
DEFAULT_POSE = dict(heading=0.0, pitch=-45.0, range=3.0, east=0.0, north=0.0, up=0.0)
EASES = {'linear': lambda t: t, 'smooth': lambda t: t * t * (3 - 2 * t)}
NAME = re.compile(r'^[a-z0-9][a-z0-9._-]*$')
VIEWER_KEYS = {'tilesetOptions': dict, 'maximumScreenSpaceError': (int, float), 'scheduler': dict}
SCHEDULER_KEYS = {'throttleRequests', 'maximumRequestsPerServer', 'maximumRequests'}
PROFILE_KEYS = {'description', 'cpuThrottlingRate', 'tilesetOptions'}
CONVERT_ENV = dict(PATH='', HOME=os.environ.get('HOME', '/'), RAYON_NUM_THREADS='2', PROJ_NETWORK='OFF', LC_ALL='C')
EVIDENCE = pathlib.Path.home() / '.cache/rusty-tiles-evidence/perf'


# ---- recipes ----

def parse_recipes(document, source='recipes'):
    """Validate one recipe set and resolve 'like' (copy another recipe, then override; viewer options and
    tilesetOptions merge). Returns {set, dataset, description, recipes:[{name, convert|output, viewer}]}."""
    def fail(message): raise ValueError(f'{source}: {message}')
    if not isinstance(document, dict): fail('expected a JSON object')
    unknown = set(document) - {'set', 'dataset', 'description', 'recipes'}
    if unknown: fail(f'unknown keys {sorted(unknown)}')
    set_name = document.get('set')
    if not isinstance(set_name, str) or not NAME.match(set_name): fail(f'set name {set_name!r} must match {NAME.pattern}')
    if not isinstance(document.get('dataset'), str) or not document['dataset']: fail('dataset is required')
    items = document.get('recipes')
    if not isinstance(items, list) or not items: fail('recipes must be a non-empty list')
    resolved = {}
    for item in items:
        if not isinstance(item, dict): fail('each recipe must be an object')
        unknown = set(item) - {'name', 'like', 'convert', 'output', 'viewer', 'description'}
        if unknown: fail(f"recipe {item.get('name')!r}: unknown keys {sorted(unknown)}")
        name = item.get('name')
        if not isinstance(name, str) or not NAME.match(name): fail(f'recipe name {name!r} must match {NAME.pattern}')
        if name in resolved: fail(f'recipe {name!r} given twice')
        recipe = dict(name=name, convert=None, output=None, viewer={}, description=item.get('description', ''))
        if 'like' in item:
            if item['like'] not in resolved: fail(f"recipe {name!r}: like {item['like']!r} is not an earlier recipe")
            base = copy.deepcopy(resolved[item['like']])
            recipe.update(convert=base['convert'], output=base['output'], viewer=base['viewer'])
        if 'convert' in item and 'output' in item: fail(f'recipe {name!r}: give convert or output, not both')
        if 'convert' in item: recipe.update(convert=item['convert'], output=None)
        if 'output' in item: recipe.update(output=item['output'], convert=None)
        if recipe['convert'] is not None:
            argv = recipe['convert']
            if not isinstance(argv, list) or not argv or not all(isinstance(a, str) for a in argv):
                fail(f'recipe {name!r}: convert must be a non-empty list of strings')
            if not any('{out}' in a for a in argv): fail(f'recipe {name!r}: convert must write to {{out}} or {{out}}.3tz')
            for a in argv:
                for key in re.findall(r'{(\w*)}', a):
                    if key not in ('out', 'corpus', 'repo'): fail(f'recipe {name!r}: unknown placeholder {{{key}}}')
        elif not isinstance(recipe['output'], str): fail(f'recipe {name!r}: convert or output is required')
        viewer = item.get('viewer', {})
        if not isinstance(viewer, dict): fail(f'recipe {name!r}: viewer must be an object')
        for key, value in viewer.items():
            if key not in VIEWER_KEYS: fail(f'recipe {name!r}: unknown viewer option {key!r}')
            if not isinstance(value, VIEWER_KEYS[key]) or isinstance(value, bool):
                fail(f'recipe {name!r}: viewer {key} has the wrong type')
        if 'maximumScreenSpaceError' in viewer and viewer['maximumScreenSpaceError'] < 0:
            fail(f'recipe {name!r}: maximumScreenSpaceError must be >= 0')
        if set(viewer.get('scheduler', {})) - SCHEDULER_KEYS: fail(f'recipe {name!r}: unknown scheduler keys')
        merged = dict(recipe['viewer'], **viewer)
        if 'tilesetOptions' in viewer:
            merged['tilesetOptions'] = dict(recipe['viewer'].get('tilesetOptions', {}), **viewer['tilesetOptions'])
        recipe['viewer'] = merged
        resolved[name] = recipe
    return dict(set=set_name, dataset=document['dataset'], description=document.get('description', ''),
                recipes=list(resolved.values()))


def load_recipe_sets(paths):
    """Parse every recipe file; recipe names must be unique across sets."""
    sets, seen = [], set()
    for path in paths:
        parsed = parse_recipes(json.loads(pathlib.Path(path).read_text()), str(path))
        for recipe in parsed['recipes']:
            if recipe['name'] in seen: raise ValueError(f"{path}: recipe {recipe['name']!r} also defined in another set")
            seen.add(recipe['name'])
        sets.append(parsed)
    return sets


def conversion_key(argv):
    return hashlib.sha256(json.dumps(argv).encode()).hexdigest()[:12]


def substitute(argv, out, corpus_dir):
    values = dict(out=str(out), corpus=str(corpus_dir), repo=str(ROOT))
    return [re.sub(r'{(\w+)}', lambda m: values[m.group(1)], a) for a in argv]


# ---- paths ----

def check_pose(pose, where):
    if not isinstance(pose, dict): raise ValueError(f'{where}: pose must be an object')
    unknown = set(pose) - set(POSE_KEYS)
    if unknown: raise ValueError(f'{where}: unknown pose keys {sorted(unknown)}')
    for key, value in pose.items():
        if not isinstance(value, (int, float)) or isinstance(value, bool) or not math.isfinite(value):
            raise ValueError(f'{where}: {key} must be a finite number')
    if 'range' in pose and pose['range'] <= 0: raise ValueError(f'{where}: range must be > 0')


def expand_path(name, path):
    """Steps of one path: [{'pose': [heading, pitch, range, east, north, up]}, ...], with 'waypoint' on hold
    steps. A motion segment of N frames emits N steps ending at its 'to' pose; heading, pitch and offsets are
    interpolated linearly in the eased parameter and range geometrically (constant zoom rate). A waypoint
    with a pose cuts there instantly. The first segment must be a waypoint with a pose; unset pose keys
    take DEFAULT_POSE."""
    segments = path.get('segments')
    if not isinstance(segments, list) or not segments: raise ValueError(f'path {name}: segments must be a non-empty list')
    first = segments[0]
    if 'waypoint' not in first or 'pose' not in first: raise ValueError(f'path {name}: must start with a waypoint with a pose')
    current, steps, names = dict(DEFAULT_POSE), [], set()
    as_list = lambda pose: [float(pose[k]) for k in POSE_KEYS]
    for index, segment in enumerate(segments):
        where = f'path {name} segment {index}'
        if not isinstance(segment, dict): raise ValueError(f'{where}: must be an object')
        if 'waypoint' in segment:
            if set(segment) - {'waypoint', 'pose'}: raise ValueError(f'{where}: a waypoint takes only waypoint and pose')
            label = segment['waypoint']
            if not isinstance(label, str) or not NAME.match(label): raise ValueError(f'{where}: waypoint name {label!r}')
            if label in names: raise ValueError(f'{where}: waypoint {label!r} repeated')
            names.add(label)
            if 'pose' in segment:
                check_pose(segment['pose'], where)
                current = dict(DEFAULT_POSE if index == 0 else current, **segment['pose'])
            steps.append(dict(pose=as_list(current), waypoint=label))
            continue
        if set(segment) - {'frames', 'to', 'ease'}: raise ValueError(f'{where}: a motion takes frames, to and ease')
        frames, ease = segment.get('frames'), segment.get('ease', 'linear')
        if not isinstance(frames, int) or isinstance(frames, bool) or frames < 1: raise ValueError(f'{where}: frames must be an integer >= 1')
        if ease not in EASES: raise ValueError(f'{where}: ease must be one of {sorted(EASES)}')
        check_pose(segment.get('to'), where)
        target = dict(current, **segment['to'])
        for k in range(1, frames + 1):
            t = EASES[ease](k / frames)
            pose = {key: current[key] + (target[key] - current[key]) * t for key in POSE_KEYS}
            pose['range'] = current['range'] * (target['range'] / current['range']) ** t
            steps.append(dict(pose=as_list(pose)))
        current = target
    return steps


def load_config(path=CONFIG):
    config = json.loads(pathlib.Path(path).read_text())
    if not isinstance(config.get('fps'), (int, float)) or config['fps'] <= 0: raise ValueError('fps must be > 0')
    for name, profile in config['profiles'].items():
        if set(profile) - PROFILE_KEYS: raise ValueError(f'profile {name}: unknown keys {sorted(set(profile) - PROFILE_KEYS)}')
        if profile.get('cpuThrottlingRate', 1) < 1: raise ValueError(f'profile {name}: cpuThrottlingRate must be >= 1')
    config['expanded'] = {name: expand_path(name, p) for name, p in config['paths'].items()}
    return config


def run_order(recipes, paths, profiles, repeats):
    """(repeat, profile, path, recipe) in run order: recipes rotate by one per repeat so no recipe always
    runs first, and every repeat of a profile completes before the next repeat."""
    order = []
    for repeat in range(repeats):
        rotated = recipes[repeat % len(recipes):] + recipes[:repeat % len(recipes)]
        for profile in profiles:
            for path in paths:
                for recipe in rotated: order.append((repeat, profile, path, recipe))
    return order


# ---- host load ----

def read_psi():
    found = {}
    for resource in ('cpu', 'memory', 'io'):
        try:
            line = pathlib.Path(f'/proc/pressure/{resource}').read_text().splitlines()[0]
            found[resource] = int(line.rsplit('total=', 1)[1])
        except (OSError, IndexError, ValueError): pass
    return found


def cpu_times():
    fields = [int(v) for v in pathlib.Path('/proc/stat').read_text().splitlines()[0].split()[1:]]
    return sum(fields), fields[3] + fields[4]  # total, idle + iowait


def mem_available_gib():
    for line in pathlib.Path('/proc/meminfo').read_text().splitlines():
        if line.startswith('MemAvailable:'): return int(line.split()[1]) / 1024 / 1024
    return None


def gpu_util():
    """GPU utilisation % and memory MiB from nvidia-smi before a run (other processes), else None."""
    try:
        text = subprocess.run(['nvidia-smi', '--query-gpu=utilization.gpu,memory.used', '--format=csv,noheader,nounits'],
                              capture_output=True, text=True, timeout=5).stdout
        util, memory = text.splitlines()[0].split(',')
        return float(util), float(memory)
    except (OSError, subprocess.SubprocessError, ValueError, IndexError):
        return None, None


class HostSampler:
    """Samples host load while one run executes: PSI totals at both ends, CPU busy and MemAvailable every 0.5 s."""
    def __enter__(self):
        self.util, self.gpu_memory = gpu_util()
        self.psi, self.start, self.stop = read_psi(), time.monotonic(), threading.Event()
        self.busy, self.memory = [], [mem_available_gib()]
        self.thread = threading.Thread(target=self.loop, daemon=True); self.thread.start()
        return self

    def loop(self):
        last = cpu_times()
        while not self.stop.wait(0.5):
            now = cpu_times()
            total, idle = now[0] - last[0], now[1] - last[1]
            if total: self.busy.append(1 - idle / total)
            self.memory.append(mem_available_gib()); last = now

    def __exit__(self, *unused):
        self.stop.set(); self.thread.join()
        self.facts = dict(wallSeconds=time.monotonic() - self.start, psiStart=self.psi, psiEnd=read_psi(),
                          cpuBusyMean=statistics.fmean(self.busy) if self.busy else None,
                          memAvailableMinGiB=min((m for m in self.memory if m is not None), default=None),
                          gpuUtilStart=self.util, gpuMemoryUsedStartMiB=self.gpu_memory, loadAverage=os.getloadavg()[0])


def drive(command, environment, budget):
    """One browser run under the shared browser lock (waiting is neither budgeted nor sampled)."""
    with cesium.browser_lock():
        with HostSampler() as host:
            child = subprocess.Popen(command, env=environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                                     start_new_session=True)
            try:
                stdout, stderr = child.communicate(timeout=budget); timed_out = False
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL); stdout, stderr = child.communicate(); timed_out = True
    if timed_out: return dict(loaded=False, timedOut=True, driverError=stderr[-4000:]), host.facts
    try:
        if child.returncode == 0: return json.loads(stdout), host.facts
    except json.JSONDecodeError: pass
    return dict(loaded=False, driverError=(stderr or stdout)[-4000:]), host.facts


def problems(result, renderer):
    found = []
    if result.get('timedOut'): found.append('run budget exceeded')
    if not result.get('loaded'): found.append('run failed')
    for key in ('failures', 'failedRequests', 'externalRequests'):
        if result.get(key): found.append(f'{key}: {len(result[key])}: {str(result[key][0])[:300]}')
    if (result.get('console') or {}).get('errors'): found.append(f"console errors: {len(result['console']['errors'])}")
    if result.get('driverError'): found.append('driver: ' + result['driverError'][-300:])
    if result.get('renderer') is not None and result['renderer'] != renderer['renderer']:
        found.append(f"renderer changed: {result['renderer']!r}")
    if (result.get('visibility') or {}).get('after') not in (None, 'visible'): found.append('tab hidden after the run')
    return found


def git_commit():
    try: return subprocess.run(['git', '-C', str(ROOT), 'rev-parse', '--short=12', 'HEAD'], capture_output=True, text=True).stdout.strip()
    except OSError: return None


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin', type=pathlib.Path, help='rusty-tiles CLI for recipes with convert')
    parser.add_argument('--recipes', action='append', required=True, type=pathlib.Path, help='recipe set JSON; repeat')
    parser.add_argument('--config', type=pathlib.Path, default=CONFIG, help='paths and profiles (default flythrough.json)')
    parser.add_argument('--paths', help='comma-separated path names (default: all)')
    parser.add_argument('--profiles', default='native', help='comma-separated profile names (default native)')
    parser.add_argument('--only', help='comma-separated recipe names (default: all)')
    parser.add_argument('--repeats', type=int, default=3)
    parser.add_argument('--evidence', type=pathlib.Path,
                        default=EVIDENCE / f'flythrough-{datetime.date.today():%Y%m%d}', help='raw output directory (not git)')
    parser.add_argument('--work', type=pathlib.Path, default=pathlib.Path.home() / '.cache/perf-lane/work/flythrough',
                        help='corpus inputs, converted outputs and run specs (reused across invocations)')
    parser.add_argument('--commit-report', type=pathlib.Path, help='also write the image-free report here (for git)')
    parser.add_argument('--hold-seconds', type=float, help='waypoint hold limit (default from the config)')
    parser.add_argument('--timeout', type=float, default=120, help='seconds per bounded browser step')
    parser.add_argument('--settle-ms', type=float, default=cesium.SETTLE_MS)
    parser.add_argument('--screenshots', choices=['first', 'none'], default='first', help='waypoint captures on repeat 1')
    parser.add_argument('--cpus', default='0,1', help='CPU affinity of this driver and of conversions')
    parser.add_argument('--cdp-url', default=cesium.CDP_URL)
    parser.add_argument('--launch', action='store_true')
    parser.add_argument('--angle', default='gl', choices=['gl', 'gl-egl', 'vulkan'])
    parser.add_argument('--software-gl', action='store_true')
    for key, value in cesium.DEFAULTS.items(): parser.add_argument('--' + key.replace('_', '-'), type=pathlib.Path, default=value)
    args = parser.parse_args()
    if args.repeats < 1: parser.error('--repeats must be >= 1')
    try:
        config, sets = load_config(args.config), load_recipe_sets(args.recipes)
    except ValueError as error: parser.error(str(error))
    paths = args.paths.split(',') if args.paths else list(config['paths'])
    profiles = args.profiles.split(',')
    for name in paths:
        if name not in config['paths']: parser.error(f'unknown path {name!r}')
    for name in profiles:
        if name not in config['profiles']: parser.error(f'unknown profile {name!r}')
    recipes = [dict(r, set=s['set'], dataset=s['dataset']) for s in sets for r in s['recipes']]
    if args.only:
        keep = set(args.only.split(','))
        recipes = [r for r in recipes if r['name'] in keep]
        if not recipes: parser.error('--only selected no recipes')
    if any(r['convert'] for r in recipes) and not args.bin: parser.error('--bin is required for recipes with convert')
    os.sched_setaffinity(0, {int(c) for c in args.cpus.split(',')}); os.setpriority(os.PRIO_PROCESS, 0, 10)
    environment = dict(os.environ, NODE_PATH=str(args.node_modules), CHROMIUM=str(args.chromium))
    renderer = cesium.preflight(args, environment)
    print(f"flythrough: renderer {renderer['renderer']!r} ({renderer['rendererClass']}), {renderer['browserVersion']}",
          file=sys.stderr, flush=True)
    work, evidence = args.work.resolve(), args.evidence.resolve()
    for sub in ('runs', 'screenshots'): (evidence / sub).mkdir(parents=True, exist_ok=True)
    (work / 'specs').mkdir(parents=True, exist_ok=True)

    # Conversions: once per distinct argument list and binary, reused when the stamp matches.
    binary = None
    if args.bin:
        binary = dict(path=str(args.bin), sha256=hashlib.sha256(args.bin.read_bytes()).hexdigest(),
                      version=subprocess.run([str(args.bin), '--version'], capture_output=True, text=True).stdout.strip())
        corpus_dir = work / 'inputs'; corpus.build(corpus_dir)
    mounts = {}
    for recipe in recipes:
        if recipe['convert']:
            key = conversion_key(recipe['convert'])
            out = work / 'outputs' / key
            argv = substitute(recipe['convert'], out, corpus_dir)
            produced = pathlib.Path(next(a for a in argv if a.startswith(str(out))))
            stamp = out.parent / f'{key}.stamp.json'
            expected = dict(argv=argv, binary=binary['sha256'])
            if not (produced.exists() and stamp.exists() and json.loads(stamp.read_text()).get('expected') == expected):
                shutil.rmtree(produced, ignore_errors=True) if produced.is_dir() else produced.unlink(missing_ok=True)
                produced.parent.mkdir(parents=True, exist_ok=True)
                began = time.monotonic()
                child = subprocess.run([str(args.bin), *argv], env=CONVERT_ENV, capture_output=True, text=True)
                if child.returncode: raise SystemExit(f"flythrough: {recipe['name']} conversion failed:\n{child.stderr[-3000:]}")
                stamp.write_text(json.dumps(dict(expected=expected, wallSeconds=time.monotonic() - began)))
            recipe['conversion'] = dict(argv=argv, wallSeconds=json.loads(stamp.read_text())['wallSeconds'])
            source = produced
        else:
            source = pathlib.Path(recipe['output']).expanduser().resolve()
        directory, kind, identity = cesium.mount(recipe['name'], source, work)
        if kind != 'tileset': raise SystemExit(f"flythrough: {recipe['name']} is a {kind} output; flythroughs need a tileset")
        mounts[recipe['name']] = directory
        recipe['output'] = identity
        recipe['directory'] = directory

    log = []
    server = cesium.serve(mounts, args.cesium.resolve(), log)
    browser = cesium.browser_spec(args)
    hold_seconds = args.hold_seconds or config['holdSeconds']
    order = run_order([r['name'] for r in recipes], paths, profiles, args.repeats)
    by_name = {r['name']: r for r in recipes}
    runs, screenshots = [], {}
    try:
        for number, (repeat, profile_name, path, name) in enumerate(order, 1):
            recipe, profile = by_name[name], config['profiles'][profile_name]
            steps = config['expanded'][path]
            run_id = f'{name}.{path}.{profile_name}.r{repeat + 1}'
            viewer = recipe['viewer']
            spec = dict(url=f'/data/{name}/tileset.json', steps=steps, fps=config['fps'], holdMs=hold_seconds * 1000,
                        settleMs=args.settle_ms, stepMs=int(args.timeout * 1000), browser=browser,
                        maximumScreenSpaceError=viewer.get('maximumScreenSpaceError', 16),
                        tilesetOptions=dict(viewer.get('tilesetOptions', {}), **profile.get('tilesetOptions', {})),
                        cpuThrottlingRate=profile.get('cpuThrottlingRate', 1),
                        screenshots=args.screenshots == 'first' and repeat == 0)
            if viewer.get('scheduler'): spec['scheduler'] = viewer['scheduler']
            spec_file = work / 'specs' / f'{run_id}.json'; spec_file.write_text(json.dumps(spec))
            waypoints = sum(1 for s in steps if 'waypoint' in s)
            rate = spec['cpuThrottlingRate']
            # Budget: browser start and page load, motion frames at no worse than 5 fps (times throttling), holds.
            budget = cesium.STARTUP_SECONDS + 3 * args.timeout + len(steps) / 5 * rate + waypoints * (hold_seconds + 10)
            start = len(log)
            result, host = drive(['node', str(HERE / 'flythrough.cjs'), f'http://127.0.0.1:{server.server_port}/',
                                  str(spec_file), str(evidence / 'screenshots' / run_id)], environment, budget)
            result['serverRequests'] = cesium.requests(log[start:], name, recipe['directory'])
            facts, noise = flythrough_report.host_noise(host)
            found = problems(result, renderer)
            summary = flythrough_report.summarize(result) if result.get('frames') else {}
            with gzip.open(evidence / 'runs' / f'{run_id}.json.gz', 'wt') as stream:
                json.dump(dict(id=run_id, spec={k: v for k, v in spec.items() if k != 'steps'}, host=host, result=result), stream)
            for waypoint in result.get('waypoints', []):
                if waypoint.get('screenshot'):
                    shot = pathlib.Path(waypoint['screenshot'])
                    screenshots[f'{name}|{path}|{profile_name}|{waypoint["name"]}'] = dict(
                        file=str(shot.relative_to(evidence)), sha256=hashlib.sha256(shot.read_bytes()).hexdigest())
            runs.append(dict(id=run_id, recipe=name, set=recipe['set'], path=path, profile=profile_name, repeat=repeat + 1,
                             host=host, hostFacts=facts, noise=noise, problems=found, summary=summary,
                             waypoints=[{k: w.get(k) for k in ('name', 'reached', 'fullQualitySeconds', 'holdFrames')}
                                        for w in result.get('waypoints', [])],
                             rendererClass=cesium.renderer_class(result.get('renderer')), gpuTimer=result.get('gpuTimer'),
                             serverRequests={k: result['serverRequests'][k] for k in ('requests', 'requestBytes', 'duplicateRequests')}))
            print(f"flythrough [{number}/{len(order)}] {run_id}: frame p95 {summary.get('frameP95Ms')} ms, "
                  f"full quality {summary.get('fullQualitySeconds')} s, requests {summary.get('requests')}"
                  + (f", noisy: {'; '.join(noise)}" if noise else '') + (f", PROBLEMS: {found}" if found else ''),
                  file=sys.stderr, flush=True)
    finally:
        server.shutdown(); server.server_close()

    aggregates = {}
    for recipe in recipes:
        for path in paths:
            for profile_name in profiles:
                group = [r for r in runs if (r['recipe'], r['path'], r['profile']) == (recipe['name'], path, profile_name)
                         and r['summary'] and not r['problems']]
                if not group: continue
                entry = flythrough_report.aggregate([r['summary'] for r in group], [bool(r['noise']) for r in group])
                used = [r for r in group if not r['noise']] if entry['clean'] >= 2 else group
                entry['waypoints'] = {}
                for label in [w['name'] for w in group[0]['waypoints']]:
                    values = [w['fullQualitySeconds'] for r in used for w in r['waypoints'] if w['name'] == label and w['reached']]
                    entry['waypoints'][label] = dict(median=statistics.median(values) if values else None,
                                                     unreached=sum(1 for r in used for w in r['waypoints']
                                                                   if w['name'] == label and not w['reached']))
                aggregates[f"{recipe['name']}|{path}|{profile_name}"] = entry
    classes = {r['rendererClass'] for r in runs if r['rendererClass'] != 'unknown'}
    if len(classes) > 1: raise SystemExit(f'flythrough: runs used several renderer classes {classes}; not comparable')
    intervals = [v for r in runs if r['summary'] for v in [r['summary'].get('frameP50Ms')] if v is not None and r['profile'] == 'native']
    artifact = dict(
        generated=datetime.datetime.now().isoformat(timespec='seconds'),
        environment=dict(evidence=str(evidence), commit=git_commit(), binary=binary, renderer=renderer['renderer'],
                         rendererClass=renderer['rendererClass'], browserVersion=renderer['browserVersion'], browser=browser,
                         viewport=renderer.get('viewport'), devicePixelRatio=renderer.get('devicePixelRatio'),
                         cesium=json.loads((args.cesium.resolve().parents[1] / 'package.json').read_text())['version'],
                         fps=config['fps'], holdSeconds=hold_seconds, settleMs=args.settle_ms, repeats=args.repeats,
                         medianIntervalMs=statistics.median(intervals) if intervals else None,
                         gpuTimer=next((r['gpuTimer'] for r in runs if r.get('gpuTimer')), None),
                         thresholds=dict(hitchMs=flythrough_report.HITCH_MS, popMinTiles=flythrough_report.POP_MIN_TILES,
                                         popFraction=flythrough_report.POP_FRACTION, sseJumpRatio=flythrough_report.SSE_JUMP_RATIO,
                                         sseJumpPx=flythrough_report.SSE_JUMP_PX, noise=flythrough_report.NOISE,
                                         stableSpread=flythrough_report.STABLE_SPREAD),
                         scriptSha256={n: hashlib.sha256((HERE / n).read_bytes()).hexdigest()
                                       for n in ('flythrough.py', 'flythrough.cjs', 'flythrough_report.py', 'flythrough.json', 'cesium.cjs')}),
        paths={name: dict(description=config['paths'][name].get('description', ''),
                          motionFrames=sum(1 for s in config['expanded'][name] if 'waypoint' not in s),
                          waypoints=[s['waypoint'] for s in config['expanded'][name] if 'waypoint' in s]) for name in paths},
        profiles={name: config['profiles'][name] for name in profiles},
        recipes=[dict(set=r['set'], dataset=r['dataset'], name=r['name'], convert=r['convert'], viewer=r['viewer'],
                      output=r['output'], conversion=r.get('conversion')) for r in recipes],
        runs=runs, aggregates=aggregates, screenshots=screenshots)
    (evidence / 'flythrough.json').write_text(json.dumps(artifact, indent=1) + '\n')
    report = evidence / 'FLYTHROUGH.md'
    report.write_text(flythrough_report.render(artifact, evidence, report, images=True))
    if args.commit_report:
        target = args.commit_report.resolve()
        target.write_text(flythrough_report.render(artifact, evidence, target, images=False))
    print(report)
    sys.exit(1 if any(r['problems'] for r in runs) else 0)


if __name__ == '__main__':
    main()
