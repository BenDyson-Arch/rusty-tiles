"""The contract's normative span definition (B1), re-implemented from its text.

A pair is a tuple (lo, hi) bounding the exponents of a value's set bits.
ZERO (None here) means the value is exactly 0. Every product and every
flat-sum result is a "quantity" and is recorded in a Tracker, which keeps the
maximum span seen. Sums are never re-associated: each `fsum` call below is
one written term list of the contract.
"""

from fractions import Fraction

from .numbers import lowbit_highbit

ZERO = None
ONE = (0, 0)
TWO = (1, 1)

# Fixed worst-case pairs for raw POSITION words (contract table).
WORD_PAIR = {"f32": (-149, 127), "i8": (0, 7), "u8": (0, 7), "i16": (0, 15), "u16": (0, 15)}


def span(p):
    return 0 if p is ZERO else p[1] - p[0] + 1


def pair_of(value):
    """Input pair of an exact dyadic value; +-0 is ZERO."""
    value = Fraction(value)
    if value == 0:
        return ZERO
    return lowbit_highbit(value)


def k_pair(k):
    """Normalization constant pair: 1 -> (0,0), 127 -> (0,6), ..., 65535 -> (0,15)."""
    return pair_of(k)


class Tracker:
    """Records quantities; `max` is the largest span recorded."""

    def __init__(self):
        self.max = 0

    def note(self, p):
        s = span(p)
        if s > self.max:
            self.max = s
        return p


def mul(a, b, t):
    if a is ZERO or b is ZERO:
        return t.note(ZERO)
    if a == ONE:
        return t.note(b)
    if b == ONE:
        return t.note(a)
    return t.note((a[0] + b[0], a[1] + b[1] + 1))


def fsum(terms, t):
    kept = [p for p in terms if p is not ZERO]
    if not kept:
        return t.note(ZERO)
    if len(kept) == 1:
        return t.note(kept[0])
    m = len(kept)
    return t.note((min(p[0] for p in kept), max(p[1] for p in kept) + (m - 1).bit_length()))


def hull(a, b):
    """Entrywise hull of two pairs; ZERO is the identity."""
    if a is ZERO:
        return b
    if b is ZERO:
        return a
    return (min(a[0], b[0]), max(a[1], b[1]))


def identity_g():
    return [[ONE if i == j else ZERO for j in range(4)] for i in range(4)]


def trs_l(t_pairs, q_pairs, s_pairs, tr):
    """Node step for a TRS node; returns L (4x4 pairs). Quantities go to `tr`."""
    x, y, z, w = q_pairs
    xx, yy, zz, ww = mul(x, x, tr), mul(y, y, tr), mul(z, z, tr), mul(w, w, tr)
    xy, wz, xz, wy = mul(x, y, tr), mul(w, z, tr), mul(x, z, tr), mul(w, y, tr)
    yz, wx = mul(y, z, tr), mul(w, x, tr)
    n = fsum([xx, yy, zz, ww], tr)
    h = [[None] * 3 for _ in range(3)]
    h[0][0] = fsum([ww, xx, yy, zz], tr)
    h[1][1] = fsum([ww, xx, yy, zz], tr)
    h[2][2] = fsum([ww, xx, yy, zz], tr)
    h[0][1] = mul(TWO, fsum([xy, wz], tr), tr)
    h[0][2] = mul(TWO, fsum([xz, wy], tr), tr)
    h[1][0] = mul(TWO, fsum([xy, wz], tr), tr)
    h[1][2] = mul(TWO, fsum([yz, wx], tr), tr)
    h[2][0] = mul(TWO, fsum([xz, wy], tr), tr)
    h[2][1] = mul(TWO, fsum([yz, wx], tr), tr)
    L = [[ZERO] * 4 for _ in range(4)]
    for r in range(3):
        for c in range(3):
            L[r][c] = mul(h[r][c], s_pairs[c], tr)
        L[r][3] = mul(n, t_pairs[r], tr)
    L[3] = [ZERO, ZERO, ZERO, n]
    return L


def matrix_l(m_pairs, tr):
    """Node step for a matrix node. Per accepted reading v3.2 (9) / D1, the raw
    L entries count toward that node's s_step; `tr` records them."""
    L = [[ZERO] * 4 for _ in range(4)]
    for r in range(3):
        for c in range(4):
            L[r][c] = tr.note(m_pairs[4 * c + r])
    L[3] = [ZERO, ZERO, ZERO, ONE]
    return L


def compose(G, L, tr):
    return [[fsum([mul(G[i][k], L[k][j], tr) for k in range(4)], tr) for j in range(4)]
            for i in range(4)]


def level0(G, a, k, tr):
    """Level-0 position pairs (after U) and W for one (instance, accessor)."""
    P = [fsum([mul(G[r][0], a, tr), mul(G[r][1], a, tr), mul(G[r][2], a, tr),
               mul(G[r][3], k, tr)], tr) for r in range(3)]
    W = mul(G[3][3], k, tr)
    # U(x, y, z) = (x, -z, y); negation leaves a pair unchanged.
    return [P[0], P[2], P[1]], W


def tile_step(T_pairs, P, W, tr):
    """T_pairs is the column-major 16-entry pair list of an affine transform."""
    return [fsum([mul(T_pairs[0 * 4 + r], P[0], tr), mul(T_pairs[1 * 4 + r], P[1], tr),
                  mul(T_pairs[2 * 4 + r], P[2], tr), mul(T_pairs[3 * 4 + r], W, tr)], tr)
            for r in range(3)]


def volume_quantities(vol, P, W, tr):
    """Box or sphere quantities at one level. `vol` is a volumes.Volume."""
    if vol.kind == "box":
        for j in range(3):
            cw = mul(vol.c_pairs[j], W, tr)
            fsum([P[j], cw], tr)
            mul(vol.h_pairs[j], W, tr)
    elif vol.kind == "sphere":
        q = []
        for j in range(3):
            cw = mul(vol.c_pairs[j], W, tr)
            d = fsum([P[j], cw], tr)
            q.append(mul(d, d, tr))
        fsum(q, tr)
        r2 = mul(vol.r_pair, vol.r_pair, tr)
        w2 = mul(W, W, tr)
        mul(r2, w2, tr)
    else:
        raise ValueError("no span rule for volume kind %s" % vol.kind)


def level_spans(pair0, s_pos, levels):
    """Per-reference spans [s_0, ..., s_L] from the per-URI hull (Work bound, step 1-2).

    pair0: (P list after U, W) hull pairs. levels: list of chain.Level; level l>=1
    carries `to_parent_pairs` (the child tile's transform, or None)."""
    P, W = list(pair0[0]), pair0[1]
    spans = []
    for index, level in enumerate(levels):
        tr = Tracker()
        if index == 0:
            tr.max = s_pos
        else:
            if level.to_parent_pairs is not None:
                P = tile_step(level.to_parent_pairs, P, W, tr)
            tr.note(W)
        for vol in level.volumes_in_order():
            volume_quantities(vol.volume, P, W, tr)
        spans.append(tr.max)
    return spans
