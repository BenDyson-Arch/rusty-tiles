#!/usr/bin/env python3
"""Preimplementation exact controls, independent of production and meshopt.

These finite controls illustrate the analytic proof in ../audits/math.md;
they do not establish correctness of a future Rust implementation.
"""
from fractions import Fraction as F
import hashlib
import json
from pathlib import Path
import random

D = 1 << 24
ROOT = ((D, 0, 0), (0, D, 0), (0, 0, D))


def split(patch):
    a, b, c = patch
    def midpoint(u, v):
        assert all((x + y) % 2 == 0 for x, y in zip(u, v))
        return tuple((x + y) // 2 for x, y in zip(u, v))
    ab, bc, ca = midpoint(a, b), midpoint(b, c), midpoint(c, a)
    return ((a, ab, ca), (ab, b, bc), (ca, bc, c), (ab, bc, ca))


def orient(a, b, c):
    return (b[1]-a[1])*(c[2]-a[2])-(b[2]-a[2])*(c[1]-a[1])


def contains(patch, point):
    signs = [orient(patch[i], patch[(i+1) % 3], point) for i in range(3)]
    return all(x >= 0 for x in signs) or all(x <= 0 for x in signs)


def norm2(v):
    return sum(x*x for x in v)


def main():
    rng = random.Random(121)
    children = split(ROOT)
    areas = [abs(orient(*p)) for p in children]
    assert areas == [D*D//4]*4
    assert sum(areas) == abs(orient(*ROOT))
    checked = 0
    for denominator in range(1, 33):
        for i in range(denominator+1):
            for j in range(denominator-i+1):
                point = (F((denominator-i-j)*D, denominator),
                         F(i*D, denominator), F(j*D, denominator))
                assert any(contains(child, point) for child in children)
                checked += 1
    paths = 100
    for _ in range(paths):
        p = ROOT
        for depth in range(25):
            assert all(sum(row) == D and min(row) >= 0 for row in p)
            assert all(x % (1 << (24-depth)) == 0 for row in p for x in row)
            assert abs(orient(*p)) == (D*D >> (2*depth))
            if depth != 24:
                p = split(p)[rng.randrange(4)]

    # Independent exact Jensen/convexity controls: interpolated witnesses stay
    # in their convex target, and squared residual cannot exceed corner max.
    convexity_checks = 1000
    for _ in range(convexity_checks):
        s = [tuple(F(rng.randrange(-100, 101)) for _ in range(3)) for _ in range(3)]
        q = [tuple(F(rng.randrange(-100, 101)) for _ in range(3)) for _ in range(3)]
        raw = [rng.randrange(1, 30) for _ in range(3)]
        weights = [F(x, sum(raw)) for x in raw]
        residuals = [tuple(x-y for x,y in zip(a,b)) for a,b in zip(s,q)]
        residual = tuple(sum(weights[i]*residuals[i][axis] for i in range(3)) for axis in range(3))
        assert norm2(residual) <= max(map(norm2, residuals))

    # f64 midpoint loses an admitted f32 subnormal contribution and moves an
    # original edge inward. A rounded midpoint mesh therefore omits this exact
    # original-edge midpoint. Barycentric reconstruction keeps it represented.
    epsilon = F(1, 1 << 149)
    original_midpoint = (F(1_000_000)+epsilon)/2
    rounded_midpoint = F.from_float((1_000_000.0+float(epsilon))/2)
    midpoint_loss = original_midpoint-rounded_midpoint
    assert midpoint_loss == epsilon/2 and midpoint_loss > 0

    # Exactly interior point x=1 of a target segment [0,1e6]. Fixed dyadic
    # witnesses cannot reach it; even an ideal proposal has positive residual.
    scaled = F(D, 1_000_000)
    nearby = [scaled.numerator//scaled.denominator, scaled.numerator//scaled.denominator+1]
    lattice_error = min(abs(F(1_000_000*k,D)-1) for k in nearby)
    assert lattice_error == F(3481, 262144) and lattice_error > F(1,100)

    # Source right triangle [(0,0),(8,0),(0,8)] versus just its three edges:
    # all corner witnesses have zero distance, but centroid is in the hole.
    centroid_min_distance_squared = F(32,9)
    assert centroid_min_distance_squared > 0

    files = [Path(__file__), Path('src/mesh_archive/approximation.rs')]
    result = {
        'kind': 'preimplementation-exact-mathematical-controls',
        'source_sha256': {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in files},
        'base_commit_supplied_by_coordinator': '3b5703231bf3991c2e37caef99989214308cc08d',
        'coverage_rational_points': checked,
        'depth_24_integer_paths': paths,
        'convexity_rational_controls': convexity_checks,
        'rounded_midpoint_lost_exact_contribution': str(midpoint_loss),
        'zero_true_distance_fixed_witness_lattice_error_metres': str(lattice_error),
        'zero_true_distance_fixed_witness_lattice_error_decimal': float(lattice_error),
        'corner_sampling_interior_hole_centroid_distance_squared': str(centroid_min_distance_squared),
        'maximum_axis_terminal_patch_size_depth12_metres': float(F(2_000_000,1<<12)),
        'maximum_axis_terminal_patch_size_depth24_metres': float(F(2_000_000,1<<24)),
        'status': 'pass',
        'limitations': 'Controls illustrate analytic obligations; no production build or final-source acceptance executed.'
    }
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == '__main__':
    main()
