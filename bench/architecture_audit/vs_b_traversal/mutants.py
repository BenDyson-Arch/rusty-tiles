"""VS-B production mutants (traversal and decoded containment at scale), author controls.

Usage (repository root, clean `src/`):
    python3 -I bench/architecture_audit/vs_b_traversal/mutants.py OUT_DIR [name ...]

Each mutant is a set of exact single-occurrence string edits. The driver applies one mutant,
runs the release validator, implicit-reader and parity tests (the VS-B controls run every
differential in the resident and the forced-spill catalog profile), records the failing
tests and restores the file. A mutant is caught when at least one test fails or the run
aborts after compiling (a timeout counts as caught). A mutant that does not compile is
reported as `invalid`, never as caught. `clean` must fail nothing. Writes OUT_DIR/report.json.

Environment: CARGO_TARGET_DIR (default ~/.cache/rt-target-vsb), VSB_LOCK (default
~/.cache/rusty-tiles-cargo-d.lock), VSB_CPUS (default 4-7), VSB_JOBS (default 2),
VSB_TIMEOUT (default 2400 s). Set TMPDIR away from /tmp.
"""
import json
import os
import re
import subprocess
import sys

V = "src/validate.rs"
FACTS = "src/validate/facts.rs"
SCHEMA = "src/validate/schema.rs"
TYPES = "src/validate/types.rs"
AVAIL = "src/content_integrity/implicit/availability.rs"
MUTANTS = [
    ("clean", []),
    # VS-B-3: a tile is pushed above stale siblings instead of cutting the stack to its parent.
    ("stack-no-truncate", [(V, "        self.path.truncate(position);\n", "")]),
    # VS-B-3: chain bits are not inherited from the parent.
    ("chain-not-inherited", [(V, "            | inherited;", "            | 0;")]),
    # VS-B-4: passing references are not logged, so nothing is streamed.
    ("log-drops-references", [(V, "        self.log(Some(stream_log::Entry::Reference {\n            tile,\n            content,\n            rank,\n        }))",
                               "        let _ = (tile, content, rank);\n        self.log(None)")]),
    # VS-B-4: the logged stream also runs when the archive left the domain.
    ("stream-ignores-domain", [(V, "    if contained {\n        check.stream()?;",
                                "    if check.stream_log.is_some() {\n        check.stream()?;\n    }\n    if contained {")]),
    # VS-B-4: the Outside message names the root's depth.
    ("stream-depth-root", [(V, "                let depth = self.c2.on_path(tile)?.depth;",
                            "                let depth = self.c2.on_path(0)?.depth;")]),
    # VS-B-7: a summary is used whatever its live-document peak.
    ("live-peak-ignored", [(V, "                self.live_documents,\n                summary.live_peak,",
                            "                self.live_documents,\n                0,")]),
    # VS-B-5: the record CRC is never checked.
    ("facts-crc-unchecked", [(FACTS, "    if crc32fast::hash(&bytes[..BODY]) != crc {", "    if false && crc32fast::hash(&bytes[..BODY]) != crc {")]),
    # Scratch accounting: the slot extent is one record short.
    ("slot-extent-short", [("src/spill/sequence.rs", "            self.file.completed(self.offset(end)?);",
                            "            self.file.completed(self.offset(index)?);")]),
    # VS-B-5: the overflow flag of Nodes is lost in the record.
    ("facts-nodes-flag", [(FACTS, "        flags |= 2;", "        flags |= 0;")]),
    # VS-B-5 / VS-B-11: the stored receipt is not the admission's.
    ("facts-receipt-zero", [(FACTS, "    put(out, &stored.replay.to_stored_bytes());", "    put(out, &[0; 64]);")]),
    # VS-B-5: repeat references are inspected again.
    ("facts-reinspect", [(V, "        if self.facts.inspected(rank) {", "        if false && self.facts.inspected(rank) {")]),
    # VS-B-6: the memo budget is never applied.
    ("memo-budget-ignored", [(V, "            .is_none_or(|n| n > memo_budget())", "            .is_none_or(|n| n > usize::MAX)")]),
    # VS-B-6: a replayed tile hangs from the placement instead of its own parent.
    ("replay-flat-parent", [(V, "Some(local) => (Some(frame.locals[*local]), None),",
                             "Some(_) => (frame.link.parent, None),")]),
    # VS-B-6: a replayed root loses its external-content volume.
    ("replay-no-external", [(V, "None => (frame.link.parent, frame.link.external),",
                             "None => (frame.link.parent, None),")]),
    # VS-B-7: a document's charge is never released.
    ("live-not-released", [(V, "                    self.live_documents -= charge;\n", "")]),
    # VS-B-7: value nodes are not charged.
    ("live-no-nodes", [(V, "(nodes as u64).saturating_mul(128)", "(nodes as u64).saturating_mul(0)")]),
    # VS-B-8: OD-7 is never decided.
    ("od7-unchecked", [(V, "            if self.subtrees.get(member.rank) {", "            if false && self.subtrees.get(member.rank) {")]),
    # VS-B-8: the index tables are not charged.
    ("tables-uncharged", [(AVAIL, "        *live += tables as u64;", "        *live += 0;")]),
    # VS-B-8: aliasing content slots count their source again.
    ("tables-no-dedupe", [(AVAIL, "                    if !used[*d] {", "                    if true {")]),
    # VS-B-9: the largest labels are kept.
    ("labels-keep-largest", [(V, "            self.kept.pop_last();", "            self.kept.pop_first();")]),
    # VS-B-9: a 256-byte label is dropped.
    ("labels-boundary", [(V, "        if label.len() > LABEL_BYTES {", "        if label.len() >= LABEL_BYTES {")]),
    # VS-B-10: the pairwise path loses the 0 / -0.0 identity.
    ("unique-pairwise-bits", [(SCHEMA, "if pairwise { h.exact } else { h.bits }", "h.bits")]),
    # VS-B-10: the hash-set path identifies 0 and -0.0 (exact buckets and no bit comparison).
    # Either edit alone is equivalent: the `bits` buckets and the `same_bits` confirmation each
    # separate them (the first runs recorded both single edits as not caught).
    ("unique-set-exact", [(SCHEMA, "if pairwise { h.exact } else { h.bits }", "h.exact"),
                          (SCHEMA, "(pairwise || same_bits(item, other))", "true")]),
    # VS-B-2: the elements budget scales at 8 per byte.
    ("kappa-elements", [(TYPES, "            total_payload_elements: at_least(self.total_payload_elements, Kappa(16, 1)),",
                         "            total_payload_elements: at_least(self.total_payload_elements, Kappa(8, 1)),")]),
    # VS-B-2: explicit limits are scaled too.
    ("absolute-scaled", [(V, "            Self::Absolute(limits) => limits,",
                          "            Self::Absolute(limits) => limits.scaled(stored_bytes),")]),
]
TESTS = ["validate::", "content_integrity::implicit", "c2_oracle_parity", "i1_oracle_parity",
         "payload_context_acceptance", "spill::"]


def sh(cmd, **kw):
    return subprocess.run(cmd, text=True, capture_output=True, **kw)


def main(argv):
    out = os.path.abspath(argv[0])
    names = set(argv[1:])
    os.makedirs(out, exist_ok=True)
    if sh(["git", "diff", "--quiet", "--", "src"]).returncode:
        sys.exit("src/ must be clean")
    home = os.path.expanduser("~")
    target = os.environ.get("CARGO_TARGET_DIR", f"{home}/.cache/rt-target-vsb")
    lock = os.environ.get("VSB_LOCK", f"{home}/.cache/rusty-tiles-cargo-d.lock")
    cpus = os.environ.get("VSB_CPUS", "4-7")
    timeout = int(os.environ.get("VSB_TIMEOUT", "2400"))
    jobs = os.environ.get("VSB_JOBS", "2")
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
                     "--lib", "-j", jobs, "--target-dir", target, "--", "--test-threads=2"] + TESTS
            try:
                run = sh(["flock", "-o", lock, "sh", "-c", 'touch src/lib.rs && exec "$@"', "sh"] + cargo,
                         timeout=timeout)
                stdout, code = run.stdout + run.stderr, run.returncode
            except subprocess.TimeoutExpired as expired:
                stdout, code = f"TIMEOUT after {timeout} s\n{expired.stdout or ''}", -1
            with open(os.path.join(out, f"{name}.log"), "w") as log:
                log.write(stdout)
            failing = re.findall(r"^test (\S+) \.\.\. FAILED$", stdout, re.M)
            invalid = "could not compile" in stdout
            aborted = bool(code) and "test result:" not in stdout and not invalid
        finally:
            for path, source in originals.items():
                open(path, "w").write(source)
        caught = (bool(failing) or aborted) and not invalid
        report.append({"name": name, "caught": caught, "invalid": invalid, "failing": failing,
                       "aborted": aborted, "patch": patch})
        print(name, "INVALID" if invalid else "caught" if caught else "NOT caught", failing, flush=True)
    ok = all(r["caught"] == (r["name"] != "clean") and not r["invalid"] for r in report)
    with open(os.path.join(out, "report.json"), "w") as f:
        json.dump({"ok": ok, "mutants": report}, f, indent=1)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main(sys.argv[1:])
