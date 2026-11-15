#!/usr/bin/env python3
"""Hold lint debt in place: each lint's total may fall, never grow (stdlib only).

    python3 scripts/lint_debt.py                   compare with scripts/lint_debt.json
    python3 scripts/lint_debt.py --baseline FILE   compare with another baseline (CI: the base branch's)
    python3 scripts/lint_debt.py --update          lower the baseline to the current counts
    python3 scripts/lint_debt.py --update --allow-growth   record growth too (needs review)
    python3 scripts/lint_debt.py --clippy-json FILE ...    use saved `--message-format=json` output

Debt is counted per file and kept per lint:

- clippy findings for the RATCHET lints in library and binary code (`cfg(test)` code is not
  compiled), from one pass per feature in PASSES so that code gated by either feature is seen;
  a finding both passes report is counted once. The native pass needs GDAL and PROJ (see
  CONTRIBUTING.md); LIBSQLITE3_SYS_USE_PKG_CONFIG=1 is set for both. These lints stay off in
  plain clippy runs; `clippy::disallowed_methods` reports the bans in clippy.toml, keyed by
  the banned path.
- exemption attributes, `#[expect(lint, reason = ...)]` and its `cfg_attr` form, in src/ and
  tests/, keyed `expect(lint)`. An expectation silences its lint, so it is counted here.

The check fails when a lint's total grows, and prints that lint's findings in each file that
gained some: fix the code rather than adding an exemption. Growth in one file that another
file's fall offsets (moved code) is a warning. A fall passes with a notice; `--update` keeps
it. `--update` never raises a lint's total unless `--allow-growth` is given, and CI compares
with the base branch's baseline, so a pull request cannot raise its own ceiling.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
PASSES = ("pure-rust-basis", "native-geospatial")
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


def clippy_command(features):
    flags = []
    for lint in RATCHET:
        flags += ["-W", lint]
    return ["cargo", "clippy", "--locked", "--lib", "--bins", "--features", features,
            "--message-format=json", "--", *flags]


def run_clippy(root):
    """JSON lines from one clippy pass per feature."""
    env = {**os.environ, "LIBSQLITE3_SYS_USE_PKG_CONFIG": "1"}
    lines = []
    for features in PASSES:
        run = subprocess.run(clippy_command(features), cwd=root, env=env,
                             stdout=subprocess.PIPE, text=True)
        lines += run.stdout.splitlines()
        if run.returncode != 0:
            lines.append(json.dumps({"reason": "compiler-message", "message": {
                "level": "error", "message": f"cargo clippy --features {features} failed",
                "code": None, "spans": []}}))
    return lines


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


def totals(table):
    return {key: sum(files.values()) for key, files in table.items()}


def compare(baseline, current):
    """Per lint: (grown, moved, fallen).

    grown: lints whose total grew, with the files that gained; moved: files that gained in a
    lint whose total did not grow; fallen: lints whose total fell. Files are (file, before, after).
    """
    before_totals, after_totals = totals(baseline), totals(current)
    grown, moved, fallen = [], [], []
    for key in sorted(set(baseline) | set(current)):
        before_files, after_files = baseline.get(key, {}), current.get(key, {})
        gained = [(name, before_files.get(name, 0), after_files.get(name, 0))
                  for name in sorted(set(before_files) | set(after_files))
                  if after_files.get(name, 0) > before_files.get(name, 0)]
        before, after = before_totals.get(key, 0), after_totals.get(key, 0)
        if after > before:
            grown.append((key, before, after, gained))
        else:
            moved += [(key, *entry) for entry in gained]
            if after < before:
                fallen.append((key, before, after))
    return grown, moved, fallen


def lowered(baseline, current, allow_growth=False):
    """The baseline after --update: each lint takes the current per-file counts unless its
    total grew, which keeps the old entry (or takes the new one with allow_growth)."""
    before_totals, after_totals = totals(baseline), totals(current)
    table = {}
    for key in sorted(set(baseline) | set(current)):
        if allow_growth or after_totals.get(key, 0) <= before_totals.get(key, 0):
            files = current.get(key, {})
        else:
            files = baseline.get(key, {})
        if files:
            table[key] = dict(sorted(files.items()))
    return table


def write_baseline(path, table):
    path.write_text(json.dumps(table, indent=1, sort_keys=True) + "\n")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--update", action="store_true",
                        help="lower the baseline to the current counts")
    parser.add_argument("--allow-growth", action="store_true",
                        help="with --update, also record lints whose total grew")
    parser.add_argument("--clippy-json", type=Path, action="append",
                        help="saved clippy JSON instead of running the passes (repeatable)")
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--baseline", type=Path, help="default: scripts/lint_debt.json")
    args = parser.parse_args(argv)
    if args.allow_growth and not args.update:
        parser.error("--allow-growth needs --update")
    root = args.root.resolve()
    baseline_path = args.baseline or root / "scripts" / "lint_debt.json"

    if args.clippy_json:
        lines = [line for path in args.clippy_json for line in path.read_text().splitlines()]
    else:
        lines = run_clippy(root)
    found, errors = findings(lines, root)
    if errors:
        print("clippy reported errors; fix them before checking lint debt:", file=sys.stderr)
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    entries = {**found, **exemptions(root)}
    current = counts(entries)
    baseline = json.loads(baseline_path.read_text()) if baseline_path.exists() else {}

    if args.update:
        table = lowered(baseline, current, args.allow_growth)
        write_baseline(baseline_path, table)
        kept = [key for key in table if totals(table)[key] < totals(current).get(key, 0)]
        kept += [key for key in current if key not in table]
        for key in sorted(kept):
            print(f"not recorded: {key} grew to {totals(current)[key]}; fix the code, or rerun "
                  "with --allow-growth for an intended increase", file=sys.stderr)
        total = sum(totals(table).values())
        print(f"wrote {baseline_path}: {total} findings and exemptions")
        return 0

    grown, moved, fallen = compare(baseline, current)
    for key, before, after, gained in grown:
        print(f"{key} grew from {before} to {after}. Fix the new code; an exemption counts "
              "as debt too.", file=sys.stderr)
        for name, file_before, file_after in gained:
            print(f"  {name}: {file_before} -> {file_after}", file=sys.stderr)
            for item in entries.get((name, key), []):
                print("    " + item.rstrip().replace("\n", "\n    "), file=sys.stderr)
    if moved:
        print("warning: these files gained debt that falls elsewhere offset (moved code?):",
              file=sys.stderr)
        for key, name, before, after in moved:
            print(f"    {name}: {key} {before} -> {after}", file=sys.stderr)
    if fallen:
        print("Lint debt fell; `python3 scripts/lint_debt.py --update` lowers the baseline:")
        for key, before, after in fallen:
            print(f"    {key} {before} -> {after}")
    if grown:
        return 1
    total = sum(totals(current).values())
    print(f"lint debt within the baseline: {total} findings and exemptions")
    return 0


if __name__ == "__main__":
    sys.exit(main())
