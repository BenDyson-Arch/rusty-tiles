"""Sensitive controls for the independent sparse-chain unpacker.
Usage: python sparse_oracle_control.py SCRATCH OUT_JSON
"""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import zipfile

scratch, receipt = map(Path, sys.argv[1:])
scratch.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory(dir=scratch, prefix="oracle-control-") as tmp:
    p = Path(tmp)
    def leaf(first, end, name):
        return dict(extras=dict(first=first, end=end, depth=4 if first < 4 else 1),
                    geometricError=0, content=dict(uri=name))
    branch = dict(extras=dict(first=0, depth=3), geometricError=1,
                  children=[leaf(0, 2, "leaves/0.glb"), leaf(2, 4, "leaves/1.glb")])
    chain = dict(extras=dict(first=0, depth=2), geometricError=1, children=[branch])
    top = dict(extras=dict(first=0, depth=1), geometricError=1,
               content=dict(uri="proxies/top.glb"), children=[chain])
    root = dict(extras=dict(first=0, depth=0), geometricError=2,
                content=dict(uri="proxies/root.glb"), children=[top, leaf(4, 8, "leaves/2.glb")])
    # The unpacker treats payloads as opaque bytes; decoding is a separate oracle.
    payloads = {f"leaves/{i}.glb": f"leaf{i}".encode() for i in range(3)}
    payloads.update({"proxies/top.glb": b"top", "proxies/root.glb": b"root"})
    baseline = dict(leaf_members={n: hashlib.sha256(v).hexdigest()
                                 for n, v in payloads.items() if n.startswith("leaves/")},
                    develop_baseline="synthetic", s1_base="synthetic")
    (p / "baseline.json").write_text(json.dumps(baseline))
    results = []
    for mutation in [False, True]:
        if mutation:
            branch["geometricError"] = .5
        with zipfile.ZipFile(p / "fixture.3tz", "w") as z:
            z.writestr("tileset.json", json.dumps(dict(root=root)))
            for n, v in payloads.items():
                z.writestr(n, v)
        r = subprocess.run([sys.executable, "-I", str(Path(__file__).with_name("unpack.py")),
                            str(p / "fixture.3tz"), str(p / "out"),
                            str(p / "baseline.json"), str(p / "receipt.json")],
                           capture_output=True, text=True)
        assert (r.returncode == 0) == (not mutation), (mutation, r.stdout, r.stderr)
        if not mutation:
            assert json.loads((p / "receipt.json").read_text())["contentless_cells"] == 2
        results.append(dict(chain_g_mutation=mutation, exit=r.returncode, accepted=r.returncode == 0))
    receipt.write_text(json.dumps(dict(domain="hoisted branching tile and shallow terminal leaf",
                                      results=results), indent=2) + "\n")
