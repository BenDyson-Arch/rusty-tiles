import importlib.util, json, sys, zipfile
spec = importlib.util.spec_from_file_location('oracle', sys.argv[1]); oracle = importlib.util.module_from_spec(spec); spec.loader.exec_module(oracle)
src = sys.argv[2]
out = {}
for path in (['maximumTileBytes'], ['encoding', 'encodedTileBytes'], ['encoding', 'primitiveReferences'], ['encoding', 'maximumTilePrimitives']):
    with zipfile.ZipFile(src) as a:
        members = {n: a.read(n) for n in a.namelist()}
    report = json.loads(members['conversion.json'])
    target = report
    for k in path[:-1]: target = target[k]
    target[path[-1]] += 1
    members['conversion.json'] = json.dumps(report).encode()
    tampered = 'tampered.3tz'
    with zipfile.ZipFile(tampered, 'w') as z:
        for n, d in members.items(): z.writestr(n, d)
    try:
        oracle.forest_inventory(tampered); out['.'.join(path)] = 'NOT CAUGHT'
    except AssertionError as e:
        out['.'.join(path)] = 'caught'
print(json.dumps(out))
