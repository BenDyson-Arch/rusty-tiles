"""Prepare the independent cut audit directly from the published explicit crop archive.
Usage: python unpack.py ARCHIVE OUT BASELINE_LEAF_HASHES OUT_RECEIPT
This deliberately supports the explicit crop domain only; forest traversal is S5's oracle.
"""
import hashlib
import json
import math
import sys
import zipfile
from pathlib import Path

archive, out, baseline, receipt = map(Path, sys.argv[1:])
out.mkdir(parents=True, exist_ok=True)
leaves, proxies, empty = [], [], []
with zipfile.ZipFile(archive) as z:
    def visit(tile):
        assert "implicitTiling" not in tile, "explicit crop oracle only"
        children = tile.get("children", [])
        child_rows = [visit(c) for c in children]
        extra = tile["extras"]
        first = int(extra["first"])
        end = max(r["end"] for r in child_rows) if children else int(extra["end"])
        assert children or "content" in tile, "a terminal leaf needs content"
        row = dict(first=first, end=end, depth=extra["depth"],
                   geometricError=tile["geometricError"])
        assert math.isfinite(row["geometricError"]) and row["geometricError"] >= 0
        if children:
            ordered = sorted(child_rows, key=lambda r: r["first"])
            assert ordered[0]["first"] == first
            assert all(a["end"] == b["first"] for a, b in zip(ordered, ordered[1:]))
            assert all(row["geometricError"] >= r["geometricError"] for r in child_rows)
        if "content" not in tile:
            empty.append(row)
            return row
        uri = tile["content"]["uri"]
        assert uri.endswith(".glb"), "external forests need the S5 oracle"
        data = z.read(uri)
        target = out / uri
        assert target.resolve().is_relative_to(out.resolve())
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        row.update(uri=uri, sha256=hashlib.sha256(data).hexdigest())
        (proxies if children else leaves).append(row)
        return row
    root = visit(json.loads(z.read("tileset.json"))["root"])
    for cell in empty:
        owners = [r for r in proxies if (r['first'], r['end']) == (cell['first'], cell['end'])
                  and r['depth'] < cell['depth']]
        assert owners, "content-less cell needs an ancestor publication of its whole range"
        owner = max(owners, key=lambda r:r['depth'])
        assert cell['geometricError'] == owner['geometricError'], "chain G differs from publication"
    assert len(leaves) + len(proxies) == sum(n.endswith(".glb") for n in z.namelist())
expected = json.loads(baseline.read_text())
actual = sorted(leaves, key=lambda r: r["first"])
assert [r["sha256"] for r in actual] == [v for k, v in sorted(expected["leaf_members"].items())]
for name, rows in [("leaves", leaves), ("proxies", proxies)]:
    (out / (name + ".jsonl")).write_text("".join(json.dumps(r) + "\n" for r in rows))
record = dict(archive_sha256=hashlib.file_digest(archive.open("rb"), "sha256").hexdigest(),
              leaf_count=len(leaves), coarse_count=len(proxies), contentless_cells=len(empty), root=root,
              leaves_byte_identical=True, develop_baseline=expected["develop_baseline"],
              s1_base=expected["s1_base"], monotone=True, archive_member_inventory_complete=True,
              members=leaves+proxies)
receipt.write_text(json.dumps(record, indent=2) + "\n")
print(json.dumps({k: v for k, v in record.items() if k != "members"}))
