"""Deterministic scaled vector source for validator-cap measurement (stdlib only).

Usage: python3 -I gen_vector.py OUT.geojson SCALE [--points N]

SCALE=1 reproduces the composition of bench/perf_lane/corpus.py write_geojson
(3,600 polygons, a fifth with holes; 400 lines of 100 vertices; 6,000 points)
on a sqrt(SCALE)-times larger grid, so SCALE multiplies every feature class.
With --points N, writes N uniformly scattered attributed points instead
(small payloads, so count-driven limits bind before byte-driven ones).
"""
import json
import math
import sys


def lcg(seed):
    while True:
        seed = (seed * 6364136223846793005 + 1442695040888963407) % 2**64
        yield seed >> 33


def points(out, count):
    noise, features = lcg(11), []
    for i in range(count):
        features.append(dict(type='Feature', properties=dict(id=i, kind='tree', girth=next(noise) % 300 / 100),
            geometry=dict(type='Point', coordinates=[12.40 + next(noise) % 400000 * 1e-6, 41.85 + next(noise) % 400000 * 1e-6])))
    with open(out, 'w') as stream:
        json.dump(dict(type='FeatureCollection', features=features), stream, separators=(',', ':'))
    print(json.dumps({'features': count, 'points': count}))


def main():
    if len(sys.argv) == 5 and sys.argv[3] == '--points':
        return points(sys.argv[1], int(sys.argv[4]))
    out, scale = sys.argv[1], float(sys.argv[2])
    side = round(60 * math.sqrt(scale))
    span = side * .002
    noise, features = lcg(7), []
    for row in range(side):
        for col in range(side):
            x, y, s = 12.40 + col * .002, 41.85 + row * .002, .0006 + next(noise) % 400 * 1e-6
            ring = [[x, y], [x + s, y], [x + s, y + s * .8], [x + s * .4, y + s * 1.2], [x, y + s], [x, y]]
            rings = [ring]
            if (row + col) % 5 == 0:
                h = s * .25
                rings.append([[x + h, y + h], [x + h, y + 2 * h], [x + 2 * h, y + 2 * h], [x + 2 * h, y + h], [x + h, y + h]])
            features.append(dict(type='Feature', properties=dict(id=len(features), kind='building', height=3 + next(noise) % 40,
                name=f'b{row}-{col}'), geometry=dict(type='Polygon', coordinates=rings)))
    lines = round(400 * scale)
    for line in range(lines):
        y0 = 41.85 + (line * span / lines)
        coords = [[12.40 + step * span / 100, y0 + .0004 * math.sin(step / 3 + line)] for step in range(100)]
        features.append(dict(type='Feature', properties=dict(id=len(features), kind='road', lanes=1 + line % 4),
            geometry=dict(type='LineString', coordinates=coords)))
    cells = int(span * 1e6)
    for _ in range(round(6000 * scale)):
        features.append(dict(type='Feature', properties=dict(id=len(features), kind='tree', girth=next(noise) % 300 / 100),
            geometry=dict(type='Point', coordinates=[12.40 + next(noise) % cells * 1e-6, 41.85 + next(noise) % cells * 1e-6])))
    with open(out, 'w') as stream:
        json.dump(dict(type='FeatureCollection', features=features), stream, separators=(',', ':'))
    print(json.dumps({'features': len(features), 'polygons': side * side, 'lines': lines,
                      'points': round(6000 * scale)}))


if __name__ == '__main__':
    main()
