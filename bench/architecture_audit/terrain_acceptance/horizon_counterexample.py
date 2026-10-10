"""Boundary counterexample using Cesium's pinned scalar horizon predicate."""
import json

def visible(p, c):
    vh = sum(v*v for v in c)-1
    vt = [p[i]-c[i] for i in range(3)]
    dot = -sum(vt[i]*c[i] for i in range(3))
    ratio = dot*dot/sum(v*v for v in vt)
    return not (dot>0 if vh<0 else dot>vh and ratio>vh), dict(vh=vh,vt_dot=dot,ratio=ratio)

q=[0.,1e15,0.]
p=[0.999999998858649,0.,0.0000477776343128956]
c=[0.9999999988586492,-1.,0.00004777763431289561]
proxy, proxy_terms=visible(q,c)
vertex, vertex_terms=visible(p,c)
assert not proxy and vertex
print(json.dumps(dict(cesium_pin="df52c781de3491a4b76839d420f7ca90a032efb6",proxy=q,vertex=p,camera=c,proxy_visible=proxy,vertex_visible=vertex,proxy_terms=proxy_terms,vertex_terms=vertex_terms,limitation="Boundary counterexample from decoded grid17 geometry; not a claim about rendering defect size."),indent=2))
