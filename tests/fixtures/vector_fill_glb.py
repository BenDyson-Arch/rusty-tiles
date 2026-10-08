"""Generate wrapped/plain-GLB pairs for the issue #75 Cesium regression.

RUSTY_TILES_BIN selects a portable or native CLI. Requires no oracle or downloads.
The plain case changes only framing and content URI suffixes; glTF bytes,
metadata, source boundaries and the tileset-wide vector declaration stay intact.
"""
import argparse
import hashlib
import json
import pathlib
import shutil
import struct
import types

from vector_compat import generate, vector


def unwrap(root):
    count = 0
    for path in root.rglob('*.b3dm'):
        data = path.read_bytes()
        magic, version, length, *tables = struct.unpack_from('<4s6I', data)
        assert magic == b'b3dm' and version == 1 and length == len(data)
        offset = 28 + sum(tables)
        glb = data[offset:]
        assert glb[:4] == b'glTF' and struct.unpack_from('<I', glb, 8)[0] == len(glb)
        path.with_suffix('.glb').write_bytes(glb)
        path.unlink()
        count += 1
    # Explicit reuse sources and implicit coordinate templates both refer to
    # these payloads. No other manifest fields or GLB bytes are rewritten.
    for path in root.rglob('*.json'):
        path.write_text(path.read_text().replace('.b3dm', '.glb'))
    return count


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=pathlib.Path)
    parser.add_argument('--compressed', action='store_true',
                        help='Exercise --quantize and --meshopt together')
    args = parser.parse_args()
    root = args.output
    root.mkdir(parents=True, exist_ok=False)
    wrapped = root / 'wrapped'
    generate(wrapped, quantize=args.compressed,
             meshopt_helper='in-process' if args.compressed else None, batch=True)
    cases = json.loads((wrapped / 'cases.json').read_text())
    countries_path = pathlib.Path(__file__).with_name('countries-source.geojson')
    countries = json.loads(countries_path.read_text())
    for country, sample in zip(countries['features'], [[30, 15, 100], [-60, -80, 100]]):
        name = country['properties']['name'].lower()
        source = wrapped / f'{name}.geojson'
        source.write_text(json.dumps(dict(type='FeatureCollection', features=[country])))
        vector.run(types.SimpleNamespace(
            input=str(source), output=str(wrapped / name), explicit=False,
            max_vertices=64, max_bytes=16384, lod_levels=3, lod_tolerance=1000,
            quantize=args.compressed, meshopt=args.compressed,
        ))
        cases.append(dict(name=name, propertyName=country['properties']['name'],
                          sample=sample, country=True, expectedId=str(country['id'])))
    (wrapped / 'cases.json').write_text(json.dumps(cases))
    plain = root / 'plain'
    shutil.copytree(wrapped, plain)
    count = unwrap(plain)
    assert count > 0, 'The test requires fragmented fill content'
    (root / 'fixture.json').write_text(json.dumps(dict(
        compressed=args.compressed, unwrappedContents=count,
        countrySourceSha256=hashlib.sha256(countries_path.read_bytes()).hexdigest(),
        cases=cases,
    ), indent=2) + '\n')
    # Bootstrap preview from the ordinary line case; the probe loads pairs.
    manifest = json.loads((wrapped / 'tileset.json').read_text())
    def prefix(value):
        if isinstance(value, dict):
            for key, item in value.items():
                if key == 'uri' and isinstance(item, str):
                    value[key] = 'wrapped/' + item
                else:
                    prefix(item)
        elif isinstance(value, list):
            for item in value:
                prefix(item)
    prefix(manifest)
    (root / 'tileset.json').write_text(json.dumps(manifest))


if __name__ == '__main__':
    main()
