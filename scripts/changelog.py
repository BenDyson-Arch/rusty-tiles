#!/usr/bin/env python3
"""Changelog fragments: one file per PR in changelog.d/, merged at release.

  changelog.py check      validate every fragment; exit 1 on any error
  changelog.py assemble   merge the fragments into the top Unreleased section
                          of CHANGELOG.md, then delete them (idempotent)

See changelog.d/README.md for the fragment format. Python 3 standard library only.
"""
import argparse
import re
import sys
from pathlib import Path

SECTIONS = ("Added", "Changed", "Deprecated", "Removed", "Fixed", "Security")
NAME = re.compile(r"[a-z0-9][a-z0-9._-]*\.md")
HEADING = re.compile(r"#{1,6}(\s|$)")


class FragmentError(Exception):
    pass


def fragment_paths(directory):
    if not directory.is_dir():
        return []
    return sorted(p for p in directory.iterdir() if p.name != "README.md" and not p.name.startswith("."))


def parse_fragment(path):
    """Return {section: [entry line, ...]} in file order; raise FragmentError."""
    errors = []
    if not NAME.fullmatch(path.name):
        errors.append("name must be lowercase <short-slug>.md (a-z, 0-9, '.', '_', '-')")
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as exc:
        raise FragmentError(f"{path.name}: unreadable: {exc}")
    sections = {}
    current = None
    for number, raw in enumerate(text.splitlines(), 1):
        line = raw.rstrip()
        where = f"line {number}"
        if line.startswith("### "):
            title = line[4:].strip()
            if title not in SECTIONS:
                errors.append(f"{where}: unknown section {title!r} (known: {', '.join(SECTIONS)})")
                current = None
            else:
                current = sections.setdefault(title, [])
        elif HEADING.match(line):
            errors.append(f"{where}: only '### <Section>' headings are allowed")
        elif not line:
            if current:
                current.append("")
        elif current is None:
            errors.append(f"{where}: entry text before a '### <Section>' heading")
        else:
            if line.startswith("- ") and not line[2:].strip():
                errors.append(f"{where}: empty bullet")
            elif not line.startswith(("- ", " ")) and not any(current):
                errors.append(f"{where}: an entry must start with '- '")
            current.append(line)
    for title, lines in sections.items():
        while lines and not lines[-1]:
            lines.pop()
        if not any(line.startswith("- ") for line in lines):
            errors.append(f"section {title!r} has no '- ' entry")
    if not sections and not errors:
        errors.append("empty fragment: add '### <Section>' and at least one '- ' entry")
    if errors:
        raise FragmentError("\n".join(f"{path.name}: {e}" for e in errors))
    return sections


def unreleased_bounds(lines):
    """Return (start, end) of the top '## ' block, which must be Unreleased."""
    heads = [i for i, line in enumerate(lines) if line.startswith("## ")]
    if not heads or "unreleased" not in lines[heads[0]].lower():
        raise FragmentError("CHANGELOG.md: the first '## ' section must be Unreleased")
    end = heads[1] if len(heads) > 1 else len(lines)
    return heads[0], end


def merge(changelog_text, parsed):
    """Return changelog_text with every fragment's entries merged in."""
    lines = changelog_text.split("\n")
    for sections in parsed:
        for title, entries in sections.items():
            start, end = unreleased_bounds(lines)
            heads = [(i, lines[i][4:].strip()) for i in range(start + 1, end) if lines[i].startswith("### ")]
            same = [i for i, name in heads if name == title]
            if same:
                at = same[-1]
                stop = next((i for i, _ in heads if i > at), end)
                while lines[stop - 1].strip() == "":
                    stop -= 1
                lines[stop:stop] = entries
            else:
                rank = SECTIONS.index(title)
                later = [i for i, name in heads if name in SECTIONS and SECTIONS.index(name) > rank]
                at = later[0] if later else end
                if not later:
                    while lines[at - 1].strip() == "":
                        at -= 1
                    block = [""] + [f"### {title}", ""] + entries
                else:
                    block = [f"### {title}", ""] + entries + [""]
                lines[at:at] = block
    return "\n".join(lines)


def check(root):
    errors = []
    paths = fragment_paths(root / "changelog.d")
    for path in paths:
        try:
            parse_fragment(path)
        except FragmentError as exc:
            errors.append(str(exc))
    for error in errors:
        print(error, file=sys.stderr)
    print(f"changelog: {len(paths)} fragment(s), {len(errors)} with errors")
    return 1 if errors else 0


def assemble(root):
    paths = fragment_paths(root / "changelog.d")
    if not paths:
        print("changelog: no fragments")
        return 0
    try:
        parsed = [parse_fragment(p) for p in paths]
        changelog = root / "CHANGELOG.md"
        merged = merge(changelog.read_text(encoding="utf-8"), parsed)
    except FragmentError as exc:
        print(exc, file=sys.stderr)
        return 1
    changelog.write_text(merged, encoding="utf-8")
    for path in paths:
        path.unlink()
    print(f"changelog: assembled {len(paths)} fragment(s)")
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("mode", choices=("check", "assemble"))
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    args = parser.parse_args(argv)
    return (check if args.mode == "check" else assemble)(args.root)


if __name__ == "__main__":
    sys.exit(main())
