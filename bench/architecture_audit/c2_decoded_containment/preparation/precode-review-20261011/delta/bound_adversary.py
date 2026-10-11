"""Delta-review probe (reviewer-authored, not production evidence).

Independent re-implementation of the C2 v2 static exponent-pair bound, written from the
contract text ("Exact evaluation", contract-draft.md:216-260), not from span_bound_probe.py.
For adversarial random chains it checks, for EVERY canonical quantity and every left-to-right
prefix of every sum, that

    bound.lo <= lowbit(actual)   and   bound.hi >= floor(log2 |actual|)

It also measures:
  * sphere tests (squares) vs box tests: the admitted rotated depth under the 4096 cap;
  * parse-tree sensitivity: flat vs nested association of the position and tile-step sums,
    which O and M must agree on bit-exactly for "span at cap and one over" controls.

Single-threaded, deterministic, stdlib only. Run with python3 -I.
"""
import json
import math
import random
import struct
import sys
from fractions import Fraction as F

ZERO = None
VIOLATIONS = []
CHECKS = [0]


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


def inp(x):
    x = F(x)
    return ZERO if x == 0 else (lowbit(x), highbit(x))


def bmul(a, b):
    if a is ZERO or b is ZERO:
        return ZERO
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


def check(val, b, what):
    """val exact Fraction, b its bound pair."""
    CHECKS[0] += 1
    if val == 0:
        return
    if b is ZERO:
        VIOLATIONS.append((what, 'nonzero value with ZERO bound', str(val)))
        return
    lo, hi = lowbit(val), highbit(val)
    if lo < b[0] or hi > b[1]:
        VIOLATIONS.append((what, (lo, hi), b))


def vsum(terms, what):
    """terms: list of (exact, bound). Check every prefix against the full-sum bound."""
    full = bsum([b for _, b in terms])
    acc = F(0)
    for v, _ in terms:
        acc += v
        check(acc, full, what + ':prefix')
    return acc, full


def vmul(a, b, what):
    v = a[0] * b[0]
    bb = bmul(a[1], b[1])
    check(v, bb, what)
    return v, bb


def X(x):  # exact input with its pair
    return (F(x), inp(x))


def H(q, what):
    x, y, z, w = q
    sq = lambda a: vmul(a, a, what + ':sq')
    two = X(2)
    def off(a, b, c, d, sign):
        p1 = vmul(a, b, what + ':p')
        p2 = vmul(c, d, what + ':p')
        p2 = (sign * p2[0], p2[1])
        s = vsum([p1, p2], what + ':off')
        return vmul(two, s, what + ':2x')
    def diag(sg):
        terms = [sq(w), sq(x), sq(y), sq(z)]
        terms = [(sg[i] * t[0], t[1]) for i, t in enumerate(terms)]
        return vsum(terms, what + ':diag')
    return [[diag((1, 1, -1, -1)), off(x, y, w, z, -1), off(x, z, w, y, 1)],
            [off(x, y, w, z, 1), diag((1, -1, 1, -1)), off(y, z, w, x, -1)],
            [off(x, z, w, y, -1), off(y, z, w, x, 1), diag((1, -1, -1, 1))]]


def local(node, what):
    if node[0] == 'M':
        m = node[1]  # column-major 16
        L = [[X(m[c * 4 + r]) for c in range(4)] for r in range(4)]
        assert [m[3], m[7], m[11], m[15]] == [0, 0, 0, 1]
        return L
    q, s, t = [X(v) for v in node[1]], [X(v) for v in node[2]], [X(v) for v in node[3]]
    n = vsum([vmul(c, c, what + ':n') for c in q], what + ':n')
    h = H(q, what)
    L = [[vmul(h[r][c], s[c], what + ':Hs') for c in range(3)] + [vmul(n, t[r], what + ':nt')] for r in range(3)]
    L.append([X(0), X(0), X(0), n])
    return L


def matmul(A, B, what):
    return [[vsum([vmul(A[i][k], B[k][j], what) for k in range(4)], what) for j in range(4)] for i in range(4)]


def ident():
    return [[X(1 if i == j else 0) for j in range(4)] for i in range(4)]


def walk(nodes, tiles, vols, pos_kind, raw, k, assoc='flat'):
    """Return max bound span over all canonical quantities; record violations."""
    mx = [0]
    def note(b):
        mx[0] = max(mx[0], span(b))
    G = ident()
    for i, nd in enumerate(nodes):
        G = matmul(G, local(nd, f'node{i}'), f'G{i}')
        for row in G:
            for e in row:
                note(e[1])
    apair = {'f32': (-149, 127), 'i8': (0, 7), 'i16': (0, 15)}[pos_kind]
    a = [(F(v), apair) for v in raw]
    kk = X(k)
    if assoc == 'flat':
        P = [vsum([vmul(G[r][c], a[c], 'Ga') for c in range(3)] + [vmul(G[r][3], kk, 'Gk')], 'P') for r in range(3)]
    else:  # nested: (G3 a) then + Gt k
        P = []
        for r in range(3):
            g3a = vsum([vmul(G[r][c], a[c], 'Ga') for c in range(3)], 'G3a')
            P.append(vsum([g3a, vmul(G[r][3], kk, 'Gk')], 'Pn'))
    W = vmul(G[3][3], kk, 'W')
    for e in P + [W]:
        note(e[1])
    P = [P[0], (-P[2][0], P[2][1]), P[1]]
    def test(vol, P, W, tag):
        kind, d = vol
        if kind == 'box':  # d = (c[3], h[3]) axis-aligned
            for j in range(3):
                cW = vmul(X(d[0][j]), W, tag + 'cW')
                diff = vsum([P[j], (-cW[0], cW[1])], tag + 'diff')
                hW = vmul(X(d[1][j]), W, tag + 'hW')
                note(diff[1]); note(hW[1]); note(cW[1])
        else:  # sphere d = (c[3], r)
            sqs = []
            for j in range(3):
                cW = vmul(X(d[0][j]), W, tag + 'cW')
                diff = vsum([P[j], (-cW[0], cW[1])], tag + 'diff')
                sq = vmul(diff, diff, tag + 'sq')
                sqs.append(sq)
                note(diff[1]); note(sq[1])
            tot = vsum(sqs, tag + 'sum')
            rW = vmul(X(d[1]), W, tag + 'rW')
            rhs = vmul(rW, rW, tag + 'rhs')  # (rW)^2; r^2*W^2 has the same pair
            note(tot[1]); note(rhs[1])
    test(vols[0], P, W, 'own')
    for li, T in enumerate(tiles):
        Tm = [[X(T[c * 4 + r]) for c in range(4)] for r in range(3)]
        if assoc == 'flat':
            P = [vsum([vmul(Tm[r][c], P[c], 'TP') for c in range(3)] + [vmul(Tm[r][3], W, 'TW')], 'tile') for r in range(3)]
        else:
            P = [vsum([vsum([vmul(Tm[r][c], P[c], 'TP') for c in range(3)], 'T3P'), vmul(Tm[r][3], W, 'TW')], 'tileN') for r in range(3)]
        for e in P:
            note(e[1])
        test(vols[li + 1], P, W, f'lvl{li}')
    return mx[0]


EXTREME = [0.0, -0.0, 5e-324, -5e-324, 2.0 ** -1022, 1e-300, 1e-17, -6.1e-17, 0.1, 1 / 3, 1.0, -1.0,
           2.0, 1e7, 2.0 ** 500, -(2.0 ** 1000), 1.7976931348623157e308 / 2 ** 40]


def rnd(rng, extreme):
    return rng.choice(EXTREME) if extreme and rng.random() < 0.5 else rng.uniform(-1e4, 1e4)


def quat(rng, style):
    if style == 'tiny':  # admissible near-axis with subnormal-ish components
        v = [rng.choice([5e-324, 1e-300, 1e-17, 0.0]), rng.choice([0.0, 1e-200]), rng.uniform(-1, 1), 1.0]
    else:
        v = [rng.gauss(0, 1) for _ in range(4)]
    n = math.sqrt(sum(c * c for c in v))
    return [c / n for c in v]


def node(rng, extreme):
    if rng.random() < 0.4:
        m = [rnd(rng, extreme) for _ in range(16)]
        if rng.random() < 0.3:
            m[rng.randrange(12)] = 0.0  # structural zeros / singular
        m[3] = m[7] = m[11] = 0.0
        m[15] = 1.0
        return ('M', m)
    q = quat(rng, rng.choice(['tiny', 'generic']))
    s = [rng.choice([0.0, -1.0, rnd(rng, extreme)]) for _ in range(3)]
    t = [rnd(rng, extreme) for _ in range(3)]
    return ('T', q, s, t)


def tile(rng, extreme):
    if rng.random() < 0.5:
        t = [rnd(rng, extreme) for _ in range(3)]
        return [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, t[0], t[1], t[2], 1]
    m = [rnd(rng, extreme) for _ in range(16)]
    m[3] = m[7] = m[11] = 0.0
    m[15] = 1.0
    return m


def vol(rng, extreme):
    c = [rnd(rng, extreme) for _ in range(3)]
    if rng.random() < 0.5:
        return ('box', (c, [abs(rnd(rng, extreme)) for _ in range(3)]))
    return ('sphere', (c, abs(rnd(rng, extreme))))


def position(rng, kind):
    if kind == 'f32':
        pool = [0.0, 2.0 ** -149, -(2.0 ** -149), 3.4028234663852886e38, -1.0, f32(0.1)]
        return [rng.choice(pool) if rng.random() < 0.5 else f32(rng.uniform(-1e6, 1e6)) for _ in range(3)]
    if kind == 'i8':
        return [rng.choice([-128, -127, 127, 0, 5]) for _ in range(3)]
    return [rng.choice([-32768, 65535, 32767, 0]) for _ in range(3)]


def main():
    rng = random.Random(20261011)
    trials = 0
    for _ in range(400):
        extreme = rng.random() < 0.7
        nodes = [node(rng, extreme) for _ in range(rng.randrange(0, 5))]
        tiles = [tile(rng, extreme) for _ in range(rng.randrange(0, 5))]
        vols = [vol(rng, extreme) for _ in range(len(tiles) + 1)]
        kind = rng.choice(['f32', 'i8', 'i16'])
        k = {'f32': 1, 'i8': rng.choice([1, 127]), 'i16': rng.choice([1, 32767, 65535])}[kind]
        raw = position(rng, kind)
        if kind == 'i8' and k == 127:
            raw = [max(v, -127) for v in raw]  # contract: clamp to -k before the rational
        walk(nodes, tiles, vols, kind, raw, k)
        trials += 1

    # Sphere vs box admitted rotated depth under 4096 (generic and near-axis), plus association.
    depth_rows = []
    for style in ('generic', 'tiny'):
        for depth in (4, 6, 8, 10, 12, 16):
            nodes = [('T', quat(rng, style), [rng.uniform(0.1, 10) for _ in range(3)],
                      [rng.uniform(-100, 100) for _ in range(3)]) for _ in range(depth)]
            tiles = [[1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 5.5, -3.25, 7.125, 1]]
            raw = [f32(12.5), f32(-3.3), f32(700.1)]
            box = [('box', ([0.5, 1.5, -2.5], [1e3, 1e3, 1e3]))] * 2
            sph = [('sphere', ([0.5, 1.5, -2.5], 1e3))] * 2
            depth_rows.append({'style': style, 'depth': depth,
                               'boxBoundSpan': walk(nodes, tiles, box, 'f32', raw, 1),
                               'sphereBoundSpan': walk(nodes, tiles, sph, 'f32', raw, 1),
                               'boxBoundSpanNested': walk(nodes, tiles, box, 'f32', raw, 1, assoc='nested')})
    # Identity-node question: absent node vs explicit identity TRS node (producer single node {"mesh":0}).
    tiles = [[1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 1234.5, -77.25, 3.0, 1] for _ in range(20)]
    box = [('box', ([0.0, 0.0, 0.0], [5e3, 5e3, 5e3]))] * 21
    raw = [f32(1.1), f32(-2.2), f32(3.3)]
    ident_rows = {
        'noNodeStep': walk([], tiles, box, 'f32', raw, 1),
        'identityTrsNodeStep': walk([('T', [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0])], tiles, box, 'f32', raw, 1),
        'noNodeStepNestedAssoc': walk([], tiles, box, 'f32', raw, 1, assoc='nested'),
    }
    out = {'scope': 'reviewer delta probe; independent bound implementation from contract text',
           'python': sys.version.split()[0], 'randomTrials': trials, 'quantityChecks': CHECKS[0],
           'boundViolations': len(VIOLATIONS), 'violationExamples': VIOLATIONS[:5],
           'rotatedDepthRows': depth_rows, 'producerTranslationChainDepth20': ident_rows}
    print(json.dumps(out, indent=1, default=str))


if __name__ == '__main__':
    main()
