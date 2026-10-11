# C2 decoded-position and frame ownership — preparation only

This package proposes a bounded replacement-first foundation under #125/#120.
It is not implementation acceptance or permission to close a parent issue.
R2 PR155 exact-head acceptance and its develop merge must be pinned before C2
implementation. Independent combined semantic/math/resource/precode review is
also required. Full A2 remains held on its literal external-template prerequisite.
No partial implicit rewrite is selected by this package.

## Concrete claim and domain

Proposed check name: `staticDecodedPositionContainment`. It proves that every
draw-referenced position in the selected static scene, in every core mesh
instance, is contained in its owning tile, optional content volume, and all
explicit ancestor tile volumes after the applicable frame chain. Positions are
stored/decoded content, not a certificate of source fidelity, simplification
error, texture/material appearance, stroke width, displacement or all-time
animated/rendered extent. Cartesian convex-volume vertex containment also
encloses core linear segments/triangles; this is not polygon-topology proof.

Use the complete existing C1 POSITION byte domain: FLOAT VEC3; admitted quantized
signed/unsigned 8/16-bit VEC3 with the specified normalization; shared/interleaved
views; meshopt NONE; zero-initialized viewless accessors. Sparse remains C1
Unsupported. Indexed draws visit referenced vertices only. Unused vertices still
receive C1 integrity/extrema checks, but do not become fabricated drawn content.
Core POINTS/LINES/LINE_STRIP/TRIANGLES and C1's pinned restart LINE_STRIP profile
retain their meaning; restart commands are not position references. Repeated
indices and repeated instances are charged as actual draw visits.

Initial useful C2 proposal: explicit self-contained 3TZ, archive-local GLB/glTF,
modern b3dm; static core node instances; Cartesian box/sphere volumes. Region
containment and implicit effective addressing/semantic overrides have a named
C2 Unsupported disposition until separately proved. This is an intentional
finite new-check profile, not complete #125 or a change to C1 byte validity.
All required generated-output classes must be inventoried before adopting a
public default; present producers are observations, not the required domain.

## Scene and deformation policies proposed for adjudication

* Select explicit root `scene` when present. Without it, select the sole scene
  only when exactly one exists (explicit engineering policy). No-scenes library
  assets and multiple-scenes-without-default have no invented scene: C2
  Unsupported. Never automatically select scene0 or render every node/mesh.
* Validate exact node/scene/mesh references, scene roots, duplicate children,
  cycles and the strict-tree parent constraint over declared nodes. A mesh may
  be referenced by multiple distinct nodes; do not deduplicate those instances.
  A root node may occur in different scenes, but is visited only for selection.
* Selected-node skin or selected-mesh morph targets/weights are C2 Unsupported,
  including nominal zero weights. Any nonempty animations array is outside this
  document-static C2 profile; do not invent a playback state. Deformation
  validity is not certified by that refusal. Benign unselected skin/morph data
  remains unclaimed by C1 rather than silently rendered by C2.
* EXT_mesh_gpu_instancing, CESIUM_RTC and unknown actual positional extensions
  are C2 Unsupported. Core multiple-node instancing is supported. A required
  extension already refused by C1 does not acquire support through C2.
* Geometry-relevant core positions include root/scene/node/mesh/primitive,
  accessor/view/buffer and b3dm feature-table extensions. Classify those actual
  extension dictionaries with a finite primary-backed allowlist before a C2
  success. Quantization/meshopt retain their admitted meaning. Known feature,
  metadata/material declarations do not imply geometry-transform support.
  Opaque extras and extension-owned schema trees are not recursively mistaken
  for core geometry positions. Unknown actual geometry extensions cannot be
  accepted merely because they were optional or undeclared checks ran elsewhere.
* Tile/content/tileset metadata capable of positional or volume semantic
  overrides requires an effective-semantics owner; C2 does not certify the raw
  JSON volume instead. Until that owner exists, active such overrides are C2
  Unsupported. Ordinary nonpositional feature metadata stays uninspected.
* glTF is Y-up. The normative conversion is fixed, with no inferred axis from
  coordinates. Legacy asset.gltfUpAxis or another axis override must receive a
  named C2 disposition (proposed non-Y Unsupported), not a guessed transform.

Present null is distinct from absent default for every consumed field. Malformed
consumed reference/type/shape is InvalidInput; valid excluded capability is
Unsupported; exceeded work/capacity is ResourceLimit. Resolver and consumer
causes must remain causal. C2 preflight refusals precede expensive position work
where the immutable admitted JSON supplies the necessary facts.

## Frame meaning and narrow mathematical boundary

The selected glTF source has column-major local matrices, TRS order T*R*S and
parent*local global composition. Local node.matrix must be TRS-decomposable;
matrix and TRS cannot coexist. Local no-shear does not prohibit shear in a valid
cumulative product. Negative and zero node scales do not become malformed.
Quaternion/local-matrix admission is a concrete proposal for independent review:
finite quaternion components in[-1,1], nonzero, with represented squared norm
within1±2^-20. This is sufficient approximate encoding admission, not exact unit
certification. A rounded f32 unit quaternion has squared-norm perturbation at
most2^-22+2^-46<2^-21, so that window conservatively admits that useful encoding.
Use supplied components without normalizing. For a supplied local matrix, use
finite affine entries and pairwise squared column dot<=2^-40 times the product
of squared column norms, allowing exact zero columns. This is approximate local
TRS compatibility, not proof of exact decomposition. Uncertifiable/window-edge
ambiguity is C2 Unsupported; type/shape/zero quaternion/component-range failures
are InvalidInput. Containment itself has no spatial tolerance. Do not apply
these admission windows to cumulative products or silently repair source values.
See primary/gltf-2.0.1.adoc lines680–784 and primary/node.schema.json.

The pinned Tiles source, corresponding to web-observed OGC22-025r4 §6.7.1.6,
places the node transform before U(x,y,z)=(x,-z,y), then the tile transform.
Tile matrices are affine and have their own meaning; no glTF-local no-shear
restriction leaks into them. External tileset root transforms participate in
the ancestor chain. Region volumes are geodetic and are not tile-transformed.
The spatial-coherence requirement concerns child content, not necessarily child
declared volumes. See primary/3d-tiles-core.adoc anchors core-transforms,
core-external-tilesets and core-bounding-volume-spatial-coherence.

For modern b3dm, RTC_CENTER is added after node+U and before tile transforms.
Support absent (zero), JSON float32[3] and Feature Table binary byteOffset forms,
with exact bounds/alignment/little-endian float32 interpretation. Proposed JSON
RTC precision policy honors its float32 semantic datatype: parse finite source
numbers then explicitly convert round-to-nearest/ties-even to f32 and promote
those represented f32 values exactly to the math owner. Finite out-of-f32-range
JSON values are Unsupported; nonfinite binary float32 is InvalidInput. This
conversion is a named proposal requiring review, not an unrecorded cast or a
claim that C1 inspected RTC already. BATCH_LENGTH is not restricted to zero and batch
table semantics remain unclaimed. See modern-b3dm.adoc lines129–135,211–221 and
feature-table.adoc lines40–83.

Prefer evaluation in each own/ancestor authored local frame using forward suffix
tile products, rather than world-ECEF subtraction and inverse transforms. The
math owner proves closed membership and represented-rounding enclosures. It owns
all interval products, normalization quotients, box predicates, sphere predicates
and ambiguity/overflow decisions. Proposal: singular tile chains are C2
Unsupported (not asserted invalid by the affine primary); zero glTF node scales
remain supported. Region/geodesy and degenerate box policy are separate math
adjudications. Current 1e-6 relative tolerance and scale_bound are not retained
by regression tests.

## Actual-consumed stream and owners

Keep one C1 admitted source, physical view decode and accessor lifetime. Extend
that live owner with a static-scene stream consumed by the bounds checker; do
not decode again or retain a positions Vec. Conceptual finite events:

* NodeEnter: exact node index and local Matrix16 or TRS facts.
* DrawPosition: mesh/primitive/draw identity; three raw f64 components exactly
  representing stored f32 or admitted integer; normalization enum None,
  Signed127, Unsigned255, Signed32767, Unsigned65535.
* NodeExit: closes that node frame.

Codec must not round normalized integers or cumulative transforms into a new
point truth. Consumer maintains the outward frame stack. Every event is borrowed
or fixed-size; no public scene IR, path, publisher, reporter or runtime object
belongs in the codec. Stream callback failure is distinguishable from format
and resolver failure and cannot yield a partial successful report.

Current private value()/normalized()/Accessor and views are candidates for
actual reuse inside payload::inspect's lifetime, not an assertion that their
current summaries already prove frames. Current unwrap_b3dm returns only GLB;
rework it to retain the three RTC facts during the shared admitted table lifetime.
Do not add another framing/parser algorithm or weaken C1 BATCH_LENGTH/table checks.

Per-URI integrity facts may be cached. Per-content-reference frame/containment
verdicts may not. Current Check::payload's URI-only PayloadReport cache must
not skip a reused GLB under a different tile transform/bound. Either stream
again under charged read/decode work, or retain genuinely consumed decoded owners
under an independently bounded ledger; the smallest initial choice is stream
again. Counter/cache meanings must keep unique integrity and actual instance
visits distinct.

## Replacement inventory and public migration gate

Retain StoredArchive's exact named range/CRC reads, private JSON admission and
exact unsigned numeric owner for their accepted domains. Rework C1 payload
internals only where a position consumer/RTC fact has an actual caller. Replace
validator-local transform/point/volume/inside/contains/scale_bound decisions with
the independently adjudicated mathematical owner where C2 consumes them. Remove
dead arithmetic after migration, not by an unused compatibility wrapper.

The existing hierarchyBounds check is explicitly stronger declared-volume
containment, separate from decoded content coherence. Decide whether to keep it
as a separately named optional claim or replace its default rejection with the
normative content check; do not call it normative spatial coherence. C1's
payload counters/checks and existing exclusions remain truthful. Once accepted,
remove only the static-scene/frame exclusion actually proved by C2; retain
deformation, metadata, material and animated/rendered exclusions.

Public selection is unresolved and must be settled once: adding strict C2 to
default inspect deliberately narrows successful inspection for the named
Unsupported classes; a separate explicit check selection preserves old C1
success but requires a small intentional public contract. No success may list
C2 as checked when it was skipped. This package does not create that API.

## Remaining concrete gates

1. Independent math/resource package and this domain draft must agree on local
   matrix/quaternion validity, singular/degenerate/region dispositions, RTC JSON
   precision, suffix frames, caps and ambiguous boundary outcomes.
2. Decide public check selection, exact finite report facts and hierarchyBounds
   migration; inventory every accepted producer's selected-scene/frame shape.
3. Pin actual representation, source/decoder/stack/record capacities and drops,
   repeated-URI reads, node/draw/ancestor work. New instance expansion needs an
   explicit limit; existing accessor component limits are not that limit. Keep
   logical decoded bytes separate from decoder backing and process RSS.
4. Independently assess witness truth/mutation sensitivity, then review the
   smallest implementation proposal. No target execution is supplied here.
5. After R2 accepted merge, implement and prove final-source Rust/CLI/wheel
   report/error parity, corruptions, cap boundaries, relevant native decoder
   parity, final independent review and exact-head CI. Parent release gates and
   A2 conformance remain open.

The primary files and source observations are byte-pinned in adjacent manifests.
The full OGC HTML/PDF is not retained: web open exceeded its response limit and a
bounded12MiB PDF capture refused a larger response. OGC web observations establish
selected text only; the exact pinned repository source is not silently promoted
to a full published-document identity.
