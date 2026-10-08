"""Install one wheel into a clean venv and test it with no executables on PATH."""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile
import venv


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("wheel", type=Path)
    args = parser.parse_args()
    wheel = args.wheel.resolve(strict=True)
    root = Path(__file__).resolve().parents[1]
    with tempfile.TemporaryDirectory(prefix="rusty-tiles-wheel-") as work:
        env_dir = Path(work) / "venv"
        venv.EnvBuilder(with_pip=True).create(env_dir)
        python = env_dir / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
        subprocess.run(
            [str(python), "-I", "-m", "pip", "install", "--no-index", "--no-deps", str(wheel)],
            check=True,
        )
        env = os.environ.copy()
        env["PATH"] = ""
        env.pop("PYTHONPATH", None)
        subprocess.run(
            [str(python), "-I", str(root / "bindings/python/tests/test_api.py"), str(root)],
            env=env,
            cwd=work,
            check=True,
            timeout=180,
        )


if __name__ == "__main__":
    main()
