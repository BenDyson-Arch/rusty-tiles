#!/usr/bin/env python3
"""Compare two mesh archives: content GLB bytes must be identical as a multiset of SHA-256
digests (names may differ); report member counts and bytes by kind.

usage: content_identity.py BEFORE.3tz AFTER.3tz
Exit 0 when the GLB digest multisets are equal, 1 otherwise. Prints one JSON object.
"""
import collections
import hashlib
import json
import sys
import zipfile


def kinds(path):
    z = zipfile.ZipFile(path)
    digests = collections.Counter()
    count = collections.Counter()
    size = collections.Counter()
    for info in z.infolist():
        name = info.filename
        if name.endswith(".glb"):
            kind = "glb"
            digests[hashlib.sha256(z.read(name)).hexdigest()] += 1
        elif name.endswith(".subtree"):
            kind = "subtree"
        elif name.endswith(".json") and name != "conversion.json":
            kind = "tileset_json"
        else:
            kind = "other"
        count[kind] += 1
        size[kind] += info.file_size
    return digests, dict(count), dict(size)


def main(before, after):
    d0, c0, s0 = kinds(before)
    d1, c1, s1 = kinds(after)
    glb_set = hashlib.sha256("\n".join(sorted(d0.elements())).encode()).hexdigest()
    same = d0 == d1
    print(json.dumps({
        "glb_digests_equal": same,
        "glb_count": [sum(d0.values()), sum(d1.values())],
        "glb_digest_set_sha256": glb_set if same else None,
        "members": [c0, c1],
        "bytes": [s0, s1],
        "only_before": sum((d0 - d1).values()),
        "only_after": sum((d1 - d0).values()),
    }, sort_keys=True))
    return 0 if same else 1


if __name__ == "__main__":
    sys.exit(main(*sys.argv[1:3]))
