# A2 decoded payload and local coherence owner settlement

2026-10-10. This settles the bounded initial decoded-content policy and private
owner needed by the A2 preparation contract. The checkout baseline supplied by
the coordinator was `e4d90897518d2b569fb5e99b876581523e4c418c`; the coordinator
subsequently fast-forwarded it to the identical reviewed tree at
`9d2973db06967f87d759c87c34d8fdd0921c86ed`. The eight inspected production files
are independently byte-pinned in both result receipts. This is a design and
read-only model result, **not production implementation or acceptance**. No
legacy producer/converter, Cargo, Git, public inspection path or
`implicit::expand_tileset` was executed or imported.

The source/profile decisions below close the question of which facts and
arithmetic the first implementation must own. They do not close archive capture,
metadata/reference preservation, final-artifact resources, viewer behavior,
lifecycle or release gates recorded in
[the nonauthor preparation review](../a2_review/review.md).

## Executed evidence and scope

[probe.py](probe.py) independently reads six byte-pinned final2 source archives
per portable/native mode: rounded flat point, rounded flat vector, normalized
quantized meshopt vector, fragmented restarted line, GLB/trivial-b3dm contents
array leaf, and shared relative buffer/image/schema resources. It checks every
non-index member identity against the retained real-source raw receipts before
decoding. It uses the pinned meshoptimizer 1.3.0 reference decoder only for its
codec; [meshopt.mjs](meshopt.mjs) is a new small wrapper. No production or earlier
audit decoder is the geometry oracle.

[portable-results.json](portable-results.json) and
[native-results.json](native-results.json) bind the same final driver SHA256
`adcddbd0fec566da95c6ba5f0cd8f86dbfe22c5b7637f196f778b13b3d9ea62a`, wrapper,
codec, production inputs, six archive hashes and retained source driver. Both
executions inspect 23 selected primitive instances and make 54 own/ancestor
containment checks. Both pass 23 sensitive scene/content controls and six numeric
controls. The largest reduced numerator/denominator is 73 bits; the largest
conservative unreduced arithmetic/comparison preflight is 116 bits. The six
source checks use 2,487 rational operations including the additional independent
per-referenced-vertex oracle. Production needs only streaming extrema, not that
oracle's retained row lists.

External original inputs remain at
`/tmp/rusty-tiles-a2-probe-artifacts-real-{portable,native}-final2`; final execution
receipts are at
`/tmp/rusty-tiles-a2-probe-artifacts-payload-settlement-{portable,native}-final4/receipt.json`.
No generated archives or binaries are tracked here. These small read-only runs
were serial at `nice -n 10`; codec subprocesses have 20-second timeouts and are
reaped by `subprocess.run`. There were zero producer, converter and Cargo runs.
Any future converter uses `RAYON_NUM_THREADS=2`, vector `--jobs 2`, and the same
priority; sustained or near-limit work requires coordinator scheduling. The
model inputs are finite pinned fixtures, not a hostile-archive resource proof.

## What the current C1 owner proves

Static source observation: `src/validate/payload.rs` already owns checked GLB
framing, buffer/view/accessor layouts, bounded meshopt decoding, decoded component
min/max, normalization validation and primitive/index checks. Its `Accessor`
caches actual decoded raw extrema. The final `PayloadInspection` retains the
JSON document and inspection counters but discards those extrema and primitive
reference bindings. `value()` can read a checked POSITION element directly from
the still-owned decoded view bytes. This is the concrete reuse opportunity.

`caps()` repeatedly constructs default `ValidationLimits`, and
`src/validate/json.rs::parse` also selects defaults itself. Those are unsuitable
as nested A2 operation owners. Modern b3dm framing is inspected but its feature
table is discarded, so C1 framing success alone cannot admit RTC/batch semantics.
The inspector explicitly reports scene transforms and rendered bounds as
uninspected. Its public report still has `decodedContentBounds` in
`not_inspected`.

Separately, `validate.rs::node` checks child bounding volumes using a tolerance
and a stronger child-box containment policy. This can reject a padded source
box whose actual geometry is coherent. It also cannot prove actual geometry
inside an accepted box. C1 retains its existing public structural policy; A2
must not call the parent C1 pathname walker as its source or final-content oracle.
There is no executed C1 defect claim in these static observations.

## Concrete private ownership and rework

Choose one private format fact owner, provisionally
`src/payload_inspection.rs`, used by exactly the existing C1 inspection consumer
and the A2 prepared-attempt consumer. Rework/move the checked accessor/GLB/meshopt
core from `validate/payload.rs` into this owner; do not copy another decoder or
create a public geometry IR. C1's URI/path traversal, schema/report policy and
public counters remain C1's responsibilities.

The private owner takes immutable captured member bytes, a caller-owned bounded
JSON/decode/work meter and a resolver that returns captured resource identity
plus bytes. It returns a private causal format failure for the caller to map,
not a fresh `RunControl` or a top-level operation. Reuse one bounded JSON parser
with explicit caller limits, including unique keys and total items, for C1 and
A2; remove nested default-limit construction. Existing C1 default limits remain
its facade choice and report values, not an A2 reset. Existing JSON/schema and
public inspection checks remain required; this fact interface adds no shortcut
around them.

The minimal returned facts are the original bounded JSON document, original C1
counters, wrapper facts (table values and lengths), captured resource bindings,
and primitive POSITION facts keyed by payload identity, mesh, primitive,
POSITION accessor, optional index accessor and topology. Each primitive fact
contains raw component encoding/normalization, checked count and **decoded
referenced** component extrema. Retain neither all vertex rows, index vectors,
expanded triangles nor another payload copy. Checked whole-accessor extrema
continue validating declared min/max independently.

For unindexed primitives, stream all POSITION values into extrema. For indexed
primitives, stream the checked index accessor and use the existing checked
layout/value reader for each ordinary referenced POSITION; restart markers are
not vertices. Repeated indices need no deduplication allocation. Charge this
second reference-to-POSITION pass to the caller's work meter. Existing C1
`accessors_checked`, `primitives_checked`, `vertices` and `elements_checked`
semantics stay unchanged; additional physical read/decode work has a separate
private meter and cannot be hidden by unchanged report counts. Do not cache by
payload digest alone: equivalent external bytes require the same captured
resource/base bindings. A shared payload's facts can be reused while each
emitted occurrence retains its own source node/slot identity and frame.

Choose an A2-private local evaluator, provisionally
`src/convert_implicit/coherence.rs`. It consumes borrowed format facts and the
planner's source occurrence/ancestor identities. It owns selected static scene
evaluation, checked rational arithmetic and the own/ancestor containment verdict.
It does not own ZIP traversal, metadata admission, resources, publication or the
C1 report. Its arithmetic is private to this bounded policy, with no generic
public matrix/numeric API. Both source admission and final candidate inspection
use these same facts/evaluation rules over their respective captured/sealed
bytes; the independent acceptance oracle remains separately authored.

## Selected scene and bounded initial source profile

The published [glTF 2.0.1 specification](https://raw.githubusercontent.com/KhronosGroup/glTF/8e798b02d254cea97659a333cfcb20875b62bdd4/specification/2.0/Specification.adoc)
defines the selected default scene, strict node forest and parent/local transform
composition. The pinned [3D Tiles specification](https://raw.githubusercontent.com/CesiumGS/3d-tiles/4d781014b52294759834018a931223b98ac1ce47/specification/README.adoc)
orders glTF node transforms, Y-up to Z-up conversion and tile transforms. The
following is a deliberately narrower admitted profile, not a claim that other
valid glTF forms are malformed.

| Case | Initial decision and owner |
| --- | --- |
| Default scene | Require an explicit valid `scene`. Select only that scene's root nodes. Absent default scene is `Unsupported`; invalid references, duplicate parents, cycles, duplicate roots or child-as-root are `InvalidInput`. Validate the whole node graph even when a scene is unselected. |
| Node transform | Finite translation and strictly positive diagonal scale; identity quaternion; or an equivalent diagonal affine matrix. Reject matrix+TRS as invalid. Nonidentity rotations, shear, negative/zero scale or spatial node extensions are `Unsupported`. Nested positive factors are supported within the numeric bounds. |
| POSITION | Checked finite unnormalized FLOAT VEC3 or normalized UNSIGNED_SHORT VEC3 under declared `KHR_mesh_quantization`. Verify decoded raw extrema against declared bounds using the component interpretation already checked by C1. Apply normalization exactly as raw integer/65535. Other POSITION encodings and sparse accessors are outside this first profile. |
| Codec/topology | Uncompressed and bounded `EXT_meshopt_compression` with filter NONE; supported POINTS, LINES, LINE_STRIP and TRIANGLES topology/cardinality. Ordinary indices reference valid POSITION entries. Restart is restricted to the declared required pinned draft `KHR_mesh_primitive_restart`, LINE_STRIP, at least two ordinary entries per segment. Other codecs/filters/topology remain `Unsupported`. |
| Dynamics | No animations, skins, morph targets/weights, instancing or unknown spatial extensions. Valid forms outside the selected static profile are `Unsupported`; malformed ordinary references remain `InvalidInput`. |
| Wrapper | GLB or modern b3dm with feature JSON exactly `BATCH_LENGTH: 0`, no RTC_CENTER, feature binary or batch tables. This includes the actual array leaf. General b3dm feature/batch/RTC cases are `Unsupported`. |
| Tile frames/bounds | Translation-only descendants, positive finite axis-aligned source tile boxes, source-ordered occurrences and ancestor chains. Padded/minimum-thickness boxes are authoritative; there is no nominal implicit-cell fit prerequisite. Root global matrix is preserved separately. |
| Passive metadata/resources | Existing feature/structural metadata, polygon and unlit declarations can be retained only under separately settled metadata/resource admission. The local geometry evaluator does not interpret those application meanings or certify resource closure. Unknown geometry-changing declarations are `Unsupported`, even if opaque bytes can be copied. |

The six actual sources exercise identity nodes, nonuniform quantized decode
scales/translations, meshopt, restart, contents arrays and relative resources.
The model additionally admits nested positive diagonal T/S and an equivalent
diagonal matrix. It admits a valid unused POSITION outlier and an unused mesh
outlier because primitive references and scene selection determine the checked
geometry. A whole-accessor-envelope fallback would falsely reject them.

Sparse accessors, rotations, RTC and general feature tables are excluded because
their spatial interpretation has not been proved here. They require later
source-derived proof and an explicit profile revision; producer labels cannot
silently admit them. No advertised point/vector use case is retired by this
settlement. The retained real-source dispositions and remaining representation
gates in [the real-source audit](../a2_real_sources/audit.md) still apply.

## Exact local arithmetic and containment

Use a reduced signed-`i128` numerator and positive-`i128` denominator, both with
at most 127 magnitude bits. Convert the stored f32 POSITION and parsed finite
f64 transform/box numbers from their IEEE binary representations to exact
rationals; no decimal approximation or declared min/max is an authority.
Normalize u16 by the exact denominator 65535. Preflight shifts and input widths
before construction, and preflight/check every numerator product, denominator
product, addition carry, negation and comparison cross-product. Values or
unreduced intermediates outside this finite profile return `Unsupported` before
allocating or overflowing. There is no bigint, epsilon, f64 fallback or
post-overflow cancellation assumption. The explicit small-result/intermediate
overflow control demonstrates this deliberate refusal. A future reduction
optimization must preserve the same bound and be independently tested.

Per positive diagonal node chain, compose `parentScale * childScale` and
`parentTranslation + parentScale * childTranslation`. Apply the resulting T/S
to referenced component extrema, then the fixed signed permutation
`[x, -z, y]`. This produces an exact enclosure of every referenced stored
position for this profile; the probe checks it against a separate transformed
per-point list. Supported line and triangle interiors remain inside their
vertex enclosure. Unselected meshes/scenes contribute no rendered geometry.

For each payload occurrence, add its descendant tile translations into each
ancestor's local frame and compare against that ancestor's **original**
`center - halfAxis` and `center + halfAxis`. Check its own box and every source
ancestor. Actual referenced geometry outside an admitted authoritative box is
`InvalidInput`. If an arithmetic/profile limit prevents deciding, the result is
`Unsupported`, not invalid geometry. Missing metadata/address witnesses cannot
be replaced by a containment pass. Source root global transform is deliberately
absent from this local test.

Reusing the existing private mesh outward-interval arithmetic is not selected.
Those helpers belong to mesh quaternion/certificate reconstruction. Their
unconditional `next_down`/`next_up` on addition/multiplication cannot prove
`1 * 1 + 0 <= 1`, or exact `(21845/65535) * 3 <= 1`. Both are contained under
the rational policy. A third control has exact upper box bound
`18014398509481983/18014398509481984`: the position 1 is actually outside;
ordinary f64 rounds that bound to 1 and could admit it, while the outward
interval overlaps and can only return `Unsupported`. These are executable
numeric design controls, not executed defects in the mesh owner.

Use a caller-owned aggregate limit of one million arithmetic work units. Reserve
257 units before each add/multiply or nontrivial reduction (one arithmetic step
plus a conservative 256 Euclidean remainder steps on bounded 127-bit operands);
reserve one unit for each comparison and already-canonical input conversion.
The implementation must bound even normalization before doing it. At/one-beyond
work exhaustion is `ResourceLimit`. The Python model preflights integer width
and counts arithmetic/comparison steps; it does not execute this conservative
Rust work-unit accounting or certify allocation. Proposed
initial ceilings are 4,096 total retained primitive-bound records and selected
primitive instances, 4 MiB aggregate additional retained fact storage, and 31
node/tile levels for this evaluator. Count tables/references before allocating
and charge decoded view bytes, JSON, source bytes and reference-to-POSITION
reads to the shared inspection/capture budget. Rust layout/peak-memory and
at/one-beyond acceptance of these ceilings remain required before implementation
is accepted; the small census only shows geometric/numeric fit, not
maximum-resource acceptance.

## Preservation, integration and remaining implementation gates

The operation must copy each original payload/resource byte sequence exactly,
including GLB JSON, metadata, wrappers and external resources. Inspection facts
do not rewrite GLBs, normalize their document serialization or relocate RTC.
Scene/root selection and source child/global matrices remain unchanged. The
source's root/global transform is applied by the consumer in the same way after
rewriting. This preserves frame algebra, not producer CRS, survey/world accuracy,
source rounding or global matrix correctness. The control changing only that
global matrix passes local coherence and fails the separate preservation
equality obligation, showing why neither can certify the other.

Retain the existing checked decoder algorithms only after the private-owner
rework above is independently verified. Rework discarded component/reference
facts, wrapper facts and hidden default budgets. Keep C1 public report policy
and its structural bound check; remove A2's dependence on the parent C1 walker
as a geometry admission oracle. The new final inspector must use the sealed
candidate handle/captured resource bindings and actual output inventory, not
recapture a pathname or create fresh control. C1's special
`conversion.json.retainedContentUris` inspection of original payloads is not a
proof of every newly emitted occurrence, alias or final resource relation.

Before production acceptance, independently test the extracted C1 fact/report
parity, caller-owned budgets/first causes, selected-scene and reference extrema,
exact arithmetic/overflow, actual source and final own/ancestor coherence, and
source/global frame preservation. Combine those with the separately blocking
capture/ZIP/index/CRC, metadata/provenance/control-reference, emitted-alias
resource closure, final sealed inventory, lifecycle and target-consumer gates.
The planned finite policy is settled here; broader geometry is explicitly
excluded pending a separately supported extension rather than left ambiguous.
