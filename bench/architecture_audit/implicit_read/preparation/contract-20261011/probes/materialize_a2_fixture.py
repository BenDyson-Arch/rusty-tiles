"""Materialize a #151 a2_semantics design fixture as a tileset directory.

Stdlib only; run with `python3 -I`. The fixture's entry document n0.json becomes
tileset.json (refused if any other member references n0.json). Writes only into
the new, empty output directory given as the second argument. Optionally applies
a single named control mutation, for observation of today's validator only.
"""
import base64
import json
import os
import sys


def main():
    fixture, out = sys.argv[1], sys.argv[2]
    mutation = sys.argv[3] if len(sys.argv) > 3 else None
    os.makedirs(out)
    members = {k: base64.b64decode(v) for k, v in json.load(open(fixture))["members_base64"].items()}
    for name, data in members.items():
        if name != "n0.json" and b"n0.json" in data:
            raise SystemExit("n0.json is referenced; cannot rename")
    members["tileset.json"] = members.pop("n0.json")
    # Correction: metadata schema.schema.json requires "id"; the #151 fixtures omit it.
    for name in list(members):
        if name.endswith(".json"):
            doc = json.loads(members[name])
            if isinstance(doc.get("schema"), dict) and "id" not in doc["schema"]:
                doc["schema"]["id"] = "a2"
                members[name] = json.dumps(doc, separators=(",", ":")).encode()
    if mutation == "rename-class":
        # Rename the semantic class to the producer's private name: semantics unchanged.
        for name in list(members):
            if name.endswith(".json"):
                members[name] = members[name].replace(b'"a2Tile"', b'"rustyTile"')
            if name.endswith(".subtree"):
                data = members[name]
                jlen = int.from_bytes(data[8:16], "little")
                js = data[24:24 + jlen].replace(b'"a2Tile"', b'"rustyTile"')
                js = js + b" " * (jlen - len(js))
                members[name] = data[:24] + js + data[24 + jlen:]
    for name, data in members.items():
        path = os.path.join(out, name)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "xb") as f:
            f.write(data)
    print(json.dumps(sorted(members)))


if __name__ == "__main__":
    main()
