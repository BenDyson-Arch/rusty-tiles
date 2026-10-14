# F1d2: bounded complete-surface proof subdivision

Implementation contract for the next #121 slice, following merged #149 at
`3b5703231bf3991c2e37caef99989214308cc08d`. The fan-out
[audits and focused probes](../../bench/architecture_audit/mesh_approximation_f1d2/)
settle the design before production coding. Final-source execution and separate
nonauthor review remain required. This does not close #121/#113 or authorize release.

## Domain and owned representation

Retain one root proxy over unchanged full-detail leaves, positions-only opaque
untextured core PBR factors/omission, original-key components, complete region
membership, rigid placement and one F0 attempt. This slice changes certification,
not output geometry, hierarchy, materials or picking semantics. Recursive LOD,
textures, normals and appearance budgets remain separate gates.

Keep the public FullDetail/RootProxy request and paired CLI/Python arguments.
Core validation returns a private validated approximation choice. A private
RootProxyLimits constructor owns positive triangle count and finite positive
local-metre error validation once. Preparation and proposal consume those limits;
the prepared root retains them for root error and report construction. Depth,
work and witness precision are fixed profile limits, with no new public options.

The approximation producer owns eligibility, original-key connectivity, region
membership and untrusted meshopt proposals. A private certificate child module
owns face supports, proof patches, interval witnesses and work accounting. It
imports no source identity/material, meshopt, encoder, paths, adapters or runtime
jobs. It receives decoded f32 faces, the already validated numeric error budget
and a fallible checkpoint. One certificate result accompanies the candidate.

## Complete surface proof

Each source proof patch has three corners represented by nonnegative u32
barycentric numerators against its original decoded face, summing exactly to
2^24. The root corners are the three scaled basis vectors. Each split makes the
three edge midpoints by exact integer averaging and replaces the parent with all
four canonical midpoint triangles, in fixed child order. At depth d each
numerator is divisible by 2^(24-d); splitting is allowed only below depth 24.
These children cover the exact parent, including degenerate supports. Proof
patches never become rounded midpoint geometry or emitted mesh vertices.

Reconstruct source corner coordinate intervals directly from the original f32
face and exact binary weights. Retain the explicit target witnesses with
nonnegative integer weights summing to 2^24. Ordinary projected points only
propose target weights. Outward interval reconstruction, residuals, squares,
sums and square root enclose each exact source-corner-to-witness distance.
Exact one-hot source corners may retain original point equality; approximate
point equality cannot certify zero. Identical complete original corner supports
may still return exact zero.

For each visited patch, scan every target face and take the minimum whole-patch
bound: the maximum of the three corner distances to explicit witnesses in that
one target face. Convexity bounds every point of that exact patch. Accept the
patch only if the outward distance bound is at most the requested error budget;
otherwise split, or refuse at depth 24. Process all original directed faces,
both directions and every region. The final bound is the maximum over every
accepted patch in these complete covers. A missing child, partial cover or
unproved patch cannot produce success. No sampling, meshopt score or fallback
whole-face mode authorizes output.

Root geometricError remains the requested positive budget after certification;
leaf errors remain zero. Report the actual accepted-cover bound separately.
Certification concerns exact decoded local f32 supports. Finite placement
rounding, general world-space error and appearance remain unproved by this bound.

## Finite work and truthful refusal

Retain checked base admission `2 * sum(original_region_faces * proxy_region_faces)`
at most 16,777,216: exhaustive root-patch target scans make that work compulsory
for a complete successful proof. A failed partial proof can consume less.
Candidate counts determine this admission before any certificate evaluation.
Charge every actual patch/target-face evaluation, including failed parents and
all child scans, to one global checked counter capped at the same value. Check
before each next evaluation; do not reset per face, direction or region.

Traverse one original face at a time with fixed-order DFS. At depth 24 at most
73 pending patches are needed. Do not retain an accepted-patch vector, witness
transcript, pair matrix or proof payload. Counters record accepted patches and
deepest accepted depth. Check cancellation at entry, patch transitions and at
most every 64 evaluations; propagate its existing error unchanged.

Base refusal precedes certificate work. Dynamic work/depth refusal occurs after
some in-memory certification. All such refusals precede workspace creation and
publication, preserve the destination and return Unsupported. Messages describe
failure to certify within the finite profile; they must not assert the true
Hausdorff distance exceeds the budget. Invalid internal support or nonfinite
arithmetic is InvalidState. No silent full-detail fallback or retry is added.

The 2^24 witness lattice and outward arithmetic impose a precision floor: an
independent zero-distance control at large admitted coordinates leaves about
0.0133 m witness error. Depth/work caps can also reject equal surfaces. This is
a conservative finite search, not a universal convergence or tightness promise.
The limits bound added traversal state and evaluations, not total RSS, process
time or one meshopt call's cancellation latency. Fresh resource observations
must be distinct from F1d1 timings.

## Report and independent acceptance

Report schema 7/profile `f1d2-adaptive-root-proxy-gltf-v1` removes the misleading
root-proxy `comparison_pairs` and duplicate top-level `certified_error_metres`.
One typed `certificate` object contains `error_metres`, `patch_face_tests`,
`accepted_patches` and `max_depth`. These are performed evaluations, accepted
proof leaves and deepest accepted depth across both directions/regions; they
are not emitted geometry counts. Prepared certificate math copies into the
public report once. Full-detail report mode remains explicit. Rust, published
conversion.json, CLI and installed Python reports must agree. Active resource
drivers must migrate with the schema; historical receipts retain their pins.

Replace the old exact unpartitioned face-pair optimum check: it is not a lower
bound on a tighter subdivided certificate. The independent reader decodes
original/root supports and uses exact rational point-to-triangle distances on
its own canonical complete patch cover, accepting a patch only when that bound
is at most the published certificate. This independently proves surface distance
at most that scalar; it does not infer truth from producer counters. Exact
optimal target witnesses are no worse than the producer's explicit witnesses,
so this checker can stop at or before each producer accepted patch. With the
same exhaustive scan policy, its visited tree is a subset and required tests,
accepted patches and depth are no greater than the reported producer values.
Enforce checker work/depth/time limits and preserve sensitive refusal controls.

Required preimplementation probes establish exact four-child coverage,
convexity, rounded midpoint loss, lattice limits, spike/interior-hole/degenerate
and mixed-magnitude references, and finite scheduler accounting. Final acceptance
requires a materially tighter real producer result, independently re-certified
decoded artifacts, corrupted bound/profile/work/depth controls, authored-key and
full-detail/placement/PBR replay, pre-workspace base/dynamic/depth refusals,
mid-proof abort and producer failure preservation, installed adapters, bounded
resource observations and separate nonauthor review with exact source/artifact
pins. A private Rust test alone is not artifact acceptance.

The [CGAL bounded Hausdorff documentation](https://doc.cgal.org/6.2/Polygon_mesh_processing/index.html)
provides established context for subdividing candidate faces. This contract uses
its own explicit complete-cover witnesses and accounting; it neither invokes
CGAL nor inherits CGAL's numerical guarantees.
