#!/usr/bin/env python3
"""Decide whether a pull request runs the full CI tier (stdlib only).

    ci_scope.py full COMPARE_JSON [--label NAME ...] [--base BRANCH]

COMPARE_JSON is GitHub's compare response for the pull request (base...head), which ci.yml's
change-scope job saves. Prints `full=true` or `full=false` and, on stderr, why. The full tier
runs when:

- the pull request targets `main` (the release branch, where `Full matrix` is required);
- it carries the `full-ci` label;
- a changed path (or a renamed file's old path) matches ci/full-tier-paths.txt (Cargo.toml,
  Cargo.lock and build.rs at any depth, among others);
- a changed Rust file contains platform-conditional code (cfg(windows),
  cfg!(target_os = ...) and similar) anywhere in the checkout, so an edit inside an existing
  platform block counts; or its diff adds or removes such a line (a removed file, or code
  moved out of the file); or its diff is not shown;
- the comparison is unreadable or lists 300 files, GitHub's cap (any doubt runs it).
"""
import argparse
import json
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
LABEL = "full-ci"
COMPARE_FILE_CAP = 300
# A cfg attribute or cfg! macro naming a platform predicate on the same line.
PLATFORM_CFG = re.compile(r"cfg!?\s*\([^\n]*\b(?:windows|unix|target_os|target_family|target_env|target_vendor)\b")


def load_patterns(path=ROOT / "ci" / "full-tier-paths.txt"):
    lines = path.read_text().splitlines()
    return [re.compile(line) for line in lines if line.strip() and not line.lstrip().startswith("#")]


def changes_platform_code(patch):
    for line in patch.splitlines():
        if line.startswith(("+++", "---")) or not line.startswith(("+", "-")):
            continue
        if PLATFORM_CFG.search(line):
            return True
    return False


def has_platform_code(path):
    try:
        return PLATFORM_CFG.search(path.read_text(encoding="utf-8", errors="replace")) is not None
    except OSError:
        return False


def reasons(compare, labels, base, patterns=None, root=ROOT):
    """Why the full tier must run (empty: the fast tier is enough).

    root is the checkout of the pull request's merge commit, where changed files are read.
    """
    patterns = load_patterns() if patterns is None else patterns
    found = []
    if base == "main":
        found.append("the pull request targets main")
    if LABEL in labels:
        found.append(f"the pull request has the {LABEL} label")
    files = compare.get("files") if isinstance(compare, dict) else None
    if not isinstance(files, list) or not files or len(files) >= COMPARE_FILE_CAP:
        found.append("the changed files could not be listed in full")
        return found
    for entry in files:
        paths = [entry.get("filename", "")]
        if entry.get("previous_filename"):
            paths.append(entry["previous_filename"])
        matched = [path for path in paths if any(pattern.fullmatch(path) for pattern in patterns)]
        if matched:
            found.append(f"{matched[0]} matches ci/full-tier-paths.txt")
        elif any(path.endswith(".rs") for path in paths):
            patch = entry.get("patch")
            if entry.get("status") != "removed" and has_platform_code(root / paths[0]):
                found.append(f"{paths[0]} contains platform-conditional code")
            elif patch is not None and changes_platform_code(patch):
                found.append(f"{paths[0]} changes platform-conditional code")
            elif patch is None and entry.get("status") != "renamed":
                found.append(f"{paths[0]} changed and its diff is not shown")
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    full = commands.add_parser("full")
    full.add_argument("compare", type=Path)
    full.add_argument("--label", action="append", default=[])
    full.add_argument("--base", default="")
    full.add_argument("--root", type=Path, default=ROOT, help="Checkout to read changed files from")
    args = parser.parse_args()
    try:
        compare = json.loads(args.compare.read_text())
    except (OSError, ValueError):
        compare = None
    found = reasons(compare, set(args.label), args.base, root=args.root)
    for reason in found:
        print(f"Full tier: {reason}", file=sys.stderr)
    if not found:
        print("Fast tier: no build, platform or CI change and no full-ci label", file=sys.stderr)
    print(f"full={'true' if found else 'false'}")


if __name__ == "__main__":
    main()
