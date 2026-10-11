"""Preorder id -> parent id for a producer hierarchy (explicit: or implicit: input), as forest_model numbers it."""
import json, pathlib, sys
HERE = pathlib.Path(__file__).resolve().parent
m = {'__name__': 'forest_model', '__file__': str(HERE / 'forest_model.py')}
exec(compile((HERE / 'forest_model.py').read_text(), 'forest_model.py', 'exec'), m)
kind, _, path = sys.argv[1].partition(':')
if kind == 'explicit':
    z, top, root = m['read_explicit'](path); scheme = 'QUADTREE'
else:
    z, top, root, scheme = m['read_implicit'](path)
m['plan'](root, scheme, 4)
parents = {}
def walk(n, p):
    parents[str(n.pid)] = None if p is None else str(p.pid)
    for c in n.children:
        walk(c, n)
walk(root, None)
pathlib.Path(sys.argv[2]).write_text(json.dumps(parents)); print(len(parents))
