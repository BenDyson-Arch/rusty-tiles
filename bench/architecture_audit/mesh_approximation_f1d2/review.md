# F1d2 separate nonauthor review

Status: fresh resume **NONAUTHOR** source and final artifact/evidence review
complete; bounded F1d2 acceptance with the explicit proof limits below.
No production or evidence blocker remains in this reviewed slice. This reviewer owns this
record and new `review_probes/` files only, authored no production or acceptance
driver change, and performed no Cargo/build or Git operation. The prior draft
was read as evidence to challenge, not adopted as acceptance truth.

The review applies `AGENTS.md`, the architecture gate, the settled
[F1d2 contract](../../../docs/architecture/f1d2-certificate-contract.md) and the
three preimplementation audits. The prior F1d1 acceptance and green regressions
do not specify the new complete-cover certificate. The scope remains one root
proxy, exact decoded local f32 supports, existing authored-key components and
full-detail leaves. This does not close #121/#113 or authorize release.

## Frozen source observations

Production is pinned to `4553533e64c1494e706888a2c8e3a38a6867ce56`.
All 93 production files match the supplied manifest. The frozen portable CLI
SHA-256 is `aa71fd57614844488d336dbc991df551fd9294c6d31d82027e1b4a43aedbf64b`.
The [identity receipt](review_probes/frozen-identities.json) retains the source
hashes and portable manifest identity. Final certificate module digest is
`b444b74efc8924cca43e009e247653202afdfe9bb08dec20251d6e0abd191aa7`.

The inspected certificate child module receives face supports, an already
validated budget and a fallible checkpoint. It imports no optimizer, source
identity/material, encoder, path or frontend. `RootProxyLimits::new` owns positive
triangle and finite positive budget policy once; preparation consumes the private
validated choice. The prepared root retains those limits independently from its
one certificate result. The report copies that result once into the typed
schema 7 certificate object; root geometric error still uses the requested
budget and leaves still use zero.

Source patch corners are integer barycentric addresses against an original f32
face. Root rows sum to 2^24. A split checks even integer sums, constructs all
four canonical midpoint triangles and increases depth below 24. Inductively,
at depth d every numerator is divisible by 2^(24-d), so each permitted split is
exact. The three corner subtriangles and central subtriangle cover the complete
parameter simplex; affine mapping preserves that cover even for collapsed
point/line supports. Floating physical midpoint coordinates never own a patch.
The DFS pushes every child in reverse to visit the fixed canonical order, drains
the whole pending stack before the next original face, and proves both
directions in every region. A failed or incomplete cover cannot return success.

Coordinate reconstruction validates each nonnegative integer row's exact sum.
Nonzero weights reconstruct intervals directly from the original decoded face.
An axis retains a singleton only when all contributing authored axis values
agree; a whole corner retains exact-point equality only when its contributing
original points agree. These shortcuts are exact affine facts. Approximate
projection points propose target weights only. Distance residual intervals use
source-low minus target-high and source-high minus target-low, then outward
squares, sum and square root. Every patch-target bound uses all three corners
against that one target face; witnesses from different target faces are never
combined to claim a complete-patch bound. Original complete corner-set equality
may establish zero, including degeneracy and permutations.

The base admission is a checked sum of two directed root face products. Every
visited patch still scans all target faces, including equal-face shortcuts.
`Work::consume` checks the single global cap before evaluating the next pair;
failed parent scans remain charged. The counter is not reset by face, direction
or region. Success aggregates the maximum accepted-leaf bound, accepted-leaf
count and deepest accepted depth. Fixed-depth DFS needs at most 73 pending
patches; it stores no accepted cover, transcript or pair matrix. Entry, patch
transition, every 64 evaluations and final-result checkpoints propagate their
existing errors. Certification occurs during preparation, before workspace
creation; dynamic refusal is consequently after in-memory proof work, while
base refusal precedes that work. Existing F0 Attempt and publication ownership
remain in the consumer.

These are observations of frozen source, separate from executed artifact proof
and coordinator-owned Rust checks. The final certificate source was inspected
again after formatting and the numeric-cap message correction.

## Independent controls executed

[interval_enclosure.py](review_probes/interval_enclosure.py) uses independently
exact `Fraction` source/target addresses to check a Python reproduction of the
inspected outward operations. Its deterministic mixed-magnitude corpus includes
signed zero, f32 subnormals, one-hot and arbitrary simplex rows, and coordinates
near the admitted million-metre limit. All 24,000 corner intervals enclose the
exact coordinates, and all 12,000 squared/norm bounds enclose the exact explicit
witness distances. The original edge midpoint between x=1,000,000 and x=2^-149
exceeds its rounded x=500,000 representative by exactly 2^-150. The interval
contains that exact midpoint and its bound to the rounded singleton is positive.
This independently exposes the rounded-midpoint zero shortcut as unsound. The
[receipt](review_probes/interval-enclosure.json) pins the driver and inspected
certificate bytes. This is arithmetic reproduction, not execution of Rust.

[checker_controls.py](review_probes/checker_controls.py) inspects and exercises
the independent `tests/f1d2_certificate.py` owner without producer code or
fixtures. A large triangle and four manually authored quarters certify exact
zero in 24 tests, eight accepted leaves and depth one. An exact 1/8-metre plane
offset certifies squared error 1/64. A degenerate boundary target contains every
original corner but leaves the centroid at exact squared distance 32/9; its
forged zero scalar fails the complete cover. Offset false-zero, depth-zero and
work-23 controls likewise return explicitly unproven. Understated work, leaves
and depth, boolean work and excessive depth are rejected. Details and checker
identity are in the [receipt](review_probes/checker-controls.json).

The checker uses exact closed-triangle/edge projections, exact rational affine
midpoints and its own complete canonical cover at the published scalar. It
does not use producer witnesses or counts as geometric truth. Its ideal corner
distances are no greater than the producer's explicit-witness distances, so
its accepted tree can stop earlier and its tests/leaves/depth are necessary
lower bounds on reported producer metrics under the exhaustive scan policy.
The reviewer confirmed that compatible inflated counters can pass those
inequalities. This is an explicit authentication limit, not exact operational
counter evidence; production accounting requires source proof and meaningful
boundary/cancellation tests. Work/depth/time exhaustion means unproven, not a
mathematical counterexample. The boundary centroid and plane offset additionally
supply exact counterexamples to their false-zero claims.

## Separate reviewer producer replay

[producer_replay.py](review_probes/producer_replay.py) constructs its own literal
indexed planar 4-by-4 grid without the implementation/evidence owners' fixture
generator. The frozen portable CLI reduces 32 source faces to 16 proxy faces
at a requested 0.5-metre budget. The published certificate is
`0.4472135954999625` metres. Independent decoded-artifact recertification proves
an exact complete-cover squared bound of 1/5, with 10,624 tests, 285 accepted
patches and depth 3. The independently calculated old whole-face squared bound
is 4, so both the requested budget and the published new certificate are
materially below the old 2-metre complete-face pairing bound. All 24 artifact
corruption controls reject; CLI and published reports agree.
Four altered actual CLI reports also reject: stale certificate, missing report,
false success and wrong profile. A separate synthetic parity control rejects
boolean `true` against integer depth 1, which ordinary Python dictionary equality
would incorrectly treat as equal.

The [small receipt](review_probes/producer-replay.json) records exact source,
archive, member, executable and independent-reader identities and every
rejected control. Source SHA-256 is
`c6aa4fa6ff7482f692ffa6ca06f4caaa1a672644d6529b17d673185515ab83a8`;
archive SHA-256 is
`20f122a9c5f697cdbd70e4f5249129fe1d83e61d8c8789b63470a514e5c46872`.
The generated source/archive remain temporary and reproducible by the driver.
This is actual frozen-CLI execution, distinct from the arithmetic reproduction
and authored checker controls above. It does not authenticate exact counters
merely because independent and producer metrics coincide in this case.

## Earlier findings and corrected expectations

A base-refusal message/checker mismatch was
reported to the coordinator: the oracle requires the numerical cap in the error
message while the initial admission message omitted it. The frozen source now
includes the cap. Two checker sensitivity gaps were also reported before execution:
the new driver checks positive CLI status without enforcing equality of its
report to the checked published report, and its Replace refusal replay can pass
on an unrelated nonzero error without requiring the intended Unsupported proof
gate. The current corrected drivers enforce canonical exact positive CLI report
equality and the same Unsupported error message on Replace refusal; stdout
corruptions were added. These are resolved checker/message gaps, not executed
production defects. Final corrected-driver pins and broad receipts were verified in the fresh
review below. Incomplete provisional broad runs and the pre-browser invocation
typo are distinct from final acceptance receipts and production defects.

## Fresh source and probe review

The fresh reviewer read `AGENTS.md`, the architecture gate, settled contract,
implementation and all three preimplementation audits, then independently
inspected the production owner and exact reader. The previous draft was not
used as source truth. The new [resume identity receipt](review_probes/resume-identities.json)
re-hashes all 93 production files against the external handoff manifest, plus
the portable, native and optimized binaries. All match production4553533.
Source-to-build provenance remains coordinator-owned; bytes were independently
checked rather than inferred from a commit label alone.

The integer-cover argument, both-support outward reconstruction, exact singleton
and corner-support shortcuts after finite admission, exhaustive target scans,
single global accounting, fixed DFS bound and pre-workspace certification
described above were checked afresh. The budget is consumed by meshopt's
absolute-error proposal tolerance; changing it can alter reduction before
certification. No budget-independent candidate premise is accepted.

After reading their implementations, the fresh reviewer successfully repeated
the [interval enclosure probe](review_probes/resume-interval-enclosure.json),
[authored complete-cover and counter controls](review_probes/resume-checker-controls.json)
and [separate literal-fixture frozen CLI replay](review_probes/resume-producer-replay.json).
These retain their current driver, checker and executable identities. The first
is arithmetic reproduction, the second authored exact geometry/checker evidence,
and the third actual producer output independently decoded and recertified.
The inflated-counter control still passes the necessary metric inequalities:
they do not authenticate exact producer operational counters.

## Corrected final evidence inspected

Each final corrected portable/native run contains five baseline artifacts with
115 rejected corruption controls, eleven admission refusals, a tight grid and
a nonzero bumped grid. The reviewer independently verified all fourteen
successful source/archive/member bindings, canonical JSON equality of actual
CLI and published reports, and all twenty-four typed refusal responses and
absent output parents. Both 0.125 m bumped requests exercise the same Unsupported
proposal gate under Replace and preserve sentinel bytes and directory. The
tight and bumped artifacts reject respectively 24 and 25 corruption controls
per executable. The self-tests remain synthetic evidence: six positive cases
and 39 rejected controls per broad run.

| Actual artifact, portable and native | Decoded faces | Published local bound | Independent squared cover bound | Needed tests / leaves / depth |
| --- | ---: | ---: | ---: | --- |
| Tight 0.5 m grid | 32 → 15 | 0.4850712500726813 m | 4/17 | 9912 / 260 / 3 |
| Nonzero 0.5 m bump | 32 → 2 | 0.25000000000000017 m | 1/16 | 1408 / 64 / 2 |

The tight grid materially improves on its independently computed historical
2 m whole-face pairing bound. That historical bound is a tightening comparison,
never a lower bound on the subdivided certificate. Matching metrics in these
receipts do not supply exact operational authentication.

[resume_evidence_bindings.py](review_probes/resume_evidence_bindings.py) and
its [receipt](review_probes/resume-evidence-bindings.json) verify final driver
bytes and actual source/archive/member/report bindings rather than trusting
receipt status alone. The receipt pins final portable/native broad executions,
placed consumer, optimized resource observations, wheel and final source map.
The broad driver digest is
`e1af1fbf244870347dd45aed050d0057d481e6ca6c3c5969ffbcfede12ae22bf`;
the exact checker digest is
`8f64bd8bd0e9a54c7e44b26a2e89f4f341dd5c88c7a7ac64ff3ff134039c75d7`;
the artifact reader digest is
`ac155f43347cd22bc06e5b7885f7e626fcf7381cf14a35abcaf5a8a4e0089ead`.
The complete map includes independent leaf reader, browser drivers and installed
API/validation drivers.

The fresh placed public consumer receipt uses native binary
`c4c4a64ed25ddfd706e2dcaebb830c9323928dc299ebc2664569f563ca0a52e3`,
Cesium 1.146.0 and Chromium 153.0.8010.12. Natural SSE 16 displays 44 root
faces from afar and 192 unchanged leaf faces nearby; six complete coarse
membership arrays and six fine original-triangle tuples match public queries.
Wrong labels produce zero matches. There are no page errors, failed requests or
external requests. Decimal80 independently defines the Brisbane WGS84 frame,
cameras and targets. Basis/origin errors are below 2e-15 / 1e-8 m; missing
placement, wrong height and swapped axes are rejected sensitive controls.
Historical F1d1 `frozen_requested_*_match=false` fields compare old constants;
the current command explicitly requires native4553533's actual hash and all
current identities were checked. This is one placed consumer and local decoded
surface proof, not an exact world Hausdorff result.

The optimized near-cap resource run uses binary
`d1e86e4f38b458262d5652b425df7a02c88924519089d505eda8da09ed5797d9`.
It emits 1023 proxy faces from 8192, reports 16,760,832 tests (99.9023% of
the fixed cap), 9215 accepted patches at depth zero and a 7.000000000000003 m
scalar. Its exact common-box reference establishes the requested 100 m budget;
the smaller 7 m scalar is explicitly **not independently recertified at this
size**. Base-limit and dynamic patch-limit cases return their intended typed
Unsupported reasons before output-parent creation. Three serial children were
reaped without timeout. The successful process's 17.439 s, wait4 26,388 KiB and
sampled 14,088 KiB RSS are host observations; sampled peaks and Python launcher
residency limit interpretation. These establish no total RSS or latency bound.

The installed local abi3 linux_x86_64 wheel SHA-256 is
`028cadd7f78de1beff544eb46f243108658759cab09d12ff1f75a5dc3d9c70c7`.
The reviewer checked its file and every pinned ZIP member against the final
source-artifacts manifest, read the adaptive grid/bump API test and runner, and
inspected the log/receipt: 44 API tests, zero failures/errors/skips, with empty
PATH on CPython 3.14.7, plus the validation fixture suite. Installed artifacts
use the same independent exact reader and reports agree with the published
report. This establishes no manylinux, other-Python/platform or publishability
claim.

Coordinator-owned native and final portable Rust logs establish meaningful
canonical address/depth, exact-cap exhaustive scan, dynamic accounting,
mixed-subnormal and cancellation controls. Cancellation interrupts at 64
evaluations in the long-scan control, preserves its first cause through the
Attempt, and cannot be overridden by final successful math. Concrete proxy
stage and partial-write tests preserve Replace bytes and clean owned candidates.
These are executed Rust controls plus inspected ownership, distinct from a CLI
mid-proof cancellation experiment. No CLI depth-cap fixture was established:
the bumped tight request changes proposal admission, and tiny nonuniform
feasibility attempts supply no depth-gate acceptance. Depth semantics rely on
direct meaningful boundary controls and source proof, with actual base/dynamic
gates established separately. Full-detail replays and core glTF consumer checks
remain bounded retained evidence listed in the final index; core validation
does not prove metadata extensions.

## Final conclusion boundary

The root crate package SHA-256 is
`0a94bfcdb123c4b67a463ae17ce5b6f62fb3d644a1657b671827558044314ffd`.
The reviewer independently opened its tar archive: all 89 included root
production files equal reviewed source bytes, `Cargo.toml.orig` equals the
original manifest, and included contract, implementation and exact oracles match
their recorded snapshot. Cargo's normalized lock removes the binding crate and
seven Python-only dependencies; no retained package entry changes and none is
added. The root package intentionally omits workspace Python binding files;
their separately pinned wheel was built and tested above. This package used
`--no-verify`; no packaged-crate rebuild or release acceptance is inferred.
Final evidence-only coordinator/review records may follow the package snapshot
without changing this production identity.

All eleven final coordinator check logs have matching byte counts/SHA-256 and
successful recorded exit statuses, including retained final portable tests,
native tests, portable/native/binding lints, binding tests, formatting, wheel
build/installed tests and package. The reviewer independently checked lossless
recovery of the indexed receipt snapshot; all 49 entries matched their stored
and original identities and still-available raw paths. The storage verifier
also passed deterministic recompression. The
[package/storage receipt](review_probes/resume-package-storage.json) pins that
snapshot; final indexing may add these reviewer records without changing any
executed production/driver/artifact identity.

No inspected production or evidence blocker remains for the bounded F1d2
complete-local-surface proof slice, bound to production4553533, the three
frozen binaries, corrected driver hashes, final artifact/member receipts,
installed wheel and package above. This is the fresh nonauthor acceptance
conclusion. It does not claim that every preimplementation CLI fixture
aspiration was executed: the absent actual CLI depth-cap fixture, separate
Rust cancellation evidence, exact-counter authentication limit, unproved large
7 m resource scalar and local/world distinction remain explicit. These limits
prevent promotion to broader approximation or release acceptance.

No topology, multiplicity, normal/texture/appearance budget, exact world Hausdorff
error, general CRS, recursive LOD, global legacy removal, universal convergence,
total RSS ceiling, meshopt cancellation latency or release claim is accepted.
