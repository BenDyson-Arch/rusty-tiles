#!/usr/bin/env python3
"""Fail when tracked content outgrows its budget.

Every tracked file must be at most FILE_LIMIT bytes, except the listed upstream
files in vendor/; the tree (without vendor/), bench/ and vendor/ each have a
total budget. Bulk evidence (raw runs, transcripts, candidate
corpora) lives outside git in ~/.cache/rusty-tiles-evidence/ and is referenced
by SHA-256; see AGENTS.md. Sizes come from the git index (blob sizes) of the
repository containing the current directory, so the check covers the whole
tree from any subdirectory, ignores the working tree and needs no history.
"""
import subprocess
import sys

FILE_LIMIT = 1_000_000
# About 1.25x the tracked sizes after evidence moved out (2026-10-12, on develop
# e29cd19, live validator tooling kept): tree 17,243,688 bytes, bench/ 9,423,400.
# The tree budget excludes vendor/, which has its own allowance (below).
BUDGETS = {"": 21_600_000, "bench/": 11_800_000}
# Third-party source vendored for a shipped feature: one pinned upstream version,
# replaced, never accumulated. Basis Universal v2.50 (vendor/README.md) is the
# compiled include closure plus licences, 10,852,575 bytes, about 1.1x of headroom
# for one upstream update.
VENDOR = "vendor/"
VENDOR_BUDGET = 12_000_000
# Upstream files over FILE_LIMIT, allowed by exact path only. Each is compiled
# as one translation unit and cannot be split without modifying upstream.
VENDOR_LARGE_FILES = {
    "vendor/basis_universal/transcoder/basisu_transcoder.cpp",
    "vendor/basis_universal/zstd/zstd.c",
}
GITLINK = "160000"


def git(*args, data=None):
    return subprocess.run(("git",) + args, input=data, capture_output=True, check=True).stdout


def main():
    top = git("rev-parse", "--show-toplevel").decode().rstrip("\n")
    entries = []
    for record in git("-C", top, "ls-files", "-s", "-z", "--", ":(top)").split(b"\0"):
        if record:
            meta, path = record.split(b"\t", 1)
            mode, oid = meta.split()[:2]
            if mode.decode() == GITLINK:
                continue  # submodule commit, not a blob
            entries.append((oid.decode(), path.decode()))
    if not entries:
        print("error: no tracked files found", file=sys.stderr)
        return 1
    sizes = git("-C", top, "cat-file", "--batch-check=%(objectsize)",
                data="".join(f"{oid}\n" for oid, _ in entries).encode()).split()
    files = [(path, int(size)) for (_, path), size in zip(entries, sizes)]
    failed = False
    scopes = [(prefix, budget, lambda path, prefix=prefix: path.startswith(prefix)
               and not path.startswith(VENDOR)) for prefix, budget in BUDGETS.items()]
    scopes.append((VENDOR, VENDOR_BUDGET, lambda path: path.startswith(VENDOR)))
    for prefix, budget, in_scope in scopes:
        scoped = [size for path, size in files if in_scope(path)]
        total = sum(scoped)
        print(f"tracked {prefix or 'tree'}: {len(scoped)} files, {total:,} bytes; "
              f"budget {budget:,} bytes ({100 * total / budget:.1f}%)")
        if total > budget:
            print(f"error: tracked {prefix or 'tree'} is {total:,} bytes, over the {budget:,} byte budget",
                  file=sys.stderr)
            failed = True
    print(f"per-file limit {FILE_LIMIT:,} bytes")
    for path, size in sorted(files, key=lambda f: -f[1])[:5]:
        print(f"  largest: {size:>12,}  {path}")
    for path in sorted(VENDOR_LARGE_FILES - {path for path, _ in files}):
        print(f"error: {path} is allowed over the per-file limit but no longer tracked; "
              "remove it from VENDOR_LARGE_FILES", file=sys.stderr)
        failed = True
    for path, size in files:
        if size > FILE_LIMIT and path not in VENDOR_LARGE_FILES:
            print(f"error: {path} is {size:,} bytes (> {FILE_LIMIT:,}); keep bulk evidence out of git",
                  file=sys.stderr)
            failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except subprocess.CalledProcessError as error:
        sys.exit(f"error: {' '.join(error.cmd)} failed: {error.stderr.decode(errors='replace').strip()}")
