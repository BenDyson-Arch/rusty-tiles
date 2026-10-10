# F1d2: bounded adaptive complete-surface certification

This is the next #121 foundation slice after merged PR #149, under #113, which
continues to block 0.4.0. The [contract](f1d2-certificate-contract.md) was settled
through separate math, ownership and evidence audits before implementation.
This slice replaces the root's whole-face proof with complete dyadic subdivision;
it does not introduce recursive LOD or expand proxy appearance admission.

## Ownership and representation

Core request validation constructs private `RootProxyLimits` once. Preparation
consumes a validated approximation choice, and the prepared root carries the
same limits and certified geometry into publication. CLI and Python keep the
paired triangle-limit/error arguments. No compatibility report is retained.

`approximation.rs` owns original-key connectivity, region membership, material
eligibility and untrusted original-position proposals. Its private
`approximation/certificate.rs` child owns only the surface proof over decoded
f32 faces, with a checkpoint closure. Meshopt's tolerance guides proposals; its
score never authorizes output. Changing the requested tolerance can also change
whether meshopt achieves a reduction, before certification is reached.

Proof patches use three exact integer barycentric addresses against an original
face. The denominator is 2^24; all four canonical children cover each failed
parent exactly. Rounded midpoint positions never become source geometry.
Ordinary projections propose finite dyadic target witnesses. Outward intervals
enclose reconstruction of both supports, residuals, squared norms and the final
square root. Exact authored singleton/equal-support cases justify zero shortcuts.
Finite support is checked before those shortcuts.

The minimum, over every target face, of the maximum three corner-witness bounds
covers an entire source patch by convexity. Every source face receives a complete
accepted cover in both directions within its region. The final certificate is
the maximum over accepted patches. Proposal identity and geometric certification
remain separate obligations; coordinates alone cannot identify coincident
authored components.

## Limits and reports

Successful proofs require the checked compulsory base
`2 * sum(source_faces * proxy_faces)` within 16,777,216 tests. One global counter
charges every visited patch/target-face evaluation, including failed parents,
in both directions and every region. All target faces are scanned, including
after a zero bound. The same fixed ceiling bounds dynamic work. Maximum depth
is 24, with at most 73 pending DFS patches and no retained proof transcript.
Cancellation is checked at entry, patch transitions, at most every 64 evaluations
and after the last proof. Individual meshopt-call cancellation latency and total
process memory remain unproved.

Work/depth exhaustion means the requested bound could not be certified within
the finite profile. It does not prove the true distance exceeds the request.
The dyadic target lattice also imposes precision limits, even for equal supports;
arbitrarily tight tolerances are not promised. Failure occurs before workspace
creation and does not fall back to full detail. F0 retains one attempt, cleanup
and publication owner.

Report schema 7/profile `f1d2-adaptive-root-proxy-gltf-v1` replaces the old flat
`certified_error_metres` and `comparison_pairs` fields with a typed `certificate`:
`error_metres`, `patch_face_tests`, `accepted_patches`, and deepest accepted
`max_depth`. Requested root geometric error remains separate and positive;
full-detail leaves retain zero error and unchanged source coverage. The bound
concerns decoded local geometry, not an exact world Hausdorff bound after finite
placement arithmetic.

## Independent acceptance

The [evidence index](../../bench/architecture_audit/mesh_approximation_f1d2/README.md)
binds frozen production source, portable/native/optimized binaries, installed
wheel, artifacts, drivers and consumers. The exact Fraction checker independently
decodes geometry and finds its own complete canonical cover below the published
scalar. It consumes no producer witnesses, private geometry, sampling estimate
or report counters as geometric truth. Its bounded exhaustion leaves a scalar
unproven. Independent work/leaves/depth provide necessary inequalities for
reported metrics; exact operational counter semantics additionally need unit
controls and source review. Compatible inflated counters are not independently
disproved by those inequalities.

Small actual artifacts establish useful tightening below the old whole-face
bound, and a nonzero gap. Synthetic spike, hole, degenerate, thin and rounding
controls establish checker sensitivity; they are labelled separately from
producer executions. Typed refusals and destination preservation must identify
their actual proposal/base/dynamic/depth gate. The resource probe's large common
box proves its requested wide budget independently; it explicitly does not
re-certify the smaller published scalar at that size.

The separate [nonauthor review](../../bench/architecture_audit/mesh_approximation_f1d2/review.md)
checks frozen source and receipts. Active full-detail resource inspectors and
installed API assertions move to schema 7 in the same slice. Historical receipts
retain their original source/profile pins.

## Remaining #121 gates

Recursive hierarchy, selection and error propagation require their own settled
contract and independent references. Textured/attribute-preserving proxies and
appearance budgets remain excluded. Broader resource/cancellation stress,
placement error and representative consumer coverage remain explicit gates.
This bounded proof foundation does not close #121/#113 or authorize a release.
