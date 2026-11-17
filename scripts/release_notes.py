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
body longer than GitHub accepts is cut at a line, closes a code fence the cut leaves open,
says so, and keeps the full-changelog link. Headings inside ``` or ~~~ fences do not start or
end a section. A summary file must close its fences and stay under 20,000 characters.
"""
import argparse
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
# GitHub refuses a release body over 125,000 characters; stay below it with some margin.
BODY_LIMIT = 120_000
# A hand-written summary is a summary; this leaves most of the body to the changelog.
SUMMARY_LIMIT = 20_000
# A CommonMark fence: three or more backticks or tildes, indented at most three spaces.
FENCE = re.compile(r" {0,3}(`{3,}|~{3,})(.*)")


class NotesError(Exception):
    pass


class MissingSection(NotesError):
    pass


def fences(lines):
    """Per line, whether it is outside every fenced code block, and the fence still open at
    the end (None if all are closed). Fence lines themselves count as inside. A fence closes
    on a run of the same character, at least as long, with nothing after it."""
    outside, fence = [], None
    for line in lines:
        match = FENCE.match(line)
        if fence is None:
            if match and not (match.group(1)[0] == "`" and "`" in match.group(2)):
                fence = match.group(1)
                outside.append(False)
            else:
                outside.append(True)
        else:
            if (match and match.group(1)[0] == fence[0] and len(match.group(1)) >= len(fence)
                    and not match.group(2).strip()):
                fence = None
            outside.append(False)
    return outside, fence


def section(changelog, version):
    """(heading, body) of the `## VERSION` block; NotesError if missing or empty.

    Headings inside fenced code blocks are text, not section boundaries.
    """
    lines = changelog.replace("\r\n", "\n").split("\n")
    outside, _ = fences(lines)
    heading = re.compile(r"## " + re.escape(version) + r"(?: - .*)?\s*")
    starts = [i for i, line in enumerate(lines) if outside[i] and heading.fullmatch(line)]
    if not starts:
        raise MissingSection(f"CHANGELOG.md has no '## {version}' section")
    if len(starts) > 1:
        raise NotesError(f"CHANGELOG.md has {len(starts)} '## {version}' sections")
    start = starts[0]
    end = next((i for i in range(start + 1, len(lines)) if outside[i] and lines[i].startswith("## ")),
               len(lines))
    body = "\n".join(lines[start + 1:end]).strip("\n")
    if not any(line.strip() and not (inside_ok and line.lstrip().startswith("#"))
               for line, inside_ok in zip(lines[start + 1:end], outside[start + 1:end])):
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
        name = summary_path.relative_to(root)
        if not summary:
            raise NotesError(f"{name} is empty; delete it or write the summary")
        if len(summary) > SUMMARY_LIMIT:
            raise NotesError(f"{name} has {len(summary)} characters; a summary has at most {SUMMARY_LIMIT}")
        if fences(summary.split("\n"))[1] is not None:
            raise NotesError(f"{name} leaves a code fence open")
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
    # Room for the closing fence a cut inside a code block needs.
    room = BODY_LIMIT - sum(len(part) + 2 for part in parts) - len(links) - len(cut) - 64
    if len(body) > room:
        kept = body[:max(room, 0)].rsplit("\n", 1)[0].rstrip()
        _, open_fence = fences(kept.split("\n"))
        if open_fence is not None:
            kept += "\n" + open_fence
        body = kept + "\n\n" + cut
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
