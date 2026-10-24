#!/usr/bin/env python3
"""Small decoded oracle: source manifest + published explicit hierarchy, never producer facts.
E2's complete segment incidence/ancestor cross-product, plus corner and T contacts.
No source-distance bound is inferred from this census. Use ../s2/audit.py separately.
"""
import argparse
import collections
import hashlib
import itertools
import json
import math
from pathlib import Path
import subprocess
from continuity import inspect as inspect_continuity

ROOT = Path(__file__).parent

class Failure(Exception):
    def __init__(self, check, detail):
        self.check, self.detail = check, detail
        super().__init__(f'{check}: {detail}')

def require(ok, check, detail):
    if not ok:
        raise Failure(check, detail)

def decode(path):
    r = subprocess.run(['node', str(ROOT / 'decode.mjs'), str(path)], capture_output=True, text=True)
    require(r.returncode == 0, 'codecStructure' if 'featureIdentity' not in r.stderr and 'metadataIntegrity' not in r.stderr else 'featureIdentity', r.stderr[-1500:])
    return json.loads(r.stdout)

def edges(faces):
    return {tuple(sorted((tuple(f['xyz'][k]), tuple(f['xyz'][(k+1)%3]))))
            for f in faces for k in range(3)}

def census(tiles):
    leaves = [t for t in tiles if not t['children']]
    point_owners, edge_owners = collections.defaultdict(set), collections.defaultdict(set)
    for i, t in enumerate(leaves):
        for e in edges(t['faces']):
            edge_owners[e].add(i)
            for p in e:
                point_owners[p].add(i)
    # All corner contacts, including those with no shared edge.
    items = [(e, owners) for e, owners in edge_owners.items() if len(owners)>1]
    points = [(p, owners) for p, owners in point_owners.items() if len(owners)>1]
    # Small oracle deliberately brute-forces the independent T/collinear join.
    contacts = []
    for (a,b), owners in edge_owners.items():
        d = [b[k]-a[k] for k in range(3)]
        l2 = sum(x*x for x in d)
        if not l2:
            continue
        for p, other in point_owners.items():
            if not other-owners or p in (a,b):
                continue
            u = sum((p[k]-a[k])*d[k] for k in range(3))/l2
            if 0<u<1 and sum((p[k]-a[k]-u*d[k])**2 for k in range(3)) <= 1e-14*l2:
                contacts.append((a,b,p,owners,other))
    pairs = set()
    checked_segments = checked_points = 0
    leaf_index = {t['id']:i for i,t in enumerate(leaves)}
    def descendant_ids(t):
        if not t['children']:
            return {leaf_index[t['id']]}
        return set.union(*(descendant_ids(c) for c in t['children']))
    for t in tiles:
        inside = descendant_ids(t)
        es = edges(t['faces'])
        ps = {p for e in es for p in e}
        for e, owners in items:
            if owners & inside and owners-inside:
                checked_segments += 1
                require(e in es, 'interfaceSegments', t['id'])
        for p, owners in points:
            if owners & inside and owners-inside:
                checked_points += 1
                require(p in ps, 'interfacePoints', t['id'])
        for a,b,p,owners,other in contacts:
            if owners & inside and other-inside:
                require(tuple(sorted((a,b))) in es, 'interfaceSegments', t['id'])
            if other & inside and owners-inside:
                require(p in ps, 'interfacePoints', t['id'])
    ancestors = {}
    for i,t in enumerate(leaves):
        chain = []
        while t is not None:
            chain.append(t)
            t = t['parent']
        ancestors[i] = chain
    for _, owners in items + [(None, owners) for _,owners in points] + [(None,owners|other) for _,_,_,owners,other in contacts]:
        for a,b in itertools.combinations(owners,2):
            for x,y in itertools.product(ancestors[a],ancestors[b]):
                if descendant_ids(x).isdisjoint(descendant_ids(y)):
                    pairs.add(tuple(sorted((x['id'],y['id']))))
    depths = {t['id']:t['depth'] for t in tiles}
    mixed = sum(depths[a]!=depths[b] for a,b in pairs)
    return dict(checkedSegments=checked_segments,checkedPoints=checked_points,
                cornerContacts=len(points),tContacts=len(contacts),mixedPairs=mixed,sameDepthPairs=len(pairs)-mixed)

def distance2(p, face):
    """Orthogonal triangle projection plus three closed segments, independent of S2."""
    a,b,c=face
    sub=lambda x,y:[x[k]-y[k] for k in range(3)]
    dot=lambda x,y:sum(u*v for u,v in zip(x,y))
    u,v,w=sub(b,a),sub(c,a),sub(p,a)
    uu,uv,vv,wu,wv=dot(u,u),dot(u,v),dot(v,v),dot(w,u),dot(w,v)
    det=uu*vv-uv*uv
    best=math.inf
    if det>0:
        x,y=(wu*vv-wv*uv)/det,(wv*uu-wu*uv)/det
        if x>=0 and y>=0 and x+y<=1:
            best=sum((w[k]-x*u[k]-y*v[k])**2 for k in range(3))
    for a,b in ((a,b),(b,c),(c,a)):
        d,w=sub(b,a),sub(p,a)
        length=dot(d,d)
        t=max(0,min(1,dot(w,d)/length)) if length else 0
        best=min(best,sum((w[k]-t*d[k])**2 for k in range(3)))
    return best

def audit(root, expected):
    root = Path(root)
    manifest = json.loads(Path(expected).read_text())
    doc = json.loads((root/'tileset.json').read_text())
    barrier = inspect_continuity(root)
    require(barrier['ok'], 'replaceContentContinuity', str(barrier['violations']))
    require(type(doc.get('geometricError')) in (int,float) and math.isfinite(doc['geometricError']) and doc['geometricError']>=0,'geometricErrorFinite','tileset G')
    tiles, observed = [], collections.Counter()
    def visit(node, parent=None, depth=0, refine='REPLACE', renderable_above=False, empty_between=False):
        require(parent is None or 'transform' not in node, 'oracleDomain', 'nested tile transforms require archive C2')
        require('implicitTiling' not in node, 'oracleDomain', 'use I1 for implicit archives')
        refine = node.get('refine',refine)
        g = node.get('geometricError')
        require(isinstance(g,(int,float)) and math.isfinite(g) and g>=0,'geometricErrorFinite',str(g))
        require(g <= (parent['g'] if parent else doc['geometricError'])+1e-8,'geometricErrorOrder',str(g))
        require(not ('content' in node and 'contents' in node),'oracleDomain','both content forms')
        contents = node.get('contents', [node['content']] if 'content' in node else [])
        content = bool(contents)
        require(not (content and renderable_above and empty_between), 'replaceContentContinuity', f'depth {depth}')
        faces=[]
        for entry in contents:
            require(not entry['uri'].endswith('.json'),'oracleDomain','source oracle requires inline explicit hierarchy')
            faces.extend(decode(root/entry['uri'])['faces'])
        tile = dict(id=str(len(tiles)),g=g,faces=faces,depth=depth,parent=parent,children=[])
        tiles.append(tile)
        for child in node.get('children',[]):
            tile['children'].append(visit(child,tile,depth+1,refine,(refine=='REPLACE') if content else renderable_above,
                False if content else empty_between or (renderable_above and refine=='REPLACE')))
        if not tile['children']:
            require(bool(content), 'leafCoverage', tile['id'])
            for f in faces:
                row=f['ids'].get('source_triangle',{})
                key=tuple(row.get(k) for k in ('node_index','mesh_index','primitive_index','triangle_index'))
                observed[key]+=1
                want=manifest['triangles'].get(','.join(map(str,key)))
                require(want is not None,'sourceIdentity',str(key))
                primitive=f['ids'].get('source_primitive',{})
                require(all(primitive.get(k)==row.get(k) for k in ('node_index','mesh_index','primitive_index','expanded_ordinal','material_index')),'sourceIdentity',str(key))
                require(all(row.get(k)==want[k] for k in ('expanded_ordinal','material_index')),'sourceIdentity',str(key))
                require(f['xyz']==want['xyz'],'leafGeometry',str(key))
                require(f['material']==want['material'],'leafMaterial',str(key))
                require([row.get(f'vertex_{k}') for k in range(3)]==want['vertices'],'sourceIdentity',str(key))
        return tile
    visit(doc['root'])
    wanted=collections.Counter({tuple(map(int,k.split(','))):1 for k in manifest['triangles']})
    require(observed==wanted,'leafCoverage',f'{observed} != {wanted}')
    result=census(tiles)
    representatives=0
    def leaf_keys(t):
        if not t['children']:
            return {tuple(f['ids']['source_triangle'][k] for k in ('node_index','mesh_index','primitive_index','triangle_index')) for f in t['faces']}
        return set.union(*(leaf_keys(c) for c in t['children']))
    for t in tiles:
        if not t['children']:continue
        candidates=sorted(leaf_keys(t))
        for f in t['faces']:
            p=[sum(v[k] for v in f['xyz'])/3 for k in range(3)]
            distances=[(distance2(p,manifest['triangles'][','.join(map(str,key))]['xyz']),key) for key in candidates]
            nearest=min(distances)[1]
            actual=f['ids'].get('source_primitive',{})
            want=manifest['triangles'][','.join(map(str,nearest))]
            require(all(actual.get(k)==want[k] for k in ('expanded_ordinal','material_index')), 'representativeIdentity',t['id'])
            require(tuple(actual.get(k) for k in ('node_index','mesh_index','primitive_index'))==nearest[:3], 'representativeIdentity',t['id'])
            representatives+=1
    result['representatives']=representatives
    result.update(ok=True,leafTriangles=sum(observed.values()),tiles=len(tiles),sourceManifestSha256=hashlib.sha256(Path(expected).read_bytes()).hexdigest())
    return result

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('root');p.add_argument('expected');a=p.parse_args()
    try:
        print(json.dumps(audit(a.root,a.expected),sort_keys=True))
    except Failure as e:
        print(json.dumps(dict(ok=False,check=e.check,detail=e.detail)))
        raise SystemExit(1)
