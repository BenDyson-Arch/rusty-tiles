"""Explicit 3TZ with hierarchy visits at 2^18 and references at exactly N (default 2^20).
Child tiles carry 4-5 `contents` naming a handful of shared in-box GLBs, so entries stay tiny
while every content is a separate C1 reference and a separate C2 phase-2 stream."""
import json, sys
from pathlib import Path
sys.path.insert(0, '/tmp/rusty-tiles-vcaps/bench/architecture_audit/validator_caps')
from stress import glb, tile, document, write  # noqa: E402

out, target = sys.argv[1], int(sys.argv[2]) if len(sys.argv) > 2 else 1 << 20
per_tile = int(sys.argv[3]) if len(sys.argv) > 3 else 4
visits_cap, per_doc = 1 << 18, 4400
k = 1
while True:
    c = visits_cap - 3 - 4 * k
    if -(-c // k) <= per_doc:
        break
    k += 1
# references: tileset.json + root glb + K link contents + K entering + K ext-root glbs + child contents
base = 2 + 3 * k
deficit = target - base - per_tile * c
assert 0 <= deficit <= c, deficit
members, links = [], []
split = [c // k + (i < c % k) for i in range(k)]
idx = 0
for i, count in enumerate(split):
    children = []
    for _ in range(count):
        n = per_tile + 1 if idx < deficit else per_tile
        t = tile(tuple(f'g/{j}.glb' for j in range(n)))
        t['extras'] = idx  # schema: children must be unique
        children.append(t)
        idx += 1
    members.append((f'e{i}.json', document(tile(('g/0.glb',), children))))
    links.append(tile((f'e{i}.json',)))
members.insert(0, ('tileset.json', document(tile(('g/0.glb',), links))))
members += [(f'g/{j}.glb', glb()) for j in range(per_tile + (1 if deficit else 0))]
n, cd = write(out, members)
print(json.dumps({'k': k, 'children': c, 'visits': 3 + 4 * k + c, 'references': base + per_tile * c + deficit,
                  'entries': n, 'maxDocBytes': max(len(d) for _, d in members)}))
