#!/usr/bin/env python3
"""C2-WB 9.D precheck-only arm: 1,047,040 tiny references (64 docs x 2,045 tiles x 8 contents) under a
100-document chain (R's probe generator wbgen.refs), and its gltfUpAxis twin.

    python3 -I precheck_arm.py OUTDIR
"""
import importlib.util
import os
import sys

spec = importlib.util.spec_from_file_location("wbgen", "/tmp/rusty-tiles-c2wb-contract-20261012/probes/wbgen.py")
G = importlib.util.module_from_spec(spec)
spec.loader.exec_module(G)

out = sys.argv[1]
m = G.refs(64, 2045, 8, depth=100)
G.write_3tz(m, os.path.join(out, "precheck-refs1047040-d100.3tz"))
G.write_3tz(G.twin(m), os.path.join(out, "precheck-refs1047040-d100-Z.3tz"))
m = G.refs(64, 2045, 8, depth=1)
G.write_3tz(m, os.path.join(out, "precheck-refs1047040-d1.3tz"))
G.write_3tz(G.twin(m), os.path.join(out, "precheck-refs1047040-d1-Z.3tz"))
