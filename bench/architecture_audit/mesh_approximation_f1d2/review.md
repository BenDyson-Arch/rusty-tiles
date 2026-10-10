# F1d2 separate nonauthor review

Status: frozen production source and portable CLI inspected; independent
arithmetic/checker controls and one separate reviewer-authored producer replay
completed. Broader final artifact, adapter and resource evidence remains pending.
No final production acceptance conclusion follows from this draft. The
reviewer owns this record and `review_probes/` only; no production edit, Cargo
build or Git operation was performed.

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

## Pending final evidence

The review still requires the complete final artifact manifest and broad pinned
executions: unchanged authored-key/full-detail/placement/PBR evidence, actual
base/dynamic/depth refusal and mid-proof abort preservation, installed
adapter/report agreement, and fresh bounded resource observations. Current
arithmetic, authored-support and single-artifact probes do not discharge the
remaining execution obligations. A base-refusal message/checker mismatch was
reported to the coordinator: the oracle requires the numerical cap in the error
message while the initial admission message omitted it. The frozen source now
includes the cap. Two checker sensitivity gaps were also reported before execution:
the new driver checks positive CLI status without enforcing equality of its
report to the checked published report, and its Replace refusal replay can pass
on an unrelated nonzero error without requiring the intended Unsupported proof
gate. The current corrected drivers enforce canonical exact positive CLI report
equality and the same Unsupported error message on Replace refusal; stdout
corruptions were added. These are resolved checker/message gaps, not executed
production defects. Final corrected-driver pins and broad receipts remain to be
verified.

No topology, multiplicity, normal/texture/appearance budget, exact world Hausdorff
error, general CRS, recursive LOD, global legacy removal, universal convergence,
total RSS ceiling, meshopt cancellation latency or release claim is accepted.
