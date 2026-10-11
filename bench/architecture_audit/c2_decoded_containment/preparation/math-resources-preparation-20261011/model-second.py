"""Tiny standalone numerical model. Not imported/executed production Rust."""
import os
os.setpriority(os.PRIO_PROCESS, 0, 10)
os.sched_setaffinity(0, {0, 1})
import json
import math
import sys
from fractions import Fraction as F
from pathlib import Path

class Unresolved(Exception):
    pass

def singleton(v):
    assert math.isfinite(v)
    return (v, v)

def widen(a, b):
    if not math.isfinite(a) or not math.isfinite(b):
        raise Unresolved('nonfinite arithmetic')
    lo, hi = math.nextafter(a, -math.inf), math.nextafter(b, math.inf)
    if not math.isfinite(lo) or not math.isfinite(hi):
        raise Unresolved('nonfinite outward endpoint')
    return lo, hi

def add(a, b):
    if a == (0., 0.): return b
    if b == (0., 0.): return a
    if a[0] == a[1] and b[0] == b[1]:
        x, y = a[0], b[0]
        if x == -y: return (0., 0.)
        if x.is_integer() and y.is_integer() and abs(x) <= 2**52 and abs(y) <= 2**52:
            return singleton(float(int(x) + int(y)))
    return widen(a[0] + b[0], a[1] + b[1])

def neg(a): return -a[1], -a[0]
def sub(a, b): return add(a, neg(b))

def mul(a, b):
    if a == (0., 0.) or b == (0., 0.): return (0., 0.)
    if a == (1., 1.): return b
    if b == (1., 1.): return a
    if a == (-1., -1.): return neg(b)
    if b == (-1., -1.): return neg(a)
    if a[0] == a[1] and b[0] == b[1]:
        x, y = a[0], b[0]
        if x.is_integer() and y.is_integer() and abs(x) <= 2**26 and abs(y) <= 2**26:
            return singleton(float(int(x) * int(y)))
    products = [x*y for x in a for y in b]
    return widen(min(products), max(products))

def square(a):
    if a[0] <= 0 <= a[1]:
        hi = max(abs(a[0]), abs(a[1]))
        return 0., mul(singleton(hi), singleton(hi))[1]
    endpoints = [mul(singleton(v), singleton(v)) for v in a]
    return min(v[0] for v in endpoints), max(v[1] for v in endpoints)

def absolute(a):
    return (0. if a[0] <= 0 <= a[1] else min(abs(a[0]),abs(a[1])), max(abs(a[0]),abs(a[1])))

def dot(a, b):
    result = (0.,0.)
    for x,y in zip(a,b): result = add(result,mul(x,y))
    return result

def cross(a,b):
    return [sub(mul(a[1],b[2]),mul(a[2],b[1])),
            sub(mul(a[2],b[0]),mul(a[0],b[2])),
            sub(mul(a[0],b[1]),mul(a[1],b[0]))]

def apply(m,p):
    return [add(dot(m[i][:3],p),m[i][3]) for i in range(3)]

def sphere(center,radius,p):
    d=[sub(x,singleton(c)) for x,c in zip(p,center)]
    q=(0.,0.)
    for x in d:q=add(q,square(x))
    rr=square(singleton(radius))
    if q[1] <= rr[0]:return 'Contained'
    if q[0] > rr[1]:return 'Outside'
    return 'Unresolved'

def box(center,axes,p):
    d=[sub(x,singleton(c)) for x,c in zip(p,center)]
    a=[[singleton(v) for v in column] for column in axes]
    diagonal=all(axes[j][i] == 0 for j in range(3) for i in range(3) if i != j)
    if diagonal:
        decisions=[]
        for i,x in enumerate(d):
            aa=absolute(x); h=abs(axes[i][i])
            decisions.append('Contained' if aa[1]<=h else 'Outside' if aa[0]>h else 'Unresolved')
    else:
        cof=[cross(a[1],a[2]),cross(a[2],a[0]),cross(a[0],a[1])]
        det=absolute(dot(a[0],cof[0]))
        if det[0] == 0:return 'Unresolved'
        nums=[absolute(dot(d,c)) for c in cof]
        decisions=['Contained' if n[1]<=det[0] else 'Outside' if n[0]>det[1] else 'Unresolved' for n in nums]
    if 'Outside' in decisions:return 'Outside'
    return 'Contained' if all(x=='Contained' for x in decisions) else 'Unresolved'

def rational_point(m,p):
    return [sum((F(x)*y for x,y in zip(row[:3],p)),F(row[3])) for row in m]

def rational_box(center,axes,p):
    # Independent reference uses Gaussian elimination, not interval Cramer products.
    rows=[[F(axes[j][i]) for j in range(3)]+[p[i]-F(center[i])] for i in range(3)]
    pivot=0
    for col in range(3):
        selected=next((i for i in range(pivot,3) if rows[i][col]),None)
        if selected is None:continue
        rows[pivot],rows[selected]=rows[selected],rows[pivot]
        scale=rows[pivot][col]; rows[pivot]=[v/scale for v in rows[pivot]]
        for i in range(3):
            if i!=pivot:
                scale=rows[i][col]; rows[i]=[x-scale*y for x,y in zip(rows[i],rows[pivot])]
        pivot+=1
    if any(not any(r[:3]) and r[3] for r in rows):return False
    if pivot<3:
        # Only the deliberate diagonal rank-deficient model cases have free axes.
        assert all(axes[j][i]==0 for j in range(3) for i in range(3) if i!=j)
        return all(abs(p[i]-F(center[i])) <= abs(F(axes[i][i])) for i in range(3))
    return all(abs(r[3])<=1 for r in rows)

I=[[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.]]
cases=[
 {'name':'unit-box-interior','point':[.25,0.,0.],'axes':[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]],'expected':'Contained'},
 {'name':'closed-box-exact-edge','point':[1.,0.,0.],'axes':[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]],'expected':'Contained'},
 {'name':'box-one-ulp-outside','point':[math.nextafter(1.,math.inf),0.,0.],'axes':[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]],'expected':'Outside'},
 {'name':'large-box-old-relative-slack','point':[1000000.5,0.,0.],'axes':[[1e6,0.,0.],[0.,1e6,0.],[0.,0.,1e6]],'expected':'Outside'},
 {'name':'flat-box-exact-plane','point':[.5,0.,0.],'axes':[[1.,0.,0.],[0.,1.,0.],[0.,0.,0.]],'expected':'Contained'},
 {'name':'flat-box-old-residual-slack','point':[0.,0.,5e-7],'axes':[[1.,0.,0.],[0.,1.,0.],[0.,0.,0.]],'expected':'Outside'},
 {'name':'integer-rotated-box-edge','point':[2.,0.,0.],'axes':[[1.,1.,0.],[-1.,1.,0.],[0.,0.,1.]],'expected':'Contained'},
 {'name':'integer-rotated-box-outside','point':[2.25,0.,0.],'axes':[[1.,1.,0.],[-1.,1.,0.],[0.,0.,1.]],'expected':'Outside'},
 {'name':'sphere-closed-edge','point':[1.,0.,0.],'radius':1.,'expected':'Contained'},
 {'name':'sphere-one-ulp-outside','point':[math.nextafter(1.,math.inf),0.,0.],'radius':1.,'expected':'Outside'},
 {'name':'large-sphere-old-relative-slack','point':[1000000.5,0.,0.],'radius':1e6,'expected':'Outside'},
 {'name':'zero-sphere-exact-center','point':[0.,0.,0.],'radius':0.,'expected':'Contained'},
 {'name':'normalized-unsigned255-interior','point':[1.,0.,0.],'denominator':255,'radius':1.,'expected':'Contained'},
 {'name':'affine-cancellation-exact-outside','point':[float(2**53),1.,0.],'matrix':[[1.,1.,0.,-float(2**53)],[0.,0.,0.,0.],[0.,0.,0.,0.]],'radius':.25,'expected':'Unresolved'},
 {'name':'large-finite-sphere-squared-overflow','point':[1e308,0.,0.],'radius':1e308,'expected':'Unresolved'},
 {'name':'small-sphere-underflow-must-not-success','point':[2e-200,0.,0.],'radius':1e-200,'expected':'Unresolved'},
]

def main():
    records=[]
    for c in cases:
        raw=[float(x) for x in c['point']]
        den=c.get('denominator',1)
        exact=[F(v)/den for v in raw]
        source=[singleton(v) if den==1 or v==0 else widen(v/den,v/den) for v in raw]
        matrix=c.get('matrix',I)
        p=rational_point(matrix,exact)
        center=c.get('center',[0.,0.,0.])
        truth=(rational_box(center,c['axes'],p) if 'axes' in c else
               sum((x-F(y))**2 for x,y in zip(p,center)) <= F(c['radius'])**2)
        try:
            observed_point=apply([[singleton(v) for v in row] for row in matrix],source)
            assert all(F(lo)<=v<=F(hi) for v,(lo,hi) in zip(p,observed_point))
            result=box(center,c['axes'],observed_point) if 'axes' in c else sphere(center,c['radius'],observed_point)
        except Unresolved:
            result='Unresolved';observed_point=None
        assert result==c['expected'],(c['name'],result,c['expected'])
        assert result!='Contained' or truth
        assert result!='Outside' or not truth
        records.append({'name':c['name'],'literal':c,'representedPointHex':[v.hex() for v in raw],
                        'exactReferenceContained':truth,'exactTransformedPoint':[str(x) for x in p],
                        'intervalPoint':observed_point,'observed':result})
    # Semantic mutants are independently checked against exact literal expectations.
    sensitive=[]
    for name in ('large-box-old-relative-slack','flat-box-old-residual-slack','large-sphere-old-relative-slack'):
        r=next(x for x in records if x['name']==name)
        assert r['exactReferenceContained'] is False
        sensitive.append({'mutation':'allow-old-1e-6-slack','case':name,'wrongResult':'Contained','rejectedByExactReference':True})
    r=next(x for x in records if x['name']=='affine-cancellation-exact-outside')
    assert r['exactTransformedPoint'][0]=='1'
    rounded=sum([float(2**53),1.])-float(2**53)
    assert rounded==0. and r['exactReferenceContained'] is False
    sensitive.append({'mutation':'treat-rounded-transform-as-exact-position','case':r['name'],'wrongResult':'Contained','roundedX':rounded,'rejectedByExactReference':True})
    r=next(x for x in records if x['name']=='small-sphere-underflow-must-not-success')
    assert (2e-200)**2 <= (1e-200)**2 and r['exactReferenceContained'] is False
    sensitive.append({'mutation':'underflow-squares-become-equal-zero','case':r['name'],'wrongResult':'Contained','rejectedByExactReference':True})
    output={'scope':'Executed tiny standalone Python model only; not Rust/native/product/resource acceptance.',
            'actualNice':os.getpriority(os.PRIO_PROCESS,0),'actualAffinity':sorted(os.sched_getaffinity(0)),
            'python':sys.version,'cases':records,'sensitiveControls':sensitive}
    dest=Path(__file__).with_name('observed.json');assert not dest.exists();dest.write_text(json.dumps(output,indent=2)+'\n')
    print(json.dumps({'status':'PASS_MODEL_ONLY','cases':len(records),'sensitiveControls':len(sensitive),
                      'actualNice':output['actualNice'],'actualAffinity':output['actualAffinity']}))

if __name__=='__main__':main()
