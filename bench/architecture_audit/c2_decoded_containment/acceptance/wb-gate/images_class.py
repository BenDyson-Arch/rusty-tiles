#!/usr/bin/env python3
"""R's images-by-URI class (C2-WB OD-WB-7): O-WB's extbuf-20000 matrix fixture with every buffer
except b0 moved to an image entry by URI (same members; C1 reads, never decodes, them).

    python3 -I images_class.py EXTBUF_20000.3tz OUT.3tz
"""
import importlib.util
import json
import sys
import zipfile

spec = importlib.util.spec_from_file_location("wbgen", "/tmp/rusty-tiles-c2wb-contract-20261012/probes/wbgen.py")
G = importlib.util.module_from_spec(spec)
spec.loader.exec_module(G)


def main(src, dst):
    with zipfile.ZipFile(src) as z:
        m = {i.filename: z.read(i) for i in z.infolist() if i.filename != G.INDEX}
    doc = json.loads(m["g.gltf"])
    doc["images"] = [{"uri": b["uri"], "mimeType": "image/png"} for b in doc["buffers"][1:]]
    doc["buffers"] = doc["buffers"][:1]
    m["g.gltf"] = json.dumps(doc, separators=(",", ":"), sort_keys=True).encode()
    G.write_3tz(m, dst)


if __name__ == "__main__":
    main(*sys.argv[1:3])
