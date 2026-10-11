"""Reviewer probe (not production evidence): exact bit span of the C2 point chain.

Question: D7 says f64 expansion arithmetic is exact within a "magnitude domain" and that
nonoverlapping expansions stay <= ~40 components. Expansion components are binary64, so an
exact intermediate is representable only if its lowest set bit is >= 2^-1074 and its highest
bit < 2^1024 (after any power-of-two rescaling, the binding quantity is the SPAN between
them). This probe measures, for chains of k TRS nodes with rotation cleared by ||q||^2
(the contract's homogeneous form) applied step-wise to an f32 point, the span and lowest
bit of the exact numerator/denominator, with no rescaling.

Single-threaded, deterministic, stdlib only.
"""
import json
import math
import random
import struct
import sys
from fractions import Fraction as F


def f32(x):
    return struct.unpack('<f', struct.pack('<f', x))[0]


def lowbit(fr):
    """Exponent of the lowest set bit of a dyadic rational (denominator power of two)."""
    if fr == 0:
        return None
    n, d = fr.numerator, fr.denominator
    assert d & (d - 1) == 0, 'not dyadic'
    tz = (n & -n).bit_length() - 1
    return tz - (d.bit_length() - 1)


def highbit(fr):
    return math.floor(math.log2(abs(fr))) if fr else None


def H(q):
    x, y, z, w = q
    return [
        [w*w + x*x - y*y - z*z, 2*(x*y - w*z), 2*(x*z + w*y)],
        [2*(x*y + w*z), w*w - x*x + y*y - z*z, 2*(y*z - w*x)],
        [2*(x*z - w*y), 2*(y*z + w*x), w*w - x*x - y*y + z*z],
    ]


def quat(rng, style):
    if style == 'generic':
        v = [rng.gauss(0, 1) for _ in range(4)]
    else:  # exporter-style near-axis rotation: tiny f64 noise components (e.g. 7e-17)
        v = [rng.choice([1e-17, -6.1e-17, 3e-9]), 0.0, rng.uniform(-1, 1), 1.0]
    n = math.sqrt(sum(c * c for c in v))
    return [c / n for c in v]  # f64-normalized, as exporters write


def chain(rng, k, style):
    p = [F(f32(rng.uniform(-1000, 1000))) for _ in range(3)]
    N, D = p, F(1)
    worst_low, worst_span = 0, 0
    for _ in range(k):
        q = [F(c) for c in quat(rng, style)]
        s = [F(rng.uniform(0.1, 10)) for _ in range(3)]
        t = [F(rng.uniform(-100, 100)) for _ in range(3)]
        n = sum(c * c for c in q)
        h = H(q)
        sN = [s[i] * N[i] for i in range(3)]
        N = [n * D * t[i] + sum(h[i][j] * sN[j] for j in range(3)) for i in range(3)]
        D = n * D
        for v in N + [D]:
            if v:
                lo, hi = lowbit(v), highbit(v)
                worst_low = min(worst_low, lo)
                worst_span = max(worst_span, hi - lo + 1)
    return worst_low, worst_span


def main():
    rng = random.Random(20261011)
    out = {'scope': 'reviewer probe; exact Fraction; no rescaling', 'python': sys.version.split()[0],
           'binary64_lowest_bit': -1074, 'binary64_total_span_bits': 2098, 'rows': []}
    for style in ('generic', 'exporter-near-axis'):
        for k in (1, 2, 3, 4, 6, 8, 12, 16):
            lows, spans = [], []
            for _ in range(20):
                lo, sp = chain(rng, k, style)
                lows.append(lo)
                spans.append(sp)
            out['rows'].append({'style': style, 'depth': k, 'min_lowbit': min(lows),
                                'max_span_bits': max(spans),
                                'min_components_if_53bit_packed': math.ceil(max(spans) / 53),
                                'underflows_without_rescale': min(lows) < -1074,
                                'exceeds_binary64_span': max(spans) > 2098})
    print(json.dumps(out, indent=1))


if __name__ == '__main__':
    main()
