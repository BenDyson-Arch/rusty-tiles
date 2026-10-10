#!/usr/bin/env python3
"""Independent exact rational checker/prototype. No Rust/math/meshopt imports.

Externally supplied supports are trusted only after an artifact decoder binds
these exact coordinates. Synthetic self-tests are checker evidence, not actual
producer acceptance. Distances below are squared; no float sqrt authorizes.
"""
from fractions import Fraction as F
from pathlib import Path
import copy
import hashlib
import json
import platform
import time

D = 1 << 24
BASIS = ((F(1), F(0), F(0)), (F(0), F(1), F(0)), (F(0), F(0), F(1)))


def demand(ok, message):
    if not ok: raise ValueError(message)


def sub(a, b): return tuple(x-y for x, y in zip(a, b))
def dot(a, b): return sum(x*y for x, y in zip(a, b))
def tri(*points): return tuple(tuple(F(x) for x in p) for p in points)
def reconstruct(face, weights): return tuple(sum(weights[i]*face[i][c] for i in range(3)) for c in range(3))


def children(face):
    a, b, c = face
    ab, bc, ca = [tuple((x+y)/2 for x, y in zip(u,v)) for u,v in ((a,b),(b,c),(c,a))]
    return ((a,ab,ca),(ab,b,bc),(ca,bc,c),(ab,bc,ca))


def patch(face, path):
    bary = BASIS
    for item in path: bary = children(bary)[item]
    return tuple(reconstruct(face, w) for w in bary)


def closest(point, target):
    """Exact rational closed-triangle projection including point/line faces."""
    a,b,c = target
    u,v,r = sub(b,a),sub(c,a),sub(point,a)
    uu,uv,vv,ur,vr = dot(u,u),dot(u,v),dot(v,v),dot(u,r),dot(v,r)
    choices = []
    den = uu*vv-uv*uv
    if den:
        s,t = (ur*vv-vr*uv)/den,(vr*uu-ur*uv)/den
        if s>=0 and t>=0 and s+t<=1: choices.append((1-s-t,s,t))
    for i,j in ((0,1),(1,2),(2,0)):
        edge = sub(target[j],target[i]); dd=dot(edge,edge)
        t=max(F(0),min(F(1),dot(sub(point,target[i]),edge)/dd)) if dd else F(0)
        weights=[F(0)]*3; weights[i]=1-t;weights[j]=t;choices.append(tuple(weights))
    def distance(w):
        diff=sub(point,reconstruct(target,w)); return dot(diff,diff)
    return min((distance(w),w) for w in choices)


def support_hash(supports):
    data=[[[str(x) for x in p] for p in face] for face in supports]
    return hashlib.sha256(json.dumps(data,separators=(',',':')).encode()).hexdigest()


def whole_squared(left,right):
    def directed(a,b): return max(min(max(closest(p,t)[0] for p in s) for t in b) for s in a)
    return max(directed(left,right),directed(right,left))


def weights24(weights):
    """Independent quantization proposal; checker does not trust this generator."""
    values=[int(w*D) for w in weights]
    missing=D-sum(values)
    order=sorted(range(3),key=lambda i:(weights[i]*D-values[i],-i),reverse=True)
    for i in order[:missing]: values[i]+=1
    return values


def generate(left,right,budget,depth=5):
    """Authored focused prototype, exact adaptive ideal vs dyadic witness bound.

    Every attempted patch tests every target. Terminal ideal-optimal witnesses
    are rounded only to produce a separate dyadic transcript; any rounding
    loss remains visible and can cause the dyadic checker to refuse.
    """
    leaves=[]; ideal=F(0);dyadic=F(0);comparisons=0
    for direction,(source,target) in enumerate(((left,right),(right,left))):
        for face_id,face in enumerate(source):
            pending=[()]
            while pending:
                path=pending.pop(); points=patch(face,path)
                choices=[]
                for target_id,t in enumerate(target):
                    witnesses=[closest(p,t) for p in points]
                    choices.append((max(x[0] for x in witnesses),target_id,[x[1] for x in witnesses]))
                    comparisons+=1
                bound,target_id,weights=min(choices)
                if bound>budget*budget and len(path)<depth:
                    pending.extend(path+(i,) for i in reversed(range(4)))
                    continue
                witness=[weights24(w) for w in weights]
                this_dyadic=max(dot(sub(p,reconstruct(target[target_id],tuple(F(w,D) for w in q))),sub(p,reconstruct(target[target_id],tuple(F(w,D) for w in q)))) for p,q in zip(points,witness))
                ideal=max(ideal,bound);dyadic=max(dyadic,this_dyadic)
                leaves.append({'direction':direction,'face':face_id,'path':list(path),'target':target_id,'weights':witness})
    return {'schema':'independent-f1d2-proof-prototype-v1','left_sha256':support_hash(left),'right_sha256':support_hash(right),'leaves':leaves},ideal,dyadic,comparisons


def complete(paths):
    """Complete prefix-free four-child tree, independent of physical triangle area.

    Degenerate physical supports still require full abstract parameter coverage.
    Kraft equality alone is insufficient if a parent and descendant overlap.
    """
    tree={}
    for path in paths:
        node=tree
        for child in path:
            demand(type(child) is int and child in range(4),'child index 0..3')
            demand('leaf' not in node,'overlapping parent/descendant patches')
            node=node.setdefault(child,{})
        demand(not node,'duplicate or overlapping patch')
        node['leaf']=True
    def visit(node):
        if 'leaf' in node: demand(len(node)==1,'leaf has descendants'); return
        demand(set(node)==set(range(4)),'incomplete four-child cover')
        for child in range(4):visit(node[child])
    visit(tree)


def check(left,right,proof,reported,budget,max_depth=12,max_leaves=100000):
    demand(set(proof)=={'schema','left_sha256','right_sha256','leaves'},'exact transcript fields')
    demand(proof['schema']=='independent-f1d2-proof-prototype-v1','proof schema')
    demand(proof['left_sha256']==support_hash(left) and proof['right_sha256']==support_hash(right),'support binding')
    demand(left and right,'nonempty supports')
    demand(F(0)<=reported<=budget and budget>0,'finite ordered budget/certificate')
    demand(0<len(proof['leaves'])<=max_leaves,'bounded proof leaves')
    covers={(d,i):[] for d,faces in enumerate((left,right)) for i in range(len(faces))}
    measured=F(0)
    for leaf in proof['leaves']:
        demand(set(leaf)=={'direction','face','path','target','weights'},'exact leaf fields; never trust supplied source points')
        d,i,j,path,w = leaf['direction'],leaf['face'],leaf['target'],leaf['path'],leaf['weights']
        demand(type(d) is int and d in (0,1),'both directions')
        source,target=(left,right) if d==0 else (right,left)
        demand(type(i) is int and 0<=i<len(source) and type(j) is int and 0<=j<len(target),'face ordinal')
        demand(type(path) is list and len(path)<=max_depth and all(type(c) is int and c in range(4) for c in path),'bounded valid path')
        demand(len(w)==3 and all(len(q)==3 and all(type(x) is int and x>=0 for x in q) and sum(q)==D for q in w),'three nonnegative dyadic simplex witnesses')
        covers[(d,i)].append(path)
        for point,weights in zip(patch(source[i],path),w):
            q=reconstruct(target[j],tuple(F(x,D) for x in weights));v=sub(point,q);squared=dot(v,v)
            demand(squared<=reported*reported,'understated exact dyadic witness distance')
            measured=max(measured,squared)
    for paths in covers.values(): complete(paths)
    return {'squared_witness_bound':str(measured),'leaves':len(proof['leaves']),'source_faces':len(left),'target_faces':len(right)}


class Unproven(ValueError):
    """Independent checker exceeded its explicit domain/work/depth ceiling."""


def recertify(left,right,reported,max_depth=24,max_tests=100000,max_seconds=10):
    """Independent exact-rational upper-bound proof against an emitted scalar.

    Do not use old unpartitioned ideal as a lower bound. This checker accepts
    its own exact full canonical cover, scanning every target for each patch.
    It never consumes producer subdivision decisions or target witnesses.
    An Unproven result is not a proof that the producer certificate is false.
    """
    demand(left and right and reported>=0,'nonempty supports / nonnegative certificate')
    tested=terminal=0;max_observed_depth=0;worst=F(0);deadline=time.monotonic()+max_seconds
    for source,target in ((left,right),(right,left)):
        for face in source:
            pending=[()]
            while pending:
                path=pending.pop()
                if tested+len(target)>max_tests:raise Unproven('independent patch-target work ceiling')
                points=patch(face,path)
                bounds=[]
                for ordinal,t in enumerate(target):
                    if ordinal%128==0 and time.monotonic()>deadline:raise Unproven('independent elapsed-time ceiling')
                    bounds.append(max(closest(p,t)[0] for p in points))
                bound=min(bounds)
                tested+=len(target);max_observed_depth=max(max_observed_depth,len(path))
                if bound<=reported*reported:
                    terminal+=1;worst=max(worst,bound)
                else:
                    if len(path)==max_depth:raise Unproven('independent exact cover not established at depth ceiling')
                    pending.extend(path+(i,) for i in reversed(range(4)))
    return {'patch_target_tests':tested,'terminal_patches':terminal,'max_depth':max_observed_depth,'ideal_cover_squared':str(worst),'left_sha256':support_hash(left),'right_sha256':support_hash(right),'proved':True}


def recertification_control(label,left,right,bound,max_tests=100000):
    try:recertify(left,right,bound,max_tests=max_tests)
    except Unproven as error:return {'control':label,'accepted':False,'classification':'unproven under independent checker limits','reason':str(error)}
    raise AssertionError('insensitive independent scalar control '+label)


def rejection(label,left,right,proof,bound,budget):
    try:check(left,right,proof,bound,budget)
    except ValueError as error:return {'control':label,'rejected':True,'reason':str(error)}
    raise AssertionError('insensitive '+label)


def main():
    started=time.monotonic()
    base=[tri((0,0,0),(8,0,0),(0,8,0))]; quarters=list(children(base[0]))
    proof,ideal,dyadic,work=generate(base,quarters,F(1,16))
    demand(whole_squared(base,quarters)==16 and ideal==dyadic==0,'4m old / exact zero new')
    scalar_checks=[{'case':'equal-planar-retriangulation','check':recertify(base,quarters,F(0))}]
    scalar_controls=[]
    positives=[{'case':'equal-planar-retriangulation','old_squared':'16','ideal_squared':str(ideal),'dyadic_squared':str(dyadic),'comparisons':work,'check':check(base,quarters,proof,F(0),F(1,16))}]
    controls=[]
    for label,mutate in [('missing-child',lambda p:p['leaves'].pop(1)),('duplicate-child',lambda p:p['leaves'].append(copy.deepcopy(p['leaves'][0]))),('missing-reverse-direction',lambda p:p.update(leaves=[x for x in p['leaves'] if x['direction']==0])),('invalid-witness-sum',lambda p:p['leaves'][0]['weights'][0].__setitem__(0,D+1)),('negative-witness',lambda p:p['leaves'][0]['weights'][0].__setitem__(0,-1)),('wrong-target',lambda p:p['leaves'][0].update(target=1)),('support-hash-corruption',lambda p:p.update(left_sha256='0'*64)),('forged-source-corners',lambda p:p['leaves'][0].update(source_points=[[0,0,0]]*3)),('invalid-path',lambda p:p['leaves'][0].update(path=[4]))]:
        changed=copy.deepcopy(proof);mutate(changed);controls.append(rejection(label,base,quarters,changed,F(0),F(1,16)))
    changed=copy.deepcopy(proof);changed['leaves'].append(dict(changed['leaves'][0],path=[]));controls.append(rejection('parent-descendant-overlap',base,quarters,changed,F(4),F(4)))
    shifted=[tri(*[(x,y,z+F(1,8)) for x,y,z in t]) for t in quarters]
    cases=[('positive-dyadic-offset',base,shifted,F(1,8),F(1,8)),('spike',base,[tri((0,0,0),(8,0,0),(0,8,100))],F(100),F(100)),('degenerate-point', [tri((3,4,0),(3,4,0),(3,4,0))],[tri((0,0,0),(0,0,0),(0,0,0))],F(5),F(5))]
    square=[tri((0,0,0),(8,0,0),(8,8,0)),tri((0,0,0),(8,8,0),(0,8,0))]
    ring=[tri((0,0,0),(8,0,0),(8,1,0)),tri((0,0,0),(8,1,0),(0,1,0)),tri((0,7,0),(8,7,0),(8,8,0)),tri((0,7,0),(8,8,0),(0,8,0)),tri((0,1,0),(1,1,0),(1,7,0)),tri((0,1,0),(1,7,0),(0,7,0)),tri((7,1,0),(8,1,0),(8,7,0)),tri((7,1,0),(8,7,0),(7,7,0))]
    cases.append(('interior-hole',square,ring,F(4),F(4)))
    thin=[tri((999999,0,0),(1000000,0,0),(999999,F(1,2**30),0))]
    thin_shift=[tri(*[(x,y,z+F(1,2**30)) for x,y,z in t]) for t in thin]
    cases.append(('mixed-magnitude-thin-nonzero',thin,thin_shift,F(1,2**30),F(1,2**30)))
    # Midpoint of two exactly represented floats is not necessarily representable.
    rounding=[tri((1,0,0),(F(1,2**149),0,0),(1,1,0))]
    exact_mid=patch(rounding[0],(0,))[1][0]
    demand(exact_mid==F(1,2)+F(1,2**150) and F(float(exact_mid))!=exact_mid,'nonrepresentable true midpoint trap')
    for name,left,right,budget,bound in cases:
        p,exact,dyadic,work=generate(left,right,budget)
        checked=check(left,right,p,bound,budget)
        scalar_checks.append({'case':name,'check':recertify(left,right,bound)})
        if dyadic>0:scalar_controls.append(recertification_control(name+'-forged-zero',left,right,F(0)))
        positives.append({'case':name,'old_squared':str(whole_squared(left,right)),'ideal_squared':str(exact),'dyadic_squared':str(dyadic),'comparisons':work,'check':checked})
        if dyadic>0:controls.append(rejection(name+'-forged-zero',left,right,p,F(0),budget))
        if name=='positive-dyadic-offset':
            scalar_controls.append(recertification_control('positive-budget-too-tight',left,right,F(1,16)))
            controls.append(rejection('positive-budget-too-tight',left,right,p,F(1,16),F(1,16)))
            rp,ri,rd,rw=generate(list(reversed(left)),list(reversed(right)),budget)
            demand(ri==exact and rd==dyadic,'reordering preserves nonzero bound')
            scalar_checks.append({'case':'reordered-nonzero','check':recertify(list(reversed(left)),list(reversed(right)),bound)})
            positives.append({'case':'reordered-nonzero','ideal_squared':str(ri),'dyadic_squared':str(rd),'check':check(list(reversed(left)),list(reversed(right)),rp,bound,budget)})
    scalar_controls.append(recertification_control('independent-work-ceiling',base,quarters,F(0),max_tests=1))
    # A non-dyadic projection exposes difference between ideal and actual witnesses.
    oblique=[tri((0,0,0),(3,1,0),(0,0,0))]
    query=[tri((1,0,0),(1,0,0),(1,0,0))]
    exact,w=closest(query[0][0],oblique[0]);q=reconstruct(oblique[0],tuple(F(x,D) for x in weights24(w)));delta=sub(query[0][0],q);rounded=dot(delta,delta)
    demand(exact==F(1,10) and rounded>exact,'ideal is a relaxation, never actual witness upper bound')
    result={'scope':'Authored independent exact adaptive scalar re-certifier, exploratory transcript checker and sensitive controls only; no production implementation or emitted archive acceptance','checker_limits':{'max_depth':24,'max_patch_target_tests':100000,'max_seconds':10,'time_limit_is_cooperative':True},'base_source_commit':'3b5703231bf3991c2e37caef99989214308cc08d','python':platform.python_version(),'driver_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'positives':positives,'controls':controls,'independent_scalar_checks':scalar_checks,'independent_scalar_controls':scalar_controls,'rounding_trap':{'exact_midpoint':str(exact_mid),'rounded_midpoint':str(F(float(exact_mid)))},'ideal_vs_dyadic_projection':{'ideal_squared':str(exact),'dyadic_squared':str(rounded),'ideal_weights':[str(x) for x in w]},'elapsed_seconds':round(time.monotonic()-started,6),'passed':True}
    print(json.dumps(result,indent=2,sort_keys=True))


if __name__=='__main__':main()
