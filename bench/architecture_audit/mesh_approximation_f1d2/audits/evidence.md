# F1d2 independent evidence audit

Preimplementation audit against F1d1 base
`3b5703231bf3991c2e37caef99989214308cc08d` (PR #149 merged). This document
settles acceptance representation, not acceptance of an F1d2 implementation.
No production code, Cargo invocation, Git mutation, optimizer or Rust geometry
routine was used by this audit. The coordinator owns the final contract.

## Replace the old geometry oracle

`tests/f1d1_oracle.py:ideal_face_certificate_squared` computes the smallest
whole-face pairing certificate with exact optimal rational witnesses. Its use
as a lower bound on the reported F1d1 certificate is justified only while each
original triangle selects one target triangle. Once source proof subdivision
is allowed, that scalar is no longer a lower reference: the 8 m triangle and
its four midpoint children have old squared certificate 16, but their complete
supports coincide and a depth-one exact proof has squared certificate zero.
Retain the old computation only as an explicit historical limitation control.
Replace its emitted-certificate assertion and whole-face work formula.

Retain the independently decoded source/root/leaf support, exact original
identity, material omission/factors, complete region membership, leaf identity,
hierarchy, inventory and conservative bounds checks subject to final-source
replay. Existing green suites do not supply new certificate truth.

## Selected acceptance representation: independently certify the scalar

The independent artifact checker receives the exact decoded local f32 source
and proxy supports, partitioned by original primitive region. It reads the
finite nonnegative emitted certificate C as its exact binary rational value,
and proves a complete cover independently in both directions:

1. Start each source face with the abstract identity barycentric triangle.
2. Reconstruct every source patch corner with exact `Fraction` arithmetic.
3. For each target face, independently calculate exact squared distance from
   each patch corner to the closed face, including point/line degeneracy.
   Exact plane projection and every clamped edge candidate suffice.
4. The least target of the maximum three corner distances is an exact
   whole-patch upper bound, by convexity. Accept the patch if it is <= C².
5. Otherwise replace it with all four exact canonical barycentric midpoint
   children. Refuse to establish the result on depth/work/time exhaustion.
6. Require every source face in both directions to finish with a complete
   accepted cover. The emitted scalar then independently bounds the complete
   decoded surfaces, regardless of how the producer proposed witnesses.

This is an upper-bound proof using the checker's own witnesses. It is not the
old invalid claim that a fixed unpartitioned lower reference must be below C.
The checker never imports private Rust math or meshopt, and it need not consume
producer patch choices or witnesses. Exact distances are defined by independent
closed-triangle projection enumeration, not floating-point sampling.

Why this is complete for the selected producer domain: if production uses the
same exact canonical abstract midpoint subdivisions and accepts each terminal
patch with explicit target witnesses bounded by C, the exact optimal witnesses
on that patch are no worse. The independent greedy tree therefore accepts each
branch no later than its production terminal patch. This depends on exact
source-patch coverage and reconstruction; rounded physical midpoints break the
argument and must not become the source representation.

Exact child coverage is a parameter-space fact, including degenerate physical
faces. For any parent barycentric weights w_i >= 0 summing to 1, if some w_i >=
1/2, that point belongs to corner child i with child coefficients
(2*w_i-1, 2*w_j, 2*w_k). If all w_i <= 1/2, it belongs to the centre child with
coefficients (1-2*w_2, 1-2*w_0, 1-2*w_1) for vertices (AB, BC, CA). Those
coefficients are nonnegative and sum to 1. Thus the four children cover the
whole closed parent, intersecting only along boundaries. This proof does not
use floating coordinate midpoints, measured area, finite point samples or
nondegeneracy. Recursively replacing every rejected patch with all four
children preserves complete coverage.

No public proof payload or private production trace is needed for these bounded
artifact checks. The exploratory transcript checker in the focused prototype
shows the alternative's required information but is **not** the selected
architecture. A scalar report alone is not a universally self-verifying artifact:
acceptance is the independent bounded re-certification plus nonauthor source
proof. If a future checker cannot finish within its declared limits, report
unproven instead of treating the result as false or weakening acceptance.

## Counts, work and precise limits

The settled candidate production profile uses maximum canonical depth 24 and
at most 16,777,216 streamed patch-target tests, counting the original root
comparisons and every refinement attempt. Root base work is independently
`sum_regions 2 * source_faces * proxy_faces`; it is an admission floor.

If production fully scans every target for every visited patch, the independent
greedy tree is a subtree of the production tree. Thus independent needed tests,
terminal patches and maximum depth are necessary lower bounds on corresponding
reported actual metrics, and root base tests <= reported actual tests <= cap.
These are inequalities, not equality checks or proofs of exact operational
counters. Exact counter semantics remain unit-test/instrumentation/source-review
obligations. Changing production to early target acceptance or changing its
canonical patch tree would require reconsidering the test-count inequality.
Region face order affects recorded proof selection, not geometric truth.

The focused probe fixes its own maximum depth at 24, patch-target work at
100,000 and cooperative elapsed-time ceiling at 10 seconds. Time checks occur
per patch and every 128 target tests; this is not a hard process latency bound.
All authored tests finish in under one second on this host. Final artifact
re-certification should choose an explicit bounded checker cap, pin it in the
receipt, and classify exhaustion separately from a proven counterexample.
Reported source/triangle counts must come from independently decoded geometry,
not the producer's counts. This evidence does not prove total memory bounds,
optimizer cancellation latency, topology, appearance, placement/world accuracy,
recursive approximation or release acceptance.

## Executed focused evidence

Run:

```sh
python3 bench/architecture_audit/mesh_approximation_f1d2/probes/proof_checker.py
```

The adjacent `probes/proof-checker-receipt.json` pins the complete independent
driver hash, Python version, exact supports' hashes, base commit, explicit
checker limits, measurements and all controls. This is authored prototype/checker
evidence only, not a receipt from an F1d2 converter.

| Exact authored supports | Independent squared bound | Canonical depth | Patch-target tests |
| --- | ---: | ---: | ---: |
| 8 m triangle versus its four midpoint children | 0 (old scheme: 16) | 1 | 24 |
| Same support shifted 1/8 m out of plane | 1/64 | 1 | 24 |
| Spike reaching 100 m | 10000 | 0 | 2 |
| Degenerate point faces 5 m apart | 25 | 0 | 2 |
| Square versus boundary ring with 3 m centre hole | 9 | 1 | 96 |
| Thin faces at 999999..1000000 m, height offset 2^-30 m | 1/2^60 | 0 | 2 |

Reordering the nonzero shifted supports preserves the exact result. The shifted
case accepts C=1/8 and fails to establish C=1/16. Forged zero certificates for
the offset, spike, degenerate point, interior hole and thin mixed-magnitude case
all fail the independently complete exact-cover checker. A deliberately tiny
work cap also returns unproven. These controls establish checker sensitivity;
reaching a depth/work limit alone does not mathematically prove the scalar false.
The authored offset, spike, hole-centre and separated point are additionally
explicit rational point counterexamples to the forged zero claims.

The exploratory transcript check independently requires a complete prefix-free
four-child abstract tree for every source face and both directions. Removing a
child or direction, duplicating a child, overlapping a parent with descendants,
wrong target index, invalid/negative dyadic weights, corrupted support binding,
forged physical source coordinates and invalid paths all fail. This establishes
why accepted leaves alone, a point list, area totals or leaf counts do not prove
coverage, especially for degenerate faces.

The non-midpoint rounding trap uses admitted exactly representable f32 endpoints
1 and 2^-149: their true midpoint 1/2+2^-150 rounds to 1/2 in binary64. Physical midpoint
rounding therefore changes support and cannot authorize a cover. Production
must reconstruct exact abstract barycentrics through outward intervals.

A point projected onto the line with endpoints (0,0,0) and (3,1,0) has exact
squared distance 1/10; its 2^24-denominator dyadic witness gives the strictly
larger squared distance 14073748835533/140737488355328. Exact adaptive ideals
are a relaxation and independently prove surface error, but cannot stand in for
the producer's actual explicit witness upper bound or justify universal
convergence. Source subdivision does not remove the finite target witness
quantization floor; the math audit owns its range-sensitive controls.

## Final-source execution gate

After the coordinator settles the profile/schema, update the full published
artifact oracle to independently re-certify decoded source and proxy supports
using this exact canonical cover. Bind source, executable, driver, archive,
GLB members, report and checker limits. A useful production reduction must pass
at a budget below its **independently calculated old whole-face certificate**;
otherwise source subdivision has not demonstrated practical tightening. Replay
positive nonzero/error/work/depth controls through the frozen CLI and installed
adapters with no workspace/publication on refusals. Retain region/material,
full-detail metadata/geometry, hierarchy/bounds and consumer checks. Finish with
a separate nonauthor inspection of final code and final evidence. No production
certificate or public output is accepted by the focused receipt alone.
