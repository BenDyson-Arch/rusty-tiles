"""Directed interval feasibility against independent Decimal authored transforms."""
from decimal import Decimal, localcontext
from pathlib import Path
import hashlib,json,math,itertools
class I:
 def __init__(self,lo,hi=None):self.lo,self.hi=lo,lo if hi is None else hi
 def __add__(self,b):return I(math.nextafter(self.lo+b.lo,-math.inf),math.nextafter(self.hi+b.hi,math.inf))
 def __neg__(self):return I(-self.hi,-self.lo)
 def __sub__(self,b):return self+-b
 def __mul__(self,b):
  p=[x*y for x in [self.lo,self.hi] for y in [b.lo,b.hi]]
  return I(math.nextafter(min(p),-math.inf),math.nextafter(max(p),math.inf))
 def hull(self,b):return I(min(self.lo,b.lo),max(self.hi,b.hi))
 def sqrt(self):return I(math.nextafter(math.sqrt(max(0,self.lo)),-math.inf),math.nextafter(math.sqrt(self.hi),math.inf))
 def inv(self):assert self.lo>0;return I(math.nextafter(1/self.hi,-math.inf),math.nextafter(1/self.lo,math.inf))
def matrix(q,t,s,C):
 x,y,z,w=q;o=C(1);two=C(2);zero=C(0)
 r=[[o-two*(y*y+z*z),two*(x*y-z*w),two*(x*z+y*w)], [two*(x*y+z*w),o-two*(x*x+z*z),two*(y*z-x*w)],[two*(x*z-y*w),two*(y*z+x*w),o-two*(x*x+y*y)]]
 return [[r[a][b]*s[b] for b in range(3)]+[t[a]] for a in range(3)]+[[zero,zero,zero,o]]
def mul(a,b,C):return [[sum((a[r][k]*b[k][c] for k in range(3)),C(0))+(a[r][3] if c==3 else C(0)) for c in range(4)] for r in range(3)]+[[C(0),C(0),C(0),C(1)]]
def authored(q,t,s,normal,C):
 q=list(map(C,q));t=list(map(C,t));s=list(map(C,s))
 if normal:
  norm=sum((x*x for x in q),C(0)).sqrt();q=[x/norm for x in q]
 return matrix(q,t,s,C)
def interval_node(q,t,s):
 iq=list(map(I,q));norm=sum((x*x for x in iq),I(0)).sqrt();normalized=[x*norm.inv() for x in iq]
 raw=matrix(iq,list(map(I,t)),list(map(I,s)),I);normalized=matrix(normalized,list(map(I,t)),list(map(I,s)),I)
 return [[a.hull(b) for a,b in zip(ar,br)] for ar,br in zip(raw,normalized)]
def evaluate(m,p,C):return [sum((m[r][k]*C(p[k]) for k in range(3)),C(0))+m[r][3] for r in range(3)]
fixtures=[('near_unit_high',[( [0,0,0.7071069,0.7071069],[100000,-200000,300000],[1,1,1])],[800000,300000,-600000]),('reflection',[([0.2,0.3,0.4,math.sqrt(.71)],[153,-27,25],[-2,3,4])],[100,200,300]),('deep_128',[([0,0,.1,math.sqrt(.99)],[1,-1,1],[1,1,1])]*128,[500000,-300000,200000]),('cancellation',[([0,0,0,1],[1e15,0,0],[1,1,1]),([0,0,0,1],[-1e15,0,0],[1,1,1])],[.1,.2,.3])]
results=[];controls=0
with localcontext() as ctx:
 ctx.prec=90
 for name,nodes,p in fixtures:
  im=[[I(int(r==c)) for c in range(4)] for r in range(4)]
  for node in nodes:im=mul(im,interval_node(*node),I)
  limits=evaluate(im,p,I);samples=[]
  choices=itertools.product([False,True],repeat=len(nodes)) if len(nodes)<=2 else [(False,)*len(nodes),(True,)*len(nodes)]
  for decisions in choices:
   dm=[[Decimal(int(r==c)) for c in range(4)] for r in range(4)]
   for node,norm in zip(nodes,decisions):dm=mul(dm,authored(*node,norm,Decimal.from_float),Decimal)
   truth=evaluate(dm,p,Decimal.from_float)
   for value,iv in zip(truth,limits):assert Decimal.from_float(iv.lo)<=value<=Decimal.from_float(iv.hi)
   samples.append(list(map(str,truth)))
  results.append({'name':name,'depth':len(nodes),'intervals':[[v.lo,v.hi] for v in limits],'decimal_samples':samples})
 # Sensitive control: normalized-only bound excludes raw authored TRS near tolerance.
 q,t,s=fixtures[0][1][0];p=fixtures[0][2]
 raw=evaluate(authored(q,t,s,False,Decimal.from_float),p,Decimal.from_float)
 normalized=evaluate(authored(q,t,s,True,Decimal.from_float),p,Decimal.from_float)
 assert any(abs(a-b)>Decimal('0.000001') for a,b in zip(raw,normalized));controls+=1
result={'scope':'Directed-interval design feasibility; Decimal mathematical raw/normalized authored-chain enclosure, not production or arbitrary GPU float32 proof','fixtures':results,'sensitive_controls_detected':controls,'passed':True,'script_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}
Path(__file__).with_name('original-bounds-feasibility.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({'passed':True,'fixtures':len(results),'sensitive_controls_detected':controls}))
