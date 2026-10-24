#!/usr/bin/env python3
"""Read-only explicit REPLACE loading-barrier check; no meshes or producer facts.
Ordinary empty intermediates are unsafe even when error/bounds equal the parent.
External-tileset links are expanded as placeholders, not treated as payloads.
Implicit traversal needs I1 expansion and is refused here rather than accepted.
"""
import argparse
import json
from pathlib import Path, PurePosixPath
import zipfile


def check(document, resolve):
    violations=[]
    pending=[(document,'tileset.json',None,False,False,'root',frozenset({'tileset.json'}))]
    visits=0
    while pending:
        node,base,refine,barrier,empty,path,active=pending.pop()
        visits+=1
        if visits>1_000_000:raise ValueError('continuity hierarchy visit limit')
        if 'root' in node:node=node['root']
        if 'implicitTiling' in node:raise ValueError('continuity requires expanded explicit hierarchy')
        refine=node.get('refine',refine or 'REPLACE')
        entries=node.get('contents', [node['content']] if 'content' in node else [])
        payloads=[];external=[]
        for content in entries:
            uri=content.get('uri',content.get('url',''))
            name=str(PurePosixPath(base).parent/uri)
            if '..' in PurePosixPath(name).parts or name.startswith('/'):raise ValueError('unsafe content URI')
            if name.endswith('.json'):external.append(name)
            else:payloads.append(name)
        renderable=bool(payloads)
        if renderable and barrier and empty:violations.append(dict(path=path,content=payloads))
        next_barrier=refine=='REPLACE' if renderable else barrier
        # External links are traversed directly; a link node without ordinary
        # children is not an Empty3DTileContent cell in Cesium's traversal.
        next_empty=False if renderable else empty or (barrier and not external)
        for i,child in enumerate(node.get('children',[])):
            pending.append((child,base,refine,next_barrier,next_empty,f'{path}/{i}',active))
        for name in external:
            if name in active:raise ValueError('external tileset cycle')
            pending.append((resolve(name),name,refine,next_barrier,next_empty,f'{path}@{name}',active|{name}))
    return dict(ok=not violations,check='replaceContentContinuity',tiles=visits,violations=violations)


def inspect(path):
    path=Path(path)
    if path.is_dir():
        resolve=lambda name:json.loads((path/name).read_text())
        return check(resolve('tileset.json'),resolve)
    with zipfile.ZipFile(path) as archive:
        resolve=lambda name:json.loads(archive.read(name))
        return check(resolve('tileset.json'),resolve)

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('archive_or_directory');a=p.parse_args()
    result=inspect(a.archive_or_directory)
    print(json.dumps(result,sort_keys=True));raise SystemExit(0 if result['ok'] else 1)
