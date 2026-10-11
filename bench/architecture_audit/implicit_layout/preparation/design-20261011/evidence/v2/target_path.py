"""Per deep-view result: is the selected tile on each view's target path (target leaf and its ancestors)
the same in every layout, and when the leaf is not selected, what is the error of the selected path tile?
Usage: python3 -I target_path.py RESULT.json PARENTS.json ERRORS_SOURCE (explicit:ARCHIVE | implicit:ARCHIVE)"""
import json, pathlib, sys
HERE = pathlib.Path(__file__).resolve().parent
m = {'__name__': 'forest_model', '__file__': str(HERE / 'forest_model.py')}
exec(compile((HERE / 'forest_model.py').read_text(), 'forest_model.py', 'exec'), m)
res = json.load(open(sys.argv[1]))['results']; parents = json.load(open(sys.argv[2]))
kind, _, path = sys.argv[3].partition(':')
if kind == 'explicit':
    z, top, root = m['read_explicit'](path); scheme = 'QUADTREE'
else:
    z, top, root, scheme = m['read_implicit'](path)
m['plan'](root, scheme, 4)
err = {}
def walk(n):
    err[str(n.pid)] = n.tile['geometricError']
    for c in n.children: walk(c)
walk(root)
names = list(res); same = 0; maxerr = 0.0; views = len(res[names[0]]['views'])
for i in range(views):
    leaf = res[names[0]]['views'][i]['name'].split('-p')[1]
    path = {leaf}; k = leaf
    while parents.get(k) is not None:
        k = parents[k]; path.add(k)
    on = [tuple(sorted(set(r['views'][i]['selectedKeys']) & path)) for r in res.values()]
    same += all(o == on[0] for o in on)
    for o in on[0]:
        if o != leaf: maxerr = max(maxerr, err[o])
print(json.dumps({'result': pathlib.Path(sys.argv[1]).name, 'layouts': names, 'views': views, 'sameTargetPathSelection': same,
                  'maxErrorOfSelectedPathTileWhenLeafNotSelected': maxerr}))
