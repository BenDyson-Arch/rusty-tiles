"""Add an 'explicit' {uri: preorder id} map (the --explicit archive of the same hierarchy) to a views file."""
import json, pathlib, sys, zipfile
ts = json.loads(zipfile.ZipFile(sys.argv[1]).read('tileset.json'))
mapping, pid = {}, [0]
def walk(t):
    c = t.get('content')
    if c:
        mapping[c['uri']] = str(pid[0])
    pid[0] += 1
    for ch in t.get('children', []):
        walk(ch)
walk(ts['root'])
doc = json.loads(pathlib.Path(sys.argv[2]).read_text()); doc['explicit'] = mapping
pathlib.Path(sys.argv[3]).write_text(json.dumps(doc)); print(len(mapping), pid[0])
