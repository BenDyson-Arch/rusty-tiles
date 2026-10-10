#!/usr/bin/env python3
"""Nonauthor exact-reference checks of the inspected interval operations.

This is a Python arithmetic reproduction, not execution of Rust or a producer
certificate. Exact Fraction addresses supply the independent enclosure truth.
"""
from fractions import Fraction as F
import hashlib
import json
import math
from pathlib import Path
import random
import struct
import sys

D = 1 << 24
ROOT = Path(__file__).resolve().parents[4]


def down(x): return math.nextafter(x, -math.inf)
def up(x): return math.nextafter(x, math.inf)


def interval(face, weights):
    assert sum(weights) == D and all(0 <= w <= D for w in weights)
    result = []
    for axis in range(3):
        selected = [p[axis] for p, w in zip(face, weights) if w]
        if all(x == selected[0] for x in selected):
            result.append((selected[0], selected[0]))
            continue
        low = high = 0.0
        for p, w in zip(face, weights):
            if not w: continue
            product = p[axis] * (w / D)
            low, high = down(low + down(product)), up(high + up(product))
        result.append((low, high))
    return result


def exact(face, weights):
    return tuple(sum(F(p[a]) * F(w, D) for p, w in zip(face, weights)) for a in range(3))


def squared(source, target):
    total = 0.0
    for (sl, sh), (tl, th) in zip(source, target):
        if sl == sh == tl == th: continue
        residual = max(abs(down(sl - th)), abs(up(sh - tl)))
        total = up(total + up(residual * residual))
    return total


def main():
    rng = random.Random(1212026)
    specials = [0.0, -0.0, 1.0, -1.0, 1_000_000.0, -1_000_000.0,
                2.0 ** -149, -2.0 ** -149, 2.0 ** -126, 999_999.9375]
    def coordinate():
        if rng.randrange(3) == 0: return rng.choice(specials)
        while True:
            x = struct.unpack('<f', struct.pack('<I', rng.getrandbits(32)))[0]
            if math.isfinite(x) and abs(x) <= 1_000_000: return x
    def face(): return tuple(tuple(coordinate() for _ in range(3)) for _ in range(3))
    def weights():
        if rng.randrange(5) == 0:
            row = [0, 0, 0]; row[rng.randrange(3)] = D; return row
        a, b = sorted([rng.randrange(D + 1), rng.randrange(D + 1)])
        return [a, b - a, D - b]
    cases = 12_000
    for _ in range(cases):
        a, b, wa, wb = face(), face(), weights(), weights()
        ia, ib, ea, eb = interval(a, wa), interval(b, wb), exact(a, wa), exact(b, wb)
        for enclosure, point in ((ia, ea), (ib, eb)):
            assert all(F(lo) <= value <= F(hi) for (lo, hi), value in zip(enclosure, point))
        truth = sum((x - y) ** 2 for x, y in zip(ea, eb))
        bound = squared(ia, ib)
        assert F(bound) >= truth
        norm = 0.0 if bound == 0 else up(math.sqrt(bound))
        assert F(norm) ** 2 >= truth
    tiny = 2.0 ** -149
    trap = ((1_000_000.0, 0., 0.), (tiny, 1., 0.), (0., 0., 0.))
    w = [D // 2, D // 2, 0]
    actual, enclosure = exact(trap, w), interval(trap, w)
    rounded = 500_000.0
    assert actual[0] > F(rounded) and F(enclosure[0][0]) < actual[0] < F(enclosure[0][1])
    singleton = ((rounded, .5, 0.),) * 3
    trap_bound = squared(enclosure, interval(singleton, [D, 0, 0]))
    assert trap_bound > 0 and F(trap_bound) >= (actual[0] - F(rounded)) ** 2
    source = ROOT / 'src/mesh_archive/approximation/certificate.rs'
    receipt = {'scope': __doc__.strip(), 'python': sys.version.split()[0],
               'driver_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
               'inspected_certificate_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
               'seed': 1212026, 'paired_corner_cases': cases,
               'exact_corner_enclosures': cases * 2, 'exact_squared_and_norm_enclosures': cases,
               'mixed_magnitude_midpoint_exact_excess': str(actual[0] - F(rounded)),
               'rounded_midpoint_control_positive_bound': trap_bound,
               'passed': True}
    print(json.dumps(receipt, indent=2))


if __name__ == '__main__': main()
