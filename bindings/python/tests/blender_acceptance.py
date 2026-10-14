"""Bootstrap executed by scripts/test_blender_wheel.py inside Blender."""
import os
from pathlib import Path
import runpy
import sys

import bpy


def main():
    phase, root, wheel, pip_root, packages = sys.argv[sys.argv.index("--") + 1:]
    if phase == "install":
        print(f"Installing wheel inside Blender {bpy.app.version_string}, Python {sys.version}", flush=True)
        sys.path.insert(0, pip_root)
        sys.argv = ["pip", "--isolated", "--disable-pip-version-check", "install",
                    "--no-index", "--no-deps", "--no-compile", "--target", packages, wheel]
        try:
            runpy.run_module("pip", run_name="__main__")
        except SystemExit as status:
            if status.code:
                raise RuntimeError(f"Wheel installation failed in Blender: {status.code}") from status
        return
    if phase != "test":
        raise ValueError(f"Unknown acceptance phase: {phase}")
    sys.path.insert(0, packages)
    os.environ["PATH"] = ""
    os.environ.pop("PYTHONPATH", None)
    suite = Path(root) / "bindings/python/tests/test_api.py"
    sys.argv = [str(suite), root]
    try:
        runpy.run_path(str(suite), run_name="__main__")
    except SystemExit as status:
        if status.code:
            raise RuntimeError("Installed wheel acceptance failed inside Blender") from status


if __name__ == "__main__":
    main()
