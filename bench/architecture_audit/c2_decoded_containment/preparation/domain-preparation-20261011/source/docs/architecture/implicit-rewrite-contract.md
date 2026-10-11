# A2: explicit archive to implicit rewrite proposal

Status: draft, unproved; this document authorizes no implementation retention,
acceptance, legacy removal or release. It is the next operation audit under
[#126](https://github.com/BenDyson-Arch/rusty-tiles/issues/126), after the wrapping
and package facade slices. Source inspected:
`e3d222a4c27a86e1f06e1e4b47db4e6a4fa5e24e`. The architecture gate in
[README.md](README.md) applies. The accompanying
[ownership audit](../../bench/architecture_audit/implicit_rewrite/audits/ownership.md)
records concrete source observations. Separate [baseline probes](../../bench/architecture_audit/implicit_rewrite/README.md)
exercise one current vector fixture and sensitive controls; they do not accept
the replacement or prove this draft's broader semantic domain.

### Current A2 settlement checkpoint — 2026-10-10

Three fresh Sol 6.1 audit lanes operate on the byte-identical production in
accepted PR #150 merge `34a76152d18b02553691f472207225f116307a67`:
[semantics and independent fixtures](../../bench/architecture_audit/implicit_rewrite/a2_semantics/audit.md),
[real-source probes](../../bench/architecture_audit/implicit_rewrite/a2_real_sources/README.md)
and [captured-source/resource models](../../bench/architecture_audit/implicit_rewrite/a2_capture_resources/audit.md).
These supplement, rather than overwrite, the historical baseline evidence.
This checkpoint does not accept a production rewrite.

The four independent QUADTREE/OCTREE design fixtures each contain five source nodes,
translated child frames, a contentless routing node and multiple content slots.
They pass exact frame/availability/row/byte checks and reject twenty-two sensitive
corruptions. This is design evidence; current producer execution and actual
consumer equivalence remain separate gates.

Whole child-box containment inside a nominal implicit cell is not the chosen
truth criterion for padded-source admission. Primary spatial coherence concerns
contained content and permits overlapping child bounding volumes. Standard
TILE_BOUNDING_BOX metadata overrides the derived box; independently inspected
Cesium code consumes that override directly. Several exercised current vector
sources fail exact whole-box cell containment. Their capability must receive an
explicit disposition, and a rounding epsilon or producer label cannot supply
that disposition. An overlapping-box profile therefore needs every available
tile's effective bounds, authored error and actual content/ancestor coherence
to be independently established. The selected finite addressing profile assigns
each source child its source-array ordinal, with at most four QUADTREE or eight
OCTREE children. Every available tile has an explicit effective box/error row;
coordinates identify availability rows and resources, rather than advertise
spatial grid-query meaning. The primary specification's metadata override and
the pinned consumer's direct use of that override justify this as a finite
profile inference, not a universal implicit-tiling equivalence theorem. The
independent source-order fixtures include coincident boxes/centers and sensitive
owner swaps. Nominal cells and center uniqueness do not supply admission truth.

The terminal external-link template interpretation remains a specific open
format gate. Root-slot availability is zero in the design fixtures, and all
external links are terminal implicit leaves; the primary text also prohibits
external tileset content at an implicit root. The audit does not establish an
unconditional conformance verdict for templates whose unavailable root slot
would otherwise name JSON. Record and justify the finite interpretation, then
prove actual consumer selection and independent format acceptance, or choose
a different representation before implementation. Keeping the explicit
internal skeleton and making only terminal payload leaves implicit is a
different, partial-implicit product; it is not an automatic fallback.

Effective inherited REPLACE and imported spatial metadata must be resolved
before planning. Source tile extras serialized as a STRING have application
meaning, not a standard tile-extras semantic. Source metadata/classes, payload
feature metadata and extras each need their own declared preservation mapping.
Do not infer equivalence from the production expander restoring a field.

The captured-source lane executes ordinary POSIX identity/capture operations
and bounded Python ZIP/JSON/accounting/lifecycle models, including the control
that admits mixed bytes after same-length in-place changes with restored mtime.
Consequently stable input during capture remains a precondition, not an atomic
snapshot guarantee. Proposed ceilings are not measured Rust acceptance limits.
Source capture and candidate inspection must share the actual private format
owner through byte/handle inputs, not call a public path operation under a new
RunControl. The existing validator's stronger child-box containment profile
also remains distinct from a proof of decoded content spatial coherence.

### Format settlement and dependency-ready work — 2026-10-10

Accepted preparation PR #151 merged as
`9d2973db06967f87d759c87c34d8fdd0921c86ed`, with an exact reviewed merge tree
and all twelve applicable CI jobs successful. Its scope remains preparation.
The new [format/client decision](../../bench/architecture_audit/implicit_rewrite/a2_format_settlement/audit.md)
executes tiny QUADTREE/OCTREE materialization and sensitive controls. The client
preserves the owned-root frames and all slots, but also tolerates forbidden
root external occupancy and ignores present TILE_TRANSFORM metadata. Its
success therefore cannot settle the literal unavailable-root JSON-template
prohibition. The full A2 production replacement remains held at that gate;
the different partial-implicit product is not selected automatically.

The [metadata/provenance decision](../../bench/architecture_audit/implicit_rewrite/a2_metadata_settlement/audit.md)
settles exact root snapshots, application extras, incoming overwritten-control
reference refusal and historical-only vector state. Trusted admitted inventory
roles must receive real payload/reference/shape checks; report labels cannot
exempt arbitrary files. Current public C1 acceptance of those new roles remains
a separate integration gate. The decoded-payload lane settles a bounded exact
coherence profile; its probes do not establish production acceptance.

The next separately reviewable production prerequisite is the
[bounded stored-archive reader](archive-read-foundation-contract.md), with
current C1 as its real consumer. A tiny exact-binary control reproduces acceptance
of nested physical member records. Fixing that finite admission/ownership gap
and admitting owned Read+Seek inputs does not close the A2 standards gate or
remove any legacy operation. Final source/resources/lifecycle/viewer and
roadmap obligations below remain required.

## Required behavior and settled ownership

The operation rewrites an admitted explicit hierarchy while preserving the
original content bytes, their resource interpretation, placement, replacement
ownership, authored refinement errors and admitted metadata. Provenance labels
are evidence about the producer, not evidence that bounds, padding or content
are correct. A typed facade alone does not establish these invariants.

One F0 `Attempt` spans request validation, source/destination binding, source
capture, admission, planning, workspace writes, report/archive serialization,
required cleanup, seal and publication. Runtime owns control and completed-file
publication only. The consumer owns the source snapshot, semantic admission,
plan, member inventory and required typed report. The archive codec knows no
operation or jobs. The subtree codec receives validated availability and
metadata; it does not infer source hierarchy meaning or resolve paths.

Source binding uses a regular opened source and absolute native identities;
source/output path and file-identity aliases are refused. Capture and preparation
finish before a domain observer callback or output workspace is created. All
later source access uses owned captured bytes, never a source pathname. A stable
source during capture is a stated precondition, with observed identity/length/
mtime changes rejected; this is not an atomic snapshot against an adversary.
Callbacks may subsequently change CWD, rewrite/delete the source or create the
destination without changing the prepared conversion. Publication policy owns
the destination race.

No nested public `package()` call or fresh default control exists. The consumer
uses the private archive codec once under its original Attempt. The artifact
examined by final candidate checks is the artifact sealed/published, not a tree
that is silently packed a second time. Required workspace cleanup finishes
before `ready_to_publish`; failure prevents commit and reports retained paths.
Successful publication carries F0 cleanup diagnostics. There is no fallible
report/source reread, observer callback or signal check after commit.

## Representation to implement after proof

Use private admitted values, rather than mutate imported JSON with sentinel
keys. A bounded source member map owns bytes and archive names. A source tree
owns node identity, finite local axis-aligned bounds, finite nonnegative error,
validated transform, ordered content identities and explicit children. Imported
opaque extras and independently admitted extension data remain owned JSON;
their presence must not authorize lossy typed deserialization of external data.

The plan assigns each emitted root its own local frame, root bounds, content
slots, child identities, checked child translations, generated member names and
availability. Each child has one checked `ChildOrdinal` from source child order,
within the declared subdivision's branching count. Addresses have no independent
spatial-grid claim: every available row carries the source node's effective box
and error. Frame/coherence proofs use the actual emitted root and child values;
a global nominal cell must not be substituted for a later tight local box.
Root placement and child
translation are distinct values. The generated terminal link bound is expressed
in the parent frame; the external root retains the child's transform exactly
once. Content headers, URI templates and availability share one ordered slot
inventory.

One candidate is the current ownership pattern: a one-level root for a leaf;
a two-level root for an internal node, with original payload slots available
only at level zero and external child links available only in terminal level-one
cells. Child-subtree availability is zero. Each external child root owns its
replacement descendants, preserving parent proxy refinement. This candidate
requires independent format, bound and selection proof before retention.

Aliases must retain the original payload's archive parent directory so unchanged
relative GLB/b3dm resources retain their base. Construct the complete output name
inventory before writing: originals retained by the chosen resource contract,
aliases, external documents, subtrees, report and index. Reject exact and
file/ancestor collisions across source and generated names, including a source
member named `subtrees` and nested aliases. A prefix test or `target.exists()`
after workspace creation is not the inventory contract.

## Proposed public boundary; unresolved domain choices

The proposed facade is `implicit_to_archive(ImplicitRequest, &RunControl) ->
Result<ImplicitResult, JobFailure>`. Request owns source, destination and
`OutputPolicy`; result owns output, required `ImplicitReport` and cleanup
diagnostics. No Reporter, force field in domain options, generic ConversionResult
or adapter type enters the operation. Naming remains a proposal.

The report has a versioned profile and explicit integer meanings: captured source
archive bytes, captured member bytes/count, original explicit nodes and content
references, distinct original payloads, emitted external roots/subtrees, alias
bytes/count and published members. It records the declared subdivision and
resource-preservation policy. Rust, CLI, Python and conversion.json use the same
typed report. Source provenance remains separately identified imported data;
the final contract must choose how its exact bytes and references are retained.
An unconditional boolean claiming all content/source semantics were validated
is not a report substitute.

Subdivision resolves once at admission through a required
`ImplicitSubdivision::{Quadtree,Octree}` request value. CLI and Python must
require the same explicit intent; encoder labels do not select it. A root's
actual child count is checked against this branching domain once. Producer
branding is not a correctness test. The bounded source/extension/resource
profile must still be settled; this enum does not admit arbitrary archives.

Exact child containment without recorded-padding tolerances remains a useful
independent control, not the selected product admission criterion.
Existing point/vector producers can emit rounding, quantization and minimum
thickness padding. Their accuracy and use cases must receive explicit support,
rewrite or retirement dispositions before a replacement entrypoint can remove
the advertised operation. The selected ordinal/override profile preserves the
actual effective boxes rather than treating recorded padding as permission to
violate them. Independently decoded content and ancestor coherence, complete
effective metadata rows and actual consumer selection remain mandatory. No
default epsilon, label or source-report claim closes that proof.

Additional unresolved source admission: 1.0 versus 1.1 input/output declarations;
inherited versus explicit REPLACE; tile/content metadata and bounding volumes;
inline/external schemas; unknown extensions; contentless nodes, arrays, b3dm,
compressed/quantized content and GLB external resources. Initial exclusions are
profile decisions only after real use cases and preservation consequences are
recorded. Authored geometric errors are preserved, not certified as approximation
accuracy. Broader accuracy/format work remains with #120/#125/#122/#123.

## Finite admission and failure policy

Admission must precede ZIP-directory allocation and payload/tree expansion.
Use checked limits for source archive bytes, central directory/count, individual
and aggregate decoded member bytes, JSON depth/items/bytes, source nodes/depth,
content references, resource traversal, generated JSON/subtree bytes, aliases and
final member inventory. Account for duplicated payload aliases and repeated
document headers; a per-subtree bit limit does not bound the whole operation.

Proposed stress starting points, not accepted limits: 64 MiB source/captured
member bytes, 1 MiB manifest, 8 MiB aggregate parsed JSON, 4,096 source nodes,
depth 31, 16 content slots per source node, 16,384 final members, 64 MiB alias
bytes and 16 MiB aggregate generated documents/subtrees. A checked final byte
ceiling must include originals, aliases, report, documents and index. Measure
RSS, descriptors and scratch at boundaries, including maximum repeated headers
and shared payload references, before fixing these numbers. No worker pool or
claim of bounded total RSS follows from these proposed byte ceilings.

Use F0 kinds by meaning: InvalidRequest for conflicting intent/destination
syntax, InvalidInput for malformed documents/resources or observed capture
change, Unsupported for valid domain features/engineering limits outside the
declared profile, Io for infrastructure failures, Conflict for destination
installation conflicts, and existing Cancelled/ObserverFailure/InvalidState.
Preserve the first gate-selected cause. Do not classify by strings, turn every
ZIP error into a missing member, or treat unavailable metadata as zero padding.
Missing required source data and actual archive corruption are InvalidInput;
valid irregular hierarchy is Unsupported. These categories require adapter
parity and causal failure probes.

At the selected request boundary, an existing destination symlink/nonregular
entry is InvalidRequest; an inadmissible source leaf is InvalidInput. Malformed
unsafe/duplicate/file-ancestor source names are InvalidInput. A valid admitted
source name colliding with this finite profile's generated inventory is
Unsupported; duplicate generated names after an admitted plan are InvalidState.
These choices supersede the earlier model's provisional collision categories;
the final model receipt records its revised gates. They are not aliases for
publication's later Conflict outcome.

## Mandatory probes before production implementation

1. Pin primary 3D Tiles/subtree/metadata semantics and independent schema sources.
   Prove terminal external links, multiple content slots, metadata overrides,
   version declarations and transform composition. A patched upstream traverser
   is disclosed supplementary evidence, not an independent conformance verdict.
2. Author bounded explicit fixtures independently of current producers: parent
   proxy plus descendants; contentless routing; deep tight local boxes;
   translated children; midpoint ties; sparse slots; distinct regular children.
   Independently decode availability/property rows and compose world transforms
   without `implicit::expand_tileset`. Wrong frame/slot/translation/availability
   controls must fail the oracle while remaining plausible documents.
3. Run actual current explicit point/vector sources through the proposed plan.
   Independently decode stored geometry, node transforms and effective metadata;
   prove content containment in the actual source tile and ancestor frames.
   Record source-derived rounding/quantization/minimum-thickness observations
   separately from the rewrite's exact-byte preservation obligation. Cover
   compressed/fragmented arrays, b3dm,
   feature metadata, nested schema resources, extras and authored error handling.
   Record every supported, replaced or deliberately retired source use case.
4. Independently verify complete original/alias payload bytes and relative
   resource closure. Exercise duplicate references, source member `subtrees`,
   nested exact/ancestor alias collisions, reserved schema classes, orphan
   resources and unknown extension references before workspace creation.
5. Prove bounded source capture and output overlap through ordinary paths,
   symlinks/hardlinks and callback source/CWD mutation. Probe source changes,
   malformed/unbounded ZIP/JSON and resource ceilings before allocations/staging.

Only after these settle representation/admission should a production vertical
slice be written. Its acceptance additionally needs source/write/report/archive/
candidate/cleanup faults, cancellation and observer failures at every bounded
stage, reentry/concurrent attempts, destination races, exact report/inventory,
Rust/CLI/installed-wheel parity, independently measured resources and actual
viewer coarse/fine ownership/picking. Bind receipts to exact final source and
artifact identities. A separate nonauthor review remains required.

## Removal gates and remaining #126 sequence

Replace the current convert_implicit lifecycle and JSON tree mutation together;
do not layer the new facade over the old semantic transformation. Delete the old
options/two entrypoints and their CLI/Python routes only after the new declared
domain, explicit use-case dispositions, independent semantics and lifecycle
acceptance are complete. Legacy `output::Job`/Reporter remain until their other
consumers migrate. Shared subtree/metadata/archive utilities have independent
owners and are not globally removed by this operation.

The dedicated compression/replacement operation still needs its own decoded
equivalence, resource/source preservation and failed-replacement contract;
doctor/preview/support surfaces need explicit capability/application ownership.
Neither avoids the implicit operation's semantic prerequisite. There is no
evidence here that either must outrank the next implicit audit/probe slice;
sequence them independently where their dependencies permit. Remaining raster
and broader mesh lifecycle dispositions retain their domain owners. This draft
closes none of #126, #125 or the #113 release gate.
