"""Render the reuse benchmark (reuse-bench.json) as markdown (stdlib)."""
import json, statistics, sys
d = json.load(open(sys.argv[1]))
rows = ['| Config | Edit | Changed features | Fresh wall s (median of 3) | Reuse wall s (median of 3) | Reuse/fresh | Reused / rebuilt contents |',
        '| --- | --- | --- | --- | --- | --- | --- |']
import os
here = os.path.dirname(os.path.abspath(__file__))
def describe(name, ids):
    j = json.load(open(os.path.join(here, f'diff-{name}.json')))
    return f"{j['edited']} edited, {j['deleted']} deleted, {j['added']} added ({100 * j['changedShare']:.2f}%){', ' + ids if ids else ''}"
share = {'small-edit': describe('small-edit', ''), 'medium-edit': describe('medium-edit', ''),
         'small-mixed': describe('small-mixed', 'ordinal ids'), 'ids-mixed': describe('small-mixed', 'stable ids')}
groups = {}
for r in d['runs']:
    if r['kind'] == 'base':
        continue
    groups.setdefault((r['config'], r['diff']), {}).setdefault(r['kind'], []).append(r)
for (cfg, diff), k in groups.items():
    f = statistics.median(x['wall'] for x in k['fresh']); u = statistics.median(x['wall'] for x in k['reuse'])
    ru = k['reuse'][0]['reuse']
    rows.append(f"| {cfg} | {diff} | {share.get(diff, '')} | {f:.2f} | {u:.2f} | {u / f:.2f} | {ru['reusedContents']:,} / {ru['rebuiltContents']:,} |")
print('\n'.join(rows))
