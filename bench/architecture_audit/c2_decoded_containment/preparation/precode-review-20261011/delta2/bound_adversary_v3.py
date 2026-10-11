"""Delta2 probe (reviewer-authored): v3 normative span definition, re-implemented from
contract-draft.md v3 (sha256 f840ff0b...) lines 224-350, independently of verify/frozen_bound_probe.py.

v3 rules exercised: ZERO for +-0; fixed raw-word pairs; exact k pairs; literal 2 = (1,1);
product with ZERO absorption and (0,0) pass-through; flat sums over the written term lists;
transform-less nodes skipped; G starts at identity; tile step only when the child has a transform;
box D/CW/HW and sphere Q/LHS/R2/W2/RHS; per-level spans sInst/s0/s_l.

Checks (a) soundness: every quantity and every left-to-right prefix of every flat sum against
exact Fractions; (b) the contract's worked example (sInst=5, levels [23,29], 8 vertex units,
128 node units); (c) hull-vs-instance divergence of the v3 precheck (Pair0 hull per URI).
Single-threaded, deterministic, stdlib only. python3 -I.
"""
import json
import math
import random
import struct
import sys
from fractions import Fraction as F

ZERO = None
VIOL = []
NCHK = [0]


def f32(x):
    return struct.unpack('<f', struct.pack('<f', x))[0]


def lowbit(v):
    n, d = v.numerator, v.denominator
    return ((n & -n).bit_length() - 1) - (d.bit_length() - 1)


def highbit(v):
    n, d = abs(v.numerator), v.denominator
    e = n.bit_length() - d.bit_length()
    if (n << max(0, -e)) < (d << max(0, e)):
        e -= 1
    return e


def ipair(x):
    x = F(x)
    return ZERO if x == 0 else (lowbit(x), highbit(x))


def bmul(a, b):
    if a is ZERO or b is ZERO:
        return ZERO
    if a == (0, 0):
        return b
    if b == (0, 0):
        return a
    return (a[0] + b[0], a[1] + b[1] + 1)


def bsum(ts):
    t = [x for x in ts if x is not ZERO]
    if not t:
        return ZERO
    if len(t) == 1:
        return t[0]
    return (min(x[0] for x in t), max(x[1] for x in t) + math.ceil(math.log2(len(t))))


def span(p):
    return 0 if p is ZERO else p[1] - p[0] + 1


def chk(v, b, what):
    NCHK[0] += 1
    if v == 0:
        return
    if b is ZERO or lowbit(v) < b[0] or highbit(v) > b[1]:
        VIOL.append((what, None if v == 0 else (lowbit(v), highbit(v)), b))


class Q:
    """exact value + bound pair; tracks max span into a sink list."""
    def __init__(s, v, p):
        s.v, s.p = v, p


def X(x):
    return Q(F(x), ipair(x))


def mul(a, b, sink, what='mul'):
    r = Q(a.v * b.v, bmul(a.p, b.p))
    chk(r.v, r.p, what)
    sink.append(span(r.p))
    return r


def neg(a):
    return Q(-a.v, a.p)


def fsum(terms, sink, what='sum'):
    p = bsum([t.p for t in terms])
    acc = F(0)
    for t in terms:
        acc += t.v
        chk(acc, p, what + ':prefix')
    sink.append(span(p))
    return Q(acc, p)


def node_L(node, sink):
    if node[0] == 'M':
        m = node[1]
        L = [[X(m[4 * c + r]) for c in range(4)] for r in range(3)]
        L.append([Q(F(0), ZERO), Q(F(0), ZERO), Q(F(0), ZERO), Q(F(1), (0, 0))])
        return L
    _, q, s, t = node
    x, y, z, w = [X(v) for v in q]
    s = [X(v) for v in s]
    t = [X(v) for v in t]
    m = lambda a, b: mul(a, b, sink)
    two = X(2)
    n = fsum([m(x, x), m(y, y), m(z, z), m(w, w)], sink)
    H = [[None] * 3 for _ in range(3)]
    H[0][0] = fsum([m(w, w), m(x, x), neg(m(y, y)), neg(m(z, z))], sink)
    H[1][1] = fsum([m(w, w), neg(m(x, x)), m(y, y), neg(m(z, z))], sink)
    H[2][2] = fsum([m(w, w), neg(m(x, x)), neg(m(y, y)), m(z, z)], sink)
    H[0][1] = m(two, fsum([m(x, y), neg(m(w, z))], sink))
    H[0][2] = m(two, fsum([m(x, z), m(w, y)], sink))
    H[1][0] = m(two, fsum([m(x, y), m(w, z)], sink))
    H[1][2] = m(two, fsum([m(y, z), neg(m(w, x))], sink))
    H[2][0] = m(two, fsum([m(x, z), neg(m(w, y))], sink))
    H[2][1] = m(two, fsum([m(y, z), m(w, x)], sink))
    L = [[m(H[r][c], s[c]) for c in range(3)] + [m(n, t[r])] for r in range(3)]
    L.append([Q(F(0), ZERO), Q(F(0), ZERO), Q(F(0), ZERO), n])
    return L


def skipped(node):
    return node[0] == 'S'


def instance(nodes):
    sink = []
    G = [[Q(F(1), (0, 0)) if i == j else Q(F(0), ZERO) for j in range(4)] for i in range(4)]
    for nd in nodes:
        if skipped(nd):
            continue
        L = node_L(nd, sink)
        G = [[fsum([mul(G[i][k], L[k][j], sink) for k in range(4)], sink) for j in range(4)] for i in range(4)]
    return G, (max(sink) if sink else 0)


def volume_q(vol, P, W, sink):
    kind, d = vol
    if kind == 'box':
        c, hcol = d  # hcol: half-length per row (from covering signed-permutation entry), 0 -> ZERO
        for j in range(3):
            CW = mul(X(c[j]), W, sink)
            fsum([P[j], neg(CW)], sink)
            mul(X(hcol[j]), W, sink)
    else:
        c, r = d
        Qs = []
        for j in range(3):
            CW = mul(X(c[j]), W, sink)
            D = fsum([P[j], neg(CW)], sink)
            Qs.append(mul(D, D, sink))
        fsum(Qs, sink)
        R2 = mul(X(r), X(r), sink)
        W2 = mul(W, W, sink)
        mul(R2, W2, sink)


def run(nodes, tiles, vols, kind, raw, k):
    """tiles: list of transforms (column-major 16) or None (no transform -> no step).
    vols: len(tiles)+1 volumes. Returns (sInst, [s0, s1, ...])."""
    G, sInst = instance(nodes)
    apair = {'f32': (-149, 127), 'i8': (0, 7), 'i16': (0, 15)}[kind]
    if kind != 'f32' and k != 1:
        raw = [max(v, -k) for v in raw]  # clamp first (signed normalized), stays in pair
    a = [Q(F(v), apair) for v in raw]
    kq = Q(F(k), ipair(k))
    sink = []
    P = [fsum([mul(G[r][0], a[0], sink), mul(G[r][1], a[1], sink), mul(G[r][2], a[2], sink),
               mul(G[r][3], kq, sink)], sink) for r in range(3)]
    W = mul(G[3][3], kq, sink)
    P = [P[0], neg(P[2]), P[1]]
    volume_q(vols[0], P, W, sink)
    spans = [max(sink + [span(W.p)])]
    for li, T in enumerate(tiles):
        sink = [span(W.p)]
        if T is not None:
            Tq = [[X(T[4 * c + r]) for c in range(4)] for r in range(3)]
            P = [fsum([mul(Tq[r][0], P[0], sink), mul(Tq[r][1], P[1], sink), mul(Tq[r][2], P[2], sink),
                       mul(Tq[r][3], W, sink)], sink) for r in range(3)]
        volume_q(vols[li + 1], P, W, sink)
        spans.append(max(sink))
    return sInst, spans


EXT = [0.0, -0.0, 5e-324, -5e-324, 2.0 ** -1022, 1e-300, 1e-17, 0.1, 1 / 3, 1.0, -1.0, 2.0, 1e7,
       2.0 ** 500, -(2.0 ** 1000), 1.7976931348623157e308 / 2 ** 40]


def rv(rng, ex):
    return rng.choice(EXT) if ex and rng.random() < 0.5 else rng.uniform(-1e4, 1e4)


def quat(rng):
    v = rng.choice([[rng.gauss(0, 1) for _ in range(4)],
                    [rng.choice([5e-324, 1e-300, 1e-17, 0.0]), 0.0, rng.uniform(-1, 1), 1.0],
                    [0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, -1.0], [1.0, 0.0, 0.0, 0.0]])
    n = math.sqrt(sum(c * c for c in v))
    return [c / n for c in v]


def rnode(rng, ex):
    u = rng.random()
    if u < 0.2:
        return ('S',)
    if u < 0.5:
        m = [rv(rng, ex) for _ in range(16)]
        for i in rng.sample(range(12), rng.randrange(0, 6)):
            m[i] = rng.choice([0.0, 1.0, -1.0])
        m[3] = m[7] = m[11] = 0.0
        m[15] = 1.0
        return ('M', m)
    return ('T', quat(rng), [rng.choice([0.0, -1.0, 1.0, rv(rng, ex)]) for _ in range(3)],
            [rng.choice([0.0, rv(rng, ex)]) for _ in range(3)])


def rtile(rng, ex):
    u = rng.random()
    if u < 0.25:
        return None
    if u < 0.6:
        t = [rv(rng, ex) for _ in range(3)]
        return [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, t[0], t[1], t[2], 1]
    m = [rv(rng, ex) for _ in range(16)]
    m[3] = m[7] = m[11] = 0.0
    m[15] = 1.0
    return m


def rvol(rng, ex):
    c = [rng.choice([0.0, rv(rng, ex)]) for _ in range(3)]
    if rng.random() < 0.5:
        return ('box', (c, [rng.choice([0.0, abs(rv(rng, ex))]) for _ in range(3)]))
    return ('sphere', (c, rng.choice([0.0, 1.0, abs(rv(rng, ex))])))


def rpos(rng, kind):
    if kind == 'f32':
        pool = [0.0, 2.0 ** -149, -(2.0 ** -149), 3.4028234663852886e38, -1.0, 1.0]
        return [rng.choice(pool) if rng.random() < 0.5 else f32(rng.uniform(-1e6, 1e6)) for _ in range(3)]
    if kind == 'i8':
        return [rng.choice([-128, -127, 127, 0, 1, -1]) for _ in range(3)]
    return [rng.choice([-32768, 65535, 32767, 0, 1]) for _ in range(3)]


def worked_example():
    nodes = [('T', [0.0, 0.0, 0.0, 1.0], [2.0, 4.0, 0.25], [10.5, 0.0, -3.0])]
    tiles = [[1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 1000.25, -20.0, 0.0, 1]]
    vols = [('box', ([1.5, 0.0, 2.0], [100.0, 0.0, 50.0])), ('box', ([0.0, 0.0, 0.0], [2048.0, 2048.0, 2048.0]))]
    sInst, spans = run(nodes, tiles, vols, 'i16', [65535, 0, 32767], 65535)
    vu = 4 * sum(math.ceil(s / 64) ** 2 for s in spans)
    nu = 128 * math.ceil(sInst / 64) ** 2
    return {'sInst': sInst, 'levelSpans': spans, 'maxSpan': max([sInst] + spans),
            'vertexUnitsCount4': vu, 'nodeUnits': nu}


def hull_divergence():
    """Two instances of one URI whose level-0 P pairs differ: per-instance max span vs the
    v3 precheck's Pair0 entrywise hull."""
    i1 = [('T', [0.0, 0.0, 0.0, 1.0], [2.0 ** -200, 2.0 ** -200, 2.0 ** -200], [0.0, 0.0, 0.0])]
    i2 = [('T', [0.0, 0.0, 0.0, 1.0], [2.0 ** 200, 2.0 ** 200, 2.0 ** 200], [0.0, 0.0, 0.0])]
    vol = [('box', ([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]))]
    out = {}
    pairs = []
    for name, nodes in (('instanceSmall', i1), ('instanceLarge', i2)):
        G, sInst = instance(nodes)
        sink = []
        a = [Q(F(1), (-149, 127))] * 3
        P = [fsum([mul(G[r][c], a[c], sink) for c in range(3)] + [mul(G[r][3], Q(F(1), (0, 0)), sink)], sink) for r in range(3)]
        pairs.append([p.p for p in P])
        out[name] = run(nodes, [], vol, 'f32', [1.0, 1.0, 1.0], 1)[1][0]
    hull = [(min(a[0], b[0]), max(a[1], b[1])) for a, b in zip(*pairs)]
    out['hullP0Span'] = max(span(h) for h in hull)
    return out


def main():
    rng = random.Random(20261011)
    for _ in range(350):
        ex = rng.random() < 0.7
        nodes = [rnode(rng, ex) for _ in range(rng.randrange(0, 5))]
        tiles = [rtile(rng, ex) for _ in range(rng.randrange(0, 5))]
        vols = [rvol(rng, ex) for _ in range(len(tiles) + 1)]
        kind = rng.choice(['f32', 'i8', 'i16'])
        k = {'f32': 1, 'i8': rng.choice([1, 127, 255]), 'i16': rng.choice([1, 32767, 65535])}[kind]
        run(nodes, tiles, vols, kind, rpos(rng, kind), k)
    out = {'scope': 'reviewer delta2 probe; v3 normative rules re-implemented from contract text',
           'python': sys.version.split()[0], 'checks': NCHK[0], 'violations': len(VIOL),
           'violationExamples': VIOL[:5], 'workedExample': worked_example(),
           'hullVsInstance': hull_divergence()}
    print(json.dumps(out, indent=1, default=str))


if __name__ == '__main__':
    main()
