"""Copy a GeoJSON adding a stable top-level feature id = properties.id (as a GPKG fid would provide).
Usage: python3 -I ids_inputs.py IN OUT"""
import json, sys
d = json.load(open(sys.argv[1]))
for f in d['features']:
    f['id'] = f['properties']['id']
json.dump(d, open(sys.argv[2], 'w'))
