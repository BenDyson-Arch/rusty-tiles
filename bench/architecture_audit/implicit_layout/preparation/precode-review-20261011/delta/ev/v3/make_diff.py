"""Realistic local edit of the perf-lane vector.geojson (stdlib, deterministic): inside one small
neighbourhood (a 0.006 x 0.004 degree box, about 500 m x 450 m near Rome), change existing attributes of every
feature, nudge polygon/line vertices by ~0.5 m, delete 3 features and add 3 new buildings.
Everything outside the box is byte-identical feature JSON.
Usage: python3 -I make_diff.py IN.geojson OUT.geojson REPORT.json [W,S,E,N|-] [edit-only]"""
import copy, hashlib, json, sys
src, dst, rep = sys.argv[1:4]
doc = json.load(open(src))
W, S, E, N = map(float, sys.argv[4].split(',')) if len(sys.argv) > 4 and sys.argv[4] != '-' else (12.410, 41.880, 12.416, 41.884)
EDIT_ONLY = len(sys.argv) > 5 and sys.argv[5] == 'edit-only'   # no deletions or insertions


def first(g):
    c = g['coordinates']
    while isinstance(c[0], list):
        c = c[0]
    return c


def inside(f):
    x, y = first(f['geometry'])[:2]
    return W <= x <= E and S <= y <= N


def nudge(c):
    if isinstance(c[0], list):
        return [nudge(v) for v in c]
    return [c[0] + 0.000005, c[1] + 0.000003] + c[2:]


out, edited, deleted = [], 0, 0
for f in doc['features']:
    if inside(f):
        if not EDIT_ONLY and deleted < 3 and f['geometry']['type'] == 'Point':
            deleted += 1
            continue
        g = copy.deepcopy(f)
        pr = g['properties']   # edit existing attributes only (no new nullable columns)
        if 'name' in pr:
            pr['name'] = pr['name'] + '-e'
        if 'height' in pr:
            pr['height'] = pr['height'] + 1
        if 'girth' in pr:
            pr['girth'] = round(pr['girth'] + 0.01, 2)
        if g['geometry']['type'] in ('Polygon', 'LineString'):
            g['geometry']['coordinates'] = nudge(g['geometry']['coordinates'])
        out.append(g); edited += 1
    else:
        out.append(f)
for i in range(0 if EDIT_ONLY else 3):
    x, y = W + 0.001 * (i + 1), S + 0.001
    ring = [[x, y], [x + 0.0003, y], [x + 0.0003, y + 0.0002], [x, y + 0.0002], [x, y]]
    out.append({"type": "Feature", "properties": {"id": 20000 + i, "kind": "building", "height": 12, "name": f"new-{i}"},
                "geometry": {"type": "Polygon", "coordinates": [ring]}})
doc['features'] = out
json.dump(doc, open(dst, 'w'))
json.dump({'input': src, 'inputSha256': hashlib.sha256(open(src, 'rb').read()).hexdigest(), 'output': dst,
           'outputSha256': hashlib.sha256(open(dst, 'rb').read()).hexdigest(), 'box': [W, S, E, N],
           'features': len(doc['features']), 'edited': edited, 'deleted': deleted, 'added': 0 if EDIT_ONLY else 3,
           'changedShare': round((edited + deleted + (0 if EDIT_ONLY else 3)) / 10000, 4)}, open(rep, 'w'), indent=1)
print(open(rep).read())
