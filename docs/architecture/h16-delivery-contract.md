# H16 streamed hierarchy and 3TZ delivery preparation

Status: design and test-only registered addressing/count preparation, not a publisher.
The initial inspection used `origin/develop` `0fe2e83`; preparation is now
based on `beedaec`. M1 was examined at `656e37f`, including core `5ffdc61`.
M1 and H8 must merge before publication wiring. Four addressing/count preparation
tests and all-target Clippy passed. No publisher, validator, archive, viewer or
resource acceptance is claimed by this preparation.

H4's independently reviewed metadata diagnosis adds a consumer integration
gate: Cesium 1.146 packs compatible structural properties into a texture whose
width equals table rows, and the pinned device limit is 8192. UINT64 scalar
properties still enter that path through a lossy GPU downcast. This does not
limit core glTF or establish a picking failure; CPU provenance must remain exact.
H8's maximum leaf triangle budget can exceed this consumer width. H8 owns any
early planned/charged per-primitive feature-table partitions, including both
source identity sets and local ID remapping. H16 must not split frozen leaves,
change their lattice or rewrite emitted GLBs to suppress a warning. Integration
must verify the chosen table layout and actual consumer picking/property access
before claiming full delivery acceptance. The source-bound diagnosis/review are
retained in `~/.cache/rusty-tiles-evidence/h4-property-budget-20261013/`.

## Ownership and retained boundaries

H1 owns immutable captured tree identities, rank ranges and exact welded minimum
leaf depth. H8 owns its frozen/versioned all-depth lattice, prepared attributes,
source picking tables, actual decoded GLB positions/bounds and leaf displacement
bounds. H16 consumes those outputs unchanged. It owns delivery layout, final
ancestor bounds, availability, paths, member counting and staged publication.

M1 remains the sole generated Tiles 1.1 JSON record/serializer boundary. H16 must
import the actual `tileset_node::{TileRecord, TileBox, NodePlacement,
GeometricError, Refine, ContentRef, MemberName, UriTemplate, ContentTemplate,
ImplicitTiling, TilesetDocument, SourceIdentity}` types after the owning branch
merges. Do not copy their implementations or create another tile JSON model.
`TileTree` is usable only for an explicitly bounded document/chunk, never the
whole capture. M1 `TileTree::preorder()` and the existing whole-tree
`implicit::tileset::write_forest` are not streaming pipeline implementations.

Retain `implicit::{Coordinates, SubdivisionScheme, Subtree, TileMetadata}` only
for one bounded subtree. Its existing schema/wire stores box/error FLOAT64:
retain that representation rather than introducing the earlier H12 design's
provisional FLOAT32 storage. Fresh C2 final-byte validation decides acceptance.
Retain runtime staging/publication semantics. The archive writer's current
resident entry/index vectors require a bounded spill replacement before H16 can
claim streamed 3TZ delivery. No work migrates retiring legacy mesh writers.

## Logical addresses and the bounded implicit forest

An H1 address concatenates the `cell_depth` spatial octants in `cell` and the
`path_len` synthetic octants in `path`, most significant group first. The slot
is `x + 2*y + 4*z`. Global depth is their sum, not truncated to 21. At accepted
height <=32, three u32 coordinate axes represent depths 0..31; a combined u64
Morton key cannot represent all 93 bits and must not be used. Parent identity
and prefix correspondence are checked. Existing H1 strict preorder key
`(first_rank, depth)` uniquely identifies a delivery node across jobs/run sizes.

A packed leaf occupies only its FIRST slot. Its last-octant interval and exact
rank range remain metadata; do not duplicate its payload into every covered
octant or create phantom siblings. Synthetic run slots are logical transport
addresses. Neither spatial centroid cells nor synthetic addresses bound the
triangles: final content and every ancestor require actual decoded bounds.

Use the already accepted bounded implicit forest: source nodes whose descendant
height is >=4 form an explicit structural skeleton. A non-leaf child with height
<4 becomes an implicit root with one subtree, `subtreeLevels == availableLevels
== height+1 <=4`, and zero child-subtree availability. Leaf children remain
explicit leaves; a whole short tree uses one implicit root. Every available
implicit tile has a metadata row, including its root. In an OCTREE L=4 chunk,
at most 585 tile positions and 4096 child-subtree bits are resident.

This avoids a demonstrated consumer defect: child-subtree placeholders are
culled using subdivided grid bounds before their actual metadata loads. Packed
and synthetic supports can escape those grid cells. Do not introduce a long
child-subtree chain or an implicit external-JSON content template. This forest
contains real implicit GLB templates and core explicit external-document links.
It does not restore a public `--explicit` exporter or maintain two producers.

Tile metadata semantics override derived tile values in the Tiles 1.1 contract;
see [OGC 22-025r4](https://docs.ogc.org/cs/22-025r4/22-025r4.html) and the retained
P-IMPL/C2 contracts. The metadata rows therefore carry the source node's actual
support, not a claimed clipped cell. This normative rule does not replace the
required actual viewer traversal proof.

## Frames, containment and errors

H8 payload positions remain in glTF-world Y-up, with its dataset-wide encoding
origin zero and unchanged meshopt/TRS bytes. Convert decoded coordinate bounds
once to Tiles-local Z-up via `(x,y,z) -> (x,-z,y)`. This signed permutation is
exact and preserves the local displacement bound. All forest roots, structural
nodes and external-document links inherit this common dataset frame. No payload
rebasing, per-root origin, additional glTF scene node or child RTC translation is
introduced. Geographic placement is one validated rigid anchor/orientation/
offset matrix on the dataset root, through M1 `NodePlacement::rigid`; retain the
placement boundary's existing units/order, not a second CRS implementation.

Precompute final snapped leaf bounds from the frozen H8 plan in a bounded pass,
then require the emitted GLB descriptor to match. Internal nodes union final
decoded children (and future own content), not H1 unsnapped centroid bounds.
Represent a box with f64 centre and half lengths, round every subtraction and
sum outward, and fold the ENCODED child endpoints into its parent. Recheck the
stored box after M1 JSON/FLOAT64 serialization. M1 `sum_up` is a retained helper;
its centred-box `grow_to_enclose` restriction must not force all tiles to centre
zero: use outward AABB union and `TileBox::offset` for tight common-frame boxes.
All available metadata boxes must nest and contain final decoded geometry under
the inherited root transform in merged C2 slice 3.

S3 has leaf payloads and empty structural parents, not invented coarse meshes.
Leaf geometric error is H8's outward source-correspondence bound. Structural
error is `max(1 metre, decoded-bound diagonal, child errors)`; this is a finite
traversal policy on an empty node, not a certified proxy error. Every parent
error is >=child errors; top-level error uses M1's existing policy. Actual
consumer checks must prove empty-parent traversal at declared near/far views.
If empty parents suppress reachable content, fix this owner before acceptance.
H17 later supplies real internal proxy certificates instead of reusing this
structural policy as a surface proof. Rigid placement/RTC rounding and all-camera
screen seam equality remain separate proof gates; local decoded seams are not
geographic screen equality.

## Streaming pass order and fixed publication plan

1. Capture/sort/tree/weld/lattice ownership stays H1/H8. Validate immutable rank
   closure and source identities. No source file reads after snapshot release.
2. Stream H1 preorder with a <=32-frame stack. Join leaf ranks and frozen lattice
   to compute predicted final bounds and leaf errors. Close frames into a fixed
   spill record containing identity, actual bounds, error and descendant height;
   external-sort back to strict preorder. Reconcile actual encoded leaf bounds
   later rather than silently rebuilding a lattice or splitting leaves.
3. Classify skeleton/implicit-root/explicit-leaf roles from descendant height.
   Freeze deterministic chunk roots and paths. A root name uses original
   `(first_rank, depth)`; relative coordinates are prefix-checked. Example GLB
   template: `implicit-content/f<rank>-d<depth>/{level}/{x}/{y}/{z}.glb`.
   Subtree templates include all OCTREE placeholders, even when its one root
   file expands to relative zero. Explicit payload names retain the same key.
4. Partition skeleton documents with a fixed structural limit: initially eight
   skeleton records/document, <=10 relative skeleton levels, and at most eight
   boundary children per record. This produces an EXACT document count before
   codecs; it is a chosen layout, not an estimate of optimal packing. Audit M1
   objects to prove <=128 value nodes/4 KiB per tile/link, so <=72 objects plus
   the shared document/schema head stay within 16,384 nodes, 2 MiB and depth32.
   The actual serialized document is checked too. Unknown schema growth is
   InvalidState, not an unplanned extra document or late member-count change.
   Perf acceptance may change these constants only through a new frozen plan.
5. Count all chosen GLBs, subtree files, root/external JSON documents and any
   explicitly selected debug members. Check the archive cap before leaf codecs
   or publication staging. During early scan, a proven leaf-count lower bound
   already exceeding the cap refuses immediately. Exact chosen counts are the
   final gate; do not refuse from a loose node/document UPPER bound.
6. Encode H8 leaves to staged members using its existing per-member byte budget.
   Verify content descriptors and join them to bounded M1 records. For implicit
   chunks use `Subtree` tile/content availability in level-major Morton order;
   property row order equals rank among AVAILABLE tile bits. Source identities
   remain in GLB `source_primitive`/`source_triangle` feature/property tables,
   including full-width original triangle/expanded/corner values. A `ContentRef`
   uses `ContentKind::Glb`, vector=false and `SourceIdentity::Features` with a
   canonical domain-separated digest of both original identity tables. Do not
   invent fake digests/byte lengths to create a serializer placeholder.
7. Serialize one bounded subtree/document at a time, release it, and append an
   archive-member spill descriptor. Metadata extras are fixed owned rank/depth/
   packed/synthetic context; large integer keys use decimal/hex strings in JSON.
   Do not store source authored names absent H8 capture. `encodedBytes` need not
   enter tile extras; keep actual byte counts in the descriptor/debug receipt so
   codec output cannot alter the frozen document partition.
8. Write the archive from the checked descriptor stream, root `tileset.json`
   first, deterministic remaining member order, index last. Spill MD5+offset
   index rows and central-directory descriptors rather than retaining Vecs for
   every member. External-sort the 24-byte index entries by the retained wire
   ordering; stream CRC/ZIP64 headers and directory in bounded buffers. Reuse
   audited codec primitives, not a new loosely interpreted ZIP framework.
   Close all spill/GLB owners before scratch close and final atomic publication.

The test-only registered preparation only implements real H1 addressing,
frame-bound conversion and checked member arithmetic. M1 records are absent on
this develop base; `publication_pending` is an honest typed gate, not a fallback
exporter. A compile/run claim requires coordinator registration and testing.

## Limits and failure semantics

The chosen cap is 178,956,970 TOTAL ZIP members, including the index, matching
the existing H1 gate. Indexed count excludes the index itself; indexed bytes
are 24*indexed_count and must fit u32. Count roots, links/subtrees and diagnostics,
not merely leaf GLBs. Reconcile this count convention against the merged #189
validator before integration; do not add another off-by-one container policy.
The H1/H8 temporary `nodes+2` count gate is not exact for this forest with empty
internals. The capture API must hand actual counting to H16 rather than treating
all nodes as payloads. This is an integration obligation, not permission to
bypass the real container cap with a huge configured value.

Keep existing height32, single GLB96MiB, document/subtree and source-domain
limits. Add no total triangle/vertex-size cap. Checked count/address/size
overflow or one-job memory/scratch exhaustion is ResourceLimit; actual container
ceiling is Unsupported; corrupt captured state or mismatched bytes/metadata is
InvalidState; user configuration/placement invalidity belongs to its existing
admission boundary. Source/codec/fidelity failure never retries by changing the
delivery tree. H8's physical-member Budget-only density policy remains separate.
Cancellation and I/O retain their typed meaning and secondary cleanup failures.

Before each pass, charge source-global retained handles, fixed record generations,
sort runs/fan-in/accessor windows, <=32 stack frames, one <=585-slot subtree, one
bounded document, one H8 leaf lease and the archive spool/index/directory buffers.
Stages lease sequentially; include tmpfs scratch in available-memory admission.
Compute physical allocation/file inventory before creating scratch/staging.
Record actual logical peak, sampled physical peak and measured RSS separately;
native codec workspace remains provisional until its measured acceptance.

## Independent acceptance and commit sequence

First commit: this contract and test-only registered preparation, after a fresh nonauthor
design/source review. Publication remains gated. After M1/H8 merge: enable the
module for production and add the actual typed bridge, bounded planner/chunk/document emitter
and spilled archive writer in reviewed bounded commits; coordinator owns builds.
Do not cherry-pick or fork M1 types to simulate satisfying the dependency.

Acceptance requires an independent raw archive/subtree/GLB reader, not producer
plan records as truth. Check full URI/rank/provenance closure, availability
parent closure and row order, 8-byte subtree framing/tail bits, bounds/errors,
FLOAT64 storage, root placement and identical GLB bytes across publication.
Sensitive controls: swapped Morton axes, packed duplicate siblings, truncated
synthetic path, missing root/content bit, incorrect metadata row, inward bound,
wrong default glTF axis conversion, double root transform, underestimated member
count, malformed index/ZIP64 and changed source identity. Exercise exact cap,
overflow, cancellation and injected cleanup failures without generating bulk.

Use tiny actual packed/mixed/synthetic-depth31 and ordinary spatial trees, plus
the >=10m textured spill run. Check deterministic complete archives across jobs,
run sizes, fan-ins and scratch locations. Run merged C2 containment on FINAL
decoded archive bytes, including child boxes escaping nominal centroid cells.
Rebinding accepted H8 geometry is allowed only on complete byte-identity proof;
it cannot accept the new hierarchy/placement/availability/archive layer.

The pinned user Chromium CDP consumer must load through the actual archive/server
bridge (not assumed direct `.3tz` loading), select target leaves through multiple
external documents and implicit roots, refine empty parents, and pick both
`source_primitive` and `source_triangle` labels. Decode selected metadata values
and compare original source IDs. Include anchor/heading/pitch/roll/offset,
reflected TBN, mixed-depth seams and boundary/near/far controls. Carry H8's known
Cesium colour-transfer limits; no producer compensation or codec-floor waiver.
Run the relevant perf comparison; hierarchy changes require their own acceptance.
Applicable exact-head CI/wheel/official Blender checks and merge remain coordinator
and user responsibilities. The 0.5 format transition does not enter this slice.

## Prior design status and remaining questions

H12 B1–B3 and H13 B1–B4 were fixed in revised PRECODE designs and independently
reviewed; the original external reviews describe historical drafts. H12 current
design `83f403f9` includes later H8 checkpoints; accepted decoder-origin revision
snapshot is `f1a09102`. H13 current `ed66c0e1` includes the later literal LEAF
normalScale correction; proxy normals still bake into the proxy frame/emit1.
See cache `h12-revised-independent-review.md`, `h13-revised-independent-review.md`,
the design revision audit, normal-scale delta and H8 producer acceptance.
Simplification, bidirectional proxy certification, baking and legal V/F topology
records remain implementation/acceptance gates, not already accepted code.

1. M1 merge plus H8 descriptor/capture count ownership must be wired explicitly.
2. Confirm #189 total-member convention and the streamed archive primitive owner.
3. Audit fixed per-document typed schema bounds and measure the initial eight-record
   partition's request/perf cost before selecting final constants.
4. Prove decoded containment and placement on final C2/consumer paths; resolve any
   empty-parent traversal or RTC precision defect at its actual owner.
5. H17/H13 need legal published V/F/reopening records before proxy integration;
   extend the shared schema through its owner, never a private draft extension.
