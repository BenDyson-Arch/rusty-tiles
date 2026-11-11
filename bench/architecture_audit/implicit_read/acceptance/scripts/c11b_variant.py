"""C-11b variant: the amended fixture with only tileset.json's top-level geometricError raised 400 -> 500, so the
root (500) no longer exceeds it. Members are copied byte for byte otherwise and repackaged by O's write_3tz.
python3 -I c11b_variant.py FIXTURELIB_DIR SRC.3tz OUT.3tz"""
import json, sys, zipfile
sys.path.insert(0, sys.argv[1])
from fixturelib import write_3tz, cjson
z = zipfile.ZipFile(sys.argv[2])
members = {n: z.read(n) for n in z.namelist() if n != "@3dtilesIndex1@"}
doc = json.loads(members["tileset.json"]); assert doc["geometricError"] == 400.0 and doc["root"]["geometricError"] == 500.0
doc["geometricError"] = 500.0
members["tileset.json"] = cjson(doc)
data = write_3tz(members)
open(sys.argv[3], "wb").write(data if isinstance(data, (bytes, bytearray)) else open(data, "rb").read())
