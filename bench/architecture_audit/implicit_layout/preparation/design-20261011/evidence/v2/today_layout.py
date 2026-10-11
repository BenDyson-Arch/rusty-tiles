"""Today's layout metrics for develop implicit archives (stdlib; uses ../layout_probe.py by path).
Usage: python3 -I today_layout.py ARCHIVE... > today-layout.json"""
import hashlib, json, pathlib, sys, zipfile
HERE = pathlib.Path(__file__).resolve().parent
probe = {'__name__': 'layout_probe'}
exec(compile((HERE.parent / 'layout_probe.py').read_text(), 'layout_probe.py', 'exec'), probe)


def nodes_depth(v, d=0):
    if isinstance(v, (dict, list)):
        n, m = 1, d
        for x in (v.values() if isinstance(v, dict) else v):
            a, b = nodes_depth(x, d + 1); n += a; m = max(m, b)
        return n, m
    return 1, d


out = []
for p in sys.argv[1:]:
    z = zipfile.ZipFile(p)
    top, tiles, dims, today = probe['expand'](z)
    names = z.namelist()
    docs = [n for n in names if n == 'tileset.json' or (n.startswith('implicit-tileset-') and n.endswith('.json'))]
    stats = [nodes_depth(json.loads(z.read(n))) + (len(z.read(n)),) for n in docs]
    rn, rd = nodes_depth(json.loads(z.read('tileset.json')))
    out.append(dict(archive=p, sha256=hashlib.sha256(open(p, 'rb').read()).hexdigest(), tiles=len(tiles),
                    levels=max(k[0] for k in tiles) + 1, documents=len(docs), subtrees=today['subtrees'],
                    subtreeBytes=today['subtreeBytes'], documentBytes=today['documentBytes'],
                    rootDoc=dict(valueNodes=rn, jsonDepth=rd, bytes=len(z.read('tileset.json'))),
                    maxDocNodes=max(s[0] for s in stats), maxDocDepth=max(s[1] for s in stats), maxDocBytes=max(s[2] for s in stats),
                    archiveEntries=len(names), script='today_layout.py',
                    scriptSha256=hashlib.sha256((HERE / 'today_layout.py').read_bytes()).hexdigest()))
print(json.dumps(out, indent=1))
