"""A/B processing benchmark of two rusty-tiles CLIs on the deterministic perf-lane corpus.

Usage: python3 bench/perf_lane/process.py --base BIN --head BIN --work DIR --out JSON [--repeat N]
Each case runs N times per binary in ABBA order, pinned to two CPUs (--cpus) at nice 10.
Wall time is best-of-N; peak RSS is the largest ru_maxrss of the measured process over
the N runs (Linux wait4; waited descendants are not summed). Inputs and outputs stay
under --work; the last output of each binary is kept for cesium.py.
"""
import argparse, datetime, fnmatch, hashlib, json, os, pathlib, platform, shutil, subprocess, sys, time, zipfile

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import corpus

ROOT = pathlib.Path(__file__).resolve().parents[2]
ANCHOR = ['153.02', '-27.47', '10']
CASES = [  # name, argv after the binary ({in}/{out} substituted), doctor command that must be ready
    ('vector', ['vector', '-i', '{in}/vector.geojson', '-o', '{out}.3tz', '--jobs', '2'], 'vector'),
    ('vector-explicit', ['vector', '-i', '{in}/vector.geojson', '-o', '{out}.3tz', '--jobs', '2', '--explicit'], 'vector'),
    ('point-cloud', ['point-cloud', '-i', '{in}/points.las', '-o', '{out}.3tz', '--sourceCrs', 'EPSG:32756',
                     '--heightOffset', '0'], 'point-cloud'),
    ('point-cloud-explicit', ['point-cloud', '-i', '{in}/points.las', '-o', '{out}.3tz', '--sourceCrs', 'EPSG:32756',
                              '--heightOffset', '0', '--explicit'], 'point-cloud'),
    ('mesh-local', ['mesh-local-to-3tz', '-i', '{in}/mesh.glb', '-o', '{out}.3tz', '--leaf-triangles', '20000',
                    '--anchor', *ANCHOR], 'mesh-local-to-3tz'),
    ('glb-wrap', ['glb-to-3tz', '-i', '{in}/mesh.glb', '-o', '{out}.3tz', '--anchor', *ANCHOR], 'glb-to-3tz'),
    ('mesh', ['mesh-to-3tz', '-i', '{in}/mesh.glb', '-o', '{out}.3tz', '--cartographicPositionDegrees', *ANCHOR], 'mesh-to-3tz'),
    ('mesh-explicit', ['mesh-to-3tz', '-i', '{in}/mesh.glb', '-o', '{out}.3tz', '--cartographicPositionDegrees', *ANCHOR,
                       '--explicit'], 'mesh-to-3tz'),
    ('terrain-fixture', ['terrain', '-i', '{repo}/tests/fixtures/t1-multipatch-plane.tif', '-o', '{out}',
                         '--cells-per-leaf', '16', '--height-offset', '10', '--fill-height', '-999'], 'terrain'),
    ('terrain', ['terrain', '-i', '{in}/dem.tif', '-o', '{out}', '--height-offset', '0', '--fill-height', '0'], 'terrain'),
    ('raster', ['raster', '-i', '{in}/rgba.tif', '-o', '{out}', '--minZoom', '10', '--maxZoom', '16', '--jobs', '2'], 'raster'),
]
CONTENT = {'.glb', '.b3dm', '.pnts', '.i3dm', '.cmpt', '.png', '.subtree'}
# Gate: head/base > RATIO and the absolute growth exceeds the floor (seconds, MiB, bytes).
RATIO, FLOORS = 1.25, {'wallSeconds': 0.1, 'peakRssMiB': 16, 'outputBytes': 0}


def sha256(path):
    return corpus.sha256(path)


def output_stats(path):
    """Bytes, digest and member/tile counts of a .3tz archive or an output directory."""
    if path.is_file():
        with zipfile.ZipFile(path) as archive:
            names = [n for n in archive.namelist() if not n.endswith('/')]
        stats = dict(outputBytes=path.stat().st_size, members=len(names), digest=sha256(path))
    else:
        files = sorted(p for p in path.rglob('*') if p.is_file())
        digest = hashlib.sha256()
        for p in files: digest.update(str(p.relative_to(path)).encode() + b'\0' + bytes.fromhex(sha256(p)))
        names = [str(p.relative_to(path)) for p in files]
        stats = dict(outputBytes=sum(p.stat().st_size for p in files), members=len(files), digest=digest.hexdigest())
    extensions = {}
    for name in names:
        suffix = pathlib.PurePath(name).suffix.lower() or name
        extensions[suffix] = extensions.get(suffix, 0) + 1
    stats.update(tiles=sum(count for suffix, count in extensions.items() if suffix in CONTENT), byExtension=extensions)
    return stats


def busy(cpus):
    """Busy clock ticks (excluding idle and iowait) summed over the given CPUs."""
    total = 0
    for line in pathlib.Path('/proc/stat').read_text().splitlines():
        name, *ticks = line.split()
        if name[3:].isdigit() and int(name[3:]) in cpus:
            values = [int(t) for t in ticks]; total += sum(values) - values[3] - values[4]
    return total


def measure(command, log, environment, cpus):
    with open(log, 'wb') as stream:
        ticks = busy(cpus)
        start = time.perf_counter()
        pid = os.posix_spawn(command[0], command, environment,
                             file_actions=[(os.POSIX_SPAWN_DUP2, stream.fileno(), 1), (os.POSIX_SPAWN_DUP2, stream.fileno(), 2)])
        _, status, usage = os.wait4(pid, 0)
        wall = time.perf_counter() - start
        foreign = (busy(cpus) - ticks) / os.sysconf('SC_CLK_TCK') - usage.ru_utime - usage.ru_stime
    return dict(exitCode=os.waitstatus_to_exitcode(status), wallSeconds=wall, cpuSeconds=usage.ru_utime + usage.ru_stime,
                peakRssMiB=usage.ru_maxrss / 1024, foreignCpuSeconds=max(foreign, 0))


def describe(binary, environment):
    run = lambda *a: subprocess.run([str(binary), *a], env=environment, capture_output=True, text=True)
    doctor = json.loads(run('doctor', '--json').stdout or '{}')
    return dict(path=str(binary), sha256=sha256(binary), version=run('--version').stdout.strip(),
                ready={name: value.get('ready') for name, value in doctor.get('commands', {}).items()}, run=run)


def admits(info, argv, command):
    """Whether this binary's doctor reports the command ready and its --help lists every flag used."""
    if not info['ready'].get(command):
        return f'doctor: {command} not ready (native GDAL build required for raster/terrain)'
    help_text = info['run'](argv[0], '--help')
    if help_text.returncode: return f'{argv[0]} --help failed'
    missing = [a for a in argv if a.startswith('--') and a not in help_text.stdout]
    return f'--help does not list {missing}' if missing else None


def compare(name, base, head, explanations):
    flags = []
    if base.get('ok') and head.get('ok'):
        for metric, floor in FLOORS.items():
            old, new = base.get(metric), head.get(metric)
            if old and new and new > old * RATIO and new - old > floor:
                flags.append(dict(metric=metric, base=old, head=new, ratio=new / old))
    elif base.get('ok'):
        flags.append(dict(metric='exit', base='ok', head='failed' if 'samples' in head else head.get('skipped')))
    # A best-wall sample is contended when other work used >10% of the two pinned CPUs during it.
    contended = [side for side, v in (('base', base), ('head', head))
                 if v.get('ok') and v['foreignCpuSecondsAtBestWall'] > 0.2 * v['wallSeconds']]
    for flag in flags:
        if flag['metric'] == 'wallSeconds':
            flag.update(contendedSides=contended, foreignCpuSecondsAtBestWall={
                'base': base['foreignCpuSecondsAtBestWall'], 'head': head['foreignCpuSecondsAtBestWall']})
        flag['explanation'] = next((text for pattern, text in explanations if fnmatch.fnmatch(f"{name}.{flag['metric']}", pattern)), None)
    return flags


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base', type=pathlib.Path, required=True)
    parser.add_argument('--head', type=pathlib.Path, required=True)
    parser.add_argument('--work', type=pathlib.Path, required=True)
    parser.add_argument('--out', type=pathlib.Path, required=True)
    parser.add_argument('--repeat', type=int, default=3)
    parser.add_argument('--cpus', default='0,1', help='CPU affinity of the run (AGENTS.md: two CPUs)')
    parser.add_argument('--only', help='comma-separated case names (validation cases follow their producers)')
    parser.add_argument('--explain', action='append', default=[], metavar='CASE.METRIC=REASON',
                        help='stated reason for an expected regression; CASE.METRIC is an fnmatch pattern')
    args = parser.parse_args()
    cpus = {int(c) for c in args.cpus.split(',')}
    os.sched_setaffinity(0, cpus); os.setpriority(os.PRIO_PROCESS, 0, 10)
    work = args.work.resolve()
    inputs = corpus.build(work / 'inputs')
    environment = dict(PATH='', HOME=os.environ.get('HOME', '/'), RAYON_NUM_THREADS='2', PROJ_NETWORK='OFF', LC_ALL='C')
    sides = {'base': args.base.resolve(), 'head': args.head.resolve()}
    infos = {side: describe(binary, environment) for side, binary in sides.items()}
    explanations = [item.split('=', 1) for item in args.explain]
    if any(len(pair) != 2 or not pair[1] for pair in explanations): parser.error('--explain needs CASE.METRIC=REASON')
    selected = set(args.only.split(',')) if args.only else None
    substitutions = {'in': str(work / 'inputs'), 'repo': str(ROOT)}
    results, load = [], os.getloadavg()

    def run_case(name, argvs, outputs, skips):
        samples = {side: [] for side in sides}
        for repeat in range(args.repeat):
            for side in (('base', 'head') if repeat % 4 in (0, 3) else ('head', 'base')):
                if argvs[side] is None: continue
                if outputs and outputs[side].exists():
                    shutil.rmtree(outputs[side]) if outputs[side].is_dir() else outputs[side].unlink()
                log = work / side / f'{name}.log'
                samples[side].append(measure([str(sides[side]), *argvs[side]], log, environment, cpus))
                print(f'{name} {side} #{repeat + 1}: {samples[side][-1]}', file=sys.stderr, flush=True)
        case = dict(case=name, argv=argvs['head'] or argvs['base'], sides={})
        for side, values in samples.items():
            if not values: case['sides'][side] = dict(case=name, skipped=skips.get(side)); continue
            ok = all(v['exitCode'] == 0 for v in values)
            summary = dict(case=name, ok=ok, samples=values, wallSeconds=min(v['wallSeconds'] for v in values),
                           cpuSeconds=min(v['cpuSeconds'] for v in values), peakRssMiB=max(v['peakRssMiB'] for v in values),
                           foreignCpuSecondsAtBestWall=min(values, key=lambda v: v['wallSeconds'])['foreignCpuSeconds'],
                           foreignCpuSecondsMax=max(v['foreignCpuSeconds'] for v in values))
            if not ok: summary['log'] = (work / side / f'{name}.log').read_text(errors='replace')[-2000:]
            if ok and outputs: summary.update(output_stats(outputs[side]))
            case['sides'][side] = summary
        base, head = case['sides']['base'], case['sides']['head']
        case['flags'] = compare(name, base, head, explanations)
        if base.get('digest') and head.get('digest'): case['identicalOutput'] = base['digest'] == head['digest']
        results.append(case)
        return case

    for name, template, command in CASES:
        if selected and name not in selected: continue
        argvs, skips, outputs = {}, {}, {}
        for side in sides:
            (work / side).mkdir(parents=True, exist_ok=True)
            argv = [part.format(out=str(work / side / name), **substitutions) for part in template]
            skips[side] = admits(infos[side], argv, command)
            argvs[side] = None if skips[side] else argv
            outputs[side] = pathlib.Path(next(a for i, a in enumerate(argv) if argv[i - 1] == '-o'))
        case = run_case(name, argvs, outputs, skips)
        if outputs['head'].suffix == '.3tz':
            skips = {side: None if case['sides'][side].get('ok') else 'no archive' for side in sides}
            run_case(f'validate:{name}', {side: None if skips[side] else ['validate', str(outputs[side])]
                                          for side in sides}, None, skips)

    regressions = [dict(case=c['case'], **f) for c in results for f in c['flags'] if not f.get('explanation')]
    report = dict(
        gate=dict(rule='regression when head/base > ratio and absolute growth > floor for best wall time, peak RSS '
                       'or output bytes, or when head fails a case base passes; --explain records a stated reason',
                  ratio=RATIO, floors=FLOORS, regressions=regressions,
                  explained=[dict(case=c['case'], **f) for c in results for f in c['flags'] if f.get('explanation')]),
        environment=dict(recordedAtUtc=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(),
            cpu=next((line.split(':', 1)[1].strip() for line in pathlib.Path('/proc/cpuinfo').read_text().splitlines()
                      if line.startswith('model name')), None), cpuAffinity=sorted(os.sched_getaffinity(0)),
            niceness=os.getpriority(os.PRIO_PROCESS, 0), loadAverageBefore=load, loadAverageAfter=os.getloadavg(), repeat=args.repeat,
            python=platform.python_version(), environment=environment,
            scriptSha256={name: sha256(pathlib.Path(__file__).with_name(name)) for name in ('process.py', 'corpus.py')},
            binaries={side: {k: v for k, v in info.items() if k != 'run'} for side, info in infos.items()},
            inputs=dict(inputs, **{'t1-multipatch-plane.tif': sha256(ROOT / 'tests/fixtures/t1-multipatch-plane.tif')}),
            rasterHoleDegrees=corpus.hole_degrees(corpus.RGBA_ORIGIN, corpus.Z14, corpus.RGBA_SIZE, corpus.RGBA_HOLE)),
        cases=results)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2) + '\n')
    for case in results:
        b, h = case['sides']['base'], case['sides']['head']
        cells = [f"{s.get('wallSeconds', float('nan')):8.3f}s {s.get('peakRssMiB', float('nan')):7.1f}MiB {s.get('outputBytes', 0):>11}B"
                 for s in (b, h)]
        marks = [f['metric'] + (' (explained)' if f['explanation'] else ' REGRESSION')
                 + (' contended: rerun' if f.get('contendedSides') else '') for f in case['flags']]
        print(f"{case['case']:<30} base {cells[0]} | head {cells[1]} {', '.join(marks)}")
    sys.exit(1 if regressions else 0)


if __name__ == '__main__':
    main()
