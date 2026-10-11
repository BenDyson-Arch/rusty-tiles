"""Measure today's --reuse-tileset against a full rebuild on realistic local edits (stdlib only).

The develop-equivalent binary converts the perf-lane vector input (default implicit mode), then
converts each edited input both fresh and with --reuse-tileset from the base archive. Rounds alternate
the order. Every run is pinned (taskset) at nice 10 with --jobs 2. Wall, CPU and peak RSS come from
os.wait4; the reuse report comes from the CLI's --json output.

Usage: python3 -I reuse_bench.py OUT.json WORKDIR CPUS ROUNDS
"""
import hashlib, json, os, pathlib, subprocess, sys, time

BIN = '/home/bend/.cache/c2-corpus/rusty-tiles-f2238cd'
INPUTS = '/home/bend/.cache/perf-lane/work/inputs/vector.geojson'
CONFIGS = {'perf-default': [], 'mf2-lod3': ['--maxFeatures', '2', '--lodLevels', '3']}


def run(args, cpus):
    cmd = ['taskset', '-c', cpus, 'nice', '-n', '10', 'env', 'RAYON_NUM_THREADS=2', BIN, '--json', 'vector'] + args
    t = time.monotonic()
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    out, err = p.communicate()
    wall = time.monotonic() - t
    return wall, p.returncode, out, err


def main():
    out, work, cpus, rounds = sys.argv[1], pathlib.Path(sys.argv[2]), sys.argv[3], int(sys.argv[4])
    work.mkdir(parents=True, exist_ok=True)
    results = {'binary': BIN, 'binarySha256': hashlib.sha256(open(BIN, 'rb').read()).hexdigest(), 'cpus': cpus,
               'script': hashlib.sha256(open(__file__, 'rb').read()).hexdigest(), 'runs': []}
    # Same file name everywhere: the layer name is part of the reuse identity. 'ids' inputs carry a
    # stable top-level GeoJSON feature id (as a GPKG fid would); the others have ordinal ids only.
    bases = {'ordinal': INPUTS, 'ids': str(work / 'ids-base' / 'vector.geojson')}
    diffs = [('small-edit', 'ordinal'), ('medium-edit', 'ordinal'), ('small-mixed', 'ordinal'), ('ids-mixed', 'ids')]
    for cfg, opts in CONFIGS.items():
        common = ['--reproducible', '--jobs', '2'] + opts
        prevs = {}
        for bname, bpath in bases.items():
            prev = work / f'prev-{cfg}-{bname}.3tz'
            if prev.exists():
                prev.unlink()
            before = os.times()
            wall, rc, o, e = run(['-i', bpath, '-o', str(prev)] + common, cpus)
            after = os.times()
            results['runs'].append({'config': cfg, 'kind': 'base', 'base': bname, 'wall': round(wall, 3),
                                    'cpu': round((after.children_user - before.children_user) + (after.children_system - before.children_system), 3), 'rc': rc})
            prevs[bname] = prev
        for dname, bname in diffs:
            dpath = work / dname / 'vector.geojson'
            for r in range(rounds):
                order = ['fresh', 'reuse'] if r % 2 == 0 else ['reuse', 'fresh']
                for kind in order:
                    target = work / f'{cfg}-{dname}-{kind}-{r}.3tz'
                    if target.exists():
                        target.unlink()
                    args = ['-i', str(dpath), '-o', str(target)] + common
                    if kind == 'reuse':
                        args += ['--reuse-tileset', str(prevs[bname])]
                    before = os.times()
                    wall, rc, o, e = run(args, cpus)
                    after = os.times()
                    try:
                        j = json.loads(o)
                        rep = {'reuse': j.get('reuse'), 'tiles': j.get('counts', {}).get('tiles')}
                    except Exception:
                        rep = {'stderr': e.decode()[-500:]}
                    results['runs'].append({'config': cfg, 'diff': dname, 'base': bname, 'round': r, 'kind': kind, 'wall': round(wall, 3),
                                            'cpu': round((after.children_user - before.children_user) + (after.children_system - before.children_system), 3),
                                            'rc': rc, **rep, 'archiveBytes': target.stat().st_size if target.exists() else None})
                    print(json.dumps(results['runs'][-1]), file=sys.stderr, flush=True)
                    if target.exists():
                        target.unlink()
    json.dump(results, open(out, 'w'), indent=1)


if __name__ == '__main__':
    main()
