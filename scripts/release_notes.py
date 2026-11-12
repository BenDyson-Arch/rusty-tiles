#!/usr/bin/env python3
"""Write a GitHub release body from CHANGELOG.md (stdlib only).

    release_notes.py VERSION --tag TAG --repo OWNER/REPO [--previous TAG] [--dry-run]
                     [--root DIR] [--output FILE]

The body is:

1. release-notes/<TAG>.md, if that file exists in the checkout: an optional hand-written
   summary, placed first;
2. the CHANGELOG.md section of VERSION: the `## VERSION` or `## VERSION - DATE` block, without
   its heading, up to the next `## ` heading;
3. a "Full changelog" line linking that section at the tag
   (https://github.com/OWNER/REPO/blob/TAG/CHANGELOG.md#<anchor>) and, with --previous, the
   compare view PREVIOUS...TAG.

It fails when the section is missing, when it has no entry text (headings and blank lines
only), or when the summary file exists but is empty. With --dry-run (a release rehearsal on
a ref that is not a release tag) a missing section is a warning instead, and the body says
which section a tag would need; an empty section still fails. A section that would make the
body longer than GitHub accepts is cut at a line, says so, and keeps the full-changelog link.
"""
import argparse
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
# GitHub refuses a release body over 125,000 characters; stay below it with some margin.
BODY_LIMIT = 120_000


class NotesError(Exception):
    pass


class MissingSection(NotesError):
    pass


def section(changelog, version):
    """(heading, body) of the `## VERSION` block; NotesError if missing or empty."""
    lines = changelog.replace("\r\n", "\n").split("\n")
    heading = re.compile(r"## " + re.escape(version) + r"(?: - .*)?\s*")
    starts = [i for i, line in enumerate(lines) if heading.fullmatch(line)]
    if not starts:
        raise MissingSection(f"CHANGELOG.md has no '## {version}' section")
    if len(starts) > 1:
        raise NotesError(f"CHANGELOG.md has {len(starts)} '## {version}' sections")
    start = starts[0]
    end = next((i for i in range(start + 1, len(lines)) if lines[i].startswith("## ")), len(lines))
    body = "\n".join(lines[start + 1:end]).strip("\n")
    if not any(line.strip() and not line.lstrip().startswith("#") for line in body.split("\n")):
        raise NotesError(f"CHANGELOG.md section '{lines[start]}' has no entries")
    return lines[start][3:].strip(), body


def anchor(heading):
    """GitHub's heading anchor: lower case, punctuation other than '-' and '_' dropped, spaces
    to '-' (so "0.4.0 - 2026-11-11" is "040---2026-11-11")."""
    text = re.sub(r"[^\w\- ]", "", heading.strip().lower())
    return text.replace(" ", "-")


def notes(root, version, tag, repo, previous=None, dry_run=False):
    changelog = (root / "CHANGELOG.md").read_text(encoding="utf-8")
    parts = []
    summary_path = root / "release-notes" / f"{tag}.md"
    if summary_path.exists():
        summary = summary_path.read_text(encoding="utf-8").strip()
        if not summary:
            raise NotesError(f"{summary_path.relative_to(root)} is empty; delete it or write the summary")
        parts.append(summary)
    try:
        heading, body = section(changelog, version)
    except MissingSection as error:
        if not dry_run:
            raise
        print(f"warning: {error}; a {tag} tag would fail here", file=sys.stderr)
        parts.append(f"_Dry run: CHANGELOG.md has no entries for {version} yet; a release tag "
                     f"needs a `## {version} - DATE` section._")
        return "\n\n".join(parts) + "\n"
    server = f"https://github.com/{repo}"
    links = f"**Full changelog**: [{heading}]({server}/blob/{tag}/CHANGELOG.md#{anchor(heading)})"
    if previous:
        links += f" · [{previous}...{tag}]({server}/compare/{previous}...{tag})"
    cut = "_The changelog is shortened here; the full changelog link below has every entry._"
    room = BODY_LIMIT - sum(len(part) + 2 for part in parts) - len(links) - len(cut) - 8
    if len(body) > room:
        body = body[:max(room, 0)].rsplit("\n", 1)[0].rstrip() + "\n\n" + cut
    parts += [body, links]
    return "\n\n".join(parts) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("version")
    parser.add_argument("--tag", required=True)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--previous", default="", help="Previous release tag for the compare link")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    try:
        text = notes(args.root, args.version, args.tag, args.repo, args.previous or None, args.dry_run)
    except NotesError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    if args.output:
        args.output.write_text(text, encoding="utf-8")
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
