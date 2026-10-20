"""VS-B Python check: Python's report equals the CLI's (limits with effective budgets, no
liveDocumentBytes, notInspectedTruncated only when set), for list and lazy payloads."""
import json, subprocess, sys
import rusty_tiles
cli = sys.argv[1]
for archive in sys.argv[2:]:
    expected = json.loads(subprocess.run([cli, "--json", "validate", archive], capture_output=True, check=True).stdout)
    for lazy in (False, True):
        report = rusty_tiles.validate(archive, lazy_payloads=lazy)
        rows = list(report["payloads"])
        rest = {k: v for k, v in report.items() if k != "payloads"}
        assert rest == {k: v for k, v in expected.items() if k != "payloads"}, archive
        assert rows == expected["payloads"]
        assert "liveDocumentBytes" not in report["limits"]
    print(json.dumps({"archive": archive.rsplit("/", 1)[-1], "rows": len(rows),
                      "hierarchyVisits": report["limits"]["hierarchyVisits"],
                      "truncated": report.get("notInspectedTruncated")}))
