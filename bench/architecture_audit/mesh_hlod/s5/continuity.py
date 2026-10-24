#!/usr/bin/env python3
"""Independent REPLACE barriers, including implicit and external forests.

Leading empty cells and external placeholders do not create loading barriers.
An ordinary empty cell below rendered REPLACE content does, even at equal G.
"""
import argparse
import copy
import json
from archive import Archive, expand


def check_tiles(tiles):
    violations=[]
    states={}
    for tile in tiles:
        parent=tile['parent']
        barrier,empty=states[parent['id']] if parent else (False,False)
        renderable=bool(tile['contents'])
        if renderable and barrier and empty:
            violations.append(dict(path=tile['id'],content=tile['contents']))
        states[tile['id']] = (tile['refine']=='REPLACE' if renderable else barrier,
            False if renderable else empty or (barrier and not tile['external']))
    return dict(ok=not violations,check='replaceContentContinuity',tiles=len(tiles),violations=violations)


def check(document,resolve):
    """Literal dictionary adapter retained for small analytic callers."""
    def defaults(value):
        value=copy.deepcopy(value)
        pending=[value['root']] if 'root' in value else [value]
        while pending:
            node=pending.pop()
            node.setdefault('geometricError',0)
            pending.extend(node.get('children',[]))
        return value
    class Documents:
        root_name='tileset.json'
        def json(self,name):
            return defaults(document if name==self.root_name else resolve(name))
        def read(self,name):
            value=resolve(name)
            return value if isinstance(value,bytes) else json.dumps(value).encode()
        def _read(self,name,prefix=None):
            raw=self.read(name)
            return raw if prefix is None else raw[:prefix]
    return check_tiles(expand(Documents()))


def inspect(path):
    with Archive(path) as archive:
        return check_tiles(expand(archive))


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('archive_or_directory');a=p.parse_args()
    try:
        result=inspect(a.archive_or_directory)
    except (ValueError,OSError) as e:
        result=dict(ok=False,check='continuityStructure',detail=str(e))
    print(json.dumps(result,sort_keys=True));raise SystemExit(0 if result['ok'] else 1)
