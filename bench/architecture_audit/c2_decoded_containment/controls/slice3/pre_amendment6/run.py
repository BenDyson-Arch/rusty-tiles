#!/usr/bin/env python3
"""Write the I1 controls exactly as they were before amendment 6, for the S3-19 controls.

    python3 -I pre_amendment6/run.py OUT_DIR

`generate.py` and `fixturelib.py` here are unmodified copies of
`bench/architecture_audit/implicit_read/witnesses/` at 92e8c041 (git blobs 89e21b89 and 71a614b5).
This runs that generator with its output root set to OUT_DIR, so it writes
OUT_DIR/controls/fixtures/*.3tz and OUT_DIR/controls/manifest.json; its pinned inputs are still read
from the repository. `controls/slice3/generate.py --pre-amendment OUT_DIR/controls` then checks each
S3-19 fixture against its `oldSha256` in `implicit_read/controls/amendment6.json`.
"""
import importlib.util
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, *[".."] * 6))


def main(argv):
    if len(argv) != 1:
        raise SystemExit(__doc__)
    spec = importlib.util.spec_from_file_location("i1_generate_92e8c041", os.path.join(HERE, "generate.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)  # puts HERE first on sys.path, so the frozen fixturelib is the one imported
    module.ROOT = os.path.abspath(argv[0])
    module.REPO = REPO
    return module.main([])


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
