#!/usr/bin/env python3
"""Fail when tracked bench/ content outgrows its budget.

Bulk evidence (raw runs, transcripts, candidate corpora) lives outside git in
~/.cache/rusty-tiles-evidence/ and is referenced by SHA-256; see AGENTS.md.
Sizes come from the git index (blob sizes), so the check is independent of the
working tree and needs no history.
"""
import subprocess
import sys

ROOT = "bench/"
FILE_LIMIT = 1_000_000
# About 1.25x the 7,994,390 tracked bytes left after evidence moved out (2026-10-12).
TOTAL_BUDGET = 10_000_000


def git(*args, data=None):
    return subprocess.run(("git",) + args, input=data, capture_output=True, check=True).stdout


def main():
    entries = []
    for record in git("ls-files", "-s", "-z", "--", ROOT).split(b"\0"):
        if record:
            meta, path = record.split(b"\t", 1)
            entries.append((meta.split()[1].decode(), path.decode()))
    sizes = git("cat-file", "--batch-check=%(objectsize)",
                data="".join(f"{oid}\n" for oid, _ in entries).encode()).split()
    files = [(path, int(size)) for (_, path), size in zip(entries, sizes)]
    total = sum(size for _, size in files)
    oversized = [(p, s) for p, s in files if s > FILE_LIMIT]
    print(f"tracked {ROOT}: {len(files)} files, {total:,} bytes; budget {TOTAL_BUDGET:,} bytes "
          f"({100 * total / TOTAL_BUDGET:.1f}%); per-file limit {FILE_LIMIT:,} bytes")
    for path, size in sorted(files, key=lambda f: -f[1])[:5]:
        print(f"  largest: {size:>12,}  {path}")
    failed = False
    for path, size in oversized:
        print(f"error: {path} is {size:,} bytes (> {FILE_LIMIT:,}); keep bulk evidence out of git",
              file=sys.stderr)
        failed = True
    if total > TOTAL_BUDGET:
        print(f"error: tracked {ROOT} is {total:,} bytes, over the {TOTAL_BUDGET:,} byte budget",
              file=sys.stderr)
        failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
