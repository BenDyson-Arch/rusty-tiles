# F1d2 pipeline, finite work and ownership audit

Status: preimplementation recommendation against coordinator-supplied baseline
`3b5703231bf3991c2e37caef99989214308cc08d` (merged PR #149). No production edits,
Cargo executions or Git operations were performed by this auditor. Read
`AGENTS.md`, the architecture gate, F1d1 contract/implementation/nonauthor
review, and approximation/source/consumer/encoder/Rust/CLI/Python boundaries.
The source hashes in `../pipeline_probes/finite-scheduler.json` bind these
observations to the inspected bytes. This document is design/audit truth, not
acceptance of a future implementation or closure of #121/#113.

## Required bounded contract

Keep one opt-in root proxy over unchanged full-detail leaves. Keep existing
source capture, placement, identity/membership and positions-only opaque,
untextured core-PBR admission. Adaptive proof patches are certificate
construction only; they create no extra output faces, hierarchy levels,
source-triangle identities or appearance promises. The certificate remains in
decoded local f32 payload coordinates and is bound to precisely the positions
serialized. Root geometric error remains the requested positive budget after
proof; leaf errors remain zero; routing error remains separately conservative.

The recommended finite proof profile uses exact 2^24-denominator integer
source-patch barycentrics and maximum depth 24, confirmed with the mathematics
auditor. Every nonterminal patch has exactly four midpoint children. Process
them in fixed-order DFS, one original directed face at a time. Acceptance
requires a complete cover by proved leaves, in both directions within every
region. A missing child, partial target scan, partial source scan or a leftover
unproved leaf never yields a certificate. Depth 24 is a declared finite profile,
not a proof that every positive tolerance or coincident triangulation succeeds.
Target witness quantization and outward arithmetic can still cause refusal.

## Work admission decision

**Rework the old ceiling's meaning; retain its numerical value provisionally.**
Count each performed source-proof-patch/target-face bound evaluation against a
single checked global limit of 16,777,216. Count initial full-face evaluations
and every child evaluation, across both directions and all regions. No reset
per face, region, direction or recursive call. Check before performing the next
evaluation; arithmetic overflow or exhausted work refuses. Stream comparisons;
there is no resident face-pair matrix, breadth-first tree or retained collection
of accepted leaves. The F1d1 near-limit timing motivates retaining a familiar
finite amount of work, but does not establish the F1d2 operation's performance.
Interval reconstruction for source patches changes cost and requires fresh
measurements before making any resource observation.

**Retain separate base admission if the settled algorithm exhaustively scans
all target faces for each visited patch.** The recommendation, coordinated with
the math auditor, is that exhaustive minimum scan. The checked baseline
`2 * sum(original_region_faces * proxy_region_faces)` then remains a lower bound
on work for a complete successful proof, and rejecting a larger baseline before
certification avoids an attempt that cannot successfully finish even without
subdivision. Early proof/depth refusal can consume less than this baseline; it
is not a lower bound on failed attempts. The baseline must not be
reported as actual total work. Base admission follows validated candidate
construction because optimizer counts are only known then; it precedes proof
work and workspace creation. Adaptive dynamic work/depth refusal also occurs
before workspace creation, but after some in-memory preparation and comparisons.
Do not describe dynamic refusal as pre-work admission.

Early acceptance after finding one qualifying target face would be sound, but
would invalidate the baseline as compulsory work. There is no demonstrated need
to add that second execution profile here. If the coordinator deliberately
chooses early exit, remove mandatory full-pair base refusal and its lower-bound
claim together; do not keep an unexplained conservative admission restriction.

## Memory and cancellation truth

A DFS quaternary tree has at most `3 * depth + 1` pending patches while processing
one root, hence at most 73 at depth 24. A path needs at most 48 bits and each
corner needs three u32 barycentric numerators; no arbitrary precision or
ever-growing geometry allocation is required. Reconstruct the current patch's
coordinate intervals as bounded temporary data. Process original faces and
regions sequentially. A success counter records accepted proof leaves without
retaining them. Math/evidence auditors have settled independent exact-Fraction
complete-cover recertification at the reported bound: no production transcript,
proof payload or observer framework is needed. Exhaustive target scans make
its optimal exact bound no greater than the producer's explicit witness bound.
Failure never leaves a private partial proof in the published archive.

This bounds *added certificate traversal state*, not total memory. Capture still
admits up to 32 MiB per file, 64 MiB unique total, 64 requested resources; decoded
geometry still admits 100,000 source triangles and 4,096 leaves. The existing
8 MiB emitted-name budget remains combined across root and leaves. In-memory
geometry, region/membership vectors, component maps, proposal positions/indices,
meshopt internals, source/proxy face copies and per-content serialization are
separate resident costs. The work limit is not memory admission for those
allocations. Avoid a new RSS option or pretend global allocator bound.

Check the existing Attempt at entry, before/after proposal, every bounded
comparison chunk (at most 64 evaluations), and around patch/region transitions
that can otherwise do significant work. Preserve checks in component scans and
proposal packing. A single meshopt call still has no proved cancellation-latency
bound; neither DFS nor the evaluation cap fixes that limit. Subdivision itself
must not recurse/allocate without checkpoints. Cancellation/observer errors
propagate unchanged rather than becoming an Unsupported proof refusal. The
existing Attempt still owns first-cause handling, observer closure, cleanup,
archive sealing and publication; no certificate-local run or retry is added.

## API and representation decision

Retain `MeshApproximation::{FullDetail, RootProxy { triangle_limit,
max_error_metres }}` and paired CLI/Python syntax. Depth/work/witness precision
are this finite implementation profile, not user options. There is no new
approximation option bag, alternate fallback certificate, loose mode or legacy
report adapter. A tight request can fail even when true surface error is zero.

Rework core validation to return a private validated approximation choice,
`ValidatedApproximation::{FullDetail, RootProxy(RootProxyLimits)}`, including a
privately constructed validated root request with private fields and one
fallible constructor. `NonZeroUsize` is a useful triangle-limit representation;
an extra positive-f64 framework is unnecessary. Lower candidate and
certificate stages consume that value; they must not receive raw request
scalars and repeat validity policy or rely solely on debug assertions. Placement
resolution remains separately owned once. Source-dependent material/attribute,
component, candidate-index and nonfinite arithmetic checks remain where actual
data first makes their answers knowable; those are not repeated request policy.

Use one certificate-owned result on the prepared root geometry: actual bound,
performed evaluations, accepted proof-patch count and deepest visited depth.
Do not store two copies of the bound/counters or a second optional certificate.
`PreparedRoot::FullDetail` versus `PreparedRoot::Proxy` remains a finite enum;
serializers consume a certified root plan without mode validation or reproof.
Metadata regions and candidate output faces remain producer-owned, while proof
patches and counters are certificate-owned. Accepted patch count is the sum
across both directed proofs and is not an output geometry count.

Remove `comparison_pairs`: it currently promises exact equality with
`2 * sum(S * T)` and cannot describe adaptive visits. Set schema 7 and profile
`f1d2-adaptive-root-proxy-gltf-v1`. Use one typed public `certificate` report
object with actual error and deliberate fields `patch_face_tests`,
`accepted_patches`, and `max_depth` (observed deepest visited patch, not the fixed
depth ceiling). Remove the old parallel top-level certified-error field if the
bound enters this object. Base evaluations remain private admission arithmetic;
the report need not mirror every internal value. The algorithm identifier and
fixed parameters are already identified by the profile; do not repeat them at
several report levels. Keep actual certified bound distinct from the emitted
requested geometric-error budget. Convert the internal pure-math result to the
public report once at the consumer boundary, without importing public consumer
types into the certificate module. Test exact report shape and
propagate it through returned Rust report, `conversion.json`, CLI JSON and
installed Python result. The current legacy `src/report.rs` owns other converter
events; no evidence calls for routing this mesh report back through that API.

## Concrete ownership change

**Retain the approximation producer and add a private certificate child module.**
This is a demonstrated dependency separation: changed patch arithmetic and
coverage/accounting can be independently exercised with face supports and a
validated error budget, without constructing source materials, node membership,
authored-key connectivity or invoking meshopt. `approximation.rs` keeps source
eligibility, original-key components, budget allocation, untrusted proposals and
`ProxyGeometry` assembly. `approximation/certificate.rs` owns exact patch
representation, interval witnesses, region-directed traversal, finite accounting
and the one certificate result. It imports no source identity, material, meshopt,
encoder, filesystem, job paths or frontend types; a cancellation closure is
sufficient. Error propagation through the existing core JobError remains the
bounded consumer boundary and does not justify a general runtime framework.

Moving the whole mesh consumer or splitting candidate generation into additional
facades has no demonstrated benefit in this slice. Domain-neutral face supports
passed to the child module avoid a reverse import from certificate math into
prepared source geometry. The changed witness primitive must accept exact patch
corners/intervals; retaining the old f32 point signature would certify rounded
points instead of the declared exact source cover.

## Failure semantics and evidence matrix

| Trigger | Required result | Owning boundary and evidence |
| --- | --- | --- |
| Zero limit, nonfinite/nonpositive budget, malformed pairing | InvalidRequest; core validation before source I/O where invoked | Shared Rust/CLI/Python invalid matrix; syntax pairing remains adapter-owned |
| Unsupported attributes/materials, absent actual reduction or impossible component count | Unsupported, no workspace or publication | Existing producer eligibility replay; source-key and membership controls |
| Checked base evaluations exceed cap | Unsupported before any certificate evaluations/workspace | Exact counts and cap-boundary admission tests; absent output parent |
| Dynamic evaluations exhaust cap | Unsupported: finite proof work exhausted; no claim true error exceeds budget | Direct certificate counter test and real producer refusal reaching this gate; no parent/staging; sensitive cap control |
| A patch fails at maximum depth | Unsupported: cannot certify within bounded depth, even at zero true error | Exact independent positive surface reference plus refused proof; depth-boundary control |
| Nonfinite interval or inconsistent supports/index/state | InvalidState, preserving the data/arithmetic invariant failure | Arithmetic stress, corrupted internal-input tests, no published partial result |
| Cancellation/observer abort during proof | Existing Cancelled/ObserverAborted, first cause preserved | Controlled mid-proof checkpoints before workspace, bounded evaluation-gap observation; no meshopt latency claim |
| Late encode/write/archive/cleanup/publication failure | Existing F0 primary/secondary/retained-path/recovery semantics | Concrete producer seam replay, Replace preservation and cleanup checks; one Attempt |
| Successful proof | Complete two-direction per-region coverage and exact serialized position binding | Independent Fraction complete-cover recertifier at reported bound, omitted-child/false-bound controls, source/root digests |
| Successful report | Counts describe performed proof work and output counts separately | Exact schema checks, corrupted bound/count/profile controls, installed adapters |
| Resource behavior | Finite evaluations, bounded added traversal state; observed total resources only | Fresh frozen optimized artifact, near-cap accepted/dynamic-refused runs, time/RSS/scratch/descriptors; sampling limits stated |

Refusal messages must say what the bounded proof could not establish. A loose
bound or exhausted profile does not prove that the true Hausdorff distance
exceeds the request. No partial successful conversion, full-detail fallback,
new identity or post-refusal workspace is permitted.

`../pipeline_probes/finite_scheduler.py` was executed independently of Rust. Its
scripted-acceptance DFS model covers exact-cap success, pre-evaluation base
refusal, admitted-base dynamic exhaustion, depth refusal, complete four-child
counts and a cancellation checkpoint no later than the next 64 evaluations.
The receipt explicitly states that these are accounting-model observations,
not execution of production geometry, F0 or adapters. It supports the proposed
representation; coordinator builds and independent final-source review remain
necessary. Final evidence must additionally address the new proof oracle:
the old whole-face ideal bound is not a valid lower bound on an adaptive result.
