"""Compare deep-view runs (run_real.py output) tile-for-tile (stdlib only).

Usage: python3 -I compare_runs.py RESULT.json PARENTS.json
For the first case (today) against each other case, per view:
  leafSelected / pickedLeaf counts per layout (the target leaf, by key);
  same selection (identical selected key sets);
  same non-null pick, null=null picks (reported separately, never counted as agreement);
  coarser: selections differ and one side selects a strict ancestor of a tile the other selects.
"""
import json, sys

res = json.load(open(sys.argv[1]))['results']
parents = json.load(open(sys.argv[2]))


def ancestors(k):
    out = set()
    while parents.get(k) is not None:
        k = parents[k]; out.add(k)
    return out


names = list(res)
base = res[names[0]]
out = {'cases': {}, 'pairs': {}}
for n in names:
    r = res[n]
    out['cases'][n] = {'views': len(r['views']), 'leafSelected': sum(v['leafSelected'] for v in r['views']),
                       'pickedLeaf': sum(v['pickedLeaf'] for v in r['views']),
                       'nonNullPicks': sum(v['pickedKey'] is not None for v in r['views']),
                       'requests': r['requests'], 'overviewRequests': r['overview']['requestCount'],
                       'overviewSeconds': round(r['overview']['seconds'], 2), 'overviewLoaded': r['overview']['loaded'],
                       'viewSecondsTotal': round(sum(v['seconds'] for v in r['views']), 2),
                       'allViewsLoaded': all(v['loaded'] for v in r['views']),
                       'failures': r.get('failures'), 'failedRequests': r.get('failedRequests')}
for n in names[1:]:
    r = res[n]
    same = samePick = nullBoth = coarserOther = coarserBase = 0
    diffs = []
    for a, b in zip(base['views'], r['views']):
        A, B = set(a['selectedKeys']), set(b['selectedKeys'])
        if A == B:
            same += 1
        else:
            ob = any(ancestors(x) & B for x in A)   # other selects an ancestor of a base tile
            oa = any(ancestors(x) & A for x in B)
            coarserOther += ob and not oa
            coarserBase += oa and not ob
            diffs.append({'view': a['name'], 'onlyBase': sorted(A - B)[:8], 'onlyOther': sorted(B - A)[:8],
                          'coverageBase': a['coverage'], 'coverageOther': b['coverage']})
        if a['pickedKey'] is None and b['pickedKey'] is None:
            nullBoth += 1
        elif a['pickedKey'] == b['pickedKey']:
            samePick += 1
    out['pairs'][f'{names[0]}~{n}'] = {'views': len(r['views']), 'sameSelection': same, 'sameNonNullPick': samePick,
                                       'nullPickBoth': nullBoth, 'otherSelectsCoarser': coarserOther,
                                       'baseSelectsCoarser': coarserBase, 'differences': diffs}
print(json.dumps(out, indent=1))
