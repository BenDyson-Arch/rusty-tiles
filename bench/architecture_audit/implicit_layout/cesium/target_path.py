"""Per deep-view result: is the selected tile on each view's target path (the target leaf and its
ancestors) the same in every layout, and when the leaf is not selected, what is the error of the
selected path tile? PO port of the design's evidence/v2/target_path.py: the parents and errors come
from prepare_real.py (content keys) instead of the design's forest model.

Usage: python3 -I target_path.py RESULT.json PARENTS.json ERRORS.json
"""
import json, pathlib, sys


def target_path(res, parents, errors):
    names = list(res)
    same, maxerr, per_view = 0, 0.0, []
    views = len(res[names[0]]['views'])
    for i in range(views):
        leaf = res[names[0]]['views'][i]['name'].split('-p', 1)[1]
        path, k = {leaf}, leaf
        while parents.get(k) is not None:
            k = parents[k]; path.add(k)
        on = [tuple(sorted(set(r['views'][i]['selectedKeys']) & path)) for r in res.values()]
        ok = all(o == on[0] for o in on)
        same += ok
        per_view.append({'view': res[names[0]]['views'][i]['name'], 'sameTargetPath': ok, 'selectedOnPath': [list(o) for o in on]})
        for o in on[0]:
            if o != leaf:
                maxerr = max(maxerr, errors.get(o) or 0.0)
    return {'layouts': names, 'views': views, 'sameTargetPathSelection': same,
            'maxErrorOfSelectedPathTileWhenLeafNotSelected': maxerr, 'perView': per_view}


if __name__ == '__main__':
    res = json.load(open(sys.argv[1]))['results']
    out = target_path(res, json.load(open(sys.argv[2])), json.load(open(sys.argv[3])))
    out['result'] = pathlib.Path(sys.argv[1]).name
    print(json.dumps({k: v for k, v in out.items() if k != 'perView'}))
