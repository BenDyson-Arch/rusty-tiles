#!/usr/bin/env python3
"""Check that workflows and the Dockerfile use the values in ci/versions.env (stdlib only).

ci/versions.env is the one place that sets CI tool versions and job time limits. GitHub cannot
read a file for a job's `timeout-minutes` or an action's inputs, so workflows write those as
literals and end each such line with `# versions: KEY [KEY ...]`. This check fails when:

- a marked line names an unknown key, or does not contain that key's value as a token;
- a job that runs steps (not a reusable-workflow call) has no `timeout-minutes`, or a
  `timeout-minutes` line has no marker;
- a pinned tool version (maturin, Node, Python, cargo-nextest) is written without a marker;
- a Dockerfile `ARG` named like a key has a different default.

`python3 scripts/check_ci_config.py` checks the repository; `--root` checks another tree.
"""
import argparse
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
MARKER = re.compile(r"#\s*versions:\s*([A-Z0-9_ ]+?)\s*$")
# Literal pins that must say where their value comes from.
PINNED = re.compile(r"(maturin-version:|node-version:|python-version:\s*'?[0-9]|maturin==|cargo-nextest@|tool:\s*cargo-nextest)")
KEY_LINE = re.compile(r"^([A-Z][A-Z0-9_]*)=(\S+)$")


def load_versions(path):
    values = {}
    for number, line in enumerate(path.read_text().splitlines(), 1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        match = KEY_LINE.match(line)
        if match is None:
            raise SystemExit(f"{path}:{number}: expected KEY=value, got {line!r}")
        if match.group(1) in values:
            raise SystemExit(f"{path}:{number}: duplicate key {match.group(1)}")
        values[match.group(1)] = match.group(2)
    return values


def contains_token(text, value):
    return re.search(r"(?<![0-9A-Za-z.])" + re.escape(value) + r"(?![0-9A-Za-z])", text) is not None


def check_lines(path, lines, values):
    errors = []
    for number, line in enumerate(lines, 1):
        where = f"{path}:{number}"
        marker = MARKER.search(line)
        if marker:
            body = line[:marker.start()]
            for key in marker.group(1).split():
                if key not in values:
                    errors.append(f"{where}: unknown key {key} in ci/versions.env marker")
                elif not contains_token(body, values[key]):
                    errors.append(f"{where}: expected {key}={values[key]} on this line")
        elif re.match(r"\s*timeout-minutes:", line):
            errors.append(f"{where}: timeout-minutes needs a '# versions: TIMEOUT_...' marker")
        elif PINNED.search(line) and "${{" not in line:
            errors.append(f"{where}: pinned version needs a '# versions: KEY' marker")
    return errors


def check_jobs(path, lines):
    """Every job that runs steps has a time limit (a job calling a workflow cannot have one)."""
    errors, job, has_limit, calls, in_jobs = [], None, False, False, False

    def finish():
        if job and not has_limit and not calls:
            errors.append(f"{path}: job {job} has no timeout-minutes")

    for line in lines:
        if re.match(r"^jobs:\s*$", line):
            in_jobs = True
            continue
        if not in_jobs:
            continue
        if re.match(r"^\S", line):
            finish()
            job, in_jobs = None, False
            continue
        named = re.match(r"^  ([A-Za-z0-9_-]+):\s*$", line)
        if named:
            finish()
            job, has_limit, calls = named.group(1), False, False
        elif re.match(r"^    timeout-minutes:", line):
            has_limit = True
        elif re.match(r"^    uses:", line):
            calls = True
    finish()
    return errors


def check_dockerfile(path, name, values):
    errors = []
    for number, line in enumerate(path.read_text().splitlines(), 1):
        match = re.match(r"^ARG ([A-Z0-9_]+)=(\S*)$", line)
        if match and match.group(1) in values and match.group(2) != values[match.group(1)]:
            errors.append(f"{name}:{number}: ARG {match.group(1)} should default to {values[match.group(1)]}")
        for checksum in re.findall(r"--checksum=sha256:([0-9a-f]{64})", line):
            if checksum not in values.values():
                errors.append(f"{name}:{number}: checksum {checksum} is not in ci/versions.env")
    return errors


def check(root):
    values = load_versions(root / "ci" / "versions.env")
    errors = []
    files = sorted((root / ".github" / "workflows").glob("*.yml"))
    files += sorted((root / ".github" / "actions").glob("*/action.yml"))
    for path in files:
        lines = path.read_text().splitlines()
        relative = path.relative_to(root)
        errors += check_lines(relative, lines, values)
        if path.parent.name == "workflows":
            errors += check_jobs(relative, lines)
    dockerfile = root / "Dockerfile"
    if dockerfile.exists():
        errors += check_dockerfile(dockerfile, dockerfile.relative_to(root), values)
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--root", type=Path, default=ROOT)
    args = parser.parse_args()
    errors = check(args.root.resolve())
    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    if errors:
        return 1
    print("CI configuration matches ci/versions.env")
    return 0


if __name__ == "__main__":
    sys.exit(main())
