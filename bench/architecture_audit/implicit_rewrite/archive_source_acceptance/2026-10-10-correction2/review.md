# Separate correction review — 2026-10-10, phase 2

Accepted bounded local correction: the test-only borrowing correction is equivalent, and an
explicit 60-second cooperative checker budget at the installed adaptive test is
justified. The exact rational geometry, work/depth ceilings and refusal semantics
remain required. The retained local capacity evidence establishes two successful
proofs and four sensitive exhaustion refusals; corrected exact-head CI
and installed artifacts remain the product acceptance gate. Earlier reader
acceptance snapshots and executable receipts are unchanged.

[Source pins](source-pins.json) bind the reviewed edits over PR #152 head
`4606ed1dea6d90aa516f1860393f109e5269040d`, at committed source
`18516cb04d2db14041a71dcc50205902b3e52a1e`. This nonauthor lane made no
production/test edits, Cargo runs, producer runs or heavy probes.

## Observed failure causes

Both inspected native CI logs complete their test stage and fail all-targets
Clippy at the test-only `src/archive3tz/read.rs:871` expression
`&[outer.clone()]`. The named lint is `cloned_ref_to_slice_refs`, promoted to an
error by `-D warnings`. The coordinator's replacement is
`std::slice::from_ref(&outer)`. `archive` only borrows the records and serializes
their fields; a one-element reference slice presents the same record bytes as
the temporary clone. The reviewer checked the exact replacement against the PR
head and verified that the source preceding `#[cfg(test)]` is unchanged. This
changes test allocation/spelling, not product behavior or the overlap control.

The inspected macOS x86 installed-wheel log runs 44 API tests and records one
error, the `grid` subtest of `test_adaptive_certificate_installed_api`. The
producer returned an artifact; `inspect_members` reached the independent exact
surface checker and raised `CertificateUnproven` at its monotonic deadline check
in `f1d2_certificate.py:119`. Its default deadline is ten seconds. This execution
establishes checker exhaustion, not a false-bound verdict or a producer failure.
The unfinished proof is still a failing installed test.

## Bounded checker capacity decision

Use the existing `max_checker_seconds` keyword only at the installed adaptive
`inspect_members` call, explicitly setting 60 seconds for its grid and bump
fixtures. Do not change `CHECKER_SECONDS=10`, other call sites or global defaults.
The reviewed source does exactly that and does not catch exhaustion, retry it or
replace it with success.

`inspect_members` forwards this parameter to the existing checker. Its 100,000
patch-face work cap, depth 24 cap, exact Fraction coordinates and point-to-triangle
distance calculation, complete four-child cover, both directed support scans and
acceptance predicate `bound <= published_certificate_squared` are unchanged.
The installed test retains its 0.5-metre producer budget, independently validated
report counters and geometry assertions. More wall-clock capacity permits the
same finite proof to finish on a slower runner; it does not relax a distance,
coverage or metric condition. The time ceiling remains cooperative, with checks
inside target scans, and is not a strict process-kill deadline.

The unchanged checker must still raise `CertificateUnproven` on time, work or
depth exhaustion. The installed test propagates that exception, so exhaustion
continues to fail. Existing independent sensitivity code includes time/work/depth
refusals and too-tight/false-zero controls; its source has not changed. The local
coordinator replay separately shows actual grid/bump proofs with the explicit
60-second capacity and sensitive time/work exhaustion controls. That replay uses
an earlier frozen producer, whose geometry is unchanged; it must not be labelled
corrected archive-executable acceptance.

The first local native-Clippy attempt stopped at the existing system-SQLite
environment guard because `LIBSQLITE3_SYS_USE_PKG_CONFIG=1` was missing. Preserve
that as an infrastructure failure, separately from the subsequently corrected
native environment and any actual lint result. Corrected exact-head CI, wheel and
official Blender checks remain the merge gates. No A2 or release acceptance is
granted by these test corrections.

## Verified execution evidence and identities

The corrected native all-targets Clippy invocation completed with exit zero in
6.97 seconds, using the required system-SQLite environment. The retained command
is `cargo clippy --locked --release --all-targets --features native-geospatial --
-D warnings`. Its initial missing-environment failure is retained separately.

The actual explicit-60-second independent checker proved the grid fixture in
1.285 seconds (9,912 patch-face tests, depth 3) and the bump fixture in 0.187
seconds (1,408 tests, depth 2). Both remained under the unchanged 100,000-test and
depth-24 ceilings. Each fixture also refused the injected tiny time allowance
and one-test work allowance with the specific `CertificateUnproven` exhaustion
reason, for four actual negative controls. These local timings demonstrate
capacity on the observed host; they do not predict hosted-runner timing.

This review verified all 93 production and seven acceptance input hashes against
the working files and committed `18516cb` contents. It verified all eight dated
compressed records against their compressed and expanded hashes, lengths and
original logs/receipts. The dated manifest hash is
`73199278c62ef7a97661c1d947fa1e62733b8a12afe1bea9001da880dd80b111`;
the dated index hash is
`df10d105ac33c69db61c31dfb435abf09db12a9bb44a94212071ae7d77444952`.
The manifest intentionally records precommit file identities; this separate
review supplies their verified committed identity.

The checker replay used the frozen original portable executable with SHA-256
`3328487c2920e07b291eebec8072954f11b0d20a39181ccbc0c4287e2f765524`.
Its actual binary, two source fixtures, two emitted archives and each recorded
member hash match the receipt. The unchanged exact checker sources match the
receipt and prior PR head. This is executed checker-capacity evidence from that
historical producer, not execution evidence for a corrected archive binary.

The original accepted review/pins, original 19-record index and first correction
review/pins remain byte-identical to their previously reviewed hashes. No prior
artifact is rebound to the corrected source. The only remaining acceptance work
for this correction is the applicable corrected exact-head CI and installed
wheel/official Blender artifact gates; this local source/test-capacity verdict
supports proceeding to those gates.
