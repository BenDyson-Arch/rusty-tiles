"""EM production mutants (external-tileset memo amendment), author controls.

Usage (repository root, clean `src/`):
    python3 -I bench/architecture_audit/external_memo/mutants.py OUT_DIR [name ...]

Each mutant is a set of exact single-occurrence string edits of `src/validate.rs`.
The driver applies one mutant, runs the release EM controls
(`validate::memo_controls`) and the explicit external-tileset tests in one test
binary, records the failing tests, and restores `src/`. A mutant is caught when at least one control
fails. `clean` and the EQUIVALENT mutants must fail nothing. Writes
OUT_DIR/report.json.

Environment: CARGO_TARGET_DIR (default ~/.cache/rt-target-c), EM_LOCK (default
~/.cache/rusty-tiles-cargo-c.lock), EM_CPUS (default 16-19).
"""
import json
import os
import re
import subprocess
import sys

V = "src/validate.rs"
MUTANTS = [
    ("clean", []),
    # A summary replayed under the first placement's C2 link: its root keeps
    # the recorded arena parent and external-content volume.
    ("cached-placement-link", [(
        "None => (frame.link.parent, frame.link.external),",
        "None => (self.c2.tiles[*first].parent, self.c2.tiles[*first].external),",
    )]),
    ("skip-root-bounds", [(
        "            if !contains(parent, &summary.root_bounds, &summary.root_transform)? {",
        "            if false && !contains(parent, &summary.root_bounds, &summary.root_transform)? {",
    )]),
    ("skip-root-error", [(
        "        if summary.root_error > bound + 1e-8 * (1. + bound) {",
        "        if false && summary.root_error > bound + 1e-8 * (1. + bound) {",
    )]),
    ("no-visit-charge", [("        self.task_visits += summary.visits;\n", "")]),
    ("no-reference-charge", [("        self.resources.reference_visits += summary.references;\n", "")]),
    ("no-byte-charge", [("        self.resources.bytes_read += summary.bytes;\n", "")]),
    # The placement's own `Tileset` visit left out of its steady charge.
    ("short-visit-charge", [("            visits: self.task_visits - 1,", "            visits: self.task_visits,")]),
    # First payload inspections charged again at every placement.
    ("once-charges-repeated", [
        ("                        - once_references,", "                        - 0 * once_references,"),
        ("                    bytes: self.resources.bytes_read - recorder.bytes - once_bytes,",
         "                    bytes: self.resources.bytes_read - recorder.bytes - 0 * once_bytes,"),
    ]),
    ("skip-node-depth-bound", [("            && deep(summary.node_depth, limits.hierarchy_depth))",
                                "            && true)")]),
    ("skip-tileset-depth-bound", [("            && deep(summary.tileset_depth, 128)\n", "")]),
    ("replay-without-references", [(
        "                    self.c2.reference(frame.locals[*tile], content, uri)?;",
        "                    let _ = (tile, content, uri);",
    )]),
    # A memoized nested placement left out of its enclosing document's summary.
    ("nested-placement-unrecorded", [(
        "                    recorder.events.push(Event::Place(Box::new(placement)));",
        "                    drop(placement);",
    )]),
    ("memo-disabled", [("        let summary = self.memo.get(name)?;", "        let summary = self.memo.get(name).filter(|_| false)?;")]),
    # The memo used for a document on the active path.
    ("memo-on-active-path", [("        if self.active.contains(name) {\n            return None;\n        }\n", "")]),
]
# Equivalent by proof: no edit can make these observable.
EQUIVALENT = {
    "memo-on-active-path": "EM-5.5: a summarized document is never on the active path, so the "
                           "defensive check never returns early",
}
TESTS = ["validate::memo_controls", "validate::tests::external", "validate::tests::shared_external",
         "validate::containment_controls::external", "validate::containment_controls::references_follow"]


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
    lock = os.environ.get("EM_LOCK", f"{home}/.cache/rusty-tiles-cargo-c.lock")
    cpus = os.environ.get("EM_CPUS", "16-19")
    report = []
    for name, edits in MUTANTS:
        if names and name not in names:
            continue
        source = open(V).read()
        mutated = source
        for old, new in edits:
            assert mutated.count(old) == 1, (name, old)
            mutated = mutated.replace(old, new)
        open(V, "w").write(mutated)
        try:
            patch = sh(["git", "diff", "--", "src"]).stdout
            # One build per mutant; touching src/lib.rs under the lock forces it
            # when another checkout shares the target directory.
            cargo = ["nice", "-n", "10", "taskset", "-c", cpus, "cargo", "test", "--locked", "--release",
                     "--lib", "-j", "4", "--target-dir", target, "--", "--test-threads=2"] + TESTS
            run = sh(["flock", "-o", lock, "sh", "-c", 'touch src/lib.rs && exec "$@"', "sh"] + cargo)
            with open(os.path.join(out, f"{name}.log"), "w") as log:
                log.write(run.stdout + run.stderr)
            failing = re.findall(r"^test (\S+) \.\.\. FAILED$", run.stdout, re.M)
            aborted = bool(run.returncode) and "test result:" not in run.stdout
        finally:
            open(V, "w").write(source)
        caught = bool(failing) or aborted
        row = {"name": name, "caught": caught, "failing": failing, "aborted": aborted, "patch": patch,
               "equivalent": EQUIVALENT.get(name)}
        report.append(row)
        print(name, "caught" if caught else "NOT caught", failing, flush=True)
    ok = all(r["caught"] == (r["name"] != "clean" and r["name"] not in EQUIVALENT) for r in report)
    with open(os.path.join(out, "report.json"), "w") as f:
        json.dump({"ok": ok, "mutants": report}, f, indent=1)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main(sys.argv[1:])
