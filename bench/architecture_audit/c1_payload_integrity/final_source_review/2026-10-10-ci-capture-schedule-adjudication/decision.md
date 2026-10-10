# Independent CI capture scheduling decision, 2026-10-10

**Approve the smallest CI-only correction: add `--test-threads=1` after the
existing test argument separator, alongside `--nocapture`, in the existing
native Cargo test/tee step.** Do not change production, selected independent
modules, assertions, fixtures, parser, record schema or the public harnesses.
Acceptance of the correction requires a fresh actual capped capture probe and
new-head full remote execution with all82 uncorrupted records; this decision is precode
adjudication, not completed CI or merge acceptance.

Separate reviewer `/root/payload_final_source_review` inspected the actual
Rust job114206817691 log from main run38049923044 and its uploaded original
c1-library.log at final head5fdf/merge-preview87b4. The uploaded raw log SHA256 is
`669eb8f547b8b868292a3ffcb9ce28feceef85505beef5706976ec25ba857023`,
133158 bytes; the full API job log SHA256 is
`a6ef3061719aa8dc4d790abd3a41424d34ca8379f6fca19709dedd7f80334d7f`.
The library reports367 passed/zero failed/ignored/filtered and the Cargo command
completes its remaining suites before the unchanged context parser raises
JSONDecodeError. This is an executed CI evidence-capture failure, not an observed
production or test assertion failure. The failed job must retain its failure
classification and original run/source/log identities.

The raw upload contains80 intact start markers:77 independently parseable JSON
lines and three visibly corrupted marker lines. These are diagnostic counts,
not an accepted82-record receipt. For example, the shared-position admitted
record's `selectedLimits.accessorElements` is interrupted by another test's
libtest name/completion text. Two other visible records have unrelated test
output inserted into a JSON key/string. Some markers themselves are split;
there is no attempt to infer, reconstruct, drop, repair or accept missing rows.
The parser correctly refuses this original log.

The selected context module performs synchronous inspect_selected calls and
typed/full-report assertions, then formats serde_json::Value directly through
eprintln. That formatter can make several writes to stderr. Independent libtest
stdout from concurrently running tests shares the combined `2>&1 | tee` stream;
atomicity across the two streams is not established by the existing call. The
failure directly exhibits this interleaving. A capture owner should avoid
unrelated concurrent harness output while retaining the independently frozen
assertions and literal inputs.

The [Rust Book](https://doc.rust-lang.org/book/ch11-02-running-tests.html) documents
passing test-binary options after `--` and using `--test-threads=1` to run tests
consecutively. The [rustc test-harness documentation](https://doc.rust-lang.org/rustc/tests/index.html)
describes the same scheduling flag. These official primary descriptions were
read on2026-10-10. The inference here combines that documented harness scheduling
with the actual synchronous emitter/call graph and corrupted raw capture.

The thirteen context tests do not spawn threads, children, detached tasks or
separate output writers. Their actual C1 graph performs no print/eprint calls.
The only thread spawn found in validate.rs is inside a separate test and is
joined. The runtime concurrency tests retain their explicit scoped threads,
barriers and joins. The directory subprocess tests inherit child output but
wait for successful child completion before returning; their child invocation
already requests test-threads1. Serializing outer test selection does not stop
those owned concurrent actors, remove their assertions or make their competing
operations sequential. Their output occurs during their own tests, not during
a simultaneously running context test. No unjoined context output producer was
identified. A green fresh full run must still demonstrate these owned concurrency
tests actually executed and completed.

This change removes incidental inter-test parallel scheduling from this capture
step. It does not promise general atomic output for arbitrary spawned writers,
nor does a serial run establish universal parallel scheduling coverage. The
original parallel all-green test results remain historical evidence. Nothing in
the approved bounded context/accounting plan requires unrelated tests to run
simultaneously while the receipt is printed. Test filters, ignored cases and
disabled ownership/concurrency assertions are not permitted by this decision.

An alternative emitter change could serialize into a String before an atomic
write and coordinate stdout/stderr ownership, but it would require a new
selected test-copy binding and recompilation. No such module/source correction
is needed to address the demonstrated capture owner here. Parser stripping or
heuristic stitching would weaken the proof and is not authorized.

Root must preserve the original failed job/upload and apply only the workflow
argument. A targeted local probe of all13 payload_context_acceptance tests using
the same already compiled, hash-verified unit with --nocapture/--test-threads1
is sufficient to test this scheduling/capture correction. Its13-pass/354-filtered
identity must be recorded honestly; it is not a new full367 result or proof that
the owned concurrency tests reran locally. The existing full367 result remains
bound to its original execution. Actual new-head remote native Cargo must still
execute the full library/applicable suites, including owned concurrency tests,
before remote acceptance. Both the probe and actual remote capture must verify
all82 exact records/41 complete report admissions/41 typed ResourceLimit rows.
The unchanged parser must consume the actual fresh raw log and bind its actual
unit/log/source/tree/input hashes. Use actual Cargo unit/header provenance when
feeding its existing unit discovery; do not fabricate test or execution output.
Inspect the new commit diff independently:
all97 production and all selected control/parser/harness bytes must be unchanged;
the workflow input pin changes deliberately. New exact-head CI must run this
capture, retain lossless raw output and satisfy the existing217 public controls
and remaining wheel/installed-wheel/official Blender gates. Old5fdf wheel or CI
results must keep their original identities; any later byte-equivalence claim
needs explicit artifact/run/tree review. There is no merge, A2, issue-closure or
release acceptance in this precode decision.

The reviewer executed no Cargo or target, changed no repository files and
performed only read-only source/log inspection. This dated decision is stored
externally and leaves all earlier frozen reviews/evidence untouched.
