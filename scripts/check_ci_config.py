#!/usr/bin/env python3
"""Check that workflows and the Dockerfile use the values in ci/versions.env (stdlib only).

ci/versions.env is the one place that sets CI tool versions and job time limits. GitHub cannot
read a file for a job's `timeout-minutes` or an action's inputs, so workflows write those as
literals and end each such line with `# versions: KEY [KEY ...]`. This check fails when:

- a marked line names an unknown key, or does not contain that key's value as a token; a
  line marked with several keys must hold exactly that many literals (numbers or versions,
  outside quoted strings in a ${{ }} expression), the i-th equal to the i-th key's value,
  so `${{ cond && 120 || 90 }} # versions: A B` binds A to 120 and B to 90;
- a job that runs steps (not a reusable-workflow call) has no `timeout-minutes`, or a
  `timeout-minutes` line has no marker;
- a pinned tool version (maturin, Node, Python, cargo-nextest) is written without a marker;
- a workflow, a composite action or scripts/ci_run.py names a target in UNSUPPORTED_TARGETS
  or a runner label in UNSUPPORTED_RUNNERS outside a comment line (0.4.1 drops Intel macOS);
- a Dockerfile `ARG` named like a key has a different default, an `ADD --checksum` is not a
  listed SHA-256, or `GEOBASE` is neither `geospatial-source` nor the base image of the
  listed GDAL and PROJ versions pinned by digest.

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


LITERAL = re.compile(r"(?<![\w.-])\d+(?:\.\d+)*(?![\w.])")


def literals(body):
    """The number and version literals of a marked line's value, in order.

    In a ${{ }} expression, quoted strings (such as 'refs/heads/main') are left out; in a
    plain value, quoted literals count (python: ['3.10', '3.14']).
    """
    value = re.sub(r"^\s*(?:-\s+)?[\w.-]+:\s*", "", body, count=1)
    expression = re.search(r"\$\{\{(.*)\}\}", value)
    if expression:
        value = re.sub(r"'[^']*'", "''", expression.group(1))
    return LITERAL.findall(value)


def check_lines(path, lines, values):
    errors = []
    for number, line in enumerate(lines, 1):
        where = f"{path}:{number}"
        marker = MARKER.search(line)
        if marker:
            body = line[:marker.start()]
            keys = marker.group(1).split()
            unknown = [key for key in keys if key not in values]
            for key in unknown:
                errors.append(f"{where}: unknown key {key} in ci/versions.env marker")
            if unknown:
                continue
            if len(keys) == 1:
                if not contains_token(body, values[keys[0]]):
                    errors.append(f"{where}: expected {keys[0]}={values[keys[0]]} on this line")
                continue
            found = literals(body)
            if len(found) != len(keys):
                errors.append(f"{where}: {len(keys)} keys but {len(found)} literals ({', '.join(found) or 'none'})")
                continue
            for key, literal in zip(keys, found):
                if literal != values[key]:
                    errors.append(f"{where}: expected {key}={values[key]} in its position, found {literal}")
        elif re.match(r"\s*timeout-minutes:", line):
            errors.append(f"{where}: timeout-minutes needs a '# versions: TIMEOUT_...' marker")
        elif PINNED.search(line) and "${{" not in line:
            errors.append(f"{where}: pinned version needs a '# versions: KEY' marker")
    return errors


def check_unsupported(path, lines, values):
    """Dropped targets and runners must not come back (comment lines may name them)."""
    names = [name for key in ("UNSUPPORTED_TARGETS", "UNSUPPORTED_RUNNERS")
             for name in values.get(key, "").split(",") if name]
    errors = []
    for number, line in enumerate(lines, 1):
        if line.lstrip().startswith("#"):
            continue
        for name in names:
            if contains_token(line, name):
                errors.append(f"{path}:{number}: {name} is not supported (ci/versions.env UNSUPPORTED_*)")
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
        if match and match.group(1) == "GEOBASE" and match.group(2) != "geospatial-source":
            expected = "{}:{}-{}".format(values.get("GEOBASE_REPOSITORY"), values.get("GDAL_VERSION"),
                                         values.get("PROJ_VERSION"))
            if re.fullmatch(re.escape(expected) + r"@sha256:[0-9a-f]{64}", match.group(2)) is None:
                errors.append(f"{name}:{number}: GEOBASE must be geospatial-source or {expected}@sha256:<digest>")
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
        errors += check_unsupported(relative, lines, values)
        if path.parent.name == "workflows":
            errors += check_jobs(relative, lines)
    reuse = root / "scripts" / "ci_run.py"
    if reuse.exists():
        errors += check_unsupported(reuse.relative_to(root), reuse.read_text().splitlines(), values)
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
