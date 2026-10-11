import json, sys
def walk(v, d, st):
    st['nodes'] += 1; st['depth'] = max(st['depth'], d)
    if isinstance(v, dict):
        for x in v.values(): walk(x, d+1, st)
    elif isinstance(v, list):
        for x in v: walk(x, d+1, st)
def tiles(t, d, out):
    out.append((d, t))
    for c in t.get('children', []): tiles(c, d+1, out)
for p in sys.argv[1:]:
    doc = json.load(open(p))
    st = {'nodes': 0, 'depth': 0}; walk(doc, 0, st)
    ts = []; tiles(doc['root'], 0, ts)
    roots = [(d, t) for d, t in ts if 'implicitTiling' in t]
    tmpl = []
    for d, t in roots:
        cs = t.get('contents') or ([t['content']] if 'content' in t else [])
        tmpl += [c['uri'] for c in cs] + [t['implicitTiling']['subtrees']['uri']]
    print(p, 'nodes', st['nodes'], 'jsondepth', st['depth'], 'explicit', len(ts), 'roots', len(roots),
          'maxTileDepth', max(d for d, _ in ts), 'nodes/tile', round(st['nodes']/len(ts),1),
          'maxTmplLen', max(map(len, tmpl)) if tmpl else 0, 'sample', tmpl[:2],
          'rootkeys', sorted(roots[0][1].keys()) if roots else None,
          'levels', sorted({(t['implicitTiling']['subtreeLevels'], t['implicitTiling']['availableLevels']) for _, t in roots}))
