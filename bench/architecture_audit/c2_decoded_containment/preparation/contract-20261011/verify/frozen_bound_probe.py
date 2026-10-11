"""C2 v3 (interface freeze) probe: the NORMATIVE static span bound, checked against exact values.

Implements contract-draft.md v3 "Normative span definition" literally:
  * flat sums over written terms; ZERO terms dropped; 1 term -> its pair; m>=2 -> (min lo, max hi + ceil(log2 m))
  * product: ZERO absorbs; a factor whose pair is exactly (0,0) (value +-1 or 0) passes the other pair;
    otherwise (lo_a+lo_b, hi_a+hi_b+1)
  * nodes with none of matrix/translation/rotation/scale are skipped; absent TRS fields use defaults
  * G starts as identity; FLOAT positions use (-149,127); integer raw (0,7)/(0,15); k pairs exact
Checks soundness (lo <= lowbit, hi >= highbit) of every quantity and every left-to-right prefix of every
flat sum against exact Fractions; reports producer spans incl. the {"mesh":0} node, identity-TRS
equivalence, sphere vs box admitted depth, tiny-quaternion blow-up, and the worked example.
Single-threaded, deterministic, stdlib only (python3 -I).
"""
import json
import math
import random
import struct
import sys
from fractions import Fraction as F
from pathlib import Path

ZERO = None
STATS = {'checks': 0, 'violations': 0}
EXAMPLES = []


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


def span(p):
    return 0 if p is ZERO else p[1] - p[0] + 1


class Q:
    """A normative quantity: exact value + bound pair; records max span."""
    __slots__ = ('v', 'b')

    def __init__(self, v, b):
        self.v, self.b = v, b
        check(v, b)


def check(v, b):
    STATS['checks'] += 1
    if v == 0:
        return
    if b is ZERO or lowbit(v) < b[0] or highbit(v) > b[1]:
        STATS['violations'] += 1
        if len(EXAMPLES) < 5:
            EXAMPLES.append({'value': str(v), 'bound': b})


class Track:
    def __init__(self):
        self.max = 0

    def note(self, q):
        self.max = max(self.max, span(q.b))
        return q


def mul(t, a, b):
    if a.b is ZERO or b.b is ZERO:
        pb = ZERO
    elif a.b == (0, 0):
        pb = b.b
    elif b.b == (0, 0):
        pb = a.b
    else:
        pb = (a.b[0] + b.b[0], a.b[1] + b.b[1] + 1)
    return t.note(Q(a.v * b.v, pb))


def fsum(t, terms):
    nz = [q.b for q in terms if q.b is not ZERO]
    if not nz:
        pb = ZERO
    elif len(nz) == 1:
        pb = nz[0]
    else:
        pb = (min(p[0] for p in nz), max(p[1] for p in nz) + math.ceil(math.log2(len(nz))))
    acc = F(0)
    for q in terms:  # every left-to-right prefix must lie inside the full-sum pair
        acc += q.v
        check(acc, pb)
    return t.note(Q(acc, pb))


def neg(q):
    return Q(-q.v, q.b)


def inp(x):
    return Q(F(x), ipair(x))


ONE, TWO = inp(1), inp(2)


def local(t, node):
    if 'matrix' in node:
        m = node['matrix']
        assert [m[3], m[7], m[11], m[15]] == [0, 0, 0, 1]
        return [[inp(m[c * 4 + r]) for c in range(4)] for r in range(3)] + [[inp(0), inp(0), inp(0), ONE]]
    x, y, z, w = [inp(v) for v in node.get('rotation', [0, 0, 0, 1])]
    s = [inp(v) for v in node.get('scale', [1, 1, 1])]
    tr = [inp(v) for v in node.get('translation', [0, 0, 0])]
    sq = {k: mul(t, v, v) for k, v in zip('xyzw', (x, y, z, w))}
    n = fsum(t, [sq['x'], sq['y'], sq['z'], sq['w']])
    def off(a, b, c, d, sign):
        return mul(t, TWO, fsum(t, [mul(t, a, b), (lambda p: p if sign > 0 else neg(p))(mul(t, c, d))]))
    H = [[fsum(t, [sq['w'], sq['x'], neg(sq['y']), neg(sq['z'])]), off(x, y, w, z, -1), off(x, z, w, y, 1)],
         [off(x, y, w, z, 1), fsum(t, [sq['w'], neg(sq['x']), sq['y'], neg(sq['z'])]), off(y, z, w, x, -1)],
         [off(x, z, w, y, -1), off(y, z, w, x, 1), fsum(t, [sq['w'], neg(sq['x']), neg(sq['y']), sq['z']])]]
    L = [[mul(t, H[r][c], s[c]) for c in range(3)] + [mul(t, n, tr[r])] for r in range(3)]
    return L + [[inp(0), inp(0), inp(0), n]]


def has_step(node):
    return any(k in node for k in ('matrix', 'translation', 'rotation', 'scale'))


def composite(t, nodes):
    G = [[ONE if i == j else inp(0) for j in range(4)] for i in range(4)]
    for nd in nodes:
        if not has_step(nd):
            continue
        L = local(t, nd)
        G = [[fsum(t, [mul(t, G[i][k], L[k][j]) for k in range(4)]) for j in range(4)] for i in range(4)]
    return G


APAIR = {'f32': (-149, 127), 'i8': (0, 7), 'u8': (0, 7), 'i16': (0, 15), 'u16': (0, 15)}
KVAL = {'f32': 1, 'i8': 127, 'u8': 255, 'i16': 32767, 'u16': 65535}


def position(t, G, kind, raw, normalized):
    k = inp(KVAL[kind] if normalized else 1)
    a = [Q(F(v), APAIR[kind]) for v in raw]
    if normalized and kind in ('i8', 'i16'):
        a = [Q(max(q.v, -F(KVAL[kind])), APAIR[kind]) for q in a]
    P = [fsum(t, [mul(t, G[r][c], a[c]) for c in range(3)] + [mul(t, G[r][3], k)]) for r in range(3)]
    W = mul(t, G[3][3], k)
    return [P[0], neg(P[2]), P[1]], W  # U


def tile_step(t, T, P, W):
    m = [[inp(T[c * 4 + r]) for c in range(4)] for r in range(3)]
    return [fsum(t, [mul(t, m[r][0], P[0]), mul(t, m[r][1], P[1]), mul(t, m[r][2], P[2]), mul(t, m[r][3], W)]) for r in range(3)]


def box(t, P, W, c, h):
    for j in range(3):
        D = fsum(t, [P[j], neg(mul(t, inp(c[j]), W))])
        HW = mul(t, inp(h[j]), W)
        _ = (D, HW)


def sphere(t, P, W, c, r):
    Qs = []
    for j in range(3):
        D = fsum(t, [P[j], neg(mul(t, inp(c[j]), W))])
        Qs.append(mul(t, D, D))
    fsum(t, Qs)
    mul(t, mul(t, inp(r), inp(r)), mul(t, W, W))


def chain_spans(nodes, kind, raw, normalized, levels):
    """levels: list of (transform_or_None_into_this_level, [('box',c,h)|('sphere',c,r)]) own level first
    (own level has transform None). Returns (sInst, [s_level...])."""
    ti = Track()
    G = composite(ti, nodes)
    t0 = Track()
    P, W = position(t0, G, kind, raw, normalized)
    out = []
    for idx, (T, vols) in enumerate(levels):
        tl = t0 if idx == 0 else Track()
        if T is not None:
            P = tile_step(tl, T, P, W)
            for q in P:
                tl.note(q)
        tl.note(W)
        for v in vols:
            (box if v[0] == 'box' else sphere)(tl, P, W, v[1], v[2])
        out.append(tl.max)
    return ti.max, out


def quat(rng, style):
    if style == 'generic':
        v = [rng.gauss(0, 1) for _ in range(4)]
    elif style == 'near-axis':
        v = [rng.choice([1e-17, -6.1e-17, 3e-9]), 0.0, rng.uniform(-1, 1), 1.0]
    else:  # tiny: an admissible quaternion with a subnormal-scale component
        v = [1e-300, 0.0, rng.uniform(-1, 1), 1.0]
    n = math.sqrt(sum(c * c for c in v))
    return [c / n for c in v]


def trans(rng, s):
    t = [rng.uniform(-s, s) for _ in range(3)]
    return [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, t[0], t[1], t[2], 1]


def main():
    rng = random.Random(20261011)
    res = {}
    pos = [f32(rng.uniform(-1000, 1000)) for _ in range(3)]
    abox = ('box', [0.0, 0.0, 0.0], [5000.0, 5000.0, 5000.0])
    # 1. producer: {"mesh":0} node (skipped) vs explicit identity TRS vs explicit identity matrix
    prod = {}
    for label, nodes in (('meshOnlyNodeSkipped', [{'mesh': 0}]),
                         ('identityTrsNode', [{'mesh': 0, 'translation': [0, 0, 0], 'rotation': [0, 0, 0, 1], 'scale': [1, 1, 1]}]),
                         ('identityMatrixNode', [{'mesh': 0, 'matrix': [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]}])):
        r2 = random.Random(7)
        levels = [(None, [abox])] + [(trans(r2, 1e4), [abox]) for _ in range(20)]
        si, sl = chain_spans(nodes, 'f32', pos, False, levels)
        prod[label] = {'sInst': si, 'maxLevelSpan': max(sl), 'level0': sl[0], 'level20': sl[-1]}
    res['producerTranslationChainDepth20'] = prod
    # vector --quantize: T,S node, u16 normalized
    r2 = random.Random(8)
    nodes = [{'mesh': 0, 'translation': [r2.uniform(-1e4, 1e4) for _ in range(3)], 'scale': [r2.uniform(1, 1e4) for _ in range(3)]}]
    levels = [(None, [abox])] + [(trans(r2, 1e4), [abox]) for _ in range(16)]
    si, sl = chain_spans(nodes, 'u16', [65535, 0, 32767], True, levels)
    res['vectorQuantizeDepth16'] = {'sInst': si, 'maxLevelSpan': max(sl)}
    # 2. rotated depth: box vs sphere, generic / near-axis / tiny
    rows = []
    for style in ('generic', 'near-axis', 'tiny'):
        for k in (1, 2, 4, 6, 8, 10, 12, 14, 16):
            nodes = [{'mesh': 0, 'rotation': quat(rng, style), 'scale': [rng.uniform(0.1, 10) for _ in range(3)],
                      'translation': [rng.uniform(-100, 100) for _ in range(3)]} for _ in range(k)]
            T = trans(rng, 10)
            sb = chain_spans(nodes, 'f32', pos, False, [(None, [abox]), (T, [abox])])
            ss = chain_spans(nodes, 'f32', pos, False, [(None, [('sphere', [1.5, 2.0, 3.0], 5000.0)]), (T, [abox])])
            rows.append({'style': style, 'depth': k, 'boxMax': max([sb[0]] + sb[1]), 'sphereMax': max([ss[0]] + ss[1])})
    res['rotatedDepth'] = rows
    cap = 4096
    res['admittedDepthAtCap4096'] = {s: {'box': max([r['depth'] for r in rows if r['style'] == s and r['boxMax'] <= cap], default=0),
                                          'sphere': max([r['depth'] for r in rows if r['style'] == s and r['sphereMax'] <= cap], default=0)}
                                      for s in ('generic', 'near-axis', 'tiny')}
    # 3. adversarial soundness: random mixtures incl. subnormals, +-1, zeros, singular, clamps
    specials = [0.0, -0.0, 1.0, -1.0, 5e-324, 1e-300, 2.0 ** 500, -2.0 ** 1000, 1.7976931348623157e308 / 2 ** 40, 0.5, 3.0]
    for trial in range(300):
        nodes = []
        for _ in range(rng.randint(0, 5)):
            c = rng.random()
            if c < 0.2:
                nodes.append({'mesh': 0})
            elif c < 0.5:
                m = [rng.choice(specials + [rng.uniform(-9, 9)]) for _ in range(16)]
                m[3] = m[7] = m[11] = 0; m[15] = 1
                nodes.append({'matrix': m})
            else:
                q = [rng.choice(specials[:6] + [rng.uniform(-1, 1)]) for _ in range(4)]
                if all(v == 0 for v in q):
                    q[3] = 1.0
                q = [max(-1.0, min(1.0, v)) for v in q]
                nd = {'rotation': q}
                if rng.random() < 0.7:
                    nd['scale'] = [rng.choice(specials + [rng.uniform(-9, 9)]) for _ in range(3)]
                if rng.random() < 0.7:
                    nd['translation'] = [rng.choice(specials + [rng.uniform(-9, 9)]) for _ in range(3)]
                nodes.append(nd)
        kind = rng.choice(list(APAIR))
        normalized = kind != 'f32' and rng.random() < 0.6
        if kind == 'f32':
            raw = [rng.choice([f32(rng.uniform(-1e30, 1e30)), 1.401298464324817e-45, 3.4028234663852886e38, 0.0, 1.0])
                   for _ in range(3)]
        else:
            lim = {'i8': (-128, 127), 'u8': (0, 255), 'i16': (-32768, 32767), 'u16': (0, 65535)}[kind]
            raw = [rng.choice([lim[0], lim[1], 0, 1, rng.randint(*lim)]) for _ in range(3)]
        levels = [(None, [abox])]
        for _ in range(rng.randint(0, 4)):
            T = [rng.choice(specials + [rng.uniform(-9, 9)]) for _ in range(16)]
            T[3] = T[7] = T[11] = 0; T[15] = 1
            vol = ('sphere', [rng.choice(specials), 1.0, -2.0], rng.choice([0.0, 1.0, 1e10])) if rng.random() < 0.5 else \
                  ('box', [rng.choice(specials), 0.0, 1.0], [rng.choice(specials), 0.0, 2.0])
            levels.append((T if rng.random() < 0.8 else None, [vol]))
        chain_spans(nodes, kind, raw, normalized, levels)
    res['soundness'] = {'trials': 300, 'quantityAndPrefixChecks': STATS['checks'], 'violations': STATS['violations'],
                        'violationExamples': EXAMPLES}
    # 4. worked example (contract text)
    ex_nodes = [{'mesh': 0, 'translation': [10.5, 0, -3], 'scale': [2, 4, 0.25]}]
    ex_levels = [(None, [('box', [1.5, 0.0, 2.0], [100.0, 0.0, 50.0])]),
                 ([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 1000.25, -20, 0, 1], [('box', [0.0, 0.0, 0.0], [2048.0, 2048.0, 2048.0])])]
    si, sl = chain_spans(ex_nodes, 'u16', [65535, 0, 32767], True, ex_levels)
    T_u = 4
    res['workedExample'] = {'sInst': si, 'levelSpans': sl, 'maxSpan': max([si] + sl),
                            'vertexUnitsPerVertex': sum(math.ceil(s / 64) ** 2 for s in sl),
                            'vertexUnitsForAccessorCount4': T_u * sum(math.ceil(s / 64) ** 2 for s in sl)}
    Path(__file__).with_name('frozen-bound-observed.json').write_text(json.dumps(res, indent=1) + '\n')
    print(json.dumps({k: res[k] for k in ('producerTranslationChainDepth20', 'vectorQuantizeDepth16', 'admittedDepthAtCap4096', 'workedExample')}))
    print(json.dumps(res['soundness']))


if __name__ == '__main__':
    main()
