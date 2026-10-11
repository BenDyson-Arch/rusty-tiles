"""C2 preparation probe: tight producer-style AABB boxes under exact vs interval policy.

Standalone, single-threaded, deterministic. Not production/Rust evidence.
Questions:
 Q1 How often does a tight f64 box (c=(lo+hi)/2, h=(hi-lo)/2, both rounded)
    fail to contain its own extreme f32 vertices in exact arithmetic?
 Q2 How often does the prior outward-interval rule (math-contract.md) return
    Unresolved for a vertex that IS exactly contained (tight boundary)?
 Q3 Same, after one translation-only child->parent transform (tileset_node.rs
    translation()) into a parent box built by enclose_children-style max().
 Q4 Does an error-free exact predicate (rational here; expansion arithmetic in
    production) decide every case (no Unresolved)?
"""
import json
import math
import random
import struct
import sys
from fractions import Fraction as F
from pathlib import Path


def f32(x):
    return struct.unpack('<f', struct.pack('<f', x))[0]


def widen(lo, hi):
    return math.nextafter(lo, -math.inf), math.nextafter(hi, math.inf)


def isub(a, b):  # interval a - b, b exact
    return widen(a[0] - b, a[1] - b)


def iadd(a, b):
    return widen(a[0] + b, a[1] + b)


def interval_axis(p_iv, c, h):
    d = isub(p_iv, c)
    lo = 0.0 if d[0] <= 0 <= d[1] else min(abs(d[0]), abs(d[1]))
    hi = max(abs(d[0]), abs(d[1]))
    if hi <= h:
        return 'Contained'
    if lo > h:
        return 'Outside'
    return 'Unresolved'


def exact_axis(p, c, h):
    return 'Contained' if abs(F(p) - F(c)) <= F(h) else 'Outside'


def main():
    rng = random.Random(20261011)
    n = 20000
    stats = {k: 0 for k in (
        'q1_exact_outside', 'q2_interval_unresolved_but_exact_contained',
        'q2_interval_wrong', 'q3_parent_exact_outside',
        'q3_interval_unresolved_but_exact_contained', 'q3_interval_wrong')}
    examples = []
    for _ in range(n):
        scale = 10 ** rng.uniform(-1, 4)  # 0.1 m .. 10 km local extents
        a = f32(rng.uniform(-scale, scale))
        b = f32(rng.uniform(-scale, scale))
        lo, hi = min(a, b), max(a, b)
        c = (lo + hi) / 2
        h = (hi - lo) / 2
        for v in (lo, hi):
            ex = exact_axis(v, c, h)
            if ex == 'Outside':
                stats['q1_exact_outside'] += 1
                if len(examples) < 3:
                    examples.append({'vertex_f32': v.hex(), 'center': c.hex(), 'half': h.hex(),
                                     'excess': str(abs(F(v) - F(c)) - F(h))})
            iv = interval_axis((v, v), c, h)
            if iv == 'Unresolved' and ex == 'Contained':
                stats['q2_interval_unresolved_but_exact_contained'] += 1
            if iv != 'Unresolved' and iv != ex:
                stats['q2_interval_wrong'] += 1
        # Q3: child box above is placed at translation delta inside a parent
        # whose half is grown by max(|delta|+child_half) (tileset_node.rs style).
        delta = rng.uniform(-scale, scale)
        parent_c = 0.0
        parent_h = abs(delta) + h  # rounded f64, like enclose_children
        # content point in parent frame: v_child_local - c + ... producers place
        # child content relative to child centre: local = v - c (rounded f64->f32?)
        local = f32(hi - c)  # positions written relative to tile centre as f32
        parent_exact = F(local) + F(delta)
        ex3 = 'Contained' if abs(parent_exact - F(parent_c)) <= F(parent_h) else 'Outside'
        # also own-tile check of the recentred f32 vertex against origin-centred h
        if ex3 == 'Outside':
            stats['q3_parent_exact_outside'] += 1
        iv3 = interval_axis(iadd((local, local), delta), parent_c, parent_h)
        if iv3 == 'Unresolved' and ex3 == 'Contained':
            stats['q3_interval_unresolved_but_exact_contained'] += 1
        if iv3 != 'Unresolved' and iv3 != ex3:
            stats['q3_interval_wrong'] += 1
    out = {
        'scope': 'standalone probe; models producer-style rounding only, not any actual producer',
        'python': sys.version.split()[0],
        'samples': n,
        'vertexChecksQ1Q2': 2 * n,
        'stats': stats,
        'q1Examples': examples,
    }
    dest = Path(__file__).with_name('tight-bounds-observed.json')
    dest.write_text(json.dumps(out, indent=2) + '\n')
    print(json.dumps(out['stats']))


if __name__ == '__main__':
    main()
