#!/usr/bin/env python3
"""Independent exact-rational, complete-cover certificate checker for F1d2.

No producer, Rust geometry, optimizer, witness transcript or report count supplies
geometric truth. A bounded exact cover proves the scalar; exhaustion is unproven.
"""
from fractions import Fraction as F
import hashlib
import json
import math
import time

MAX_DEPTH = 24
PRODUCER_TEST_LIMIT = 16777216
CHECKER_TEST_LIMIT = 100000
CHECKER_SECONDS = 10


class CertificateError(ValueError):
    pass


class CertificateUnproven(CertificateError):
    """This bounded checker did not establish a proof, not a false-bound verdict."""


def require(condition, message):
    if not condition: raise CertificateError(message)


def sub(a, b): return tuple(x-y for x, y in zip(a, b))
def dot(a, b): return sum(x*y for x, y in zip(a, b))


def exact_face(face):
    require(len(face)==3 and all(len(p)==3 for p in face), 'three 3D face corners')
    require(all(isinstance(x,(int,float,F)) and not isinstance(x,bool) and (not isinstance(x,float) or math.isfinite(x)) for p in face for x in p), 'finite exact face coordinates')
    return tuple(tuple(F(x) for x in p) for p in face)


def children(face):
    """Exact affine midpoint cover; no rounded coordinates or point sampling.

    Any parent weights w summing to 1 lie in a corner child if w_i>=1/2,
    with weights (2*w_i-1,2*w_j,2*w_k), otherwise in the central child with
    weights (1-2*w_2,1-2*w_0,1-2*w_1) against (AB,BC,CA).
    """
    a,b,c=face
    ab,bc,ca=[tuple((x+y)/2 for x,y in zip(u,v)) for u,v in ((a,b),(b,c),(c,a))]
    return ((a,ab,ca),(ab,b,bc),(ca,bc,c),(ab,bc,ca))


class Target:
    """Precompute exact closed-face/edge data; projection is checker-owned."""
    def __init__(self, face):
        self.a,self.b,self.c=face
        self.u,self.v=sub(self.b,self.a),sub(self.c,self.a)
        self.uu,self.uv,self.vv=dot(self.u,self.u),dot(self.u,self.v),dot(self.v,self.v)
        self.den=self.uu*self.vv-self.uv*self.uv
        self.edges=[]
        for a,b in ((self.a,self.b),(self.b,self.c),(self.c,self.a)):
            e=sub(b,a);self.edges.append((a,e,dot(e,e)))

    def distance_squared(self, p):
        distances=[]
        for a,e,den in self.edges:
            r=sub(p,a);t=min(F(1),max(F(0),dot(r,e)/den)) if den else F(0)
            residual=tuple(r[i]-t*e[i] for i in range(3))
            distances.append(dot(residual,residual))
        if self.den:
            r=sub(p,self.a);ur,vr=dot(self.u,r),dot(self.v,r)
            s,t=(ur*self.vv-vr*self.uv)/self.den,(vr*self.uu-ur*self.uv)/self.den
            if s>=0 and t>=0 and s+t<=1:
                residual=tuple(r[i]-s*self.u[i]-t*self.v[i] for i in range(3))
                distances.append(dot(residual,residual))
        return min(distances)


def point_triangle_squared(point, face):
    return Target(exact_face(face)).distance_squared(tuple(F(x) for x in point))


def support_digest(faces):
    payload=[[[str(x) for x in p] for p in f] for f in faces]
    return hashlib.sha256(json.dumps(payload,separators=(',',':')).encode()).hexdigest()


def certify_regions(regions, certificate, *, max_depth=MAX_DEPTH,
                    max_tests=CHECKER_TEST_LIMIT, max_seconds=CHECKER_SECONDS):
    """Each (source faces, proxy faces) region proves both directed covers.

    Full target scans make needed metrics necessary lower bounds on the producer's
    metrics under the same exact canonical cover. Do not assert equality.
    Fraction midpoint coordinates are an exact affine reconstruction of the
    original source barycentrics, with no floating midpoint loss.
    """
    require(isinstance(certificate,(int,float,F)) and not isinstance(certificate,bool) and (not isinstance(certificate,float) or math.isfinite(certificate)) and certificate>=0, 'finite nonnegative certificate')
    require(type(max_depth) is int and 0<=max_depth<=MAX_DEPTH, 'bounded checker depth')
    require(type(max_tests) is int and 0<max_tests<=PRODUCER_TEST_LIMIT, 'bounded checker work')
    require(isinstance(max_seconds,(int,float)) and math.isfinite(max_seconds) and max_seconds>0, 'positive finite checker time')
    certificate=F(certificate);threshold=certificate*certificate
    tested=accepted=deepest=base=roots=0;worst=F(0);identities=[]
    deadline=time.monotonic()+max_seconds
    require(regions, 'nonempty proof regions')
    for left,right in regions:
        left=tuple(exact_face(f) for f in left);right=tuple(exact_face(f) for f in right)
        require(left and right,'nonempty source and proxy region supports')
        base+=2*len(left)*len(right);roots+=len(left)+len(right)
        identities.append({'source_sha256':support_digest(left),'proxy_sha256':support_digest(right),'source_faces':len(left),'proxy_faces':len(right)})
        for source,target in ((left,right),(right,left)):
            targets=[Target(f) for f in target]
            for face in source:
                stack=[(face,0)]
                while stack:
                    patch,depth=stack.pop()
                    if tested+len(targets)>max_tests:raise CertificateUnproven('independent exact cover work ceiling')
                    bound=None
                    for i,t in enumerate(targets):
                        if i%128==0 and time.monotonic()>deadline:raise CertificateUnproven('independent exact cover cooperative time ceiling')
                        b=max(t.distance_squared(p) for p in patch)
                        bound=b if bound is None else min(bound,b)
                        tested+=1
                    if bound<=threshold:
                        accepted+=1;deepest=max(deepest,depth);worst=max(worst,bound)
                    else:
                        if depth==max_depth:raise CertificateUnproven('independent exact cover depth ceiling; published scalar unproven')
                        stack.extend((child,depth+1) for child in reversed(children(patch)))
    return {'proved':True,'base_patch_face_tests':base,'root_patches':roots,
            'patch_face_tests':tested,'accepted_patches':accepted,'max_depth':deepest,
            'ideal_cover_squared':str(worst),'regions':identities,
            'checker_limits':{'max_depth':max_depth,'max_patch_face_tests':max_tests,
                              'max_seconds':max_seconds,'time_limit_is_cooperative':True}}


def validate_metrics(certificate, independent):
    require(set(certificate)=={'error_metres','patch_face_tests','accepted_patches','max_depth'},'exact F1d2 certificate fields')
    tests,leaves,depth=(certificate[k] for k in ('patch_face_tests','accepted_patches','max_depth'))
    require(all(type(x) is int for x in (tests,leaves,depth)),'integral proof metrics')
    require(independent['base_patch_face_tests']<=tests<=PRODUCER_TEST_LIMIT,'performed patch-face tests cover base within ceiling')
    require(independent['patch_face_tests']<=tests,'reported tests cover independent proof work')
    require(independent['accepted_patches']<=leaves<=tests,'reported accepted patches cover independent proof')
    require(independent['max_depth']<=depth<=MAX_DEPTH,'reported depth covers independent proof')
    roots=independent['root_patches']
    require(leaves>=roots and (leaves-roots)%3==0,'complete four-child forest leaf cardinality')
    require(leaves>=roots+3*depth,'deepest accepted branch requires complete sibling covers')
