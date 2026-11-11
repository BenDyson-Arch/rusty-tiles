#!/usr/bin/env python3
"""Permanent independent controls for bounded sampled seam diagnostics.

Run inside the caller's resource scope. Prints its receipt; --receipt optionally
writes it outside the source tree. No producer imports or generated GLBs.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import random
import tempfile
from fractions import Fraction as F
import seam_diagnostic as m

def segments(points):
    return list(zip(points, points[1:]))

def reference(p, edge):
    # Independent closed-segment projection with explicit endpoint cases.
    a, b = [tuple(map(F, v)) for v in edge]
    v = tuple(x-y for x,y in zip(p,a))
    d = tuple(x-y for x,y in zip(b,a))
    dot = sum(x*y for x,y in zip(v,d))
    den = sum(x*x for x in d)
    if dot <= 0 or den == 0:
        return sum(x*x for x in v)
    if dot >= den:
        return sum((x-y)**2 for x,y in zip(p,b))
    return sum(x*x for x in v) - dot*dot/den

def exact_sampled(a, b, count):
    best = F(0)
    for source, target in [(a,b),(b,a)]:
        for x,y in source:
            x,y = tuple(map(F,x)),tuple(map(F,y))
            for i in range(count):
                p = tuple(((count-1-i)*s+i*t)/(count-1) for s,t in zip(x,y))
                best = max(best,min(reference(p,e) for e in target))
    return best


def source_open_trace_probe(connected=True):
    """Original exact T split, two mixed-depth cuts, no inferred locked chord."""
    from oracle import require
    from spatial import Store
    from winding import inspect
    a,b,c,t,e=(0,0,0),(2,0,0),(0,2,0),(1,0,0),(1,-1,0)
    opposite=c if connected else (0,-2,0)
    source=[[[a,b,c]],[[a,t,opposite],[t,b,opposite]]]
    root=dict(id='0',children=[],parent=None,lo=0,hi=2,rendered=False,depth=0)
    tiles=[root];leaves=[]
    for i in range(2):
        coarse=dict(id=str(1+2*i),children=[],parent=root,lo=i,hi=i+1,rendered=True,depth=1)
        leaf=dict(id=str(2+2*i),children=[],parent=coarse,lo=i,hi=i+1,rendered=True,depth=2)
        coarse['children']=[leaf];root['children'].append(coarse);tiles.extend((coarse,leaf));leaves.append(leaf)
    with tempfile.TemporaryDirectory() as temporary:
        store=Store(Path(temporary)/'source.sqlite')
        try:
            for leaf,triangles in enumerate(source):
                for rank,xyz in enumerate(triangles):
                    store.source_face(f'{leaf},0,0,{rank}',{'xyz':xyz},leaf,actual={'xyz':xyz},depth=2)
                store.put_faces(2+2*leaf,({'xyz':xyz} for xyz in triangles))
            store.put_faces(1,[{'xyz':[a,b,c]}])
            store.put_faces(3,[{'xyz':[a,e,opposite]},{'xyz':[e,b,opposite]}])
            census=store.census(tiles,leaves,require)
            inspect(store,tiles,require)
            measured=m.inspect_open_edges(store,tiles,leaves)
            return census,measured
        finally:store.close()

def run():
    checks=[]
    def record(name, **detail):
        checks.append(dict(name=name,passed=True,**detail))
    straight = [((0,0),(4,0))]
    detour = segments([(0,0),(2,1),(4,0)])
    r = m.measure(straight,detour)
    assert r['status']=='ok' and r['sampled_lower_bound']==1 and r['exact_nearest_sampled']
    assert not r['continuous_maximum_certified'] and r['sampled_lower_bound_certified']
    record('complete_detour_width_positive', bound=r['sampled_lower_bound'])

    left = segments([(0,0),(0,3),(4,0),(4,3)])
    right = segments([(0,0),(4,0),(0,3),(4,3)])
    results = [m.measure(left,right,samples_per_segment=n) for n in (2,3,9)]
    assert [r['sampled_lower_bound'] for r in results]==[0.0,1.2,1.5]
    # An unsampled source point independently proves Hausdorff >= 1.5.
    witness = (F(3,2),F(0))
    assert min(reference(witness,e) for e in left)==F(9,4)
    assert all(r['exact_nearest_sampled'] and not r['continuous_maximum_certified'] for r in results)
    record('different_sampling_does_not_certify_continuous_maximum', samples=[2,3,9], bounds=[r['sampled_lower_bound'] for r in results], continuous_witness_lower_bound=1.5)

    chain=m.trace(detour,(0,0),(4,0))
    assert chain['complete'] and chain['segments']==detour and chain['edge_count']==2
    assert chain['points'][1]==(2,1)
    assert m.trace(detour+[((2,1),(3,3))],(0,0),(4,0))['status']=='ambiguous'
    assert m.trace(detour+[detour[0]],(0,0),(4,0))['status']=='ambiguous'
    assert m.trace(detour+[((0,0),(4,0))],(0,0),(4,0))['status']=='ambiguous'
    assert m.trace([((0,0),(0,0))]+detour,(0,0),(4,0))['status']=='ambiguous'
    assert m.trace(detour,(0,0),(0,0))['status']=='ambiguous'
    assert m.trace(detour,(0,0),(9,0))['status']=='missing'
    assert m.trace([((0,0),(1,0)),((3,0),(4,0))],(0,0),(4,0))['status']=='missing'
    assert m.trace(detour,(2,1),(4,0))['status']=='ambiguous'
    other=m.trace(detour+[((20,0),(21,0))],(0,0),(4,0))
    assert other['status']=='ok' and other['ignored_edges']==1
    assert m.trace(detour,(0,0),(4,0),max_edges=1)['status']=='limit'
    record('trace_preserves_detour_and_refuses_ambiguity_missing_cycles_duplicates')

    # Root then first leaf consume the two-node budget. Its box contains the query,
    # but its actual squared distance is 20; unresolved leaf has box lower bound 1/4.
    target=[((-10,-10),(10,0)),((0.5,-1),(1,1))]
    meter=m._Work(None)
    tree=m._bvh(target,meter)
    p=(F(0),F(0))
    lo,complete,visits=m._nearest(p,target,tree,2,meter)
    actual=min(reference(p,e) for e in target)
    visited_upper=reference(p,target[0])
    assert not complete and visits==2 and lo==F(1,4)
    assert lo < actual == F(9,17) < visited_upper == 20
    full,complete,_=m._nearest(p,target,tree,3,meter)
    assert complete and full==actual
    record('unresolved_frontier_is_lower_bound_not_visited_upper', lower_squared=str(lo), true_squared=str(actual), visited_upper_squared=str(visited_upper), node_visits=visits)

    zero=m.measure(left,right,query_budget=0)
    assert zero['status']=='unresolved' and zero['unresolved_queries']==zero['queries']
    assert not zero['exact_nearest_sampled'] and zero['sampled_lower_bound']==0
    record('zero_budget_has_explicit_unresolved_queries', queries=zero['queries'])

    for args,status in [((None,straight),'missing'),(([],straight),'empty'),((straight,None),'missing'),((straight,[]),'empty'),(([((float('nan'),0),(1,0))],straight),'invalid'),(([((0,0,0),(1,0,0))],straight),'invalid'),(([((2**53+1,0),(1,0))],straight),'invalid')]:
        r=m.measure(*args)
        assert r['status']==status and r['sampled_lower_bound'] is None and r['unresolved_queries'] is None and not r['sampled_lower_bound_certified']
    assert m.trace([], (0,0),(1,0))['status']=='empty'
    assert m.trace(None,(0,0),(1,0))['status']=='missing'
    assert m.measure(straight,straight,query_budget=-1)['status']=='invalid'
    assert m.measure(straight,straight,samples_per_segment=34)['status']=='invalid'
    record('missing_empty_invalid_are_explicit_never_numeric_zero')

    rng=random.Random(3791)
    trials=0
    for _ in range(16):
        a=segments([tuple(rng.randint(-7,7) for k in range(3)) for i in range(5)])
        b=segments([tuple(rng.randint(-7,7) for k in range(3)) for i in range(6)])
        expected=exact_sampled(a,b,3)
        for budget in (0,1,2,3,5,9,256):
            r=m.measure(a,b,query_budget=budget)
            assert F(r['sampled_lower_bound'])**2 <= expected
            assert all(d['node_visits'] <= d['queries']*budget for d in r['directions'])
            assert r['exact_nearest_sampled']==(r['unresolved_queries']==0)
            if budget==256:
                assert r['exact_nearest_sampled']
                # Downward rounding can differ from sqrt by at most one ulp here.
                assert math.nextafter(r['sampled_lower_bound'],math.inf)**2 >= float(expected)
            trials+=1
    record('independent_exact_reference_and_query_caps', comparisons=trials)

    for a,b in [([((0.,0.),(5e-324,0.))],[((0.,5e-324),(5e-324,5e-324))]), ([((1e100,0.),(-1e100,0.))],[((1e100,1e100),(-1e100,1e100))]), ([((0.,0.),(0.,0.))],[((1.,1.),(1.,1.))])]:
        r=m.measure(a,b,samples_per_segment=4)
        exact=exact_sampled(a,b,4)
        assert r['exact_nearest_sampled'] and F(r['sampled_lower_bound'])**2 <= exact
    # sqrt downward check against irrational and adjacent representable numbers.
    for q in [F(2),F(1,3),F(1,10**700),F(10**200),F(1,2**2148)]:
        assert F(m._sqrt_lower(q))**2<=q
    record('exact_numeric_roundoff_subnormal_large_degenerate_controls')

    class Charge:
        def __init__(self): self.used=0
        def charge(self,n=1): self.used+=n
    w=Charge(); r=m.measure(straight,detour,work=w)
    assert w.used==r['work'] and w.used>r['queries']
    w=Charge(); r=m.trace(detour,(0,0),(4,0),work=w)
    assert w.used==r['work'] and w.used>2*len(detour)
    class Refuse(Exception): pass
    class Limit:
        def charge(self,n=1): raise Refuse('external work exhaustion')
    for call in [lambda:m.measure(straight,detour,work=Limit()),lambda:m.trace(detour,(0,0),(4,0),work=Limit())]:
        try: call()
        except Refuse: pass
        else: raise AssertionError('external refusal swallowed')
    # Local cap also applies when no external meter exists.
    saved=m.MAX_WORK
    m.MAX_WORK=1
    try:
        try: m.measure(straight,detour)
        except m.WorkLimit: pass
        else: raise AssertionError('local refusal swallowed')
    finally: m.MAX_WORK=saved
    record('work_charges_and_external_local_refusals_propagate')

    n=1024
    many=[((i,0),(i+1,0)) for i in range(n)]
    w=m._Work(None); nodes,root=m._bvh(many,w)
    assert len(nodes)==2*n-1 and root==len(nodes)-1
    assert w.used < 30*n*math.ceil(math.log2(n))
    def height(i):
        _,leaf,a,b=nodes[i]
        return 1 if leaf is not None else 1+max(height(a),height(b))
    assert height(root)==11
    # Input cap checks before materializing a cap+1 edge.
    saved=m.MAX_EDGES
    # _segments' explicit cap path also underlies trace; no large payload needed.
    try:
        m._segments(iter(many),m._Work(None),cap=3)
    except m._InputError as e:
        assert e.status=='limit'
    else: raise AssertionError('edge cap not enforced')
    record('balanced_bvh_linear_nodes_bounded_build_work_and_input_cap', edges=n, nodes=len(nodes), height=height(root), work=w.used)
    census,result=source_open_trace_probe()
    assert census['sharedPoints']==3 and census['sharedSegments']==2
    assert census['sourceOpenEdges']==3 and census['sourceOpenEdgesWithSharedEndpoints']==1
    assert census['exemptOpenEdgeChanges']==0  # Retained source AB alone cannot prove no crack.
    assert result['candidatePairs']==4 and result['measuredPairs']==4
    assert result['positiveSampledWidthPairs']==2 and result['maximumExemptCrackWidthSampledLowerBound']==1
    assert result['sampledLowerBoundCertified'] and not result['continuousMaximumCertified']
    record('source_open_T_complete_trace_detour_width',pairs=result['measuredPairs'],positivePairs=2,bound=1)
    _,result=source_open_trace_probe(False)
    assert result['ambiguousTracePairs']>0 and not result['measuredPairs']
    assert result['maximumExemptCrackWidthSampledLowerBound'] is None
    record('source_open_T_ambiguous_correspondence_is_unmeasured_not_zero')

    return dict(result='PASS',controls=checks,
        sourceSha256=hashlib.sha256(Path(m.__file__).read_bytes()).hexdigest(),
        continuousMaximumCertified=False,
        scope='independent exact sampled-distance references and bounded trace controls; no real archive acceptance')


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--receipt',type=Path)
    a=p.parse_args();result=run();text=json.dumps(result,indent=2,sort_keys=True)+'\n'
    if a.receipt:a.receipt.write_text(text)
    print(text,end='')
