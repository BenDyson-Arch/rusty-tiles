import random,struct,math,json
from fractions import Fraction as Q
rng=random.Random(121);up=lambda x:math.nextafter(x,math.inf);down=lambda x:math.nextafter(x,-math.inf)
f32=lambda x:struct.unpack('<f',struct.pack('<f',x))[0]
def box(lo,hi):
 c=(lo+hi)/2;h=0. if lo==hi else up(max(c-lo,hi-c));return c,h
def parent(leaves,outward):
 endpoints=[((down(c-h),up(c+h)) if outward and h else (c-h,c+h)) for c,h in leaves]
 return box(min(x for x,y in endpoints),max(y for x,y in endpoints))
def violations(leaves,p):
 c,h=map(Q,p);return sum(Q(lc)-Q(lh)<c-h or Q(lc)+Q(lh)>c+h for lc,lh in leaves)
control=[box(-4.,0.)]*2
old=parent(control,False);new=parent(control,True)
assert violations(control,old)>0;assert violations(control,new)==0
old_failures=0
for _ in range(10000):
 values=[rng.choice([0.,1e6,-1e6,f32(2**-149),f32(-2**-149)]) if rng.random()<.5 else f32(rng.uniform(-1,1)*2**rng.randrange(-149,20)) for _ in range(18)]
 leaves=[box(min(a),max(a)) for a in [values[:6],values[6:12],values[12:]]]
 old_failures+=bool(violations(leaves,parent(leaves,False)))
 assert violations(leaves,parent(leaves,True))==0
print(json.dumps({'scope':'Independent operation replay, exact Fraction child endpoint containment; no Rust execution', 'negative_control_old_parent':old,'corrected_parent':new,'old_child_failures_control':violations(control,old),'random_controls':10000,'old_random_failures':old_failures,'outward_random_failures':0},indent=2))
