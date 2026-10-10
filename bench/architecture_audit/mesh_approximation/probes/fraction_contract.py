#!/usr/bin/env python3
"""Independent exact-rational contract probes; no production geometry imports."""
import hashlib
import json
import platform
from fractions import Fraction as F
from pathlib import Path


def sub(a, b):
    return tuple(x - y for x, y in zip(a, b))


def dot(a, b):
    return sum(x * y for x, y in zip(a, b))


def triangle(*points):
    return tuple(tuple(map(F, p)) for p in points)


def distance_squared(p, face):
    """Exact plane/edge witnesses, with exact simplex membership comparisons."""
    a, b, c = face
    e, f, r = sub(b, a), sub(c, a), sub(p, a)
    g00, g01, g11 = dot(e, e), dot(e, f), dot(f, f)
    h0, h1 = dot(e, r), dot(f, r)
    candidates = []
    den = g00 * g11 - g01 * g01
    if den > 0:
        u = (h0 * g11 - h1 * g01) / den
        v = (h1 * g00 - h0 * g01) / den
        if u >= 0 and v >= 0 and u + v <= 1:
            candidates.append(tuple(a[i] + u * e[i] + v * f[i] for i in range(3)))
    for x, y in [(a, b), (b, c), (c, a)]:
        d = sub(y, x)
        dd = dot(d, d)
        s = max(F(0), min(F(1), dot(sub(p, x), d) / dd)) if dd else F(0)
        candidates.append(tuple(x[i] + s * d[i] for i in range(3)))
    return min(dot(sub(p, q), sub(p, q)) for q in candidates)


def directed_squared(source, target):
    return max(min(max(distance_squared(p, t) for p in s) for t in target) for s in source)


def symmetric_squared(a, b):
    return max(directed_squared(a, b), directed_squared(b, a))


def subdivide(t):
    a, b, c = t
    def midpoint(x, y):
        return tuple((u + v) / 2 for u, v in zip(x, y))
    ab, bc, ca = midpoint(a, b), midpoint(b, c), midpoint(c, a)
    return [(a, ab, ca), (ab, b, bc), (ca, bc, c), (ab, bc, ca)]


def main():
    parent = [triangle((0, 0, 0), (8, 0, 0), (0, 8, 0))]
    child = subdivide(parent[0])
    flat = [triangle((0, 0, 0), (1, 0, 0), (0, 1, 0))]
    spike = [triangle((0, 0, 0), (1, 0, 100), (0, 1, 100))] + flat * 2999
    hole = [triangle((0, 0, 0), (1, 0, 0), (0, 1, 0)),
            triangle((8, 0, 0), (7, 0, 0), (7, 1, 0)),
            triangle((0, 8, 0), (0, 7, 0), (1, 7, 0))]
    point = [triangle((0, 0, 0), (0, 0, 0), (0, 0, 0))]
    tiny = F(1, 2**149)
    mixed = [triangle((-tiny, 0, 0), (1000000, 0, 0), (1, 0, 0))]
    checks = {
        "planar_whole_face_squared_bound": symmetric_squared(child, parent),
        "planar_subdivided_cover_squared_bound": symmetric_squared(child, subdivide(parent[0])),
        "spike_squared_bound": directed_squared(spike, flat),
        "spike_reverse_squared_bound": directed_squared(list(reversed(spike)), flat),
        "hole_vertex_only_squared_distance": max(min(distance_squared(p, t) for t in hole) for p in parent[0]),
        "hole_whole_face_squared_bound": directed_squared(parent, hole),
        "degenerate_point_squared_bound": directed_squared(flat, point),
        "mixed_magnitude_identical_squared_bound": symmetric_squared(mixed, mixed),
    }
    expected = dict(zip(checks, [16, 0, 10000, 10000, 0, 49, 1, 0]))
    assert checks == expected, (checks, expected)
    assert checks["spike_squared_bound"] > 0  # Sensitive sampled-zero negative control.
    assert checks["hole_whole_face_squared_bound"] > checks["hole_vertex_only_squared_distance"]
    print(json.dumps({
        "driver_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "python": platform.python_version(),
        "scope": "Exact analytic contract probes; no production execution or artifact acceptance",
        "checks": {key: str(value) for key, value in checks.items()},
        "passed": True,
    }, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
