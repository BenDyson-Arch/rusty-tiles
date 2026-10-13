"""Generate the M1 serializer fixtures A and B (design §4) from authored ENU offsets.

Stdlib only. The anchor A is lon 7.4385, lat 46.948, h 540.0 m (WGS84). Every vertex is an
asymmetric, mixed-sign east/north/up offset in metres from A, converted to lon/lat/h with a
forward (geodetic -> ECEF) and an independent inverse (ECEF -> geodetic, Bowring then
Newton) WGS84 transform; each written vertex must round-trip to its authored offset within
1e-4 m after rounding to 10 decimals. The checked-in GeoJSON is the truth; this script
documents how it was made.

Usage: python3 -I tests/m1_fixture_gen.py OUT_DIR
"""
import json
import math
import pathlib
import sys

A_SEMI = 6378137.0
F = 1 / 298.257223563
E2 = F * (2 - F)
ANCHOR = (7.4385, 46.948, 540.0)


def to_ecef(lon, lat, h):
    lam, phi = math.radians(lon), math.radians(lat)
    n = A_SEMI / math.sqrt(1 - E2 * math.sin(phi) ** 2)
    return ((n + h) * math.cos(phi) * math.cos(lam),
            (n + h) * math.cos(phi) * math.sin(lam),
            (n * (1 - E2) + h) * math.sin(phi))


def to_geodetic(x, y, z):
    lam = math.atan2(y, x)
    p = math.hypot(x, y)
    b = A_SEMI * (1 - F)
    ep2 = (A_SEMI ** 2 - b ** 2) / b ** 2
    theta = math.atan2(z * A_SEMI, p * b)
    phi = math.atan2(z + ep2 * b * math.sin(theta) ** 3, p - E2 * A_SEMI * math.cos(theta) ** 3)
    for _ in range(5):
        n = A_SEMI / math.sqrt(1 - E2 * math.sin(phi) ** 2)
        h = p / math.cos(phi) - n
        phi = math.atan2(z, p * (1 - E2 * n / (n + h)))
    n = A_SEMI / math.sqrt(1 - E2 * math.sin(phi) ** 2)
    h = p / math.cos(phi) - n
    return math.degrees(lam), math.degrees(phi), h


def enu_axes(lon, lat):
    lam, phi = math.radians(lon), math.radians(lat)
    east = (-math.sin(lam), math.cos(lam), 0.0)
    north = (-math.sin(phi) * math.cos(lam), -math.sin(phi) * math.sin(lam), math.cos(phi))
    up = (math.cos(phi) * math.cos(lam), math.cos(phi) * math.sin(lam), math.sin(phi))
    return east, north, up


ORIGIN = to_ecef(*ANCHOR)
AXES = enu_axes(ANCHOR[0], ANCHOR[1])


def from_enu(offset):
    point = [ORIGIN[i] + sum(offset[k] * AXES[k][i] for k in range(3)) for i in range(3)]
    lon, lat, h = to_geodetic(*point)
    vertex = [round(lon, 10), round(lat, 10), round(h, 10)]
    back = to_ecef(*vertex)
    enu = [sum((back[i] - ORIGIN[i]) * AXES[k][i] for i in range(3)) for k in range(3)]
    if max(abs(enu[k] - offset[k]) for k in range(3)) > 1e-4:
        raise SystemExit(f'round trip of {offset} drifts to {enu}')
    return vertex


def line(start, end, displacements):
    """`len(displacements)` interior vertices, evenly spaced, each displaced horizontally
    perpendicular to the chord by its signed amount."""
    de, dn = end[0] - start[0], end[1] - start[1]
    length = math.hypot(de, dn)
    perpendicular = (-dn / length, de / length)
    count = len(displacements) + 1
    vertices = [start]
    for i, d in enumerate(displacements, 1):
        t = i / count
        vertices.append((start[0] + t * de + d * perpendicular[0],
                         start[1] + t * dn + d * perpendicular[1],
                         start[2] + t * (end[2] - start[2])))
    vertices.append(end)
    return vertices


def collection(line_offsets):
    def feature(fid, geometry):
        return {'type': 'Feature', 'id': fid, 'properties': {'name': fid}, 'geometry': geometry}
    return {
        'type': 'FeatureCollection',
        'features': [
            feature('pt-alpha', {'type': 'Point', 'coordinates': from_enu((0.0, 0.0, 0.0))}),
            feature('ln-bravo', {'type': 'LineString',
                                 'coordinates': [from_enu(v) for v in line_offsets]}),
        ],
    }


START, END = (120.0, -45.0, 7.0), (180.0, 60.0, 3.0)
FIXTURES = {
    # A: 7 interior vertices alternately displaced +-0.30 m.
    'm1_a.geojson': line(START, END, [0.30 * (-1) ** i for i in range(7)]),
    # B: 4 interior vertices at t = 0.2, 0.4, 0.6, 0.8, displaced so that each LOD level
    # (tolerance 0.5, 1, 2, 4 m) removes exactly one vertex under Douglas-Peucker: v2 is 3.0 m
    # off the chord; v3 is 1.5 m off v2->end; v4 is 0.75 m off v3->end; v1 is 0.3 m off
    # start->v2. Observed chain (leaf to L4): 6, 5, 4, 3, 2 vertices.
    'm1_b.geojson': line(START, END, [1.8, 3.0, 0.5, 1.0]),
}

if __name__ == '__main__':
    out = pathlib.Path(sys.argv[1])
    for name, offsets in FIXTURES.items():
        text = json.dumps(collection(offsets), indent=1) + '\n'
        (out / name).write_text(text)
        print(name, len(offsets), 'line vertices')
