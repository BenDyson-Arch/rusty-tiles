# Raw-only alternative: actual-consumer adjudication

This read-only addendum evaluates the coordinator's smaller alternative. It
supersedes the earlier review's preference for a global arbitrary_precision
feature. No production changes, Cargo run or new serde execution was performed.

**Prefer raw_value-only borrowed consumed records over global normalization or
arbitrary_precision.** Normalization can fix in-range payload integers but is
not transparent across actual C1 consumers. A bounded existing Value parse
followed by borrowed records is concrete and adequate for finite-f64 tokens,
including exact integers above u64, but its ordering has a specific unresolved
blocker for valid integer lexemes beyond f64. That case must be resolved rather
than silently labeled malformed.

## Why global normalization changes behavior

Existing serde_json Number variants distinguish native positive/signed integers
from F64. Number equality also compares matching variants only. Normalizing an
exact integral decimal/exponent to a native integer makes as_u64 succeed and
integer constant comparisons succeed. Keeping a true fractional value in F64
preserves rejection by those unsigned consumers, even if its f64 value rounds
to an integer. This correctly fixes in-range payload fields and direct targets.

But shared C1 parsing also supplies these actual consumers:

| Consumer | Effect of globally normalizing numeric variants |
| --- | --- |
| conversion.json budget bytes/vertices/maxPoints/tiles checks, validate.rs642/647/652/848 | Optional as_u64 checks previously ignored decimal/exponent variants; normalization activates them and can reject previously successful inputs. No new primary evidence here adjudicates that engineering report convention. |
| tileset extras encodedBytes/vertices, validate.rs658 | The same activation can turn previously opaque/ignored numeric forms into semantic counter comparisons. |
| implicit boundary Value equality, implicit/tileset.rs797–798 | Changes equality across numeric variants; affects structural bounds/error link acceptance. This slice must not silently claim new geometry semantics. |
| subtree integers, buffer==0 and availability constant checks | Normalization changes accepted literal variants and direct comparisons. These are outside the payload unsigned correction proof and full A2 remains held. |
| implicit serialized node admission, implicit/tileset.rs1144 | Normalized integer serialization changes measured bytes; converting 1e18 to its twenty-digit integer can increase bytes and change exact/one-over presentation acceptance. |
| payload document return, validate.rs393 | The sole current returned-document consumer is structural metadata schemaUri, a string. Payload opaque numeric values have no numeric downstream consumer and do not need global normalization. |

Restricting normalization to payload/table JSON would need a second numeric
mode in the common parser and still loses exact oversized-vs-fractional token
meaning. Native-integer normalization alone cannot distinguish exact
18446744073709551616.0 from fractional 18446744073709551614.9 once both become
F64; the former must meet the count/length/resource or reference/domain owning
gate, while the latter is InvalidInput as an integer declaration.

## Minimal borrowed actual-consumer representation

Retain ordinary owned Value and its numeric variants for the current generic
document/opaque consumer. After bounded JSON admission, deserialize selected
record arrays from original immutable bytes as borrowed RawValue slices. Numeric
fields are gathered once per record by a short serde visitor or ordinary
borrowed-record struct. Unknown fields are ignored in that borrowed view and
remain in Value. The raw source lifetime is the payload call; no boxed raw
strings, per-token copies, generic path registry, numeric marker, separate
full exact-number AST, duplicate dependency or global number feature is needed.

| Actual record | Unsigned consumed properties |
| --- | --- |
| glTF buffer | byteLength |
| bufferView | buffer, byteLength, byteOffset, byteStride, target |
| EXT_meshopt_compression view extension | buffer, byteOffset, byteLength, count, byteStride |
| accessor | count (including preflight), componentType, bufferView, byteOffset |
| mesh primitive | mode, indices, material; every attribute accessor reference |
| EXT_mesh_polygon primitive extension | count, indicesOffsets, loopIndices, loopIndicesOffsets |
| image | bufferView |
| texture | source, sampler |
| material texture-info records | baseColorTexture.index, metallicRoughnessTexture.index, normalTexture.index, occlusionTexture.index, emissiveTexture.index |
| b3dm feature table | numeric BATCH_LENGTH, checked against uint32 using its own schema authority |

bufferView target checks in polygon/attribute/index roles must consume the same
validated target, not compare the old Value directly. Accessor view stride must
consume the validated parent stride, not silently fall back after as_u64 fails.
All array references in the table use exact tokens before range comparison.
Uninspected node/scene/skin/animation/morph and metadata values remain uninspected.
Bounds use their actual finite component semantics; they are not unsigned fields.

The authoritative exact helper must retain a bounded classification of a token:
non-number/non-integral/nonzero negative versus nonnegative integral with a
native value or mathematically larger magnitude. The last case is a result for
the caller's gate, not a truncated/saturated replacement stored in Value.
Count/declared length beyond admitted work yields ResourceLimit; references
beyond actual arrays/ranges and values outside enum/uint32 rules fail their
domain checks. Fractional boundary lexemes remain InvalidInput. Zero is handled
before exponent magnitude, including -0 and 0e99999. Cancel trailing zeros,
bound exponent comparison, compare digit lengths and lexicographic magnitudes;
do not allocate powers of ten or bigint expansions.

Each selected record is fetched once; its actual selected numeric tokens are
borrowed once. Attributes with arbitrary semantic names should be visited once
as pairs rather than rescanning a large attributes object once per key. Known
material records have five fixed texture-info roles. All record/slice slots are
charged structurally by already admitted JSON values (<=65,536 default nodes).
Sum raw text length need not charge every overlapping slice as owned storage:
the bytes are borrowed from the one admitted <=8 MiB JSON source. Record-view
storage is O(value_nodes) slots plus actual parsed fixed fields, and total scan
work is O(bytes * maximum selected nesting levels) with a small fixed schema
nesting ceiling, rather than keys squared or components times JSON bytes.
Record arrays should be processed/released sequentially where dependencies
allow. Actual requested capacities and the number of retained views must be
shown by the tiny proof and final source; this is not an RSS promise.

## Structural raw-scan bound and parser-order blocker

Local serde_json RawValue ignore_value uses an iterative scratch Vec for
enclosing containers before any logical visitor sees their depth or nodes.
Recursively borrowing whole containers before logical admission can retain one
such decoder scratch capacity at every admitted level. A deep invalid input can
therefore require about (admitted_depth+1) * raw_scan_depth scratch, not one
linear scratch pool. Running the existing bounded Value parser first establishes
depth/nodes before the borrowed-record stage and avoids this amplification.
The derived stage then scans only admitted depth, but still owns temporary
scratch and may allocate record arrays; record those bounds honestly.

However, the existing Value parser with float_roundtrip rejects 1e400 with
NumberOutOfRange before its numeric visitor or any derived record stage. The
local de.rs f64_from_parts checks infinity at lines619–633; the ordinary Number
has only finite F64 or native i64/u64 variants. Thus a valid accessor count or
buffer byteLength 1e400 cannot reach its independently applicable resource
ceiling if Value is constructed first. The selected glTF integer rule does not
make such an exact positive integer malformed because f64 cannot represent it.
This is a static source/order blocker, not a newly executed defect.

Resolve this before choosing the complete representation. A justified larger
rework is to make bounded borrowed JSON admission the shared primary operation,
without constructing numeric Value scalars, and derive owned Value only at
actual consumers that require it. The payload could consume borrowed records
and return the currently needed schemaUri plus existing counters/exclusions,
rather than an otherwise unused owned document. That is an actual dependency
benefit, not an A2 fact framework. It requires a revised contract and proof of
unique keys, nodes, depth, syntax and bounded raw pre-scan work; it cannot be
introduced as an unreviewed second JSON interpretation. A shallow syntax-aware
depth preflight before RawValue scans may bound scratch, but is another concrete
parser seam needing proof and explicit compound-error precedence.

Alternatively, coordinator must give an independently justified truthful
disposition for numeric forms beyond ordinary Value representation, including
the actual component/resource fields. A valid-form Unsupported engineering
choice is different from malformed input, but the requested domain must not be
excluded merely to retain host/f64 implementation limitations. No saturation,
fake numeric objects or hidden fallback is acceptable.

## Coordinator tiny-proof requirements

The finite-f64 borrowed-record proof should include 3.0/3e0, fractional rounded
integer guards, exact u64 maximum and one-over, large finite tokens 1e100,
large exponent cancellation, -0, and exact reference/stride/target/table paths.
Include count and byteLength 1e400 to expose parser-order disposition. Check
opaque magic-key objects and unknown numeric fields remain ordinary unchanged
Value data, and no global serde feature is enabled. Pair duplicate/trailing/
depth/node and bytes controls; demonstrate borrowed scans run after the bounded
stage with storage/work counters. Numeric records must be complete, including
the preflight and target/stride bypasses, before any production coding gate is
marked ready.

## Revised contender: one iterative borrowed admission owner

The coordinator's later candidate resolves the Value-first ordering blocker
without global numeric features. Admit source bytes, obtain one borrowed root
RawValue, then validate that root with an explicit iterative worklist of raw
slice/depth pairs. A per-container serde visitor checks immediate unique keys
and gathers borrowed children. Charge a child's logical node/depth in its
DeserializeSeed before scanning/retaining its RawValue; processing an already
charged work item must not charge it twice. Drop the container deserializer,
duplicate-key set and scratch before descending to queued children. Scalar
values remain borrowed and numeric grammar is validated without f64 conversion.

This is a justified minimal representation: one borrowed source root plus
bounded pending/immediate child references, rather than a retained exact AST or
recursive scratch stack. Selected payload record views use the already checked
root; they gather actual numeric properties once. Returning the actual needed
schemaUri string with existing counters/exclusions removes a formerly owned
document whose only downstream use was schemaUri extraction. That concrete
consumer change is justified and does not introduce unused facts.

Before proof acceptance, bound requested scratch by admitted source bytes plus
Vec growth, combined pending/immediate child slots by the logical node ceiling,
and decoded duplicate-key storage by source bytes. Root skip performs one full
syntax scan even before logical depth/node discovery; do not claim it avoids
all scanning/allocation before those gates. Each subsequently processed admitted
container causes immediate children to be skipped at most once for that level;
full scan work is bounded by root scan plus (max_depth+1)*source bytes. Typed
depth failure may stop after the root full skip, with no recursive scratch
coexistence. Reserve/capacity arithmetic and slot multiplication are checked.

Stack children in reverse order for left-to-right depth-first processing, but
collecting every immediate sibling and duplicate key before descent still
changes some multiple-fault precedence relative to the old recursive visitor.
The revised freeze must explicitly choose this immediate-container admission
order; pair a root duplicate key with a deeply limited first child as a sensitive
control. Single-cause categories and exact valid admission boundaries remain.

Exact scalar helpers need a JSON numeric-type guard in addition to established
JSON grammar. RawValue can contain true, null, a string, array or object;
feeding those into digit-minus-zero arithmetic is invalid and can panic. The
coordinator's finite-f64 exploratory proof has this unproved assumption; add
count:"3", count:true, count:null, count:[3], count:{} controls before relying
on that helper. Invalid numeric syntax is the raw grammar owner's responsibility.

An ordinary owned-Value adapter may derive Value after this same structural
admission; that is a representation adapter, not a second duplicate/depth/node
interpretation. Its inability to represent valid 1e400 is separate from invalid
JSON syntax. A revised finite-number profile must classify that representational
exclusion truthfully as Unsupported, or a required finite-f64 consuming field
must own its semantic failure. Do not classify by serde diagnostic strings or
silently grandfather a blanket InvalidInput. The payload's borrowed exact
integer semantics must reach its resource/reference/domain gates regardless.

**Current design verdict:** approve this iterative raw-only contender as the
smallest justified candidate for actual serde proof. It has no remaining
fundamental representation blocker identified by this review. Do not yet mark
implementation-ready until its typed causes, storage/work controls, complete
record field inventory and actual-source tiny proof are durably inspected and
the revised frozen contract selects this owner/adapter distinction.
