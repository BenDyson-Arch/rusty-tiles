"""Per-frame flythrough statistics and the FLYTHROUGH.md renderer.

Usage: python3 -I bench/perf_lane/flythrough_report.py --from DIR [--out FLYTHROUGH.md] [--images]
DIR is a flythrough.py evidence directory (flythrough.json plus runs/ and screenshots/). Without --out
the evidence copy DIR/FLYTHROUGH.md is rewritten; --images embeds the waypoint screenshots by a path
relative to the output file (use it only for copies outside git).

Comparisons are made only within one recipe set (one dataset), one profile and one renderer class.
Each run's host load is recorded and a run is marked noisy when it crosses NOISE; aggregates use the
clean runs when at least two are clean, else every run, and say which.
"""
import argparse, json, os, pathlib, statistics

HITCH_MS = (33, 100)  # frame intervals strictly above these count as hitches
# A LOD pop: the selected set changes by at least this many tiles and this fraction of the previous
# selection in one frame; an SSE jump: the largest selected-tile SSE changes by at least the ratio and px.
POP_MIN_TILES, POP_FRACTION = 4, 0.5
SSE_JUMP_RATIO, SSE_JUMP_PX = 2.0, 8.0
RATE_WINDOW_MS = 1000
# Host load over one run that marks it noisy: pressure is the PSI 'some' stall share of the run's wall
# time, memory is the lowest MemAvailable seen, gpu is another process's utilisation before the run.
NOISE = dict(cpuPressure=0.25, memoryPressure=0.02, ioPressure=0.10, memAvailableMinGiB=4.0, gpuUtilStart=30.0)
# A metric is stable across repeats when max - min <= max(STABLE_SPREAD * median, its floor).
STABLE_SPREAD = 0.10
FLOORS = {'Ms': 1.0, 'Seconds': 0.05, 'MiB': 0.25}
COUNT_FLOOR = 2
STABILITY_METRICS = ('frameP50Ms', 'frameP95Ms', 'frameP99Ms', 'hitches33', 'firstRenderSeconds', 'fullQualitySeconds',
                     'requests', 'requestMiB', 'drawCallsP50', 'trianglesP50', 'gpuMemoryPeakMiB', 'lodPops')
MIB = 1024 * 1024


def percentile(values, q):
    """Linear-interpolated percentile (NumPy's default 'linear', Hyndman-Fan type 7); None when empty."""
    data = sorted(v for v in values if v is not None)
    if not data: return None
    position = (len(data) - 1) * q / 100
    low = int(position)
    high = min(low + 1, len(data) - 1)
    return data[low] + (data[high] - data[low]) * (position - low)


def basis(frames, waypoints):
    """('motion' or 'hold', per-frame mask) for the headline metrics. Motion frames (hold == 0) when the path
    has any; otherwise (snap-teleport) hold frames, without the first frame of each waypoint, which is the
    cut itself. A gap frame (harness work such as a screenshot before it) is never in the basis."""
    n = len(frames['t'])
    holds, gaps = frames.get('hold') or [0] * n, frames.get('gap') or [0] * n
    if any(not h for h in holds): return 'motion', [not h and not g for h, g in zip(holds, gaps)]
    cuts = {w['frame'] for w in waypoints}
    return 'hold', [i not in cuts and not g for i, g in enumerate(gaps)]


def intervals(times, mask=None):
    """Presented-frame intervals in ms: frame i contributes times[i] - times[i - 1] when mask[i] holds."""
    return [times[i] - times[i - 1] for i in range(1, len(times)) if mask is None or mask[i]]


def hitches(values, threshold):
    return sum(1 for v in values if v > threshold)


def peak_rate(times, counts, window_ms=RATE_WINDOW_MS):
    """Largest sum of counts over any window_ms span of frame times, scaled to per second."""
    best, total, low = 0, 0, 0
    for high in range(len(times)):
        total += counts[high]
        while times[high] - times[low] >= window_ms:
            total -= counts[low]; low += 1
        best = max(best, total)
    return best * 1000 / window_ms


def pops(selected, added, removed, mask=None):
    """Frames (in mask) whose selected-tile set changed by at least POP_MIN_TILES and POP_FRACTION of the
    previous one."""
    found = 0
    for i in range(1, len(selected)):
        if mask is not None and not mask[i]: continue
        churn = added[i] + removed[i]
        if selected[i - 1] > 0 and churn >= max(POP_MIN_TILES, POP_FRACTION * selected[i - 1]): found += 1
    return found


def sse_jumps(values, mask=None):
    """Frames (in mask) where the largest selected-tile screen-space error moved from the previous frame by
    SSE_JUMP_RATIO and SSE_JUMP_PX."""
    found = 0
    for i in range(1, len(values)):
        if mask is not None and not mask[i]: continue
        a, b = values[i - 1], values[i]
        if not a or not b: continue
        if max(a, b) / min(a, b) >= SSE_JUMP_RATIO and abs(a - b) >= SSE_JUMP_PX: found += 1
    return found


def summarize(run):
    """Scalar metrics of one run. Frame, CPU and GPU times, hitches, pops and SSE jumps use the headline
    basis (motion frames, or hold frames for a path without motion); allFrame* use every non-gap frame."""
    f, waypoints = run['frames'], run.get('waypoints', [])
    times = f['t']
    kind, mask = basis(f, waypoints)
    every = [not g for g in (f.get('gap') or [0] * len(times))]
    frame, whole = intervals(times, mask), intervals(times, every)
    pick = lambda column: [v for v, keep in zip(f.get(column) or [], mask) if keep and v is not None]
    reached = [w for w in waypoints if w.get('reached')]
    full = [w['fullQualitySeconds'] for w in reached]
    memory = [(g or 0) + (t or 0) for g, t in zip(f['geometryBytes'], f['textureBytes'])]
    cpu, gpu = pick('cpu'), pick('gpu')
    out = dict(frames=len(times), frameBasis=kind, basisFrames=sum(mask),
               frameP50Ms=percentile(frame, 50), frameP95Ms=percentile(frame, 95), frameP99Ms=percentile(frame, 99),
               frameMaxMs=max(frame) if frame else None,
               hitches33=hitches(frame, HITCH_MS[0]), hitches100=hitches(frame, HITCH_MS[1]),
               allFrameP50Ms=percentile(whole, 50), allFrameP95Ms=percentile(whole, 95), allFrameP99Ms=percentile(whole, 99),
               allHitches33=hitches(whole, HITCH_MS[0]), allHitches100=hitches(whole, HITCH_MS[1]),
               cpuP50Ms=percentile(cpu, 50), cpuP95Ms=percentile(cpu, 95),
               gpuP50Ms=percentile(gpu, 50), gpuP95Ms=percentile(gpu, 95), gpuTimedFrames=len(gpu),
               firstRenderSeconds=run.get('firstRenderSeconds'),
               fullQualitySeconds=sum(full) if len(reached) == len(waypoints) else None,
               fullQualityMaxSeconds=max(full) if full else None, unreachedWaypoints=len(waypoints) - len(reached),
               visitedMean=statistics.fmean(f['visited']) if times else None, selectedMean=statistics.fmean(f['selected']) if times else None,
               selectedMax=max(f['selected'], default=0), drawCallsP50=percentile(f['drawCalls'], 50),
               drawCallsMax=max(f['drawCalls'], default=0), trianglesP50=percentile(f['triangles'], 50),
               trianglesMax=max(f['triangles'], default=0),
               gpuMemoryPeakMiB=max(memory, default=0) / MIB, gpuMemoryFinalMiB=(memory[-1] if memory else 0) / MIB,
               tilesetMemoryPeakMiB=max((v for v in f['memoryBytes'] if v is not None), default=0) / MIB,
               loads=sum(f['loads']), evictions=sum(f['unloads']), reloads=sum(f['reloads']),
               requests=sum(f['requests']), requestMiB=sum(f['requestBytes']) / MIB,
               peakRequestsPerSecond=peak_rate(times, f['requests']),
               peakMiBPerSecond=peak_rate(times, f['requestBytes']) / MIB,
               lodPops=pops(f['selected'], f['selAdded'], f['selRemoved'], mask), sseJumps=sse_jumps(f['maxSse'], mask),
               minMemoryAdjustedSse=min((v for v in f.get('memorySse', []) if v is not None), default=None),
               wallSeconds=(times[-1] - times[0]) / 1000 if len(times) > 1 else 0)
    return out


TRACE_JS = {'FunctionCall', 'EvaluateScript', 'v8.compile', 'V8.Execute', 'v8.callFunction', 'v8.run',
            'FireAnimationFrame', 'TimerFire', 'EventDispatch', 'RunMicrotasks'}


def union_ms(spans):
    """Total length in ms of the union of (start, end) spans in microseconds (nested events count once)."""
    total, reach = 0, None
    for start, end in sorted(spans):
        if reach is None or start > reach:
            total += end - start; reach = end
        elif end > reach:
            total += end - reach; reach = end
    return total / 1000


def trace_summary(events, page_url):
    """Split a Chrome trace of one flythrough into busy time per role (ms, overlapping events counted once):
    the page's renderer main thread (all work, JS, GC, GPU command issue and waits), its workers (decoding)
    and image decode, plus the GPU process main thread, which other tabs share. The renderer is the
    process whose frame committed page_url."""
    threads, processes, pids = {}, {}, set()
    for e in events:
        if e.get('ph') == 'M' and e.get('name') == 'thread_name': threads[(e['pid'], e['tid'])] = e['args'].get('name', '')
        elif e.get('ph') == 'M' and e.get('name') == 'process_name': processes[e['pid']] = e['args'].get('name', '')
        elif e.get('name') in ('TracingStartedInBrowser', 'FrameCommittedInBrowser'):
            data = (e.get('args') or {}).get('data') or {}
            for frame in data.get('frames') or [data]:
                if str(frame.get('url', '')).startswith(page_url) and frame.get('processId'): pids.add(frame['processId'])
    buckets = {k: [] for k in ('mainBusy', 'js', 'gc', 'gpuCommands', 'gpuWait', 'imageDecode', 'gpuProcessBusy')}
    workers = {}
    for e in events:
        if e.get('ph') != 'X' or 'dur' not in e: continue
        span, name, category = (e['ts'], e['ts'] + e['dur']), e.get('name', ''), e.get('cat', '')
        thread = threads.get((e['pid'], e['tid']), '')
        if e['pid'] in pids:
            if thread == 'CrRendererMain':
                buckets['mainBusy'].append(span)
                if name in TRACE_JS: buckets['js'].append(span)
                if 'GC' in name: buckets['gc'].append(span)
                if 'gpu' in category.split(','):
                    buckets['gpuCommands'].append(span)
                    if any(word in name for word in ('Wait', 'Finish', 'Sync')): buckets['gpuWait'].append(span)
            elif 'Worker' in thread:
                workers.setdefault((e['tid'], thread), []).append(span)
            if 'Decode' in name and 'Image' in name: buckets['imageDecode'].append(span)
        elif processes.get(e['pid']) == 'GPU Process' and thread == 'CrGpuMain':
            buckets['gpuProcessBusy'].append(span)
    out = {f'{key}Ms': union_ms(spans) for key, spans in buckets.items()}
    out.update(workerBusyMs=sum(union_ms(spans) for spans in workers.values()), workerThreads=len(workers),
               rendererPids=sorted(pids), events=len(events))
    return out


def pressure_share(start, end, key, wall):
    """PSI 'some' stall share of wall seconds between two /proc/pressure snapshots."""
    if not wall or key not in (start or {}) or key not in (end or {}): return None
    return (end[key] - start[key]) / 1e6 / wall


def host_noise(host):
    """(facts, reasons): the host-load facts of one run and why it counts as noisy (empty when clean)."""
    wall = host.get('wallSeconds') or 0
    facts = dict(cpuPressure=pressure_share(host.get('psiStart'), host.get('psiEnd'), 'cpu', wall),
                 memoryPressure=pressure_share(host.get('psiStart'), host.get('psiEnd'), 'memory', wall),
                 ioPressure=pressure_share(host.get('psiStart'), host.get('psiEnd'), 'io', wall),
                 memAvailableMinGiB=host.get('memAvailableMinGiB'), gpuUtilStart=host.get('gpuUtilStart'),
                 cpuBusyMean=host.get('cpuBusyMean'), loadAverage=host.get('loadAverage'))
    reasons = []
    for key, limit in NOISE.items():
        value = facts.get(key)
        if value is None: continue
        if key == 'memAvailableMinGiB':
            if value < limit: reasons.append(f'MemAvailable {value:.1f} GiB < {limit:g}')
        elif value >= limit:
            reasons.append(f'{key} {value:.0%} >= {limit:.0%}' if 'Pressure' in key else f'{key} {value:g}% >= {limit:g}%')
    return facts, reasons


def floor_for(metric):
    for suffix, floor in FLOORS.items():
        if metric.endswith(suffix): return floor
    return COUNT_FLOOR


def aggregate(summaries, noisy):
    """{metric: {median, min, max, n, of, stable}} over the clean runs when at least two are clean, else all;
    plus 'runs', 'clean', 'used' and 'usedNoisy'. n is how many of the `of` runs used had a value; stable is
    None (unknown) with fewer than two values."""
    clean = [s for s, bad in zip(summaries, noisy) if not bad]
    used = clean if len(clean) >= 2 else summaries
    out = dict(runs=len(summaries), clean=len(clean), used=len(used), frameBasis=(summaries[0] if summaries else {}).get('frameBasis'),
               usedNoisy=used is summaries and len(clean) < len(summaries), metrics={})
    for metric in summaries[0] if summaries else []:
        values = [s[metric] for s in used if isinstance(s.get(metric), (int, float)) and not isinstance(s.get(metric), bool)]
        if not values: out['metrics'][metric] = None; continue
        median, low, high = statistics.median(values), min(values), max(values)
        stable = None if len(values) < 2 else high - low <= max(STABLE_SPREAD * abs(median), floor_for(metric))
        out['metrics'][metric] = dict(median=median, min=low, max=high, n=len(values), of=len(used), stable=stable)
    return out


# ---- rendering ----

def fmt(value, digits=1):
    if value is None: return 'n/a'
    if isinstance(value, int) or float(value).is_integer() and abs(value) >= 100: return f'{int(round(value)):,}'
    return f'{value:,.{digits}f}'


def cell(aggregated, metric, digits=1, spread=False):
    entry = (aggregated or {}).get('metrics', {}).get(metric)
    if not entry: return 'n/a'
    text = fmt(entry['median'], digits)
    if spread and entry['n'] > 1: text += f" ({fmt(entry['min'], digits)}-{fmt(entry['max'], digits)})"
    if entry['n'] != entry['of']: text += f" [{entry['n']}/{entry['of']}]"  # runs with a value / runs used
    return text + {True: '', False: ' ~', None: ' ?'}[entry['stable']]


COLUMNS = [  # header, metric, digits, show range
    ('basis', 'basis', 0, False),
    ('frame p50/p95/p99 ms', ('frameP50Ms', 'frameP95Ms', 'frameP99Ms'), 1, False),
    ('hitch >33/>100', ('hitches33', 'hitches100'), 0, False),
    ('all frames p95 / >33', ('allFrameP95Ms', 'allHitches33'), 1, False),
    ('cpu p95 ms', ('cpuP95Ms',), 1, False),
    ('gpu p95 ms', ('gpuP95Ms',), 2, False),
    ('first render s', ('firstRenderSeconds',), 2, False),
    ('full quality s', ('fullQualitySeconds',), 2, True),
    ('requests / MiB', ('requests', 'requestMiB'), 2, False),
    ('peak req/s', ('peakRequestsPerSecond',), 0, False),
    ('draws p50', ('drawCallsP50',), 0, False),
    ('tris p50 k', ('trianglesP50k',), 1, False),
    ('selected mean', ('selectedMean',), 1, False),
    ('GPU MiB peak', ('gpuMemoryPeakMiB',), 1, False),
    ('load/evict/reload', ('loads', 'evictions', 'reloads'), 0, False),
    ('pops / SSE jumps', ('lodPops', 'sseJumps'), 0, False),
    ('runs used/total (clean)', None, 0, False),
]


def row(aggregated):
    cells = []
    for _, metrics, digits, spread in COLUMNS:
        if metrics is None:
            cells.append(f"{aggregated['used']}/{aggregated['runs']} ({aggregated['clean']} clean"
                         f"{', noisy used' if aggregated['usedNoisy'] else ''})"); continue
        if metrics == 'basis': cells.append(aggregated.get('frameBasis') or 'n/a'); continue
        cells.append(' / '.join(cell(aggregated, m, digits, spread) for m in metrics))
    return cells


def table(header, rows):
    lines = ['| ' + ' | '.join(header) + ' |', '|' + '---|' * len(header)]
    lines += ['| ' + ' | '.join(str(c) for c in r) + ' |' for r in rows]
    return '\n'.join(lines)


def with_kilo(aggregated):
    """Add trianglesP50k (thousands) for the table."""
    metrics = aggregated['metrics']
    source = metrics.get('trianglesP50')
    if source: metrics['trianglesP50k'] = dict(source, median=source['median'] / 1000, min=source['min'] / 1000, max=source['max'] / 1000)
    return aggregated


def render(artifact, directory, output, images=False):
    """Markdown for the artifact; image links are relative to output, and only with images=True."""
    env, runs = artifact['environment'], artifact['runs']
    out = [f"# Flythrough report", '',
           f"Generated {artifact['generated']} by `bench/perf_lane/flythrough.py`. "
           f"Raw data: `{env['evidence']}` (not in git).", '',
           f"- Binary: `{env.get('binary', {}).get('version')}` sha256 `{(env.get('binary', {}).get('sha256') or '')[:12]}`, commit `{env.get('commit')}`",
           f"- Renderer: `{env['renderer']}` ({env['rendererClass']}), {env['browserVersion']}, {env['browser']['mode']}; "
           f"viewport {env['viewport']} at DPR {env['devicePixelRatio']}; cache disabled; Cesium {env['cesium']}",
           f"- Paths replay at a fixed {env['fps']} frames per simulated second; the display presented frames at a median "
           f"{fmt(env.get('medianIntervalMs'), 2)} ms. GPU timer queries: {env.get('gpuTimer') or 'unavailable'}.",
           f"- Repeats: {env['repeats']}, order rotated per repeat. Waypoints hold until full quality (finish rule, "
           f"{env['settleMs']:g} ms) or {env['holdSeconds']:g} s.",
           '- Never compare across profiles or renderer classes: each table holds one recipe set, one profile and one renderer.',
           f"- Cells are medians over clean timed runs (all timed runs when fewer than two are clean); `[k/N]` means only "
           f"k of the N runs used had a value; `~` marks a metric whose repeats spread beyond max({STABLE_SPREAD:.0%} of "
           "the median, floor) and `?` one with fewer than two values. Frame times are presented-frame intervals (rAF); "
           f"hitches are intervals over {HITCH_MS[0]} and {HITCH_MS[1]} ms.",
           '- Basis: frame, CPU and GPU times, hitches, LOD pops and SSE jumps count motion frames only (`motion`); a '
           'path without motion (snap-teleport) counts its hold frames except each cut (`hold`). `all frames` adds the '
           'holds. Frames after a screenshot are never counted.',
           f"- Matrix `{env.get('matrix', {}).get('name')}`: {env.get('matrix', {}).get('description', '')}",
           '- Screenshots and Chrome traces come only from one extra, untimed run per selected cell; timed runs '
           'capture nothing, trace nothing and render without preserveDrawingBuffer.', '']
    out += ['## Paths', '', table(['path', 'motion frames', 'waypoints', 'description'],
                                  [[name, p['motionFrames'], ', '.join(p['waypoints']), p['description']]
                                   for name, p in artifact['paths'].items()]), '']
    out += ['## Profiles', '', table(['profile', 'settings', 'description'],
                                     [[name, json.dumps({k: v for k, v in p.items() if k != 'description'}) or '{}', p.get('description', '')]
                                      for name, p in artifact['profiles'].items()]), '']
    out += ['## Recipes', '', table(['set', 'recipe', 'conversion', 'viewer', 'output'],
                                    [[r['set'], r['name'], '`' + ' '.join(r['convert'] or ['(given output)']) + '`',
                                      '`' + json.dumps(r['viewer']) + '`',
                                      f"{fmt(r['output'].get('bytes', 0) / MIB, 2)} MiB, {r['output'].get('files', 'archive')} files"]
                                     for r in artifact['recipes']]), '']
    groups = artifact['aggregates']
    for set_name in dict.fromkeys(r['set'] for r in artifact['recipes']):
        recipes = [r['name'] for r in artifact['recipes'] if r['set'] == set_name]
        for profile in artifact['profiles']:
            rows = []
            for path in artifact['paths']:
                for recipe in recipes:
                    entry = groups.get(f'{recipe}|{path}|{profile}')
                    if entry: rows.append([path, recipe, *row(with_kilo(entry))])
            if not rows: continue
            out += [f'## {set_name}: {profile}', '', table(['path', 'recipe', *[c[0] for c in COLUMNS]], rows), '']
            wrows = []
            for path, info in artifact['paths'].items():
                for waypoint in info['waypoints']:
                    cells = []
                    for recipe in recipes:
                        entry = groups.get(f'{recipe}|{path}|{profile}')
                        values = (entry or {}).get('waypoints', {}).get(waypoint)
                        cells.append('n/a' if not values else
                                     f"{fmt(values['median'], 2)}" + (f" [{values['n']}/{values['of']}]" if values['n'] != values['of'] else '')
                                     + (f" ({values['unreached']} unreached)" if values['unreached'] else ''))
                    wrows.append([path, waypoint, *cells])
            out += [f'Time to full quality per waypoint (s, median), {set_name}, {profile}:', '',
                    table(['path', 'waypoint', *recipes], wrows), '']
    out += ['## Screenshots', '', 'Canvas captures at full quality, from the untimed capture run of each recipe, path and profile.', '']
    shots = artifact.get('screenshots', {})
    for set_name in dict.fromkeys(r['set'] for r in artifact['recipes']):
        recipes = [r['name'] for r in artifact['recipes'] if r['set'] == set_name]
        for profile in artifact['profiles']:
            rows = []
            for path, info in artifact['paths'].items():
                for waypoint in info['waypoints']:
                    cells = []
                    for recipe in recipes:
                        shot = shots.get(f'{recipe}|{path}|{profile}|{waypoint}')
                        if not shot: cells.append('n/a'); continue
                        if images:
                            target = os.path.relpath(directory / shot['file'], output.parent)
                            cells.append(f"![{recipe} {path} {waypoint}]({target})")
                        else:
                            cells.append(f"`{shot['sha256'][:10]}`")
                    same = len({shots.get(f'{r}|{path}|{profile}|{waypoint}', {}).get('sha256') for r in recipes}) == 1
                    rows.append([path, waypoint, *cells, 'yes' if same else 'no'])
            if any(c != 'n/a' for r in rows for c in r[2:-1]):
                out += [f'{set_name}, {profile}' + ('' if images else f" (files under `{env['evidence']}/screenshots/`)") + ':', '',
                        table(['path', 'waypoint', *recipes, 'identical'], rows), '']
    traced = [r for r in runs if r.get('trace')]
    if traced:
        keys = ('mainBusyMs', 'jsMs', 'gcMs', 'gpuCommandsMs', 'gpuWaitMs', 'workerBusyMs', 'imageDecodeMs', 'gpuProcessBusyMs')
        out += ['## Chrome traces', '',
                'One untimed, traced run per selected cell (tracing slows it). Busy ms over the flythrough, overlapping '
                'events counted once: the page renderer main thread in total, its JS, GC and GPU command issue and '
                'waits; its worker threads (decoding); image decode; and the GPU process main thread, which other tabs '
                'share. Traces are under `traces/` in the evidence directory.', '',
                table(['run', *keys], [[r['id'], *(fmt(r['trace'].get(k), 0) if 'error' not in r['trace'] else r['trace']['error'][:60]
                                                   for k in keys)] for r in traced]), '']
    out += ['## Host load and noise', '',
            'Per run: PSI stall share of the run wall time (cpu, memory, io), lowest MemAvailable, GPU utilisation '
            f"before the run (another process) and mean CPU busy. Noisy when: {', '.join(f'{k} {v:g}' for k, v in NOISE.items())} "
            '(MemAvailable below, others at or above).', '']
    by_profile = {}
    timed = [r for r in runs if not r.get('untimed')]
    for run in timed: by_profile.setdefault(run['profile'], []).append(run)
    rows = []
    for profile, items in by_profile.items():
        facts = [r['hostFacts'] for r in items]
        med = lambda k: statistics.median([f[k] for f in facts if f.get(k) is not None] or [0])
        rows.append([profile, len(items), sum(1 for r in items if r['noise']), f"{med('cpuPressure'):.0%}",
                     f"{med('memoryPressure'):.1%}", f"{med('ioPressure'):.1%}", fmt(min((f['memAvailableMinGiB'] or 0) for f in facts), 1),
                     fmt(med('gpuUtilStart'), 0), f"{med('cpuBusyMean'):.0%}"])
    out += [table(['profile', 'runs', 'noisy', 'cpu PSI (median)', 'mem PSI', 'io PSI', 'min MemAvail GiB', 'GPU util % (median)',
                   'CPU busy (median)'], rows), '']
    noisy = [r for r in timed if r['noise']]
    if noisy:
        out += ['Noisy runs:', ''] + [f"- `{r['id']}`: {'; '.join(r['noise'])}" for r in noisy[:40]]
        if len(noisy) > 40: out.append(f'- ... and {len(noisy) - 40} more (see flythrough.json)')
        out.append('')
    out += ['## Stability across repeats', '',
            'Spread (max - min) of each metric over the runs used, as a share of the median. `~` marks a spread beyond '
            f'max({STABLE_SPREAD:.0%}, floor: {", ".join(f"{k} {v:g}" for k, v in FLOORS.items())}, counts {COUNT_FLOOR}).', '']
    rows, unstable = [], 0
    for key, entry in groups.items():
        recipe, path, profile = key.split('|')
        cells = []
        for metric in STABILITY_METRICS:
            m = entry['metrics'].get(metric)
            if not m: cells.append('n/a'); continue
            if m['stable'] is None: cells.append('?'); continue
            share = (m['max'] - m['min']) / abs(m['median']) if m['median'] else 0.0
            cells.append(f"{share:.0%}" + ('' if m['stable'] else ' ~'))
            unstable += m['stable'] is False
        rows.append([recipe, path, profile, entry['runs'], *cells])
    out += [f'{unstable} unstable metric cells of {len(rows) * len(STABILITY_METRICS)}.', '',
            table(['recipe', 'path', 'profile', 'runs', *STABILITY_METRICS], rows), '']
    failed = [r for r in runs if r.get('problems')]
    out += ['## Failures', '']
    out += [f"- `{r['id']}`: {'; '.join(r['problems'])[:400]}" for r in failed] or ['None.']
    return '\n'.join(out) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--from', dest='source', type=pathlib.Path, required=True)
    parser.add_argument('--out', type=pathlib.Path)
    parser.add_argument('--images', action='store_true')
    args = parser.parse_args()
    artifact = json.loads((args.source / 'flythrough.json').read_text())
    output = (args.out or args.source / 'FLYTHROUGH.md').resolve()
    output.write_text(render(artifact, args.source.resolve(), output, images=args.images or args.out is None))
    print(output)


if __name__ == '__main__':
    main()
