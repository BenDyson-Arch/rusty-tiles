"""C2 v2 preparation probe for R1/R2: exact homogeneous chain spans vs a static exponent-pair bound.

Standalone, single-threaded, deterministic, stdlib only. Not production evidence.

Canonical formulas (contract v2, "Exact evaluation"):
  node step  Matrix: G' = G * [[M, t],[0,1]]        TRS: G' = G * [[H(q)*diag(s), n*t],[0, n]]
             (G is the composite 4x4 homogeneous matrix, built root-first, last row [0,0,0,g])
  position   P = G3 * a + Gt * k ;  W = g * k      (a raw f32/int value, k normalization constant)
  U          P = (P0, -P2, P1)
  tile step  P' = T3 * P + Tt * W ; W' = W
  box test   |P_j - c_j W|  vs  h_j W          (axis-aligned)
Static bound: each scalar carries a pair (lo, hi) of possible set-bit exponents, or ZERO.
  input x != 0: (lowbit(x), highbit(x)); f32 position input: (-149, 127); int u16: (0, 15)
  product: (lo_a+lo_b, hi_a+hi_b+1); sum of m nonzero terms: (min lo, max hi + ceil(log2 m))
  span = hi - lo + 1.
Measures: max exact span (canonical dyadic, trailing zeros stripped) vs max static-bound span
over all scalars, and the largest node depth admitted under candidate caps.
"""
import json
import math
import random
import struct
import sys
from fractions import Fraction as F
from pathlib import Path

ZERO = None


def lowbit(v):
    n, d = v.numerator, v.denominator
    assert d & (d - 1) == 0
    return ((n & -n).bit_length() - 1) - (d.bit_length() - 1)


def highbit(v):
    n, d = abs(v.numerator), v.denominator
    # floor(log2(n/d)) exactly
    e = n.bit_length() - d.bit_length()
    if (n << max(0, -e)) < (d << max(0, e)):
        e -= 1
    return e


def span(v):
    return 0 if v == 0 else highbit(v) - lowbit(v) + 1


def pair(v):
    return ZERO if v == 0 else (lowbit(v), highbit(v))


def bmul(a, b):
    if a is ZERO or b is ZERO:
        return ZERO
    return (a[0] + b[0], a[1] + b[1] + 1)


def bsum(terms):
    t = [x for x in terms if x is not ZERO]
    if not t:
        return ZERO
    return (min(x[0] for x in t), max(x[1] for x in t) + math.ceil(math.log2(len(t))) if len(t) > 1 else t[0][1])


def bspan(p):
    return 0 if p is ZERO else p[1] - p[0] + 1


def f32(x):
    return struct.unpack('<f', struct.pack('<f', x))[0]


def H(q):
    x, y, z, w = q
    return [[w*w + x*x - y*y - z*z, 2*(x*y - w*z), 2*(x*z + w*y)],
            [2*(x*y + w*z), w*w - x*x + y*y - z*z, 2*(y*z - w*x)],
            [2*(x*z - w*y), 2*(y*z + w*x), w*w - x*x - y*y + z*z]]


def Hb(qb):  # bound version, same expression shapes
    x, y, z, w = qb
    sq = lambda a: bmul(a, a)
    two = (1, 1)
    d = lambda *ts: bsum(list(ts))
    m = lambda a, b: bmul(two, d(bmul(a, b), bmul(a, b)))  # 2*(ab - cd): two products, then *2
    def off(a, b, c, e):
        return bmul(two, d(bmul(a, b), bmul(c, e)))
    return [[d(sq(w), sq(x), sq(y), sq(z)), off(x, y, w, z), off(x, z, w, y)],
            [off(x, y, w, z), d(sq(w), sq(x), sq(y), sq(z)), off(y, z, w, x)],
            [off(x, z, w, y), off(y, z, w, x), d(sq(w), sq(x), sq(y), sq(z))]]


def local_exact(node):
    if node[0] == 'M':
        m3, t = node[1], node[2]
        return [[F(m3[r][c]) for c in range(3)] + [F(t[r])] for r in range(3)] + [[F(0), F(0), F(0), F(1)]]
    q, s, t = [F(v) for v in node[1]], [F(v) for v in node[2]], [F(v) for v in node[3]]
    n = sum(c * c for c in q)
    h = H(q)
    return [[h[r][c] * s[c] for c in range(3)] + [n * t[r]] for r in range(3)] + [[F(0), F(0), F(0), n]]


def local_bound(node):
    one = (0, 0)
    if node[0] == 'M':
        m3, t = node[1], node[2]
        return [[pair(F(m3[r][c])) for c in range(3)] + [pair(F(t[r]))] for r in range(3)] + [[ZERO, ZERO, ZERO, one]]
    qb = [pair(F(v)) for v in node[1]]
    sb = [pair(F(v)) for v in node[2]]
    tb = [pair(F(v)) for v in node[3]]
    nb = bsum([bmul(c, c) for c in qb])
    hb = Hb(qb)
    return [[bmul(hb[r][c], sb[c]) for c in range(3)] + [bmul(nb, tb[r])] for r in range(3)] + [[ZERO, ZERO, ZERO, nb]]


def matmul(a, b):
    return [[sum(a[i][k] * b[k][j] for k in range(4)) for j in range(4)] for i in range(4)]


def matmul_b(a, b):
    return [[bsum([bmul(a[i][k], b[k][j]) for k in range(4)]) for j in range(4)] for i in range(4)]


IDENT = [[F(int(i == j)) for j in range(4)] for i in range(4)]
IDENT_B = [[(0, 0) if i == j else ZERO for j in range(4)] for i in range(4)]


def quat(rng, style):
    if style == 'generic':
        v = [rng.gauss(0, 1) for _ in range(4)]
    else:
        v = [rng.choice([1e-17, -6.1e-17, 3e-9]), 0.0, rng.uniform(-1, 1), 1.0]
    n = math.sqrt(sum(c * c for c in v))
    return [c / n for c in v]


def run(rng, nodes, tiles, positions, k=1, int_pos=False):
    G, Gb = IDENT, IDENT_B
    ex, bd = 0, 0
    for nd in nodes:
        G = matmul(G, local_exact(nd))
        Gb = matmul_b(Gb, local_bound(nd))
        ex = max([ex] + [span(v) for row in G for v in row])
        bd = max([bd] + [bspan(v) for row in Gb for v in row])
    apos = (0, 15) if int_pos else (-149, 127)
    kb = pair(F(k))
    for a in positions:
        a = [F(v) for v in a]
        P = [sum(G[r][c] * a[c] for c in range(3)) + G[r][3] * k for r in range(3)]
        W = G[3][3] * k
        Pb = [bsum([bmul(Gb[r][c], apos) for c in range(3)] + [bmul(Gb[r][3], kb)]) for r in range(3)]
        Wb = bmul(Gb[3][3], kb)
        P = [P[0], -P[2], P[1]]
        Pb = [Pb[0], Pb[2], Pb[1]]
        for T in tiles:
            P = [sum(F(T[r][c]) * P[c] for c in range(3)) + F(T[r][3]) * W for r in range(3)]
            Pb = [bsum([bmul(pair(F(T[r][c])), Pb[c]) for c in range(3)] + [bmul(pair(F(T[r][3])), Wb)]) for r in range(3)]
            # axis-aligned box test operands: P_j - c_j W and h_j W with f64 c, h
            c = F(rng.uniform(-50, 50)); h = F(rng.uniform(1, 1e4))
            ex = max(ex, span(P[0] - c * W), span(h * W))
            bd = max(bd, bspan(bsum([Pb[0], bmul(pair(c), Wb)])), bspan(bmul(pair(h), Wb)))
        ex = max([ex, span(W)] + [span(v) for v in P])
        bd = max([bd, bspan(Wb)] + [bspan(v) for v in Pb])
    return ex, bd


def translation_tile(rng, scale):
    t = [rng.uniform(-scale, scale) for _ in range(3)]
    return [[1.0, 0.0, 0.0, t[0]], [0.0, 1.0, 0.0, t[1]], [0.0, 0.0, 1.0, t[2]]]


def main():
    rng = random.Random(20261011)
    rows = []
    pos = [[f32(rng.uniform(-1000, 1000)) for _ in range(3)] for _ in range(3)]
    # producer-like: identity node, translation tile chains (point/vector), depth up to 24
    for depth in (1, 8, 16, 24):
        ex, bd = run(rng, [], [translation_tile(rng, 1e4) for _ in range(depth)], pos)
        rows.append({'scenario': 'point/vector: identity node, translation tiles', 'nodeDepth': 0, 'tileDepth': depth, 'exactMaxSpan': ex, 'boundMaxSpan': bd})
    # vector --quantize: TRS node translation=lo, scale=extent, u16 normalized, translation tiles
    for depth in (1, 16):
        node = ('T', [0.0, 0.0, 0.0, 1.0], [rng.uniform(1, 1e4) for _ in range(3)], [rng.uniform(-1e4, 1e4) for _ in range(3)])
        ex, bd = run(rng, [node], [translation_tile(rng, 1e4) for _ in range(depth)], [[65535, 0, 32767]], k=65535, int_pos=True)
        rows.append({'scenario': 'vector --quantize: T,S node, u16/65535, translation tiles', 'nodeDepth': 1, 'tileDepth': depth, 'exactMaxSpan': ex, 'boundMaxSpan': bd})
    # W1 / third-party: k rotated TRS levels, one tile (root transform never applied)
    for style in ('generic', 'near-axis'):
        for k in (1, 2, 4, 6, 8, 12, 16):
            nodes = [('T', quat(rng, style), [rng.uniform(0.1, 10) for _ in range(3)], [rng.uniform(-100, 100) for _ in range(3)]) for _ in range(k)]
            ex, bd = run(rng, nodes, [translation_tile(rng, 10)], pos[:1])
            rows.append({'scenario': f'rotated TRS nodes ({style})', 'nodeDepth': k, 'tileDepth': 1, 'exactMaxSpan': ex, 'boundMaxSpan': bd})
    caps = {}
    for cap in (1024, 2048, 4096, 8192):
        caps[str(cap)] = {s: max([r['nodeDepth'] for r in rows if r['scenario'] == s and r['boundMaxSpan'] <= cap], default=None)
                          for s in ('rotated TRS nodes (generic)', 'rotated TRS nodes (near-axis)')}
    out = {'scope': 'standalone probe; canonical v2 formulas; exact Fraction vs static exponent-pair bound',
           'python': sys.version.split()[0], 'rows': rows, 'maxRotatedDepthAdmittedByBoundCap': caps}
    Path(__file__).with_name('span-bound-observed.json').write_text(json.dumps(out, indent=1) + '\n')
    for r in rows:
        print(r)
    print(json.dumps(caps))


if __name__ == '__main__':
    main()
