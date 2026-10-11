"""Map today's implicit-archive content URIs to producer preorder ids (stdlib only).

The explicit archive (same input and options, --explicit) gives the producer hierarchy H; preorder
ids match forest_model.py's numbering. Today's writer addresses child i of a tile with slot i
(vector: no implicitChildIndex), so global coordinates follow from the slot path.

Usage: python3 -I today_map.py EXPLICIT.3tz TODAY_IMPLICIT.3tz VIEWS_IN.json VIEWS_OUT.json
Writes VIEWS_OUT = VIEWS_IN plus a 'today' map {uri: id}; reports coverage.
"""
import json, pathlib, sys, zipfile

HERE = pathlib.Path(__file__).resolve().parent
probe = {'__name__': 'layout_probe'}
exec(compile((HERE.parent / 'layout_probe.py').read_text(), 'layout_probe.py', 'exec'), probe)


def main():
    explicit, today, vin, vout = sys.argv[1:5]
    ts = json.loads(zipfile.ZipFile(explicit).read('tileset.json'))
    coords, pid = {}, [0]

    def walk(t, level, xyz, dims):
        coords[(level, xyz)] = pid[0]; pid[0] += 1
        for i, c in enumerate(t.get('children', [])):
            s = c.get('extras', {}).get('implicitChildIndex', i)
            walk(c, level + 1, tuple((xyz[k] << 1) | ((s >> k) & 1) for k in range(dims)), dims)
    z = zipfile.ZipFile(today)
    top, tiles, dims, _ = probe['expand'](z)
    walk(ts['root'], 0, (0,) * dims, dims)
    mapping, missing = {}, 0
    for key, t in tiles.items():
        if key not in coords:
            missing += 1
            continue
        for _, uri in t['uris']:
            mapping[uri] = str(coords[key])
    doc = json.loads(pathlib.Path(vin).read_text())
    doc['today'] = mapping
    pathlib.Path(vout).write_text(json.dumps(doc))
    print(json.dumps({'todayTiles': len(tiles), 'explicitTiles': len(coords), 'unmatched': missing, 'mappedUris': len(mapping)}))


if __name__ == '__main__':
    main()
