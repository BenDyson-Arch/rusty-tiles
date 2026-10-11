"""Today's URI -> preorder id map for a today's implicit archive whose forest model was built from
that same archive (forest_model.py 'implicit:' input). Usage: implicit_today_map.py ARCHIVE VIEWS_IN VIEWS_OUT"""
import json, pathlib, sys
HERE = pathlib.Path(__file__).resolve().parent
m = {'__name__': 'forest_model', '__file__': str(HERE / 'forest_model.py')}
exec(compile((HERE / 'forest_model.py').read_text(), 'forest_model.py', 'exec'), m)
z, top, root, scheme = m['read_implicit'](sys.argv[1])
m['plan'](root, scheme, 4)
mapping = {}
def walk(n):
    c = n.tile.get('content')
    if c:
        mapping[c['uri']] = str(n.pid)
    for ch in n.children:
        walk(ch)
walk(root)
doc = json.loads(pathlib.Path(sys.argv[2]).read_text()); doc['today'] = mapping
pathlib.Path(sys.argv[3]).write_text(json.dumps(doc)); print(len(mapping))
