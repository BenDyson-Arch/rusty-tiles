"""Generate invented vector cases for the optional CesiumJS IIFE matrix.

Usage: python3 tests/fixtures/vector_compat.py /path/to/new/output/directory
Requires the converter's GDAL/GEOS and NumPy dependencies; downloads nothing.
"""
import json
import math
import pathlib
import sys
import types

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[2] / 'scripts'))
import vector


def pos(x, y, z=100):
    return [9 + x / 78800, 45 + y / 111100, z]


def ring(radius, count, wave=0):
    positions = [
        pos(radius * math.cos(angle), radius * math.sin(angle),
            100 + wave * math.sin(5 * angle))
        for angle in (2 * math.pi * i / count for i in range(count))
    ]
    return positions + [positions[0]]


def generate(root):
    root.mkdir(parents=True, exist_ok=False)
    features = {
        'line': dict(type='LineString', coordinates=[
            pos(i * .2, .15 * math.sin(i * .1), 100 + .02 * math.cos(i * .1))
            for i in range(201)
        ]),
        'polygon': dict(type='Polygon', coordinates=[ring(20, 100, .005), ring(5, 40, .005)]),
        'fragmented': dict(type='Polygon', coordinates=[ring(20, 40), ring(5, 20)]),
        'outline': dict(type='Polygon', coordinates=[
            [pos(0, 0), pos(1, 0), pos(1.5, 0), pos(1, 0), pos(0, 0)]
        ]),
        'point': dict(type='Point', coordinates=pos(0, 0)),
    }
    cases = []
    for name, geometry in features.items():
        source = root / (name + '.geojson')
        source.write_text(json.dumps(dict(type='FeatureCollection', features=[
            dict(type='Feature', id=name, properties=dict(name=name), geometry=geometry)
        ])))
        vector.run(types.SimpleNamespace(
            input=str(source), output=str(root / name), max_features=64,
            max_vertices=8 if name == 'fragmented' else 65536, max_bytes=16384,
            lod_tolerance=.2, lod_levels=3, repair=name == 'outline',
            ambiguous_outlines=name == 'outline',
        ))
        if name == 'line':
            sample = pos(10, .15 * math.sin(5), 100 + .02 * math.cos(5))
        elif name in ('polygon', 'fragmented'):
            sample = pos(12, 0)
        elif name == 'outline':
            sample = pos(.75, 0)
        else:
            sample = pos(0, 0)
        cases.append(dict(
            name=name, sample=sample,
            hole=pos(0, 0) if name in ('polygon', 'fragmented') else None,
            boundary=pos(20, 0) if name == 'fragmented' else None,
        ))
    (root / 'cases.json').write_text(json.dumps(cases))
    # The repository preview boots from a single annotations/tileset.json.
    manifest = json.loads((root / 'line/tileset.json').read_text())

    def prefix(node):
        contents = node.get('contents', [node['content']] if 'content' in node else [])
        for content in contents:
            content['uri'] = 'line/' + content['uri']
        for child in node.get('children', []):
            prefix(child)

    prefix(manifest['root'])
    (root / 'tileset.json').write_text(json.dumps(manifest))


if __name__ == '__main__':
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    generate(pathlib.Path(sys.argv[1]))
