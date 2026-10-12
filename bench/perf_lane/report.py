"""Compare a head CLI with the latest release and the last milestone; regenerate REPORT.md.

Usage: python3 -I bench/perf_lane/report.py --head BIN [--milestone-label TEXT] [--explain CASE.METRIC=REASON]
Builds (once) and caches the release baseline binary, runs process.py (release vs head) and
cesium.py on both sides' outputs, writes the raw results to the local evidence directory and
rewrites bench/perf_lane/REPORT.md. The last milestone's numbers are read from the committed
REPORT.md (git HEAD) and carried forward; --milestone-label makes this run the new milestone.
Raw results never go into git. --from-raw DIR re-renders a recorded evidence directory.
"""
import argparse, datetime, fnmatch, hashlib, io, json, os, pathlib, re, shlex, shutil, subprocess, sys, tarfile, time

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
import corpus  # noqa: E402

CACHE = pathlib.Path.home() / '.cache'
REPORT = HERE / 'REPORT.md'
RATIO = 1.25  # same rule as process.py and cesium.py
# metric: (row label, process/cesium metric name for --explain, floor, gated?)
METRICS = {
    'wall': ('wall s', 'wallSeconds', 0.1, True),
    'rss': ('peak RSS MiB', 'peakRssMiB', 16, True),
    'bytes': ('output bytes', 'outputBytes', 0, True),
    'vwall': ('validate s', 'wallSeconds', 0.1, True),  # validate:<case>; data block and flags only
    'load': ('Cesium load (console errors)', None, None, False),
    'errors': ('console errors', None, None, False),
    'tiles': ('tile requests', 'requests', 2, False),  # advisory, as cesium.py --baseline
    'secs': ('s to tiles-loaded', 'secondsToLoaded', 1.0, False),
}
ROWS = ['wall', 'rss', 'bytes', 'load', 'tiles', 'secs']  # 'load' shows the console error count too
PROCESS_METRICS = {'wall', 'rss', 'bytes', 'vwall'}
BLOCK_START, BLOCK_END = '<!-- perf-lane milestone data (read by report.py; do not edit)', '-->'
BLOCK_PREFIX = '<!-- perf-lane milestone data'


# ---------- extraction ----------

def side_metrics(process, cesium, side):
    """{case: metrics} for one side ('base' or 'head') of a process.py report plus a cesium.py report."""
    by_case = {c['case']: c for c in process['cases']}
    loads = {c['name']: c for c in (cesium or {}).get('cases', [])}
    out = {}
    for name, case in by_case.items():
        if name.startswith('validate:'): continue
        data = case['sides'][side]
        if 'skipped' in data and 'ok' not in data:
            out[name] = dict(na=data['skipped'] or 'not run'); continue
        if not data.get('ok'):
            lines = [line.strip() for line in data.get('log', '').splitlines() if line.strip()]
            code = data['samples'][-1]['exitCode']
            error = next((i for i, line in enumerate(lines) if line.startswith('error:')), None)
            if code == 2 and error is not None:  # clap usage error: this CLI takes different arguments
                out[name] = dict(na='usage error: ' + ' '.join(lines[error:error + 2])[:160]); continue
            out[name] = dict(failed=f"exit {code}: {(lines[error] if error is not None else lines[-1] if lines else 'no log')[:120]}")
            continue
        m = dict(wall=data['wallSeconds'], rss=data['peakRssMiB'], bytes=data['outputBytes'])
        check = by_case.get(f'validate:{name}', {}).get('sides', {}).get(side, {})
        if check.get('ok'): m['vwall'] = check['wallSeconds']
        elif check.get('samples'): m['vfailed'] = True
        load = loads.get(name)
        if load:
            r = load['requests']
            m.update(load=bool(load.get('loaded')), errors=len(load.get('console', {}).get('errors', [])),
                     tiles=sum(r[k]['requests'] for k in ('content', 'subtree', 'imagery')),
                     secs=load.get('secondsToLoaded'), problems=list(load.get('problems', [])))
        out[name] = m
    return out


def usable(m):
    return bool(m) and 'na' not in m and 'failed' not in m


# ---------- comparison ----------

def pct(old, new):
    if old is None or new is None or isinstance(old, bool): return ''
    if old == 0: return '' if new == 0 else 'new'
    return f'{(new - old) / old * 100:+.1f}%'


def explained(pattern_pairs, key):
    return next((reason for pattern, reason in pattern_pairs if fnmatch.fnmatch(key, pattern)), None)


def compare(case, ref_name, ref, head, explanations):
    """Flags for one case against one reference column (same rule as process.py: >RATIO and > floor)."""
    flags = []
    if 'failed' in head: return flags  # all_flags flags a head failure once, whatever the references
    if not usable(ref): return flags
    if not usable(head):
        key = f'{case}.exit'
        return [dict(case=case, ref=ref_name, metric='exit', key=key, gated=True, explanation=explained(explanations, key),
                     text=f"head {head.get('na') or head.get('failed') or 'missing'}")]
    if head.get('vfailed') and 'vwall' in ref:
        key = f'validate:{case}.exit'
        flags.append(dict(case=case, ref=ref_name, metric='vwall', key=key, gated=True,
                          explanation=explained(explanations, key), text='head validate failed'))
    for metric, (label, name, floor, gated) in METRICS.items():
        if name is None: continue
        old, new = ref.get(metric), head.get(metric)
        if old is None or new is None or not (old > 0 and new > old * RATIO and new - old > floor): continue
        key = f"{'validate:' if metric == 'vwall' else ''}{case}.{name}"
        flags.append(dict(case=case, ref=ref_name, metric=metric, key=key, gated=gated, explanation=explained(explanations, key),
                          text=f'{label} {fmt(metric, old)} -> {fmt(metric, new)} ({pct(old, new)})'))
    return flags


def all_flags(columns, explanations):
    """columns: {'release': {case: m} | None, 'milestone': {...} | None, 'head': {...}}; returns (flags, gate failures)."""
    gate_ref = 'milestone' if columns.get('milestone') else 'release'
    flags = []
    for case, head in columns['head'].items():
        if 'failed' in head:  # a head failure is gated even with no usable reference column
            key = f'{case}.exit'
            flags.append(dict(case=case, ref='head', metric='exit', key=key, gated=True, explanation=explained(explanations, key),
                              text=f"head failed: {head['failed']}"))
        for ref_name in ('milestone', 'release'):
            if columns.get(ref_name): flags += compare(case, ref_name, columns[ref_name].get(case), head, explanations)
        for problem in head.get('problems', []):
            flags.append(dict(case=case, ref='cesium', metric='load', key=f'{case}.cesium', gated=True, explanation=None,
                              text=f'Cesium: {problem}'))
    for flag in flags:
        flag['fails'] = flag['gated'] and not flag['explanation'] and flag['ref'] in (gate_ref, 'cesium', 'head')
    return flags, [f for f in flags if f['fails']]


# ---------- rendering ----------

def fmt(metric, value):
    if value is None: return '—'
    if metric in ('bytes', 'errors', 'tiles'): return f'{int(value):,}'
    if metric == 'rss': return f'{value:.1f}'
    if metric == 'secs': return f'{value:.2f}'
    return f'{value:.3f}'


def cell(metric, m):
    if m is None: return '—'
    if 'na' in m: return 'n/a'
    if 'failed' in m: return 'failed'
    if metric == 'load':
        return '—' if m.get('load') is None else f"{'ok' if m['load'] else 'FAIL'} ({fmt('errors', m.get('errors'))})"
    return fmt(metric, m.get(metric))


def delta(metric, ref, head, bold):
    if not usable(ref) or not usable(head): return ''
    if metric == 'load':
        old, new = ref.get('load'), head.get('load')
        errors = (head.get('errors') or 0) - (ref.get('errors') or 0)
        text = ('' if old is None or new is None else 'changed' if old != new
                else f'{errors:+d} errors' if errors else '')
    else:
        text = pct(ref.get(metric), head.get(metric))
    return f'**{text}**' if bold and text else text


def case_table(case, columns, flagged, tag='release'):
    release, milestone, head = (columns.get(k, {}) or {} for k in ('release', 'milestone', 'head'))
    r, m, h = release.get(case) if columns.get('release') else None, milestone.get(case), head.get(case)
    lines = [f'| {case} | {tag} | last milestone | head | Δ {tag} | Δ milestone |', '| --- | ---: | ---: | ---: | ---: | ---: |']
    for metric in ROWS:
        if metric in ('load', 'tiles', 'secs') and not any(x and metric in x for x in (r, m, h)): continue
        lines.append(f"| {METRICS[metric][0]} | {cell(metric, r)} | {cell(metric, m)} | {cell(metric, h)} | "
                     f"{delta(metric, r, h, (case, metric, 'release') in flagged)} | "
                     f"{delta(metric, m, h, (case, metric, 'milestone') in flagged)} |")
    return lines


def chromium(path):
    found = re.search(r'chromium-(\d+)', path or '')
    return found.group(1) if found else (path or '—')


def headline(head):
    ran = [m for m in head.values() if usable(m)]
    loads = [m for m in ran if 'load' in m]
    return dict(wall=sum(m['wall'] for m in ran), rss=max((m['rss'] for m in ran), default=0),
                mib=sum(m['bytes'] for m in ran) / 2**20, ok=sum(1 for m in loads if m['load'] and not m.get('problems')),
                loads=len(loads), secs=sum(m.get('secs') or 0 for m in loads))


def milestone_row(meta, head):
    h = headline(head)
    return (f"| {meta['date']} | `{meta['head']['commit'][:7]}` | {meta['label']} | {h['wall']:.2f} | {h['rss']:.1f} | "
            f"{h['mib']:.1f} | {h['ok']}/{h['loads']} | {h['secs']:.1f} |")


def encode_block(meta, head):
    keep = ('wall', 'rss', 'bytes', 'vwall', 'load', 'errors', 'tiles', 'secs', 'na', 'failed')
    lines = [BLOCK_START, 'milestone ' + json.dumps(dict(commit=meta['head']['commit'], label=meta['label'], date=meta['date']))]
    for case, m in head.items():
        lines.append(f'case {case} ' + json.dumps({k: round(v, 4) if isinstance(v, float) else v
                                                   for k, v in m.items() if k in keep}, sort_keys=True))
    return lines + [BLOCK_END]


def parse_previous(text):
    """(milestone info, {case: metrics}, [milestone table rows]) from an earlier REPORT.md.

    Tolerates hand edits (indentation, a changed marker or heading suffix, rows without the
    space after '|', note lines) but stops when the data block or the rows are missing or
    disagree, rather than silently dropping the last milestone."""
    info, cases, rows = None, {}, []
    if not text or not text.strip(): return info, cases, rows
    lines = [line.strip() for line in text.splitlines()]
    fail = lambda why: SystemExit(f'report: previous REPORT.md: {why} (fix it from git history, or pass '
                                  '--previous /dev/null to start the milestone log over)')
    starts = [i for i, line in enumerate(lines) if line.startswith(BLOCK_PREFIX)]
    if len(starts) != 1: raise fail(f'expected one milestone data block, found {len(starts)}')
    number = starts[0] + 1
    for number, line in enumerate(lines[starts[0] + 1:], starts[0] + 2):
        if line.startswith(BLOCK_END): break
        if not line: continue
        kind, _, rest = line.partition(' ')
        try:
            if kind == 'milestone': info = json.loads(rest)
            elif kind == 'case':
                name, _, payload = rest.partition(' '); cases[name] = json.loads(payload)
            else: raise fail(f'line {number}: unrecognised data line {line[:60]!r}')
        except json.JSONDecodeError as error:
            raise fail(f'line {number}: bad JSON ({error})')
    else:
        raise fail('milestone data block is not closed with -->')
    if not info or not cases: raise fail('milestone data block lacks the milestone line or its case lines')
    headings = [i for i, line in enumerate(lines) if line.startswith('## Milestones')]
    if len(headings) != 1: raise fail(f"expected one '## Milestones' section, found {len(headings)}")
    for number, line in enumerate(lines[headings[0] + 1:], headings[0] + 2):
        if line.startswith('## ') or line.startswith(BLOCK_PREFIX): break
        if not line.startswith('|'): continue  # blank or note line
        cells = [cell.strip() for cell in line.strip('|').split('|')]
        if cells[0].lower() == 'date' or all(set(cell) <= set('-: ') for cell in cells): continue
        if not re.fullmatch(r'\d{4}-\d{2}-\d{2}', cells[0]) or len(cells) < 3:
            raise fail(f'line {number}: unrecognised Milestones row {line[:60]!r}')
        rows.append('| ' + ' | '.join(cells) + ' |')
    if not rows: raise fail("'## Milestones' has no rows")
    if not any(row.split('|')[2].strip().strip('`') == info['commit'][:7] for row in rows):
        raise fail(f"no Milestones row for the data block's commit {info['commit'][:7]}")
    return info, cases, rows


def render(meta, columns, previous, explanations):
    """Markdown REPORT.md text. previous = parse_previous() of the committed report."""
    info, _, rows = previous
    flags, failures = all_flags(columns, explanations)
    flagged = {(f['case'], f['metric'], f['ref']) for f in flags}
    env, ces = meta['environment'], meta.get('cesium') or {}
    release = meta.get('release')
    tag = release['tag'] if release else 'release'
    out = ['# Performance lane report', '',
           'Generated by `bench/perf_lane/report.py`; see the README for when to commit it. Raw results stay local.', '',
           f"- Head: `{meta['head']['commit'][:12]}`, recorded {meta['date']}"
           + (f" (milestone: {meta['label']})" if meta.get('label') else ' (not a milestone)')
           + f"; binary sha256 `{meta['head']['sha256'][:16]}`.",
           (f"- {tag} column: tag commit `{release['commit'][:12]}`, binary sha256 `{release['sha256'][:16]}`, "
            f"{release['build']}." if release else '- Release column: not recorded in this run.'),
           f"- Last milestone: " + (f"`{info['commit'][:12]}` ({info['label']}, {info['date']})." if info else 'none.'),
           f"- CPU: {env['cpu']}; pinned to CPUs {','.join(map(str, env['cpuAffinity']))} at nice {env['niceness']}, "
           f"RAYON_NUM_THREADS=2; best of {env['repeat']} ABBA runs; "
           + (f"contended best samples (other work >20% of the wall time): {', '.join(meta['contended'])}."
              if meta.get('contended') else 'no contended best samples.'),
           *([f"- Per-core idle over 5 s before each step (need ≥{meta.get('minIdle', 0.9):.0%}): " + '; '.join(
               f"{step} CPUs {','.join(map(str, v['cpus']))} " + '/'.join(f'{x:.0%}' for x in v['idle'].values())
               for step, v in meta['pinned'].items()) + '.'] if meta.get('pinned') else []),
           f"- Cesium {ces.get('cesium', '—')}, Playwright {ces.get('playwright', '—')}, Chromium "
           f"{chromium(ces.get('chromium'))} (SwiftShader, 1024×768).",
           f"- Corpus sha256 `{meta['corpus'][:16]}` (pinned `corpus.py` inputs and the T1 fixture).",
           f"- Gate: >{RATIO:g}× worse than the last milestone and over the floor (0.1 s, 16 MiB, 0 B), unless "
           '`--explain`ed; any Cesium load failure, console error or failed request. Cesium time and tile '
           f'requests are advisory; flags against {tag} are reported, not gated.',
           f"- Raw results bundle: `{meta['bundle']['path']}`, sha256 `{meta['bundle']['sha256']}`.", '', '## Flags', '']
    if not flags: out.append('None.')
    for f in sorted(flags, key=lambda f: (not f['fails'], f['case'], f['ref'])):
        verdict = ('REGRESSION' if f['fails'] else f"explained: {f['explanation']}" if f['explanation']
               else 'advisory' if not f['gated'] else 'not gated')
        ref = {'release': f'vs {tag}', 'milestone': 'vs milestone', 'cesium': 'head', 'head': 'head'}[f['ref']]
        out.append(f"- {f['case']} {ref}: {f['text']} — {verdict}")
    grouped = {}  # (column, reason): [cases]
    for col, label in (('release', tag), ('head', 'head')):
        for case, m in (columns.get(col) or {}).items():
            if m and ('na' in m or 'failed' in m): grouped.setdefault((label, m.get('na') or m.get('failed')), []).append(case)
    if grouped:
        out += ['', '## Not run or failed', '', *(f"- {label}: {reason} ({', '.join(cases)})"
                                                  for (label, reason), cases in grouped.items())]
    out += ['', '## Cases', '']
    for case in columns['head']:
        out += case_table(case, columns, flagged, tag) + ['']
    if meta.get('label'): rows = [r for r in rows if f"`{meta['head']['commit'][:7]}`" not in r] + [milestone_row(meta, columns['head'])]
    out += ['## Milestones', '',
            '| date | commit | label | head Σ wall s | max RSS MiB | Σ output MiB | Cesium ok | Σ s to loaded |',
            '| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |', *rows, '']
    block = encode_block(meta, columns['head']) if meta.get('label') else (
        [BLOCK_START, 'milestone ' + json.dumps(info), *(f'case {k} ' + json.dumps(v, sort_keys=True)
                                                         for k, v in columns['milestone'].items()), BLOCK_END] if info else [])
    return '\n'.join(out + block) + '\n', failures


# ---------- running ----------

def run(command, **kwargs):
    print('report:', ' '.join(map(str, command)), file=sys.stderr, flush=True)
    return subprocess.run([str(c) for c in command], **kwargs)


def sha256(path):
    return corpus.sha256(pathlib.Path(path))


def latest_tag():
    tags = subprocess.run(['git', '-C', ROOT, 'tag', '--list', 'v[0-9]*', '--sort=-v:refname'], capture_output=True,
                          text=True, check=True).stdout.split()
    if not tags: raise SystemExit('report: no release tag found')
    return tags[0]


def baseline_binary(tag, cache, target, wrap):
    """The release CLI, built once from `git archive TAG` and reused (BUILD.json records how)."""
    home = cache / tag
    binary, record = home / 'rusty-tiles', home / 'BUILD.json'
    if binary.exists() and record.exists():
        build = json.loads(record.read_text())
        if build['sha256'] != sha256(binary): raise SystemExit(f'report: {binary} does not match {record}')
        return binary, build
    commit = subprocess.run(['git', '-C', ROOT, 'rev-parse', f'{tag}^{{commit}}'], capture_output=True, text=True,
                            check=True).stdout.strip()
    source = home / 'src'
    shutil.rmtree(source, ignore_errors=True); source.mkdir(parents=True)
    archive = subprocess.run(['git', '-C', ROOT, 'archive', '--format=tar', commit], capture_output=True, check=True).stdout
    with tarfile.open(fileobj=io.BytesIO(archive)) as tar: tar.extractall(source, filter='data')
    features = ['--release', '--locked', '--features', 'native-geospatial', '--bin', 'rusty-tiles']
    # CONTRIBUTING.md: native builds share the system SQLite with GDAL/PROJ.
    environment = dict(os.environ, CARGO_BUILD_JOBS='2', CARGO_TARGET_DIR=str(target), LIBSQLITE3_SYS_USE_PKG_CONFIG='1')
    run([*wrap, 'cargo', 'build', *features], cwd=source, env=environment, check=True)
    shutil.copy2(target / 'release' / 'rusty-tiles', binary)
    rustc = subprocess.run(['rustc', '--version'], capture_output=True, text=True).stdout.strip()
    build = dict(tag=tag, commit=commit, sha256=sha256(binary), rustc=rustc, cargo=f"cargo build {' '.join(features)}",
                 builtAtUtc=datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='seconds'),
                 build=f"`cargo build {' '.join(features[:4])}` with {rustc}")
    record.write_text(json.dumps(build, indent=2) + '\n')
    shutil.rmtree(source)
    return binary, build


def cpu_ticks():
    """{cpu: (idle ticks, total ticks)} from /proc/stat; idle includes iowait."""
    ticks = {}
    for line in pathlib.Path('/proc/stat').read_text().splitlines():
        name, *values = line.split()
        if name[3:].isdigit():
            values = [int(v) for v in values]; ticks[int(name[3:])] = (values[3] + values[4], sum(values[:8]))
    return ticks


def idle_fractions(cpus, seconds=5.0):
    """Per-core idle fraction of the given CPUs over a short /proc/stat sample."""
    before = cpu_ticks(); time.sleep(seconds); after = cpu_ticks()
    return {c: (after[c][0] - before[c][0]) / max(after[c][1] - before[c][1], 1) for c in cpus}


def sibling_pairs(pool):
    """Hyperthread sibling pairs inside the pool (from sysfs), else consecutive pairs."""
    pairs = []
    for cpu in sorted(pool):
        try:
            text = pathlib.Path(f'/sys/devices/system/cpu/cpu{cpu}/topology/thread_siblings_list').read_text().strip()
            siblings = tuple(sorted(int(x) for part in text.split(',') for x in
                                    (range(int(part.split('-')[0]), int(part.split('-')[-1]) + 1))))
        except (OSError, ValueError):
            siblings = ()
        pair = siblings if len(siblings) == 2 and set(siblings) <= pool else None
        if pair and pair not in pairs: pairs.append(pair)
    if not pairs:
        ordered = sorted(pool); pairs = [tuple(ordered[i:i + 2]) for i in range(0, len(ordered) - 1, 2)]
    return pairs


def pick_pair(idle, pairs, current, min_idle):
    """Keep the current pair if both cores are idle enough, else the idlest qualifying pair, else None."""
    ok = [pair for pair in pairs if all(idle[c] >= min_idle for c in pair)]
    if current in ok: return current
    return max(ok, key=lambda pair: min(idle[c] for c in pair), default=None)


def wait_for_idle(state, step, pairs, min_idle, interval, give_up):
    """Before a benchmark step: wait until a CPU pair from the pool is at least min_idle idle per core
    over a 5 s sample (keeping the previous pair when it qualifies) and record the figures."""
    deadline, pool = time.monotonic() + give_up, sorted({c for pair in pairs for c in pair})
    while True:
        idle = idle_fractions(pool)
        pair = pick_pair(idle, pairs, state.get('pair'), min_idle)
        if pair:
            state['pair'] = pair
            state['steps'].append(dict(step=step, cpus=list(pair), idle={str(c): round(idle[c], 3) for c in pair}))
            print(f'report: {step} on CPUs {pair}, idle ' + ', '.join(f'{idle[c]:.0%}' for c in pair), file=sys.stderr, flush=True)
            return ','.join(map(str, pair))
        figures = ' '.join(f'{c}:{idle[c]:.0%}' for c in pool)
        if time.monotonic() > deadline: raise SystemExit(f'report: no CPU pair {min_idle:.0%} idle for {give_up:g} s ({figures})')
        print(f'report: no pair {min_idle:.0%} idle (idle {figures}); waiting {interval:g} s', file=sys.stderr, flush=True)
        time.sleep(interval)


def contended(process):
    """Cases whose best-wall sample had other work using >20% of it on the pinned CPUs (process.py's rule)."""
    return sorted(f"{c['case']} {side}" for c in process['cases'] for side, v in c['sides'].items()
                  if v.get('ok') and v['foreignCpuSecondsAtBestWall'] > 0.2 * v['wallSeconds'])


BUNDLES = ('raw.tar.zst', 'raw.tar.gz')  # tarfile 'w:zst' needs Python 3.14+; older ones use the zstd CLI or gzip


def bundle(directory, stdlib_zstd=None):
    """Bundle every other file in the evidence directory with fixed metadata; returns the bundle path.

    raw.tar.zst via tarfile (Python 3.14+) or the zstd CLI, else raw.tar.gz (gzip mtime 0)."""
    if stdlib_zstd is None: stdlib_zstd = 'zst' in tarfile.TarFile.OPEN_METH
    def tidy(info):
        info.mtime, info.uid, info.gid, info.uname, info.gname = 0, 0, 0, '', ''
        return info
    files = sorted(p for p in directory.rglob('*') if p.is_file() and p.name not in (*BUNDLES, 'raw.tar', 'REPORT.md'))
    def write(target, mode):
        with tarfile.open(target, mode) as tar:
            for path in files: tar.add(path, arcname=str(path.relative_to(directory)), filter=tidy)
        return target
    if stdlib_zstd: return write(directory / 'raw.tar.zst', 'w:zst')
    plain = write(directory / 'raw.tar', 'w')
    if shutil.which('zstd'):
        subprocess.run(['zstd', '-q', '-19', '--rm', '-o', str(directory / 'raw.tar.zst'), str(plain)], check=True)
        return directory / 'raw.tar.zst'
    import gzip
    with open(plain, 'rb') as source, gzip.GzipFile(directory / 'raw.tar.gz', 'wb', mtime=0) as target:
        shutil.copyfileobj(source, target)
    plain.unlink()
    return directory / 'raw.tar.gz'


def corpus_digest(environment_inputs):
    return hashlib.sha256(''.join(f'{k}:{v}\n' for k, v in sorted(environment_inputs.items())).encode()).hexdigest()


def collect(args, explanations):
    """Run the lane and return the evidence directory."""
    wrap = shlex.split(args.cargo_wrap)
    tag = args.baseline_tag or latest_tag()
    target = args.cargo_target_dir or (args.work / f'target-{tag}')
    base, build = baseline_binary(tag, args.baseline_cache, target, wrap)
    if not args.cargo_target_dir: shutil.rmtree(target, ignore_errors=True)
    head = args.head.resolve()
    commit = args.head_commit or subprocess.run(['git', '-C', ROOT, 'rev-parse', 'HEAD'], capture_output=True, text=True,
                                                check=True).stdout.strip()
    date = datetime.datetime.now(datetime.timezone.utc).date().isoformat()
    evidence = args.evidence / f'{date}-{commit[:7]}'
    for n in range(2, 100):
        if not evidence.exists(): break
        evidence = args.evidence / f'{date}-{commit[:7]}.{n}'
    evidence.mkdir(parents=True)
    work = args.work / evidence.name
    shutil.rmtree(work, ignore_errors=True)
    py = [sys.executable, '-I']
    pool = {int(c) for part in args.cpu_pool.split(',') for c in
            range(int(part.split('-')[0]), int(part.split('-')[-1]) + 1)}
    pairs = [tuple(int(c) for c in args.cpus.split(','))] if args.cpus else sibling_pairs(pool)
    quiet = dict(pair=None, steps=[])
    idle = lambda step: wait_for_idle(quiet, step, pairs, args.min_idle, args.idle_interval, args.idle_give_up)
    best = None
    (work / 'inputs').mkdir(parents=True)  # one corpus, shared by every attempt
    for attempt in range(1, args.attempts + 1):  # rerun while other work contended the pinned CPUs
        cpus = idle(f'process attempt {attempt}')
        out, attempt_work = evidence / f'process-attempt{attempt}.json', work / f'process-attempt{attempt}'
        attempt_work.mkdir(); (attempt_work / 'inputs').symlink_to(work / 'inputs')
        process = run([*py, HERE / 'process.py', '--base', base, '--head', head, '--work', attempt_work, '--out', out,
                       '--repeat', args.repeat, '--cpus', cpus, *(x for e in args.explain for x in ('--explain', e))])
        if process.returncode not in (0, 1) or not out.exists(): raise SystemExit('report: process.py failed')
        noisy = contended(json.loads(out.read_text()))
        print(f'report: attempt {attempt}: {len(noisy)} contended case sides {noisy}', file=sys.stderr, flush=True)
        if best is None or len(noisy) < best[1]:
            if best: shutil.rmtree(best[3], ignore_errors=True)
            best = (out, len(noisy), quiet['steps'][-1], attempt_work)
        else:
            shutil.rmtree(attempt_work, ignore_errors=True)
        if not noisy: break
    shutil.copy2(best[0], evidence / 'process.json')
    kept = best[3]  # Cesium and the copied logs use the outputs of the attempt whose numbers are reported
    pinned = dict(process=best[2])
    result = json.loads((evidence / 'process.json').read_text())
    hole = ','.join(map(str, result['environment']['rasterHoleDegrees']))
    for side in ('base', 'head'):
        targets = []
        for case in result['cases']:
            data = case['sides'][side]
            if case['case'].startswith('validate:') or not data.get('ok'): continue
            name = pathlib.PurePath(case['argv'][case['argv'].index('-o') + 1]).name
            targets.append(f"{case['case']}={kept / side / name}")
        if not targets: continue
        cpus = idle(f'cesium {side}')
        pinned[f'cesium-{side}'] = quiet['steps'][-1]
        check = run([*py, HERE / 'cesium.py', '--work', work / f'cesium-{side}', '--out', evidence / f'cesium-{side}.json',
                     '--hole', hole, '--cpus', cpus, *targets])
        if check.returncode not in (0, 1) or not (evidence / f'cesium-{side}.json').exists():
            raise SystemExit(f'report: cesium.py failed for {side}')
        shutil.copytree(work / f'cesium-{side}' / 'screenshots', evidence / f'screenshots-{side}')
        logs = evidence / f'logs-{side}'; logs.mkdir()
        for log in (kept / side).glob('*.log'): shutil.copy2(log, logs / log.name.replace(':', '_'))
    meta = dict(date=date, label=args.milestone_label, corpus=corpus_digest(result['environment']['inputs']),
                head=dict(commit=commit, sha256=sha256(head), path=str(head)),
                release=dict(tag=tag, commit=build['commit'], sha256=build['sha256'], build=build['build'], path=str(base)),
                explanations=args.explain, pinned=pinned, idleChecks=quiet['steps'], minIdle=args.min_idle)
    (evidence / 'meta.json').write_text(json.dumps(meta, indent=2) + '\n')
    if not args.keep_work: shutil.rmtree(work, ignore_errors=True)
    return evidence


def merge_explanations(recorded, given):
    """--explain pairs: those recorded with the run (meta.json) plus new ones; a new reason for the same
    pattern wins, because explained() takes the first match."""
    pairs = [item.split('=', 1) for item in given] + [item.split('=', 1) for item in recorded]
    seen, merged = set(), []
    for pattern, reason in pairs:
        if pattern not in seen: seen.add(pattern); merged.append([pattern, reason])
    return merged


def load_raw(directory):
    read = lambda name: json.loads((directory / name).read_text()) if (directory / name).exists() else None
    meta, process = read('meta.json'), read('process.json')
    columns = dict(head=side_metrics(process, read('cesium-head.json'), 'head'),
                   release=side_metrics(process, read('cesium-base.json'), 'base') if meta.get('release') else None)
    meta['environment'] = process['environment']
    meta['contended'] = contended(process)
    meta.setdefault('corpus', corpus_digest(process['environment']['inputs']))
    meta['cesium'] = (read('cesium-head.json') or {}).get('environment')
    if 'bundle' not in meta:
        path = next((directory / name for name in BUNDLES if (directory / name).exists()), None) or bundle(directory)
        meta['bundle'] = dict(path=str(path).replace(str(pathlib.Path.home()), '~'), sha256=sha256(path))
    return meta, columns


def committed_report(path):
    relative = path.resolve().relative_to(ROOT)
    shown = subprocess.run(['git', '-C', ROOT, 'show', f'HEAD:{relative}'], capture_output=True, text=True)
    if shown.returncode == 0: return shown.stdout
    return path.read_text() if path.exists() else ''


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--head', type=pathlib.Path, help='head rusty-tiles binary (a native-geospatial release build)')
    parser.add_argument('--head-commit', help='commit the head binary was built from (default: git HEAD)')
    parser.add_argument('--milestone-label', help='make this run the new milestone and append a Milestones row')
    parser.add_argument('--explain', action='append', default=[], metavar='CASE.METRIC=REASON',
                        help='stated reason for an expected regression (fnmatch pattern, as process.py)')
    parser.add_argument('--baseline-tag', help='release tag for the v0.3.0 column (default: latest v* tag)')
    parser.add_argument('--baseline-cache', type=pathlib.Path, default=CACHE / 'perf-lane/baselines')
    parser.add_argument('--cargo-target-dir', type=pathlib.Path, help='target dir for the one-off baseline build')
    parser.add_argument('--cargo-wrap', default=f'flock {CACHE}/rusty-tiles-cargo.lock nice -n 10 taskset -c 0,1',
                        help='command prefix for cargo (shared build lock, nice, two CPUs)')
    parser.add_argument('--work', type=pathlib.Path, default=CACHE / 'perf-lane/work/report')
    parser.add_argument('--evidence', type=pathlib.Path, default=CACHE / 'rusty-tiles-evidence/perf')
    parser.add_argument('--report', type=pathlib.Path, default=REPORT)
    parser.add_argument('--previous', type=pathlib.Path, help='earlier REPORT.md (default: the committed one at git HEAD)')
    parser.add_argument('--from-raw', type=pathlib.Path, help='re-render a recorded evidence directory instead of running')
    parser.add_argument('--repeat', type=int, default=3)
    parser.add_argument('--cpu-pool', default='4-7,12-15',
                        help='CPUs to benchmark on, as sibling pairs (cargo build locks use 0-3 and 8-11)')
    parser.add_argument('--cpus', help='fixed CPU pair instead of choosing an idle pair from --cpu-pool')
    parser.add_argument('--min-idle', type=float, default=0.9, help='per-core idle fraction required before each step')
    parser.add_argument('--idle-interval', type=float, default=30)
    parser.add_argument('--idle-give-up', type=float, default=4 * 3600)
    parser.add_argument('--attempts', type=int, default=4, help='process.py runs while best samples are contended')
    parser.add_argument('--keep-work', action='store_true', help='keep the outputs under --work')
    args = parser.parse_args()
    explanations = [item.split('=', 1) for item in args.explain]
    if any(len(pair) != 2 or not pair[1] for pair in explanations): parser.error('--explain needs CASE.METRIC=REASON')
    if not args.from_raw and not args.head: parser.error('--head is required unless --from-raw is given')
    evidence = args.from_raw.resolve() if args.from_raw else collect(args, explanations)
    meta, columns = load_raw(evidence)
    explanations = merge_explanations(meta.get('explanations', []), args.explain)
    if args.milestone_label: meta['label'] = args.milestone_label
    previous = parse_previous(args.previous.read_text() if args.previous else committed_report(args.report))
    columns['milestone'] = previous[1] or None
    text, failures = render(meta, columns, previous, explanations)
    args.report.write_text(text)
    (evidence / 'REPORT.md').write_text(text)
    print(f'report: wrote {args.report} ({text.count(chr(10))} lines); raw results in {evidence}', file=sys.stderr)
    for f in failures: print(f"report: REGRESSION {f['case']}: {f['text']}", file=sys.stderr)
    sys.exit(1 if failures else 0)


if __name__ == '__main__':
    main()
