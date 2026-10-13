"""VS-A production mutants (archive catalog at scale), author controls.

Usage (repository root, clean `src/`):
    python3 -I bench/architecture_audit/vs_a_catalog/mutants.py OUT_DIR [name ...]

Each mutant is a set of exact single-occurrence string edits of one source file. The
driver applies one mutant, runs the release archive and validator unit tests (which run
the catalog controls under both the resident and the forced-spill profile), records the
failing tests and restores the file. A mutant is caught when at least one test fails or
the run aborts (a timeout counts as caught). `clean` must fail nothing. Writes
OUT_DIR/report.json.

Environment: CARGO_TARGET_DIR (default ~/.cache/rt-target-c), VSA_LOCK (default
~/.cache/rusty-tiles-cargo-c.lock), VSA_CPUS (default 16-19), VSA_TIMEOUT (default 1800 s).
Scratch for the forced-spill controls goes to TMPDIR; set it away from /tmp.
"""
import json
import os
import re
import subprocess
import sys

READ = "src/archive3tz/read.rs"
CATALOG = "src/archive3tz/catalog.rs"
ROWS = "src/validate/rows.rs"
VALIDATE = "src/validate.rs"
TYPES = "src/validate/types.rs"
MUTANTS = [
    ("clean", []),
    # The fast path admits any index with a valid CRC.
    ("index-skip-diagnosis", [(READ, "        if matched {\n            return Ok(());\n        }",
                               "        if true {\n            return Ok(());\n        }")]),
    # Ordering and membership failures at the same row: membership wins.
    ("index-order-tie", [(READ, "first.is_none_or(|first| unordered <= first)",
                          "first.is_none_or(|first| unordered < first)")]),
    # A repeated row is not a failure.
    ("diagnosis-no-repeats", [(READ, "let failing = if present { second } else { Some(lowest) };",
                               "let failing = if present { None } else { Some(lowest) };")]),
    # An absent pair fails only when repeated.
    ("diagnosis-absent-late", [(READ, "let failing = if present { second } else { Some(lowest) };",
                                "let failing = if present { second } else { second };")]),
    ("duplicates-undetected", [(READ, "duplicate |= a == b;", "duplicate |= false && a == b;")]),
    # Each extent is checked against the directory only, not the next record.
    ("extents-ignore-next", [(READ, "Some(Ok(next)) => next.start,", "Some(Ok(_)) => offset,")]),
    # The fence search starts one page late.
    ("fence-page-late", [(CATALOG, "let Some(page) = below.checked_sub(1) else {",
                          "let Some(page) = below.checked_sub(0) else {")]),
    # The page cache serves whatever page its slot holds.
    ("page-cache-stale", [(CATALOG, ".is_none_or(|(held, _)| *held != page)", ".is_none()")]),
    # The lookup memo answers every name with the last result.
    ("memo-stale", [(CATALOG, "            if held == name {", "            if !held.is_empty() {")]),
    ("closure-shows-seventeen", [(VALIDATE, "const UNREFERENCED_SHOWN: usize = 16;",
                                  "const UNREFERENCED_SHOWN: usize = 17;")]),
    # The closure keeps the largest names.
    ("closure-keeps-largest", [(VALIDATE, "is_some_and(|last| name < *last)",
                                "is_some_and(|last| name > *last)")]),
    # conversion.json is admitted but not marked used.
    ("conversion-unmarked", [(VALIDATE, "        check.resources.used.set(member.rank);\n",
                              "        let _ = member;\n")]),
    # Only the vector-build digest is kept: it is never recorded.
    ("vector-digest-dropped", [(VALIDATE, "            vector_build = Some(hash);",
                                "            let _ = hash;")]),
    # Tied rows take the group's first position instead of their run's.
    ("rows-run-start", [(ROWS, "                run_start = position;", "                run_start = row.group;")]),
    # The streamed report drops the separator between payload rows.
    ("json-row-separator", [(TYPES, "                    if row > 0 {\n                        out.write_all(b\",\")?;",
                             "                    if row > 0 {\n                        out.write_all(b\"\")?;")]),
]
TESTS = ["archive3tz::", "validate::catalog_controls", "validate::tests", "validate::memo_controls"]


def sh(cmd, **kw):
    return subprocess.run(cmd, text=True, capture_output=True, **kw)


def main(argv):
    out = os.path.abspath(argv[0])
    names = set(argv[1:])
    os.makedirs(out, exist_ok=True)
    if sh(["git", "diff", "--quiet", "--", "src"]).returncode:
        sys.exit("src/ must be clean")
    home = os.path.expanduser("~")
    target = os.environ.get("CARGO_TARGET_DIR", f"{home}/.cache/rt-target-c")
    lock = os.environ.get("VSA_LOCK", f"{home}/.cache/rusty-tiles-cargo-c.lock")
    cpus = os.environ.get("VSA_CPUS", "16-19")
    timeout = int(os.environ.get("VSA_TIMEOUT", "1800"))
    report = []
    for name, edits in MUTANTS:
        if names and name not in names:
            continue
        originals = {}
        for path, old, new in edits:
            source = originals.setdefault(path, open(path).read())
            current = open(path).read()
            assert current.count(old) == 1, (name, old)
            open(path, "w").write(current.replace(old, new))
        try:
            patch = sh(["git", "diff", "--", "src"]).stdout
            cargo = ["nice", "-n", "10", "taskset", "-c", cpus, "cargo", "test", "--locked", "--release",
                     "--lib", "-j", "4", "--target-dir", target, "--", "--test-threads=2"] + TESTS
            try:
                run = sh(["flock", "-o", lock, "sh", "-c", 'touch src/lib.rs && exec "$@"', "sh"] + cargo,
                         timeout=timeout)
                stdout, code = run.stdout + run.stderr, run.returncode
            except subprocess.TimeoutExpired as expired:
                stdout, code = f"TIMEOUT after {timeout} s\n{expired.stdout or ''}", -1
            with open(os.path.join(out, f"{name}.log"), "w") as log:
                log.write(stdout)
            failing = re.findall(r"^test (\S+) \.\.\. FAILED$", stdout, re.M)
            aborted = bool(code) and "test result:" not in stdout
        finally:
            for path, source in originals.items():
                open(path, "w").write(source)
        caught = bool(failing) or aborted
        report.append({"name": name, "caught": caught, "failing": failing, "aborted": aborted,
                       "patch": patch})
        print(name, "caught" if caught else "NOT caught", failing, flush=True)
    ok = all(r["caught"] == (r["name"] != "clean") for r in report)
    with open(os.path.join(out, "report.json"), "w") as f:
        json.dump({"ok": ok, "mutants": report}, f, indent=1)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main(sys.argv[1:])
