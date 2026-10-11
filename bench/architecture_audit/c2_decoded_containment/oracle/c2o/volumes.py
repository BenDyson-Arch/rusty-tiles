"""Bounding volumes: admission into the slice-1 profile and exact predicates.

Box: allowed when each half-axis column has at most one nonzero entry and the
nonzero entries lie in distinct rows (signed permutation). Row j has
half-length |entry| of the covering column, else 0. Membership:
|p_j - c_j| <= h_j for every row. Sphere: sum (p_j - c_j)^2 <= r^2.
Region: excluded (`regionVolume`). Other boxes: `rotatedBox`.
"""

from fractions import Fraction

from .numbers import OracleUndecided, exact, is_number
from .spans import ZERO, pair_of


class Volume:
    def __init__(self, kind, c=None, h=None, r=None, raw=None):
        self.kind = kind  # "box" | "sphere" | "region" | "rotatedBox"
        self.c = c
        self.h = h
        self.r = r
        self.raw = raw
        if kind == "box":
            self.c_pairs = [pair_of(v) for v in c]
            self.h_pairs = [pair_of(v) for v in h]
        elif kind == "sphere":
            self.c_pairs = [pair_of(v) for v in c]
            self.r_pair = pair_of(r)

    def exclusion(self):
        if self.kind == "region":
            return "regionVolume"
        if self.kind == "rotatedBox":
            return "rotatedBox"
        if self.kind == "extensionOnly":
            return "tilesetExtension"
        return None

    def contains(self, p):
        """Exact membership of a point given as three Fractions (slow reference path)."""
        if self.kind == "box":
            return all(abs(p[j] - self.c[j]) <= self.h[j] for j in range(3))
        if self.kind == "sphere":
            return sum((p[j] - self.c[j]) ** 2 for j in range(3)) <= self.r * self.r
        raise ValueError("no predicate for %s" % self.kind)


def _numbers(value, n, what):
    if not isinstance(value, list) or len(value) != n or not all(is_number(v) for v in value):
        raise OracleUndecided("malformed %s (C1 is expected to reject it)" % what)
    return [exact(v) for v in value]


def parse_volume(bv, where):
    """Parse a tile or content boundingVolume object into a Volume."""
    if not isinstance(bv, dict):
        raise OracleUndecided("boundingVolume is not an object at %s" % where)
    kinds = [k for k in ("box", "region", "sphere") if k in bv]
    if len(kinds) != 1:
        # Only an extension (e.g. 3DTILES_bounding_volume_S2) or several core shapes.
        if not kinds and isinstance(bv.get("extensions"), dict) and bv["extensions"]:
            return Volume("extensionOnly", raw=bv)
        raise OracleUndecided("boundingVolume at %s has %d core shapes" % (where, len(kinds)))
    kind = kinds[0]
    if kind == "region":
        _numbers(bv["region"], 6, "region at %s" % where)
        return Volume("region", raw=bv)
    if kind == "sphere":
        values = _numbers(bv["sphere"], 4, "sphere at %s" % where)
        if values[3] < 0:
            raise OracleUndecided("negative sphere radius at %s (C1 rejects r < 0)" % where)
        return Volume("sphere", c=values[:3], r=values[3], raw=bv)
    values = _numbers(bv["box"], 12, "box at %s" % where)
    c = values[:3]
    columns = [values[3:6], values[6:9], values[9:12]]
    h = [Fraction(0)] * 3
    covered = [False] * 3
    for col in columns:
        nonzero = [row for row in range(3) if col[row] != 0]
        if len(nonzero) > 1:
            return Volume("rotatedBox", raw=bv)
        if nonzero:
            row = nonzero[0]
            if covered[row]:
                return Volume("rotatedBox", raw=bv)
            covered[row] = True
            h[row] = abs(col[row])
    return Volume("box", c=c, h=h, raw=bv)


def _lcm(a, b):
    from math import gcd
    return a // gcd(a, b) * b


class LevelTest:
    """Integer form of one volume test at one level for one (instance, accessor).

    The level coordinate is coord_j = sum_c A[j][c] * s * v_c + b_j, where v is
    the integer raw word vector and s its exact scale. Everything is multiplied
    by a common positive integer so that the test is pure integer arithmetic.
    """

    def __init__(self, vol, A, b, scale):
        self.vol = vol
        rows = []
        dens = 1
        terms = []
        for j in range(3):
            alpha = [A[j][c] * scale for c in range(3)]
            beta = b[j] - vol.c[j]
            terms.append((alpha, beta))
            for x in alpha + [beta]:
                dens = _lcm(dens, x.denominator)
        if vol.kind == "box":
            for j in range(3):
                dens = _lcm(dens, vol.h[j].denominator)
        else:
            dens = _lcm(dens, vol.r.denominator)
        for alpha, beta in terms:
            rows.append(([int(a * dens) for a in alpha], int(beta * dens)))
        self.rows = rows
        if vol.kind == "box":
            self.bounds = [int(vol.h[j] * dens) for j in range(3)]
        else:
            self.r2 = int(vol.r * dens) ** 2

    def inside(self, v):
        v0, v1, v2 = v
        if self.vol.kind == "box":
            for (a, beta), bound in zip(self.rows, self.bounds):
                d = a[0] * v0 + a[1] * v1 + a[2] * v2 + beta
                if d > bound or -d > bound:
                    return False
            return True
        total = 0
        for a, beta in self.rows:
            d = a[0] * v0 + a[1] * v1 + a[2] * v2 + beta
            total += d * d
        return total <= self.r2
