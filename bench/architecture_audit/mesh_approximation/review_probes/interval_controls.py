import math,random,struct,json,hashlib
from fractions import Fraction as Q
from pathlib import Path
ROOT=Path(__file__).resolve().parents[4]
rng=random.Random(121)
D=1<<24
up=lambda x:math.nextafter(x,math.inf)
down=lambda x:math.nextafter(x,-math.inf)
def f32(x):return struct.unpack('<f',struct.pack('<f',x))[0]
def certified(p,t,w):
    result=0.
    for axis,coordinate in enumerate(p):
        low=high=0.
        for vertex,weight in zip(t,w):
            if weight==0:continue
            product=vertex[axis]*(weight/D)
            low=down(low+down(product));high=up(high+up(product))
        residual=max(abs(down(coordinate-high)),abs(up(coordinate-low)))
        result=up(result+up(residual*residual))
    return result
def exact_squared(p,t,w):
    return sum((Q(p[axis])-sum(Q(v[axis])*weight/D for v,weight in zip(t,w)))**2 for axis in range(3))
def ordinary(p,t,w):
    return sum((p[a]-sum(v[a]*(weight/D) for v,weight in zip(t,w)))**2 for a in range(3))
values=[0.,-0.,f32(2**-149),f32(-2**-149),f32(2**-126),f32(-2**-126),1.,-1.,1e6,-1e6]
controls=[([f32(2**-149),0.,0.],[[1e6,0.,0.],[-1e6,0.,0.],[0.,0.,0.]],[D//2,D//2,0])]
negative=None
for i in range(10000):
    def coord():
        return rng.choice(values) if rng.random()<.5 else f32(rng.uniform(-1,1)*2**rng.randrange(-149,20))
    p=[coord() for _ in range(3)];t=[[coord() for _ in range(3)] for _ in range(3)]
    u=rng.randrange(D+1);v=rng.randrange(D-u+1);w=[D-u-v,u,v]
    controls.append((p,t,w))
for p,t,w in controls:
    actual=exact_squared(p,t,w);bound=certified(p,t,w)
    assert math.isfinite(bound) and Q(bound)>=actual
    error=up(math.sqrt(bound)) if bound else 0.
    assert Q(error)**2>=actual
    if negative is None and Q(ordinary(p,t,w))<actual:
        negative={'point':p,'triangle':t,'weights':w,'exact_squared':str(actual),'ordinary_squared':ordinary(p,t,w),'outward_squared':bound}
assert negative is not None
result={'scope':'Independent Python IEEE operation replay of source witness arithmetic, exact Fraction truth; does not execute Rust binary',
 'production_source_sha256':hashlib.sha256((ROOT/'src/mesh_archive/approximation.rs').read_bytes()).hexdigest(),
 'controls':len(controls),'outward_understatements':0,'non_outward_negative_control':negative}
print(json.dumps(result,indent=2))
