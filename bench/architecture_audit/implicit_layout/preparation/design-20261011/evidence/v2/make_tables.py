"""Render the v2 evidence tables (markdown) from the result files (stdlib only)."""
import glob, json, os, statistics, subprocess, sys

E = os.path.dirname(os.path.abspath(__file__))
V = os.path.expanduser('~/.cache/perf-lane/work/pimpl/v2')


def scale():
    rows = ['| Case | Tiles | L | Explicit | Roots | Links = docs−1 | Max nodes/doc | Max bytes/doc | Max JSON depth | Root doc (nodes / bytes) | Largest subtree | Entries | Visits |',
            '| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |']
    worst = {'n': 0, 'b': 0, 'd': 0}
    for f in sorted(glob.glob(os.path.join(E, 'scale', '*-S4-4.json'))):
        d = json.load(open(f)); n = os.path.basename(f)[:-len('-S4-4.json')]
        if 'layoutError' in d:
            rows.append(f'| {n} | error: {d["layoutError"]} |'); continue
        worst['n'] = max(worst['n'], d['maxDocNodes']); worst['b'] = max(worst['b'], d['maxDocBytes']); worst['d'] = max(worst['d'], d['maxDocJsonDepth'])
        r = d['rootDoc']
        rows.append(f'| {n} | {d["tiles"]:,} | {d["levels"]} | {d["explicitTiles"]:,} | {d["implicitRoots"]:,} | {d["linkTiles"]:,} | '
                    f'{d["maxDocNodes"]:,} | {d["maxDocBytes"]:,} | {d["maxDocJsonDepth"]} | {r["valueNodes"]:,} / {r["bytes"]:,} | '
                    f'{d["maxSubtreeBytes"]:,} | {d["archiveEntries"]:,} | {d["hierarchyVisits"]:,} |')
    return '\n'.join(rows), worst


def fixtures():
    d = json.load(open(os.path.join(E, 'cesium', 'fixtures-results.json')))
    rows = ['| Layout | Leaf selected | Leaf picked | Holes | Control | Requests (JSON / subtree / GLB) | Errors |', '| --- | --- | --- | --- | --- | --- | --- |']
    for v, r in d['results'].items():
        if 'views' not in r:
            rows.append(f'| {v} | load failed | | | | | {r.get("failures") or r.get("error", "")[:120]} |'); continue
        ok = [w for w in r['views'] if not w['control']]
        c = [w for w in r['views'] if w['control']]
        q = r['requests']
        errs = sorted(set((r.get('failures') or [])))
        rows.append(f'| {v} | {sum(w["leafSelected"] for w in ok)}/{len(ok)} | {sum(w["pickedLeaf"] for w in ok)}/{len(ok)} | '
                    f'{sum(w["coverage"] < .5 for w in ok)} | {"pass" if all(w["leafSelected"] and w["pickedLeaf"] for w in c) else "fail"} | '
                    f'{q["json"]} / {q["subtree"]} / {q["glb"]} | {"; ".join(e[:90] for e in errs[:1]) + (f" (×{len(r.get("failures"))})" if errs else "")} |')
    return '\n'.join(rows), d['receipt']


def deep():
    parents = {'mf2-lod3': 'parents-mf2-lod3.json', 'mf2-lod3-s6': 'parents-mf2-lod3.json', 'mf2-lod16': 'parents-mf2-lod16.json',
               'vec-perf': 'parents-vec-perf.json', 'pc-deep': 'parents-pc-deep.json', 'mf2-lod3-explicit': 'parents-mf2-lod3.json'}
    rows = ['| Case | Child-bounds cull | Target leaf selected (today / other) | Same selection | Other selects an ancestor | Same non-null pick (null=null separate) | Requests today (JSON / subtree / GLB) | Requests other | View seconds today / other |',
            '| --- | --- | --- | --- | --- | --- | --- | --- | --- |']
    for f in sorted(glob.glob(os.path.join(E, 'deep', '*.json'))):
        tag = os.path.basename(f)[:-5]
        base = tag.rsplit('-cb', 1)[0]
        out = subprocess.run([sys.executable, '-I', os.path.join(E, 'compare_runs.py'), f, os.path.join(V, parents[base])], capture_output=True, text=True)
        c = json.loads(out.stdout)
        cases = list(c['cases'])
        t = c['cases'][cases[0]]
        for pair, p in c['pairs'].items():
            o = c['cases'][pair.split('~')[1]]
            rq = lambda x: f'{x["requests"]["json"]} / {x["requests"]["subtree"]} / {x["requests"]["glb"]}'
            rows.append(f'| {base} ({pair.replace("~", " vs ")}) | {"on" if tag.endswith("cb1") else "off"} | {t["leafSelected"]} / {o["leafSelected"]} of {p["views"]} | '
                        f'{p["sameSelection"]}/{p["views"]} | {p["otherSelectsCoarser"]} (today coarser: {p["baseSelectsCoarser"]}) | '
                        f'{p["sameNonNullPick"]} (null both: {p["nullPickBoth"]}) | {rq(t)} | {rq(o)} | {t["viewSecondsTotal"]} / {o["viewSecondsTotal"]} |')
    return '\n'.join(rows)


def perf():
    rows = ['| Case | Run | Seconds to loaded | Requests (JSON / subtree / content) | Request bytes | Gate problems | Busy % on 13/15 (siblings 12/14) |',
            '| --- | --- | --- | --- | --- | --- | --- |']
    summary = {}
    for case in ('vector', 'mf2-lod3', 'pc-deep'):
        for tag in ('base1', 'head1', 'head2', 'base2', 'base3', 'head3', 'head4', 'base4'):
            f = os.path.join(E, 'perf', f'{tag}-{case}.json')
            if not os.path.exists(f):
                continue
            d = json.load(open(f)); c = d['cases'][0]; r = c['requests']
            load = json.load(open(f.replace('.json', '.cpuload')))
            summary.setdefault((case, tag[:4]), []).append(c.get('secondsToLoaded'))
            rows.append(f'| {case} | {tag} | {c.get("secondsToLoaded") or 0:.2f} | {r["manifest"]["requests"]} / {r["subtree"]["requests"]} / {r["content"]["requests"]} | '
                        f'{r["requestBytes"]:,} | {"; ".join(c["problems"]) or "none"} | {load["cpu13"]}/{load["cpu15"]} ({load["cpu12"]}/{load["cpu14"]}) |')
    med = ['| Case | Base median s (n) | Head median s (n) | Base range | Head range |', '| --- | --- | --- | --- | --- |']
    for case in ('vector', 'mf2-lod3', 'pc-deep'):
        b = [x for x in summary.get((case, 'base'), []) if x]; h = [x for x in summary.get((case, 'head'), []) if x]
        if b and h:
            med.append(f'| {case} | {statistics.median(b):.2f} ({len(b)}) | {statistics.median(h):.2f} ({len(h)}) | {min(b):.2f}–{max(b):.2f} | {min(h):.2f}–{max(h):.2f} |')
    return '\n'.join(rows) + '\n\n' + '\n'.join(med)


def deep_abba():
    rows = ['| Invocation | Order | Today: view s / requests (JSON/subtree/GLB) | Forest: view s / requests | Busy % 13/15 (12/14) |', '| --- | --- | --- | --- | --- |']
    t_all, f_all = [], []
    for f in sorted(glob.glob(os.path.join(E, 'deep-abba', '*.json'))):
        tag = os.path.basename(f)[:-5]
        r = json.load(open(f))['results']
        load = json.load(open(f.replace('.json', '.cpuload')))
        def cell(x):
            q = x['requests']
            return f"{sum(v['seconds'] for v in x['views']):.2f} / {q['json']}/{q['subtree']}/{q['glb']}"
        t_all.append(sum(v['seconds'] for v in r['today']['views'])); f_all.append(sum(v['seconds'] for v in r['forest4']['views']))
        rows.append(f"| {tag} | {' then '.join(r)} | {cell(r['today'])} | {cell(r['forest4'])} | {load['cpu13']}/{load['cpu15']} ({load['cpu12']}/{load['cpu14']}) |")
    if t_all:
        rows.append(f"| median | | {statistics.median(t_all):.2f} s | {statistics.median(f_all):.2f} s | |")
    return '\n'.join(rows)


if __name__ == '__main__':
    what = sys.argv[1]
    if what == 'scale':
        t, w = scale(); print(t); print(json.dumps(w))
    elif what == 'fixtures':
        t, r = fixtures(); print(t); print(json.dumps(r))
    elif what == 'deep':
        print(deep())
    elif what == 'perf':
        print(perf())
    elif what == 'deepabba':
        print(deep_abba())
