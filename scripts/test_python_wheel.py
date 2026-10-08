"""Install one wheel into a clean venv and test it with no executables on PATH."""
import argparse
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import venv

from wheel_acceptance import clear_requested_report, copy_evidence, require_completion


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("wheel", type=Path)
    parser.add_argument("--report-json", type=Path, help="Write runtime and test evidence as JSON")
    args = parser.parse_args()
    requested_report = clear_requested_report(args.report_json, args.wheel)
    wheel = args.wheel.resolve(strict=True)
    wheel_sha256 = hashlib.sha256(wheel.read_bytes()).hexdigest()
    root = Path(__file__).resolve().parents[1]
    with tempfile.TemporaryDirectory(prefix="rusty-tiles-wheel-") as work:
        evidence = Path(work) / "acceptance.json"
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
        env.pop("PYTHONHOME", None)
        env["RUSTY_TILES_ACCEPTANCE_PACKAGE_ROOT"] = str(env_dir)
        env["RUSTY_TILES_ACCEPTANCE_WHEEL_SHA256"] = wheel_sha256
        env["RUSTY_TILES_ACCEPTANCE_REPORT"] = str(evidence)
        try:
            subprocess.run(
                [str(python), "-I", str(root / "bindings/python/tests/test_api.py"), str(root)],
                env=env,
                cwd=work,
                check=True,
                timeout=180,
            )
            require_completion(evidence, wheel_sha256)
        finally:
            copy_evidence(evidence, requested_report)


if __name__ == "__main__":
    main()
