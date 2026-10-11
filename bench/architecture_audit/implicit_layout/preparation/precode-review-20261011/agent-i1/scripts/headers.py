import json, sys, collections
doc = json.load(open(sys.argv[1]))
seen = collections.defaultdict(collections.Counter)
def walk(t):
    cs = t.get('contents') or ([t['content']] if 'content' in t else [])
    for i, c in enumerate(cs):
        h = {k: v for k, v in c.items() if k != 'uri'}
        seen[(i, c['uri'].rsplit('.', 1)[-1])][json.dumps(h, sort_keys=True)] += 1
    for c in t.get('children', []): walk(c)
walk(doc['root'])
for k, v in seen.items(): print(k, dict(v))
