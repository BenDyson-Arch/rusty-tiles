"""Generate fragmented glTF vector-fill cases for the Cesium fill acceptance probe.

RUSTY_TILES_BIN selects a portable or native CLI. Requires no oracle or downloads.
Fragmented fills are published as glTF vector content: every fill triangle is one
EXT_mesh_polygon polygon in the same GLB as the source boundary lines (issue #75).
"""
import argparse
import hashlib
import json
import pathlib
import struct
import types

from vector_compat import generate, vector


def fill_contents(root):
    """Count GLBs whose triangle primitive has one polygon per triangle; refuse b3dm."""
    count = 0
    for path in root.rglob('*'):
        assert path.suffix != '.b3dm', f'legacy b3dm fill {path}'
        if path.suffix != '.glb':
            continue
        data = path.read_bytes()
        assert data[:4] == b'glTF' and struct.unpack_from('<I', data, 8)[0] == len(data)
        size = struct.unpack_from('<I', data, 12)[0]
        doc = json.loads(data[20:20 + size])
        for primitive in doc['meshes'][0]['primitives']:
            polygon = primitive.get('extensions', {}).get('EXT_mesh_polygon')
            if polygon and polygon['count'] * 3 == doc['accessors'][primitive['indices']]['count']:
                count += 1
    return count


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=pathlib.Path)
    parser.add_argument('--compressed', action='store_true',
                        help='Exercise --quantize and --meshopt together')
    args = parser.parse_args()
    root = args.output
    root.mkdir(parents=True, exist_ok=False)
    fills = root / 'fills'
    generate(fills, quantize=args.compressed,
             meshopt_helper='in-process' if args.compressed else None, batch=True)
    cases = json.loads((fills / 'cases.json').read_text())
    countries_path = pathlib.Path(__file__).with_name('countries-source.geojson')
    countries = json.loads(countries_path.read_text())
    for country, sample in zip(countries['features'], [[30, 15, 100], [-60, -80, 100]]):
        name = country['properties']['name'].lower()
        source = fills / f'{name}.geojson'
        source.write_text(json.dumps(dict(type='FeatureCollection', features=[country])))
        vector.run(types.SimpleNamespace(
            input=str(source), output=str(fills / name), explicit=False,
            max_vertices=64, max_bytes=16384, lod_levels=3, lod_tolerance=1000,
            quantize=args.compressed, meshopt=args.compressed,
        ))
        cases.append(dict(name=name, propertyName=country['properties']['name'],
                          sample=sample, country=True, expectedId=str(country['id'])))
    (fills / 'cases.json').write_text(json.dumps(cases))
    count = fill_contents(fills)
    assert count > 0, 'The test requires fragmented fill content'
    (root / 'fixture.json').write_text(json.dumps(dict(
        compressed=args.compressed, fillContents=count,
        countrySourceSha256=hashlib.sha256(countries_path.read_bytes()).hexdigest(),
        cases=cases,
    ), indent=2) + '\n')
    # Bootstrap preview from the ordinary line case; the probe loads pairs.
    manifest = json.loads((fills / 'tileset.json').read_text())
    def prefix(value):
        if isinstance(value, dict):
            for key, item in value.items():
                if key == 'uri' and isinstance(item, str):
                    value[key] = 'fills/' + item
                else:
                    prefix(item)
        elif isinstance(value, list):
            for item in value:
                prefix(item)
    prefix(manifest)
    (root / 'tileset.json').write_text(json.dumps(manifest))


if __name__ == '__main__':
    main()
