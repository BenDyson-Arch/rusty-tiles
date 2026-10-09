"""Install a wheel into temporary storage using Blender's Python, then test it."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import venv

from wheel_acceptance import clear_requested_report, copy_evidence, failure_reason, require_completion
from download_blender import validate_distribution


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("wheel", type=Path)
    parser.add_argument("--blender", default="blender", help="Blender executable name or path")
    parser.add_argument("--report-json", type=Path, help="Write runtime and test evidence as JSON")
    parser.add_argument("--distribution-json", type=Path,
                        help="Require the pinned official bundle recorded by download_blender.py")
    args = parser.parse_args()
    requested_report = clear_requested_report(args.report_json, args.wheel)
    wheel = args.wheel.resolve(strict=True)
    wheel_sha256 = hashlib.sha256(wheel.read_bytes()).hexdigest()
    blender = shutil.which(args.blender)
    if blender is None:
        parser.error(f"Blender executable not found: {args.blender}")
    distribution = None
    if args.distribution_json is not None:
        distribution = json.loads(args.distribution_json.read_text())
        manifest, spec = validate_distribution(distribution, Path(blender))
    root = Path(__file__).resolve().parents[1]
    with tempfile.TemporaryDirectory(prefix="rusty-tiles-blender-") as work:
        work = Path(work)
        evidence = work / "acceptance.json"
        installer = work / "installer"
        venv.EnvBuilder(with_pip=True).create(installer)
        python = installer / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
        # Provide pip's pure-Python installer to Blender without depending on pip
        # being bundled with Blender. Blender itself evaluates the wheel's tags.
        pip_root = subprocess.check_output(
            [str(python), "-I", "-c", "import pip; from pathlib import Path; print(Path(pip.__file__).parent.parent)"],
            text=True,
        ).strip()
        env = os.environ.copy()
        for name in ("PYTHONPATH", "PYTHONHOME", "PYTHONUSERBASE"):
            env.pop(name, None)
        for name in ("CONFIG", "SCRIPTS", "DATAFILES"):
            directory = work / name.lower()
            directory.mkdir()
            env[f"BLENDER_USER_{name}"] = str(directory)
        env["RUSTY_TILES_ACCEPTANCE_PACKAGE_ROOT"] = str(work / "packages")
        env["RUSTY_TILES_ACCEPTANCE_WHEEL_SHA256"] = wheel_sha256
        env["RUSTY_TILES_ACCEPTANCE_REPORT"] = str(evidence)
        completed = False
        runner_error = "Acceptance did not complete"
        try:
            for phase in ("install", "test"):
                subprocess.run(
                    [str(Path(blender).resolve()), "--background", "--factory-startup", "--disable-autoexec",
                     "--python-exit-code", "1", "--python",
                     str(root / "bindings/python/tests/blender_acceptance.py"), "--", phase,
                     str(root), str(wheel), pip_root, str(work / "packages")],
                    cwd=work,
                    env=env,
                    check=True,
                    timeout=240,
                )
            require_completion(evidence, wheel_sha256, require_blender=True)
            if distribution is not None:
                report = json.loads(evidence.read_text())
                if report.get("blender_version") != [int(part) for part in manifest["version"].split(".")]:
                    raise RuntimeError("The API suite ran a different Blender version from the pinned bundle")
                if report.get("machine", "").lower() not in spec["machines"]:
                    raise RuntimeError("The API suite ran a different architecture from the pinned bundle")
                report["blender_distribution"] = distribution
                evidence.write_text(json.dumps(report, indent=2) + "\n")
            completed = True
        except BaseException as error:
            runner_error = failure_reason(error)
            raise
        finally:
            copy_evidence(evidence, requested_report, runner_error=None if completed else runner_error)


if __name__ == "__main__":
    main()
