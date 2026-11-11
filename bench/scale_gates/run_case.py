#!/usr/bin/env python3
"""Run one scale-gate case: launch a rusty-tiles binary with a caller-given command line,
measure it, validate its output, optionally load it in Cesium, and compare against
declared budgets. Writes report.json and SUMMARY.md under --work.

The tiler CLI is not final, so the command is a template. Placeholders:
  {binary} {input} {input_dir} {output} {output_dir} {scratch} {work} {jobs} {cpus}
{input} is the manifest's truth.input_file when --input is a generator directory.

  python3 -I bench/scale_gates/run_case.py --case gate-a --budget gate-a \\
      --binary target/release/rusty-tiles --input ~/.cache/rusty-tiles-scale/inputs/mesh-526m \\
      --output-name out.3tz \\
      --command '{binary} --json mesh -i {input} -o {output} --jobs {jobs} --scratch-dir {scratch}'

  # validate-only gate on a synthetic archive (no producer):
  python3 -I bench/scale_gates/run_case.py --case implicit-2p22 --budget d-implicit \\
      --binary target/release/rusty-tiles --input ~/.cache/rusty-tiles-scale/archives/implicit-2p22

Measurements (Linux):
  * peak RSS: wait4 ru_maxrss of the command, taken by the small launch.py (largest single
    process; children are not summed). Linux carries the launcher's resident high-water mark
    across exec, so each value includes a floor of a few MiB, recorded as rss_floor_bytes. With --memory cgroup the command also runs in a transient systemd --user
    scope and the scope's memory.peak (anon + page cache, whole process tree) is recorded,
    with periodically sampled anon bytes from memory.stat.
  * scratch: allocated bytes (st_blocks) under {scratch} plus staging entries in {output_dir}
    other than the final output, sampled every --sample-seconds. A periodic sample is a lower
    bound, not an atomic peak. Free-space drop of the scratch filesystem is a coarse cross-check
    (other writers on the same filesystem pollute it).
  * wall/CPU time, output bytes, zip members, and `validate` with its own wall/RSS.
  * --cesium: bench/perf_lane/cesium.py load and SSE-0 refinement against the running GPU
    Chromium over CDP. cesium.py itself flocks ~/.cache/rusty-tiles-chromium.lock around each
    browser case; do not wrap this script in another flock on that file (it would self-deadlock).
"""
import argparse, contextlib, datetime, json, os, pathlib, re, shlex, shutil, signal, subprocess, sys, threading, time
import tomllib, zipfile

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import scalecommon as sc  # noqa: E402

HERE = pathlib.Path(__file__).resolve().parent
DEFAULT_BUDGETS = HERE / 'budgets.toml'
DEFAULT_POOL = '4-7,12-15'
RUNS = sc.SCALE_ROOT / 'runs'
LAUNCHER = HERE / 'launch.py'
REF = re.compile(r'^(producer|manifest|validate|metrics):')


# ---------------------------------------------------------------- CPUs

def parse_cpus(text):
    out = []
    for part in text.split(','):
        if '-' in part:
            a, b = part.split('-'); out += range(int(a), int(b) + 1)
        elif part:
            out.append(int(part))
    return out


def cpu_ticks():
    ticks = {}
    for line in pathlib.Path('/proc/stat').read_text().splitlines():
        name, *fields = line.split()
        if name.startswith('cpu') and name != 'cpu':
            values = [int(v) for v in fields]
            ticks[int(name[3:])] = (values[3] + values[4], sum(values))
    return ticks


def idle_fractions(cpus, seconds):
    a = cpu_ticks(); time.sleep(seconds); b = cpu_ticks()
    out = {}
    for c in cpus:
        if c in a and c in b:
            total = b[c][1] - a[c][1]
            out[c] = (b[c][0] - a[c][0]) / total if total else 1.0
    return out


def choose_cpus(pool, count, min_idle, sample_seconds, wait_seconds):
    """The `count` idlest CPUs of the pool that are each >= min_idle idle; waits up to wait_seconds."""
    deadline = time.monotonic() + wait_seconds
    while True:
        idle = idle_fractions(pool, sample_seconds)
        ready = sorted((c for c in pool if idle.get(c, 0) >= min_idle), key=lambda c: -idle[c])
        if len(ready) >= count:
            return sorted(ready[:count]), idle
        if time.monotonic() >= deadline:
            raise SystemExit(f'no {count} CPUs of {pool} reached {min_idle:.0%} idle within {wait_seconds:.0f}s: '
                             + ', '.join(f'{c}:{v:.0%}' for c, v in sorted(idle.items())))
        time.sleep(5)


# ---------------------------------------------------------------- sampling

def allocated(path, skip=()):
    """Allocated bytes (st_blocks) of the regular files under path, skipping `skip` paths."""
    total = 0
    path = pathlib.Path(path)
    if not path.exists():
        return 0
    for root, dirs, files in os.walk(path):
        rootp = pathlib.Path(root)
        dirs[:] = [d for d in dirs if rootp / d not in skip]
        for p in [rootp / f for f in files if rootp / f not in skip]:  # files only: directory blocks vary by fs
            try:
                total += p.lstat().st_blocks * 512
            except FileNotFoundError:
                pass
    return total


def launcher_children(pid):
    try:
        return [int(c) for c in pathlib.Path(f'/proc/{pid}/task/{pid}/children').read_text().split()]
    except OSError:
        return []


def cgroup_of(pid):
    try:
        line = pathlib.Path(f'/proc/{pid}/cgroup').read_text().strip().splitlines()[0]
        return pathlib.Path('/sys/fs/cgroup' + line.split('::', 1)[1])
    except (OSError, IndexError):
        return None


def memory_stat_anon(cgroup):
    try:
        for line in (cgroup / 'memory.stat').read_text().splitlines():
            if line.startswith('anon '):
                return int(line.split()[1])
    except OSError:
        pass
    return None


def measure(argv, env, log_prefix, cpus=None, scratch_roots=(), skip=(), memory='wait4', memory_max=None,
            sample_seconds=0.5, timeout=None):
    """Spawn argv (pinned, nice 10), sample scratch/cgroup until exit, return a measurement record."""
    prefix = []
    unit = None
    command = list(argv)
    rusage_path = pathlib.Path(f'{log_prefix}.rusage.json')
    if memory == 'cgroup':
        unit = f'rt-scale-{os.getpid()}-{int(time.time() * 1000)}'
        prefix += ['systemd-run', '--user', '--scope', '--quiet', '--collect', f'--unit={unit}',
                   '-p', 'MemoryAccounting=yes']
        if memory_max:
            prefix += ['-p', f'MemoryMax={int(memory_max)}', '-p', 'MemorySwapMax=0']
    if cpus:
        prefix += ['taskset', '-c', ','.join(map(str, cpus))]
    prefix += ['nice', '-n', '10', sys.executable, '-I', '-S', str(LAUNCHER), str(rusage_path)]
    argv = list(argv)
    argv[0] = shutil.which(argv[0], path=env.get('PATH')) or argv[0]
    full = prefix + argv
    full[0] = shutil.which(full[0]) or full[0]
    stdout_path, stderr_path = pathlib.Path(f'{log_prefix}.stdout'), pathlib.Path(f'{log_prefix}.stderr')
    scratch_fs = next((pathlib.Path(r) for r in scratch_roots), None)
    free_before = shutil.disk_usage(scratch_fs).free if scratch_fs else None
    state = dict(peak_scratch=0, min_free=free_before, anon_peak=None, samples=0, cgroup=None, timed_out=False)
    done = threading.Event()

    def sample(pid):
        """Sampler thread: the main thread blocks in wait4, so wall time is exact."""
        started_ = time.monotonic()
        while True:
            if memory == 'cgroup':
                found = cgroup_of(pid)
                if found and found.name == f'{unit}.scope':  # only once the child joined its scope
                    state['cgroup'] = found
                if state['cgroup']:
                    anon = memory_stat_anon(state['cgroup'])
                    if anon is not None:
                        state['anon_peak'] = max(state['anon_peak'] or 0, anon)
            if scratch_roots:
                state['peak_scratch'] = max(state['peak_scratch'], sum(allocated(r, skip) for r in scratch_roots))
                state['min_free'] = min(state['min_free'], shutil.disk_usage(scratch_fs).free)
            state['samples'] += 1
            if timeout and time.monotonic() - started_ > timeout:
                if not state['timed_out']:  # kill the command; the launcher then still writes its rusage
                    state['timed_out'], state['killed_at'] = True, time.monotonic()
                    for child in launcher_children(pid):
                        with contextlib.suppress(ProcessLookupError):
                            os.kill(child, signal.SIGKILL)
                elif time.monotonic() - state['killed_at'] > 10:
                    with contextlib.suppress(ProcessLookupError):
                        os.killpg(pid, signal.SIGKILL)
            if done.wait(sample_seconds):
                return

    started = time.monotonic()
    with open(stdout_path, 'wb') as out, open(stderr_path, 'wb') as err:
        pid = os.posix_spawn(full[0], full, env, setsid=True,
                             file_actions=[(os.POSIX_SPAWN_DUP2, out.fileno(), 1), (os.POSIX_SPAWN_DUP2, err.fileno(), 2)])
        sampler = threading.Thread(target=sample, args=(pid,), daemon=True)
        sampler.start()
        _, status, usage = os.wait4(pid, 0)
        wall = time.monotonic() - started
        done.set(); sampler.join()
    peak_scratch, min_free, anon_peak = state['peak_scratch'], state['min_free'], state['anon_peak']
    samples, timed_out = state['samples'], state['timed_out']
    try:
        launched = json.loads(rusage_path.read_text())
    except (OSError, json.JSONDecodeError):
        launched = {}
    record = dict(command=command, argv=full, exit_code=os.waitstatus_to_exitcode(status), timed_out=timed_out,
                  wall_seconds=round(wall, 3),
                  cpu_seconds=round(launched['utime'] + launched['stime'], 3) if launched else
                  round(usage.ru_utime + usage.ru_stime, 3),
                  peak_rss_bytes=launched.get('maxrss_bytes'), launcher_peak_rss_bytes=usage.ru_maxrss * 1024,
                  samples=samples, sample_seconds=sample_seconds, stdout=str(stdout_path), stderr=str(stderr_path))
    if scratch_roots:
        record.update(scratch_peak_bytes=peak_scratch, scratch_roots=[str(r) for r in scratch_roots],
                      scratch_end_bytes=sum(allocated(r, skip) for r in scratch_roots),
                      fs_free_drop_peak_bytes=(free_before - min_free) if free_before is not None else None)
    if memory == 'cgroup':
        record['cgroup_memory_peak_bytes'] = launched.get('cgroup_memory_peak_bytes')
        record['cgroup'] = launched.get('cgroup')
        record['cgroup_anon_sampled_peak_bytes'] = anon_peak
        record['cgroup_memory_max'] = memory_max
    return record


def json_stdout(path):
    text = pathlib.Path(path).read_text(errors='replace').strip()
    if not text:
        return None
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        try:  # last line of mixed output
            return json.loads(text.splitlines()[-1])
        except json.JSONDecodeError:
            return None


# ---------------------------------------------------------------- output facts

def output_facts(path, hash_limit):
    path = pathlib.Path(path)
    facts = dict(path=str(path), exists=path.exists())
    if not path.exists():
        return facts
    if path.is_dir():
        files = [p for p in path.rglob('*') if p.is_file()]
        facts.update(kind='directory', bytes=sum(p.stat().st_size for p in files), files=len(files),
                     tile_files=sum(p.suffix in ('.glb', '.b3dm', '.pnts', '.subtree') for p in files))
        return facts
    st = path.stat()
    facts.update(kind='file', bytes=st.st_size, allocated_bytes=st.st_blocks * 512)
    if st.st_size <= hash_limit:
        facts['sha256'] = sc.sha256_file(path)
    try:
        with zipfile.ZipFile(path) as z:
            names = z.namelist()
        facts.update(members=len(names), glb_members=sum(n.endswith('.glb') for n in names),
                     subtree_members=sum(n.endswith('.subtree') for n in names))
    except (zipfile.BadZipFile, OSError) as e:
        facts['zip_error'] = str(e)
    return facts


# ---------------------------------------------------------------- budgets

def dotted(record, path):
    cur = record
    for part in path.split('.'):
        if isinstance(cur, dict) and part in cur:
            cur = cur[part]
        elif isinstance(cur, list) and part.isdigit() and int(part) < len(cur):
            cur = cur[int(part)]
        else:
            return None
    return cur


def load_budget(path, name):
    data = tomllib.loads(pathlib.Path(path).read_text())
    sets = data.get('budget', {})
    if name not in sets:
        raise SystemExit(f'budget set {name!r} not in {path} (have: {", ".join(sorted(sets))})')
    chain, seen = [], set()
    while name:
        if name in seen:
            raise SystemExit(f'budget inheritance cycle at {name}')
        seen.add(name); chain.append(sets[name]); name = sets[name].get('extends')
    merged = {}
    for layer in reversed(chain):
        for key, value in layer.items():
            if isinstance(value, dict):
                merged.setdefault(key, {}).update(value)
            else:
                merged[key] = value
    return merged


def resolve(value, sources):
    """A budget value is a number/bool or 'source:dotted.path[*factor]' (sources: producer, manifest,
    metrics)."""
    if not isinstance(value, str) or not REF.match(value):
        return value, None
    factor = 1.0
    ref = value
    if '*' in value:
        ref, f = value.rsplit('*', 1); factor = float(f)
    source, _, path = ref.strip().partition(':')
    got = dotted(sources.get(source) or {}, path)
    if got is None:
        return None, value
    return (got * factor if factor != 1.0 else got), value


def check_budgets(budget, metrics, sources, ran=()):
    """Compare metrics with max/min/expect budgets; metrics of a stage that did not run (e.g. cesium_*
    without --cesium, validate_* for a directory output) are 'skipped', other missing metrics fail."""
    results = []
    for kind, op in (('max', lambda m, b: m <= b), ('min', lambda m, b: m >= b), ('expect', lambda m, b: m == b)):
        for key, raw in sorted(budget.get(kind, {}).items()):
            bound, ref = resolve(raw, sources)
            metric = metrics.get(key)
            entry = dict(metric=key, kind=kind, budget=bound, budget_ref=ref, value=metric)
            if bound is None:
                entry['status'] = 'unresolved'
            elif metric is None:
                stage = key.split('_', 1)[0]
                entry['status'] = 'skipped' if stage in ('cesium', 'validate') and stage not in ran else 'missing'
            else:
                entry['status'] = 'pass' if op(metric, bound) else 'fail'
            results.append(entry)
    scale = []
    for key, bound in sorted(budget.get('scale', {}).items()):
        got = dotted(sources.get('manifest') or {}, key)
        scale.append(dict(metric=key, budget=bound, value=got,
                          status='pass' if got is not None and got >= bound else 'below'))
    return results, scale


# ---------------------------------------------------------------- main flow

def substitute(template, values):
    return [part.format(**values) for part in shlex.split(template)]


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--case', required=True, help='case label (report name)')
    p.add_argument('--budget', required=True, help='budget set name in --budgets')
    p.add_argument('--budgets', type=pathlib.Path, default=DEFAULT_BUDGETS)
    p.add_argument('--binary', type=pathlib.Path, required=True)
    p.add_argument('--input', type=pathlib.Path, required=True,
                   help='generator directory with manifest.json, or a plain file/archive')
    p.add_argument('--command', help='producer command template; omit for a validate-only case (the input is '
                                     'then the archive under test)')
    p.add_argument('--output-name', default='out.3tz', help='output name inside {output_dir}')
    p.add_argument('--env', action='append', default=[], help='KEY=VALUE (templated) for the producer')
    p.add_argument('--producer-report', help='path template of a JSON producer report; default: producer stdout')
    p.add_argument('--no-validate', action='store_true')
    p.add_argument('--validate-command', default='{binary} --json validate {output}')
    p.add_argument('--cesium', action='store_true', help='load + refine in Cesium via bench/perf_lane/cesium.py')
    p.add_argument('--cesium-arg', action='append', default=[], help='extra cesium.py argument (e.g. --cdp-url=...)')
    p.add_argument('--work', type=pathlib.Path, help=f'run directory (default {RUNS}/<case>-<utc>)')
    p.add_argument('--jobs', type=int, default=2, help='CPUs to pin (and {jobs})')
    p.add_argument('--cpus', help=f'fixed CPU list; default: idle CPUs from --cpu-pool')
    p.add_argument('--cpu-pool', default=DEFAULT_POOL)
    p.add_argument('--min-idle', type=float, default=0.90)
    p.add_argument('--idle-wait', type=float, default=600.0, help='seconds to wait for idle CPUs')
    p.add_argument('--memory', choices=['wait4', 'cgroup'], default='cgroup')
    p.add_argument('--enforce-memory', action='store_true',
                   help='cgroup MemoryMax = the resolved max.peak_rss_bytes budget (OOM kill on overrun)')
    p.add_argument('--sample-seconds', type=float, default=0.2)
    p.add_argument('--timeout', type=float, help='kill the producer after this many seconds')
    p.add_argument('--hash-limit-gib', type=float, default=8.0, help='sha256 outputs up to this size')
    p.add_argument('--keep-output', action='store_true', help='keep the output after the run')
    p.add_argument('--reserve-gib', type=float, default=20.0)
    a = p.parse_args(argv)

    binary = a.binary.resolve()
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
    work = sc.resolve_output(a.work or RUNS / f'{a.case}-{stamp}')
    if work.exists() and any(work.iterdir()):
        raise SystemExit(f'{work} exists and is not empty')
    work.mkdir(parents=True, exist_ok=True)
    budget = load_budget(a.budgets, a.budget)

    input_path = a.input.expanduser().resolve()
    manifest = None
    if input_path.is_dir() and (input_path / 'manifest.json').exists():
        manifest = json.loads((input_path / 'manifest.json').read_text())
        input_file = input_path / (manifest.get('truth', {}).get('input_file') or manifest.get('primary'))
    else:
        input_file = input_path
    output_dir, scratch = work / 'output', work / 'scratch'
    output_dir.mkdir(); scratch.mkdir()
    validate_only = not a.command
    output = input_file if validate_only else output_dir / a.output_name
    if a.cpus:
        cpus, idle = parse_cpus(a.cpus), None
    else:
        cpus, idle = choose_cpus(parse_cpus(a.cpu_pool), a.jobs, a.min_idle, 3.0, a.idle_wait)
    values = dict(binary=str(binary), input=str(input_file), input_dir=str(input_file.parent), output=str(output),
                  output_dir=str(output_dir), scratch=str(scratch), work=str(work), jobs=str(a.jobs),
                  cpus=','.join(map(str, cpus)))
    env = dict(PATH='/usr/bin:/bin', HOME=os.environ.get('HOME', ''), RAYON_NUM_THREADS=str(a.jobs),
               RUST_TEST_THREADS=str(a.jobs), TMPDIR=str(scratch), PROJ_NETWORK='OFF', LANG='C.UTF-8')
    if a.memory == 'cgroup':  # systemd-run talks to the user manager
        for k in ('XDG_RUNTIME_DIR', 'DBUS_SESSION_BUS_ADDRESS'):
            if k in os.environ:
                env[k] = os.environ[k]
    for item in a.env:
        k, _, v = item.partition('=')
        env[k] = v.format(**values)
    binary_sha = sc.sha256_file(binary)
    disk = sc.disk_preflight(work, 0, a.reserve_gib * sc.GiB, what='run')
    report = dict(schema=1, case=a.case, budget_set=a.budget, budgets_file=str(a.budgets),
                  budgets_sha256=sc.sha256_file(a.budgets), runner_sha256=sc.sha256_file(__file__),
                  started_utc=stamp, host=os.uname().nodename, kernel=os.uname().release,
                  binary=str(binary), binary_sha256=binary_sha, input=str(input_file),
                  input_manifest=str(input_path / 'manifest.json') if manifest else None,
                  input_parameters=(manifest or {}).get('parameters'), input_truth_summary=None,
                  cpus=cpus, cpu_idle_before=idle, env={k: v for k, v in env.items() if k != 'HOME'},
                  disk_preflight=disk, memory_mode=a.memory, work=str(work))
    if manifest:
        truth = manifest.get('truth', {})
        report['input_truth_summary'] = {k: v for k, v in truth.items() if not isinstance(v, (dict, list))}
        report['input_bytes'] = manifest.get('total_bytes')
    metrics = {}
    sources = dict(manifest=manifest or {}, metrics=metrics)
    floor = measure(['/bin/true'], env, work / 'floor', cpus, memory=a.memory, sample_seconds=a.sample_seconds)
    if a.memory == 'cgroup' and (floor['exit_code'] != 0 or floor.get('cgroup_memory_peak_bytes') is None):
        print('cgroup scope unavailable (systemd --user / memory.peak); falling back to wait4', file=sys.stderr)
        a.memory = report['memory_mode'] = 'wait4'
        floor = measure(['/bin/true'], env, work / 'floor', cpus, memory='wait4', sample_seconds=a.sample_seconds)
    report['rss_floor_bytes'] = floor['peak_rss_bytes']  # launcher RSS inherited across exec
    memory_max = None
    if a.enforce_memory and 'peak_rss_bytes' in budget.get('max', {}):
        memory_max, _ = resolve(budget['max']['peak_rss_bytes'], dict(manifest=manifest or {}))
        if not isinstance(memory_max, (int, float)):
            raise SystemExit('--enforce-memory needs a literal or manifest: peak_rss_bytes budget '
                             '(producer: references resolve only after the run)')

    if not validate_only:
        argv_ = substitute(a.command, values)
        print(f'[{a.case}] producer on CPUs {cpus}: {shlex.join(argv_)}', file=sys.stderr, flush=True)
        run = measure(argv_, env, work / 'producer', cpus, (scratch, output_dir), skip={output},
                      memory=a.memory, memory_max=memory_max, sample_seconds=a.sample_seconds, timeout=a.timeout)
        producer = None
        if a.producer_report:
            rp = pathlib.Path(a.producer_report.format(**values))
            producer = json.loads(rp.read_text()) if rp.exists() else None
        else:
            producer = json_stdout(run['stdout'])
        run['report'] = producer
        report['producer'] = run
        sources['producer'] = producer or {}
        metrics.update(producer_exit=run['exit_code'], wall_seconds=run['wall_seconds'],
                       cpu_seconds=run['cpu_seconds'], peak_rss_bytes=run['peak_rss_bytes'],
                       cgroup_memory_peak_bytes=run.get('cgroup_memory_peak_bytes'),
                       cgroup_anon_peak_bytes=run.get('cgroup_anon_sampled_peak_bytes'),
                       scratch_peak_bytes=run.get('scratch_peak_bytes'),
                       scratch_end_bytes=run.get('scratch_end_bytes'),
                       timed_out=run['timed_out'],
                       producer_error=error_field(producer, 'code'))
        print(f'[{a.case}] producer exit {run["exit_code"]} in {run["wall_seconds"]}s, '
              f'RSS {mib(run["peak_rss_bytes"])} MiB', file=sys.stderr, flush=True)

    facts = output_facts(output, 0)  # hashed after validate, so hashing does not warm validate's page cache
    report['output'] = facts
    metrics.update(output_exists=facts['exists'], output_bytes=facts.get('bytes'), output_members=facts.get('members'))

    if not a.no_validate and facts.get('exists') and facts.get('kind') == 'file':
        argv_ = substitute(a.validate_command, values)
        print(f'[{a.case}] validate: {shlex.join(argv_)}', file=sys.stderr, flush=True)
        vrun = measure(argv_, env, work / 'validate', cpus, (), memory=a.memory, sample_seconds=a.sample_seconds)
        vrep = json_stdout(vrun['stdout'])
        vrun['report'] = {k: v for k, v in (vrep or {}).items() if k != 'payloads'} if vrep else None
        if vrep and 'payloads' in vrep:
            vrun['report']['payload_count'] = len(vrep['payloads'])
        report['validate'] = vrun
        sources['validate'] = vrep or {}
        metrics.update(validate_exit=vrun['exit_code'], validate_ok=bool(vrep and vrep.get('ok', False)),
                       validate_wall_seconds=vrun['wall_seconds'], validate_peak_rss_bytes=vrun['peak_rss_bytes'],
                       validate_cgroup_memory_peak_bytes=vrun.get('cgroup_memory_peak_bytes'),
                       tiles=(vrep or {}).get('tiles'), content_references=(vrep or {}).get('contentReferences'),
                       validate_timed_out=vrun['timed_out'],
                       validate_error=((vrep or {}).get('error') or {}).get('code'),
                       validate_message=((vrep or {}).get('error') or {}).get('message'))
        print(f'[{a.case}] validate exit {vrun["exit_code"]} in {vrun["wall_seconds"]}s, tiles {metrics["tiles"]}, '
              f'RSS {mib(vrun["peak_rss_bytes"])} MiB', file=sys.stderr, flush=True)

    if a.cesium and facts.get('exists'):
        out_json = work / 'cesium.json'
        cmd = [sys.executable, '-I', str(HERE.parent / 'perf_lane/cesium.py'), '--work', str(work / 'cesium'),
               '--out', str(out_json), '--cpus', ','.join(map(str, cpus[:2])), *a.cesium_arg, f'{a.case}={output}']
        print(f'[{a.case}] cesium: {shlex.join(cmd)}', file=sys.stderr, flush=True)
        started = time.monotonic()
        proc = subprocess.run(cmd, capture_output=True, text=True)
        ces = json.loads(out_json.read_text()) if out_json.exists() else None
        case = (ces or {}).get('cases', [{}])[0] if ces else {}
        refined = case.get('refined') or {}
        report['cesium'] = dict(exit_code=proc.returncode, seconds=round(time.monotonic() - started, 2),
                                stderr_tail=proc.stderr[-2000:], json=str(out_json),
                                renderer=(ces or {}).get('environment', {}).get('renderer'),
                                loaded=case.get('loaded'), seconds_to_loaded=case.get('secondsToLoaded'),
                                refined_loaded=refined.get('loaded'),
                                refined_seconds_to_loaded=refined.get('secondsToLoaded'),
                                requests=(case.get('requests') or {}).get('requests'),
                                refined_requests=(refined.get('requests') or {}).get('requests'),
                                problems=case.get('problems'))
        metrics.update(cesium_loaded=bool(case.get('loaded')) and bool(refined.get('loaded', True)),
                       cesium_load_seconds=case.get('secondsToLoaded'),
                       cesium_refined_seconds=refined.get('secondsToLoaded'),
                       cesium_problems=len(case.get('problems') or []) if ces else None)
        if work.joinpath('cesium').exists():
            shutil.rmtree(work / 'cesium', ignore_errors=True)  # unpacked archive copy

    if facts.get('exists') and facts.get('kind') == 'file' and facts['bytes'] <= a.hash_limit_gib * sc.GiB:
        facts['sha256'] = sc.sha256_file(output)
    ran = {stage for stage in ('validate', 'cesium') if stage in report}
    results, scale = check_budgets(budget, metrics, sources, ran)
    failed = [r for r in results if r['status'] not in ('pass', 'skipped')]
    for key in ('timed_out', 'validate_timed_out'):  # a killed run never passes
        if metrics.get(key):
            failed.append(dict(metric=key, kind='expect', budget=False, budget_ref=None, value=True, status='fail'))
            results.append(failed[-1])
    below = [s for s in scale if s['status'] != 'pass']
    status = 'FAIL' if failed else ('BELOW-SCALE' if below else 'PASS')
    report.update(metrics=metrics, budget=budget, checks=results, scale_obligation=scale, status=status,
                  finished_utc=datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ'))
    if not validate_only and not a.keep_output and output.exists():
        shutil.rmtree(output) if output.is_dir() else output.unlink()
        report['output']['deleted_after_run'] = True
    leftover = [str(p) for p in scratch.rglob('*')][:20]
    report['scratch_leftover_entries'] = leftover
    shutil.rmtree(scratch, ignore_errors=True)
    (work / 'report.json').write_text(json.dumps(report, indent=2, default=str) + '\n')
    (work / 'SUMMARY.md').write_text(summary(report))
    print(json.dumps(dict(case=a.case, status=status, work=str(work),
                          failed=[f"{r['metric']} {r['kind']} {r['budget']} got {r['value']}" for r in failed],
                          below_scale=[s['metric'] for s in below])))
    return 0 if status != 'FAIL' else 1


def error_field(report, field):
    """report['error'][field] of a {"ok":false,"error":{...}} JSON report, tolerating other shapes."""
    error = report.get('error') if isinstance(report, dict) else None
    return error.get(field) if isinstance(error, dict) else None


def mib(value):
    return '-' if value is None else f'{value / sc.MiB:.1f}'


def human(metric, value):
    if value is None:
        return '-'
    if isinstance(value, bool):
        return str(value).lower()
    if isinstance(value, (int, float)) and metric.endswith('bytes'):
        return f'{value / sc.MiB:,.1f} MiB' if value < 10 * sc.GiB else f'{value / sc.GiB:,.2f} GiB'
    if isinstance(value, float):
        return f'{value:,.3f}'
    if isinstance(value, int):
        return f'{value:,}'
    return str(value)


def summary(r):
    lines = [f"# Scale gate `{r['case']}`: {r['status']}", '',
             f"- budget set `{r['budget_set']}` ({r['budgets_file']}, sha256 `{r['budgets_sha256'][:16]}`)",
             f"- binary `{r['binary']}` sha256 `{r['binary_sha256'][:16]}`",
             f"- input `{r['input']}`" + (f" ({human('bytes', r.get('input_bytes'))})" if r.get('input_bytes') else ''),
             f"- CPUs {r['cpus']}, memory mode {r['memory_mode']}, started {r['started_utc']}"]
    if r.get('producer'):
        lines.append(f"- producer: `{shlex.join(r['producer']['command'])}`")
    if r['budget'].get('provisional'):
        lines.append('- budgets are provisional planning values (frozen only after the final checks pass)')
    lines += ['', '| metric | value | budget | status |', '| --- | --- | --- | --- |']
    shown = set()
    for c in r['checks']:
        sym = {'max': '<=', 'min': '>=', 'expect': '=='}[c['kind']]
        ref = f" ({c['budget_ref']})" if c['budget_ref'] else ''
        lines.append(f"| {c['metric']} | {human(c['metric'], c['value'])} | {sym} {human(c['metric'], c['budget'])}{ref}"
                     f" | {c['status']} |")
        shown.add(c['metric'])
    for k, v in r['metrics'].items():
        if k not in shown and v is not None:
            lines.append(f'| {k} | {human(k, v)} | | |')
    if r['scale_obligation']:
        lines += ['', '| scale obligation (input manifest) | value | required | status |', '| --- | --- | --- | --- |']
        for s in r['scale_obligation']:
            lines.append(f"| {s['metric']} | {human(s['metric'], s['value'])} | >= {human(s['metric'], s['budget'])} "
                         f"| {s['status']} |")
    if r.get('validate', {}).get('report', {}) and r['metrics'].get('validate_message'):
        lines += ['', f"validate refused: `{r['metrics']['validate_error']}`: {r['metrics']['validate_message']}"]
    if r['metrics'].get('producer_error'):
        msg = error_field(r['producer'].get('report'), 'message')
        lines += ['', f"producer refused: `{r['metrics']['producer_error']}`: {msg}"]
    lines += ['', 'Scratch is a periodic allocated-bytes sample (a lower bound on the true peak). Peak RSS is '
              'wait4 ru_maxrss of the launched process; cgroup peak includes page cache.', '']
    return '\n'.join(lines)


if __name__ == '__main__':
    sys.exit(main())
