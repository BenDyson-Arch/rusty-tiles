# A2 independent semantics, frames and availability audit

2026-10-10. Preparatory evidence only. Inspected source is the accepted F1d2
head `43d14b1`, also present byte-identically in PR #150 merge
`34a76152d18b02553691f472207225f116307a67`; [results.json](results.json)
records SHA256s of the eight relevant source/test files. This lane changed only
this subtree. It performed no Cargo/build, production modification, Git mutation
or release action. Current producer artifacts were supplied by the independent
real-source lane; their archive hashes are independently captured here.

## Primary references and authority

[sources.json](sources.json) pins the primary specification source and independent
schema documents to CesiumGS/3d-tiles commit
`4d781014b52294759834018a931223b98ac1ce47`, including exact downloaded byte
lengths and SHA256s. This is the same upstream identity named by the bundled
schema, but these primary files were separately retrieved, not inferred from
the bundled generated schema. Browsing also checked current primary text and
the [OGC 3D Tiles 1.1 publication](https://docs.ogc.org/cs/22-025r4/22-025r4.pdf).
OGC HTML retrieval failed because of its size; no conclusion depends on that
failed fetch. Relevant pinned primary documents are:

- [Core tiles, external tilesets, spatial coherence and transforms](https://github.com/CesiumGS/3d-tiles/blob/4d781014b52294759834018a931223b98ac1ce47/specification/README.adoc).
- [Implicit subdivision, availability, rows, template bases and binary format](https://github.com/CesiumGS/3d-tiles/blob/4d781014b52294759834018a931223b98ac1ce47/specification/ImplicitTiling/README.adoc).
- [Tile and content metadata semantics](https://github.com/CesiumGS/3d-tiles/blob/4d781014b52294759834018a931223b98ac1ce47/specification/Metadata/Semantics/README.adoc).
- [Metadata type system, defaults and noData](https://github.com/CesiumGS/3d-tiles/blob/4d781014b52294759834018a931223b98ac1ce47/specification/Metadata/README.adoc).
- [Subtree schema](https://github.com/CesiumGS/3d-tiles/blob/4d781014b52294759834018a931223b98ac1ce47/specification/schema/Subtree/subtree.schema.json),
  [availability schema](https://github.com/CesiumGS/3d-tiles/blob/4d781014b52294759834018a931223b98ac1ce47/specification/schema/Subtree/availability.schema.json),
  [implicit-root schema](https://github.com/CesiumGS/3d-tiles/blob/4d781014b52294759834018a931223b98ac1ce47/specification/schema/tile.implicitTiling.schema.json),
  [class-property schema](https://github.com/CesiumGS/3d-tiles/blob/4d781014b52294759834018a931223b98ac1ce47/specification/schema/Schema/class.property.schema.json).

Schema documents establish shapes, not runtime refinement equivalence or
geometric accuracy. No patched upstream traverser or production expander is an
oracle here.

## Settled meanings and remaining interpretation

The intended Cesium consumer was separately inspected at commit
`df52c781de3491a4b76839d420f7ca90a032efb6`, whose package version is 1.146.0.
Exact primary URLs and downloaded hashes are in sources.json. This is static
consumer evidence, not a browser execution or evidence about the historical
browser package. In pinned
[Implicit3DTileContent.js](https://github.com/CesiumGS/cesium/blob/df52c781de3491a4b76839d420f7ca90a032efb6/packages/engine/Source/Scene/Implicit3DTileContent.js#L637),
getTileBoundingVolume assigns the semantic bounding volume directly at lines
661-663, without clipping/intersecting it with a nominal child cell. Lines
472-478 remove the copied root transform before constructing implicit children
to avoid applying that transform repeatedly. The external-root transform
chain is independently established by the core specification and our exact
paired-source probes.

Refinement is inherited when a nonroot tile omits it. Geometric error is an
authored required nonnegative field, not inherited error or a certificate of
approximation accuracy. Top-level tileset error and root-tile error have distinct
selection roles. Admission must resolve effective refinement before requiring
REPLACE; source `check_tree` currently refuses omitted child refinement.

TILE_BOUNDING_BOX, TILE_GEOMETRIC_ERROR, TILE_REFINE and TILE_TRANSFORM identify
meaning through schema semantics, not property spelling. Present semantic
values override the corresponding fields; missing/noData values retain those
fields, subject to metadata type/default rules. A valid ADD override is outside
the proposed REPLACE profile; ignoring it is not preservation. An effective
transform is applied once. Admitting metadata without interpreting its spatial
semantics is unsound. The probe's five small precedence cases model these
rules; they do not implement general imported metadata admission.

Implicit subdivision derives nominal boxes and half-errors from the implicit
root; semantic tile rows may override them. Rows are ranked by available tile
indices. Content slots have separate availability streams and, where present,
separate ranked metadata tables. Content bound metadata does not appear by
automatically subdividing a tile box. Subtree child availability describes the
next boundary level. Zero child-subtree availability and availableLevels equal
to subtreeLevels make the modeled terminal link cells implicit leaves.

**Spatial coherence concerns content containment.** The primary core text
explicitly allows child bounding volumes to extend outside a parent's volume
when child contents remain enclosed. Thus whole child-box containment in an
assigned nominal cell is a conservative addressing profile, not a universal
3D Tiles validity condition. Padding beyond a cell is not itself an executed
format defect. Rejecting all such boxes would intentionally narrow current
use cases, and needs a product disposition.

**Chosen semantic policy recommendation: admit overlapping semantic boxes.**
The implicit text permits computed-box overrides, the semantic box is equivalent
to the tile's box, and the core content-coherence rule states no nominal-cell
containment requirement. Reading those rules together supports direct override
without a padding epsilon or whole-box nominal-cell gate. This inference is
corroborated by the pinned intended consumer's direct assignment. It does not
permit arbitrary incoherent contents: source and ancestor content containment,
finite effective fields and unique allocated availability addresses remain required. The
overlap model in results.json has content at (8.125,8.125,2), beyond nominal
child XY maximum 8 but inside its semantic maximum 8.25 and parent maximum16.
Clipping the override would lose that valid bounded content and fails the
model's control. No consumer execution is claimed.

The primary repository's
[bounds/subdivision discussion #777](https://github.com/CesiumGS/3d-tiles/issues/777)
and [Cesium issue #13195](https://github.com/CesiumGS/cesium/issues/13195)
show unresolved behavior when a root's inline metadata changes its subdivision
box. This profile avoids that case: inline metadata is omitted on generated
implicit roots, header/root-row effective boxes are identical, and every
available terminal cell has its own explicit semantic override. It does not
depend on inferring further nominal descendants from a tight metadata box.

External referencing tiles cannot have explicit children or cycles. Their
transforms compose with the external root. The modeled link cell carries the
child box translated into its parent's frame, no additional link transform;
the external root retains the child's original local transform. This yields
the source world placement exactly once, with all descendant replacement
ownership still under that root. A sibling proxy outside this ownership has
no equivalent guarantee.

The primary implicit-root text prohibits external tileset content at the root.
The current design interprets this as the actually available root content:
the link template slot has availability zero at level zero and references
external JSON only at terminal level one. This interpretation is consistent
with the separately stated availability and leaf-link rules, but the text does
not explicitly say whether the template's eventual target type is prohibited
even when the root slot is unavailable. **This audit does not promote that
interpretation to an unconditional standards verdict.** The coordinator must
record the finite interpretation and its justification, or obtain primary
clarification/change the representation before implementation. Schema success,
our model and a tolerant browser cannot individually close that textual gap.

The smallest clearly compliant byte-preserving alternative is a **different
partial-implicit product**: retain each explicit internal proxy and its explicit
child skeleton, and convert only terminal leaves to one-level implicit roots
whose templates reference payloads only. There are no implicit external-JSON
templates and source replacement ownership stays unchanged. It must be named
and reported as partial implicit conversion, not silently substituted for full
conversion. A full single-implicit-tree alternative could encode source local
translations in TILE_TRANSFORM metadata, but needs independently proved target
consumer support; baking transforms into GLB changes the required source bytes.
Neither alternative is selected here.

Generated manifests need asset.version 1.1. Explicit 1.0 input may be migrated
only after checking its features and documenting promotion; blindly copying
1.0 into generated core implicit output leaves the declaration unresolved.
Entry schema must include classes used by external tilesets; all generated
extension inventories must cover descendants. Relative template URIs use the
manifest base; subtree external buffers use the subtree base.

## Executed independent evidence

Run from the repository root:

```sh
nice -n 10 python3 -B bench/architecture_audit/implicit_rewrite/a2_semantics/probe.py
nice -n 10 python3 -B bench/architecture_audit/implicit_rewrite/a2_semantics/real_probe.py /tmp/rusty-tiles-a2-probe-artifacts-real-portable-final2
```

The first command verifies checked-in deterministic results and authored
fixtures. `--write` regenerates only this subtree. The second requires the
exact real-source archives named by its driver and verifies their pinned
results; those archives are not stored here. Runs have a ten-second budget,
small explicit node/member/byte ceilings and use `nice -n 10`. No CLI launch
occurred in this lane. Sustained expensive work remains coordinator-owned;
CPU constraints from AGENTS.md were observed.

The [QUADTREE](quadtree.json) and [OCTREE](octree.json) fixtures each contain
five independently authored source nodes, six tiny valid uncompressed GLB
point payloads, four terminal links and one contentless routing node. They
include a rotated/transformed root, translated children, deep tight local
boxes, two original payload slots, sparse terminal slots and authored errors
19/7/3/11/0. Emitted child addresses are hand assigned separately from the
explicit-tree oracle. Exact rational matrix multiplication, all eight box
corners, raw GLB float32 positions, binary subtree headers/alignment/LSB bits,
availability counts and ranked semantic FLOAT64 rows are independently decoded.
No epsilon or producer label enters the oracle.

Result: four fixtures pass. All twenty-two corruptions are rejected: wrong link
frame, wrong authored error, exchanged semantic rows, moved availability slots
with counts preserved, doubled external translation, changed payload identity,
available root external slot, copied 1.0 declaration, unexpected child subtree
and exchanged content slots. Several are structurally plausible documents
whose errors require source/frame/slot identity checks. The root-external,
version and child-subtree controls enforce the declared model profile; they
are not evidence of an actually observed production corruption.

The additional [QUADTREE source-order](quadtree-source-order.json) and
[OCTREE source-order](octree-source-order.json) fixtures assign ordinal addresses
from source child array order. Two different child identities have coincident
frames, boxes and centers, but distinct error/payload/descendant ownership.
Complete semantic rows and external root composition preserve both exactly.
Swapping their external owners fails the independent oracle in both schemes.
No source-derived center/cell choice builds these addresses.

The deep tight-box example demonstrates the representation mismatch by exact
arithmetic: the grandchild is in QUADTREE local slot 3 (OCTREE slot 7), while
the old root-derived cell convention gives slot 0 (OCTREE slot 4). This is an
executed independent design counterexample plus a static correspondence to the
current algorithm, **not an execution of that source through the converter**.
Zero-extent midpoint centers select the high side under the declared policy;
a positive-extent midpoint-crossing child fails conservative whole-box admission.

[real-results.json](real-results.json) separately audits eleven current portable
explicit point/vector archives and six supplied successful implicit outputs. All
six outputs preserve paired source/root/header/error values, payload alias
bytes, terminal boxes in the parent frame, original transforms exactly once
and center-selected local slots. Geometry is not decoded by this real-source
probe: compressed/quantized GLB positions, b3dm feature tables and feature
identity remain the real-source lane's independent obligations.

The paired inputs now come from the stabilized final portable run, whose
execution driver SHA256 is
`562ab065e41c538e7600ba7f5ca99e0fe1c16d044d9ffda191da7e9abf058643`.
The independently captured receipt hash is
`a813f58ea9903333f055ffbf36700644041d58011ed5f3557bb6ef640b875224`.
real-results records the frozen source/manifest/binary and all selected archive
hashes. Its driver verifies the upstream execution identity and equality of all
relevant inspected production hashes with the unchanged semantic audit. This
updates paired artifacts only; the reviewed four-fixture model remains unchanged.

None of the eleven sources satisfies whole-box exact cell
admission. Four shallow rounded/compressed/quantized fixtures exceed it by
approximately 2.59e-15 to 4.68e-15 metres; deep-tight exceeds it by 100.001
metres, fragmented-line by approximately 0.068 metres. Exact fractions for
every edge and archive hashes are recorded. Point-flat-rounded exceeds OCTREE
cell admission by 1e-6 metres because of minimum Z thickness. The five missing
implicit outputs match the real-source lane's reported domain refusals; their
causal command receipts remain owned by that lane. This audit observes absence
and does not independently execute those refusals.

| Exercised use case | Proposed semantic disposition | Observed/proof limit |
| --- | --- | --- |
| Flat rounded point, shallow vector, meshopt, quantization and both codecs | Support through overlapping-box policy | Six successful outputs include shared-resource case; source content accuracy/consumer codec evidence remains separately required |
| Shared payload slots and nested relative resources | Support principle | Same-base alias bytes and frames pass; full resource closure is separate |
| Deep tight point/vector and fragmented line | Rework admission against actual geometry and source-order addresses | Legacy whole-box/profile refusals do not prove invalidity |
| Point duplicate midpoint and fragmented circle array/b3dm | Support principle under source-order address profile when branch count fits | Independent raw boxes produce duplicate center slots, which no longer determine identity; codec/feature/viewer proof remains required |
| b3dm, restart, content bounds, tile metadata and external schemas | No blanket retirement | Codec/semantic/schema/resource-specific evidence before advertised replacement removal |

The fragmented line declares KHR_mesh_primitive_restart; maximal indices are
restart commands under the [pinned primary draft](https://github.com/CesiumGS/glTF/blob/9811e8407d4533500cfc6b10e3bc408345035a6f/extensions/2.0/Khronos/KHR_mesh_primitive_restart/README.md),
not ordinary vertex references. That draft requires its declaration and supports
only selected strip/loop/fan modes. General core glTF rules alone are an invalid
oracle for those admitted payloads; target consumer support is still required.

## Concrete representation and field decisions

**Choose source child order for availability addresses.** With mandatory
effective bounds/error rows on every available tile, no primary rule found
requires an overridden box or center to match the nominal Morton cell. The
override replaces the calculated volume; intended Cesium consumes it without
that comparison. Therefore explicit child array order maps to ordinals
0..children.len()-1, converted to one-level coordinate bit lanes. This is a
finite profile inference supported by the specification and consumer source,
not a new normative requirement. It admits at most four/eight children under
the selected scheme and preserves explicit order, identities and ownership.
No producer label, center tie, global nominal frame or padding claim determines
identity. Coordinates are availability/resource addresses; do not advertise
their nominal geographic cells as spatial-query truth for this profile.
Actual semantic boxes and decoded content/ancestor coherence own spatial truth.
The source-order fixtures independently establish the bounded representation,
while actual viewer and terminal-template assumptions remain separately gated.

Use private SourceNode identity, effective REPLACE, finite effective local
axis-aligned Box, finite nonnegative authored error, validated local transform,
ordered ContentId inventory and explicit ChildId list. Root placement permits
only the separately proved finite transform domain; initially translation-only
child frames make same-frame slot checks explicit. Reject overflow/nonfinite
derived values before planning; finite inputs alone do not establish this.

Each EmittedRootPlan owns its actual source-local box, placement, exact
one/two-level availability, ordered original payload slots, optional terminal
link slot and typed ChildOrdinal values allocated uniquely from source array
order within the requested branch count. Source-bound meaning is separately
owned by effective semantic rows in this same local frame.
The row sequence is increasing available index; source node identity is kept
separately from rank/address. Each link owns a finite parent-frame box, child
authored error and external document identity. External roots own descendants;
never reuse a global nominal cell to classify a newly tight root.

Generated schema class identity must not overwrite an imported reserved class.
Choose a collision-free private generated class name and expose its standard
semantic properties. Decode imported metadata at admission if supporting it;
otherwise valid unproved types/semantics are Unsupported, malformed instances
InvalidInput. Do not treat absent spatial metadata as zero padding.

Source tile `extras` copied into a subtree STRING named `extras` is merely an
application property with no standard semantic. Production expansion happens
to turn it back into tile extras; that is not portable extras equivalence.
Choose and document a source-extras application mapping plus source identity,
or explicitly narrow extras behavior. Do not claim arbitrary imported JSON
semantics preserved from the string's bytes. Per-content bounds/groups/metadata
need slot-specific treatment; feature metadata already inside unchanged payloads
still needs closure and consumer proof. Unknown extensions with resource or
spatial meaning cannot become admitted just because their JSON is copied.

The conservative exact-box profile is now a demonstrated design model, **not
the chosen replacement domain**. All eleven actual exercised sources fail it.
Adopt the justified overlapping-box semantic policy above if the coordinator
selects full owned-root conversion, then measure genuine decoded geometry and
test real viewer culling/refinement/picking before accepting implementation.
Source-order address uniqueness is product policy; neither provenance labels nor a
fallback epsilon supplies geometric truth. The unresolved external-template
gate independently prevents production readiness.

## Retain/rework/replace/remove ledger and implementation gates

| Component | Disposition | Remaining obligation |
| --- | --- | --- |
| Root ownership pattern, one-level leaves/two-level internals | Retain as a bounded candidate; rework representation | Terminal-template interpretation; actual coarse/fine viewer ownership and no skipped source payloads |
| Global-cell check_tree, JSON sentinel child indices | Replace | Source-order ChildOrdinal plan plus same emitted-root semantic frames; current real padded domain disposition |
| Explicit-only REPLACE check | Replace | Effective inherited and metadata-overridden refinement; ADD explicitly Unsupported |
| Standard tile bounds/error metadata | Retain principle; rework field admission | Semantic types/noData/defaults/transform/refine, source schema merging and finite arithmetic |
| Production subtree byte codec | Candidate retention only | Independent production bytes compared against these fixture truths, source/artifact binding; tests sharing expander cannot establish this |
| STRING extras restored by production expander | Replace equivalence claim; choose mapping | Document application metadata meaning and external consumer behavior |
| Ordered contents and unchanged same-base aliases | Retain principle | Slot headers/content metadata/groups, b3dm/external resource closure, collisions and amplification bounds |
| Producer-labelled scheme/padding admission | Remove truth authority | Explicit subdivision intent and finite independently verified numerical/resource domain |
| Current tests/current producer output | Rework as bounded acceptance cases | Independent oracle, sensitive controls, final artifacts and dispositions for every current use case |

Mandatory probe 1 is partially settled with pinned meanings and the identified
external-template interpretation gap. Probe 2 has independently authored exact
fixtures and sensitive controls; it is design evidence, not acceptance of
production or arbitrary metadata. Semantic probe 3 establishes actual frame,
slot, row, error and byte observations plus the failure of exact-box admission
for most cases. It does not certify source rounding budgets, stored content
spatial coherence, compressed geometry or arbitrary extensions. Current point
OCTREE document/frame evidence is included, but actual decoded geometry remains
separate; authored OCTREE fixtures do not replace it.

Before implementation, settle terminal-template interpretation, padded versus
retired use cases, effective metadata support and extras mapping. Before
legacy removal, add actual point/vector geometry/resource/metadata coverage,
production codec comparisons, independently bound capture/limits, F0 failure
and publication probes, Rust/CLI/installed-wheel parity and final viewer
selection/picking with a separate nonauthor review. Parent #113/#120/#121/#125/
#126 and release gates remain open.
