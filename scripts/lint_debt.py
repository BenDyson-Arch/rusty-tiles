#!/usr/bin/env python3
"""Hold lint debt in place: a file may lose findings, never gain them (stdlib only).

    python3 scripts/lint_debt.py             run clippy and compare with scripts/lint_debt.json
    python3 scripts/lint_debt.py --update    rewrite the baseline from the current counts
    python3 scripts/lint_debt.py --clippy-json FILE   compare saved `--message-format=json` output

Two kinds of debt are counted per file:

- clippy findings for the RATCHET lints in library and binary code (`cfg(test)` code is not
  compiled), built with `--features pure-rust-basis` as CI's lint job is. These lints stay off
  in plain clippy runs; `clippy::disallowed_methods` reports the bans in clippy.toml, keyed
  by the banned path.
- exemption attributes, `#[expect(lint, reason = ...)]` and its `cfg_attr` form, in src/ and
  tests/, keyed `expect(lint)`. An expectation silences its lint, so it is counted here.

Any difference from the baseline fails. A count that grew prints each finding of that file
and lint with clippy's message (for a ban, the reason from clippy.toml): fix the code rather
than adding an exemption. A count that fell asks for `--update`, so the gain is kept.
"""
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
FEATURES = "pure-rust-basis"
RATCHET = (
    "clippy::cast_possible_truncation",
    "clippy::cast_possible_wrap",
    "clippy::cast_sign_loss",
    "clippy::disallowed_methods",
    "clippy::expect_used",
    "clippy::iter_over_hash_type",
    "clippy::panic",
    "clippy::unwrap_used",
)
BANNED = re.compile(r"use of a disallowed method `([^`]+)`")
# `#[expect(a, b, reason = "...")]`, `#![expect(...)]` and `#[cfg_attr(cond, expect(...))]`,
# formatted on one line or several. Lint names run up to `reason`.
EXPECT = re.compile(
    r"#!?\[\s*(?:cfg_attr\((?:[^\[\]]|\[[^\]]*\])*?,\s*)?expect\(\s*((?:[a-z_]+(?:::[a-z_]+)?\s*,\s*)+)reason\b"
)


def clippy_command():
    flags = []
    for lint in RATCHET:
        flags += ["-W", lint]
    return ["cargo", "clippy", "--locked", "--lib", "--bins", "--features", FEATURES,
            "--message-format=json", "--", *flags]


def findings(lines, root=ROOT):
    """Ratchet findings from clippy JSON lines: {(file, key): [rendered, ...]}.

    A finding reported for both the library and a binary is counted once.
    """
    seen = set()
    found = {}
    errors = []
    for line in lines:
        line = line.strip()
        if not line.startswith("{"):
            continue
        record = json.loads(line)
        if record.get("reason") != "compiler-message":
            continue
        message = record["message"]
        code = (message.get("code") or {}).get("code")
        if message["level"] == "error":
            errors.append(message.get("rendered") or message["message"])
            continue
        if code not in RATCHET:
            continue
        spans = [s for s in message["spans"] if s["is_primary"]]
        if not spans:
            continue
        span = spans[0]
        name = span["file_name"]
        if Path(name).is_absolute() or not (root / name).is_file():
            continue
        key = code
        if code == "clippy::disallowed_methods":
            match = BANNED.search(message["message"])
            key = f"{code}({match.group(1) if match else '?'})"
        identity = (name, span["line_start"], span["column_start"], key)
        if identity in seen:
            continue
        seen.add(identity)
        found.setdefault((name, key), []).append(message.get("rendered") or message["message"])
    return found, errors


def exemptions(root=ROOT):
    """Exemption attributes: {(file, "expect(lint)"): [location, ...]}."""
    found = {}
    for top in ("src", "tests"):
        for path in sorted((root / top).rglob("*.rs")):
            text = path.read_text(encoding="utf-8")
            name = path.relative_to(root).as_posix()
            for match in EXPECT.finditer(text):
                line = text.count("\n", 0, match.start()) + 1
                for lint in match.group(1).replace("\n", " ").split(","):
                    lint = lint.strip()
                    if lint:
                        found.setdefault((name, f"expect({lint})"), []).append(f"{name}:{line}")
    return found


def counts(entries):
    table = {}
    for (name, key), items in entries.items():
        table.setdefault(key, {})[name] = len(items)
    return {key: dict(sorted(files.items())) for key, files in sorted(table.items())}


def compare(baseline, current):
    """(grown, fallen): lists of (key, file, before, after)."""
    grown, fallen = [], []
    for key in sorted(set(baseline) | set(current)):
        before_files, after_files = baseline.get(key, {}), current.get(key, {})
        for name in sorted(set(before_files) | set(after_files)):
            before, after = before_files.get(name, 0), after_files.get(name, 0)
            if after > before:
                grown.append((key, name, before, after))
            elif after < before:
                fallen.append((key, name, before, after))
    return grown, fallen


def write_baseline(path, table):
    path.write_text(json.dumps(table, indent=1, sort_keys=True) + "\n")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--update", action="store_true", help="rewrite the baseline")
    parser.add_argument("--clippy-json", type=Path, help="saved clippy JSON instead of a run")
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--baseline", type=Path)
    args = parser.parse_args(argv)
    root = args.root.resolve()
    baseline_path = args.baseline or root / "scripts" / "lint_debt.json"

    if args.clippy_json:
        lines = args.clippy_json.read_text().splitlines()
    else:
        run = subprocess.run(clippy_command(), cwd=root, stdout=subprocess.PIPE, text=True)
        lines = run.stdout.splitlines()
    found, errors = findings(lines, root)
    if errors:
        print("clippy reported errors; fix them before checking lint debt:", file=sys.stderr)
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    entries = {**found, **exemptions(root)}
    current = counts(entries)

    if args.update:
        write_baseline(baseline_path, current)
        total = sum(sum(files.values()) for files in current.values())
        print(f"wrote {baseline_path.relative_to(root)}: {total} findings and exemptions")
        return 0

    baseline = json.loads(baseline_path.read_text()) if baseline_path.exists() else {}
    grown, fallen = compare(baseline, current)
    for key, name, before, after in grown:
        print(f"{name}: {key} grew from {before} to {after}. Fix the new code; an exemption "
              "counts as debt too.", file=sys.stderr)
        for item in entries.get((name, key), []):
            print("    " + item.rstrip().replace("\n", "\n    "), file=sys.stderr)
    if fallen:
        print("Lint debt fell; keep the gain with `python3 scripts/lint_debt.py --update`:",
              file=sys.stderr)
        for key, name, before, after in fallen:
            print(f"    {name}: {key} {before} -> {after}", file=sys.stderr)
    if grown or fallen:
        return 1
    total = sum(sum(files.values()) for files in current.values())
    print(f"lint debt unchanged: {total} findings and exemptions")
    return 0


if __name__ == "__main__":
    sys.exit(main())
