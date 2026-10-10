#!/usr/bin/env python3
"""Exact rational oracle for the inspected bounded TwoSum budget algorithm.

No Rust/production module imports or binary calls. This checks mathematical
algorithm behavior against exact binary64 rational input values.
"""
from decimal import Decimal, localcontext
from fractions import Fraction
from pathlib import Path
import hashlib,itertools,json,math,random
BOUND=58_998_676.192_431_12
ROOT=Path('/home/bend/.cache/rusty-tiles-f1c1-placement/src/mesh_archive/placement.rs')

def candidate(components):
    components=list(map(abs,components))
    if any(x>BOUND for x in components):return False,0
    expansion=[];max_count=0
    for value in [*components,-BOUND]:
        next_terms=[];total=value
        for component in expansion:
            combined=total+component
            virtual_component=combined-total
            residual=(total-(combined-virtual_component))+(component-virtual_component)
            if residual!=0:next_terms.append(residual)
            total=combined
        if total!=0:next_terms.append(total)
        expansion=next_terms;max_count=max(max_count,len(expansion))
    return not expansion or expansion[-1]<0,max_count

def oracle(components):return sum((Fraction(abs(x)) for x in components),Fraction())<=Fraction(BOUND)

named={
 'at_boundary':[BOUND,0,0,0],
 'next_up':[math.nextafter(BOUND,math.inf),0,0,0],
 'next_down':[math.nextafter(BOUND,-math.inf),0,0,0],
 'boundary_plus_min_subnormal':[BOUND,math.ulp(0.),0,0],
 'boundary_plus_three_subnormals':[BOUND,math.ulp(0.),-math.ulp(0.),math.ulp(0.)],
 'next_down_plus_three_tenths_nanometre':[math.nextafter(BOUND,-math.inf),1e-10,-1e-10,1e-10],
 'next_down_plus_ten_nanometres':[math.nextafter(BOUND,-math.inf),1e-8,0,0],
 'large_terms_cannot_cancel':[BOUND,-BOUND,0,0],
 'finite_max':[float.fromhex('0x1.fffffffffffffp+1023'),0,0,0],
 'four_equal_shares':[BOUND/4]*4,
}
counts={'cases':0,'disagreements':0,'maximum_expansion_terms':0,'naive_rounded_sum_false_acceptances':0}
def check(values):
    actual,n=candidate(values);expected=oracle(values)
    counts['cases']+=1;counts['maximum_expansion_terms']=max(counts['maximum_expansion_terms'],n)
    if sum(map(abs,values))<=BOUND and not expected:counts['naive_rounded_sum_false_acceptances']+=1
    if actual!=expected:
        counts['disagreements']+=1
        raise AssertionError((values,actual,expected))
    assert n<=5

named_result=[]
for name,values in named.items():
    for values_permutation in itertools.permutations(values):check(values_permutation)
    named_result.append({'case':name,'components':values,'accepted':oracle(values)})
rng=random.Random(142)
for _ in range(30000):
    values=[math.ldexp(rng.random(),rng.randint(-1073,26)) for _ in range(4)]
    check(values)
for _ in range(10000):
    offsets=[math.ldexp(rng.random(),rng.randint(-1073,20)) for _ in range(3)]
    exact_remaining=Fraction(BOUND)-sum(map(Fraction,offsets))
    height=float(exact_remaining)
    for h in (height,math.nextafter(height,-math.inf),math.nextafter(height,math.inf)):
        check([h,*offsets]);check([*offsets,h])
with localcontext() as context:
    context.prec=100
    available=Decimal(2)**26-Decimal(6378137)-Decimal(3).sqrt()*Decimal(1000000)
    lower=Decimal(BOUND);upper=Decimal(math.nextafter(BOUND,math.inf))
    assert lower<=available<upper
    remaining={'exact_remaining_budget_decimal':str(available),'bound_binary64_exact_decimal':str(lower),'next_up_binary64_exact_decimal':str(upper),'floor_gap_m':str(available-lower)}
output={'scope':'Independent algorithm probe against exact Fraction arithmetic; does not execute candidate Rust or certify compiled code.',
        'inspected_placement_source_sha256':hashlib.sha256(ROOT.read_bytes()).hexdigest(),
        'probe_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        'reference':'Exact binary64 rational inputs via Fraction;100-digit Decimal irrational original-domain bound',
        'counts':counts,'remaining_budget':remaining,'named_controls':named_result,'passed':True}
Path('/home/bend/.cache/rusty-tiles-f1c1-spatial-probes/budget-results.json').write_text(json.dumps(output,indent=2)+'\n')
print(json.dumps(counts));print(json.dumps(remaining))
