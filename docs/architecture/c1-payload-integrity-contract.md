# C1 payload integrity and explicit JSON admission

Status: all A1–A14 prerequisite dispositions are incorporated for the bounded
implementation. The final frozen revision requires a separate nonauthor
incorporation decision before coding; final production acceptance remains
separate. Independent selected-primary controls demonstrate six existing
outcome defects; no implementation acceptance is recorded here.
Initial inventory baseline: `c8cbfb187de863ae21fe904275794894aa0a8381`
(historical). Accepted reader merge and actual rebased implementation baseline:
`0eebdeebf013591460e6ac77485ad91b7401aada`, complete tree
`58aadfdc1bb78c507d020ab91494dc83a3d646a3`. A13 is cleared by coordinator
acceptance and separate retained-evidence verification; A14 selects the corrected
iterative borrowed-JSON mechanism.
This is the next bounded foundation under [the architecture gate](README.md),
not a new geometry profile. Its live consumer is C1 `Check::payload`, ordinary
document inspection and implicit structural presentation. Its outcome is one
private payload-integrity owner and one private explicit-limit JSON owner,
with C1 selecting its existing fixed profile once. Preparation and the immutable
independent next-scope audit are in
[c1_payload_integrity](../../bench/architecture_audit/c1_payload_integrity/README.md).

## Required domain and claim

Preserve the entire admitted [C1 profile](../VALIDATION.md), its successful report
meaning and its InvalidInput/Unsupported/ResourceLimit/Io distinctions. Existing
code, fixtures and historical digests identify candidates to retain; independent
format rules and explicit engineering profile choices govern acceptance.
Unproved behavior is recorded, not grandfathered or silently corrected.

The private owner checks immutable supplied bytes: GLB 2.0 framing, archive-local
glTF 2.0, modern 28-byte b3dm v1 framing/table JSON/embedded GLB, actual buffers,
views, accessors, finite values, declared bounds, attribute agreement, ordinary
indices and the admitted draft restart rules. Preserve data buffers for both
standard-base64 media types, zero-initialized viewless accessors, quantized
attributes, matrix column alignment including omitted final padding, meshopt
NONE decoding and required-meshopt URI-less placeholders. Preserve supported
optional extension declarations and opaque application `extras`/extension-owned
objects. Preserve the existing bounded optional polygon-reference checks without
claiming full polygon topology.

C1 admits core POINTS, LINES, LINE_STRIP and TRIANGLES, with its existing finite
topology/cardinality rules. Pinned draft primitive restart is required and used,
indexed LINE_STRIP only, with at least two ordinary vertices in each segment;
empty/singleton segments remain Unsupported, not declared invalid by the draft.
No A2 restriction to positive diagonal transforms, selected scenes, a narrower
POSITION encoding, BATCH_LENGTH-zero wrappers or translation-only tile bounds
may leak into this owner. Scene/rendered geometry and metadata semantics remain
uninspected. Full A2 remains held on literal external-template conformance; this
slice neither changes the implicit product nor supplies a partial fallback.

## Independent authority and proof limits

The selected core authority is the published glTF 2.0.1 registry specification,
source revision `8e798b02d254cea97659a333cfcb20875b62bdd4`, not unpublished main.
The preparation [reference plan](../../bench/architecture_audit/c1_payload_integrity/reference-plan.json)
records exact references for quantization, meshopt, modern b3dm, primitive restart
and the independently sourced meshoptimizer compressed/decoded golden bytes.
The separate owner freshly fetched/read all five selected raw references and
three pinned b3dm table dependencies; its
[dated verification](../../bench/architecture_audit/c1_payload_integrity/primary_controls/2026-10-10-pinned-reference-verification/verification.md)
retains raw bytes, retrieval receipts and exact golden literal comparisons.
The nonauthor reviewer independently checked all eight source identities/read
the relevant rules and closes A9 provenance. Golden identity is static source
provenance, not a new decoder execution or broad codec acceptance.

The independent primary-controls owner verified the registry header's published
revision and fetched that exact source: 148,064 bytes, SHA256
`10aebb6a8362a155b196448912bf6250bf046f150431fa73c6cd657c44185aff`,
byte-identical to the locally read cache. The fetched pinned accessor schema is
6,517 bytes, SHA256
`86f9aacb0b616e1f6a1d5b8ff81e92294602e9e3c98c7f13d2cefdf987c87e35`.
[The independent receipt and raw bundle](../../bench/architecture_audit/c1_payload_integrity/primary_controls/README.md)
retain provenance; the complete registry HTML fetch returned 403, so only its
web-observed header and the retained exact pinned raw source are claimed.

That owner executed fourteen tiny read-only controls against portable binary
SHA256 `3328487c2920e07b291eebec8072954f11b0d20a39181ccbc0c4287e2f765524`,
source `5c68f7b52b72e2f6e72096e930f85d27f2897448`, source-manifest SHA256
`913e15540cfe4b387905fdaf00aff270614285ba88d9e12acfe4ffccaa18aa13`.
The four inspected C1 source hashes equal this contract's baseline. Five valid
integer-valued decimal/exponent controls (count, componentType, POSITION
reference, buffer length, combined local glTF) and one valid viewless nonsparse
arbitrary-bounds control were rejected as InvalidInput. Stored/viewless zero
positives passed; stored-zero arbitrary bounds and fractional integer controls
failed as required. These are independently demonstrated finite outcome defects,
not merely source exceptions or a valid Unsupported profile choice. Correct
their owning integer interpretation and declared-bounds rule before acceptance;
legacy tests cannot veto the corrections. This author inspected the receipts
and pins, and did not author or execute that independent lane.

The fourteen controls do not establish universal glTF conformance or complete
C1 acceptance. The original contract snapshot and freeze remain in
`bench/architecture_audit/c1_payload_integrity/history/20261010-initial/`;
the revised freezes bind subsequent independent records separately. The
post-primary snapshot is preserved under `history/20261010-post-primary/`.

## Demonstrated correction invariants

Integer semantic properties admit mathematically integral JSON decimal/exponent
lexemes within their checked signed/unsigned and host/domain range. Reject
nonzero fractional mathematical values as InvalidInput **before** binary float
rounding erases the fraction. The independent controls
`1.0000000000000001` and `10000000000000001e-16` both round to f64 `1.0`;
`as_f64().fract() == 0` alone cannot enforce this rule. Select exact unsigned
interpretation of **borrowed actual consumed numeric lexemes**, after shared
borrowed JSON admission. Classify a numeric token as native nonnegative u64 or
exact integral Overflow; nonnumeric, nonzero negative and nonzero fractional
unsigned declarations are InvalidInput. Overflow carries mathematical range
meaning to the caller's work/reference/enum/uint32 gate; it is neither a syntax
error nor a saturated native value. Handle zero before exponent magnitude,
cancel trailing zeros, compare decimal digit lengths, then accumulate with
checked arithmetic. Bound exponent comparisons by admitted token length and
needed native/domain range; allocate no power-of-ten expansion or bigint.

Use `raw_value` without `arbitrary_precision`, global number normalization,
numeric marker objects, a sidecar path registry or another full DOM. The
private borrowed document has one immutable admitted source lifetime; actual
record views gather each selected numeric field once. Visit arbitrary attribute
name/reference pairs once, not once per key by rescanning an entire object.
Release record-array views when dependencies allow. Preserve field presence
separately from its raw value: absent optional offsets may default, while a
present JSON null remains an invalid numeric type. Do not derive
`Option<&RawValue>` where serde's null-to-None conversion would erase that
meaning. Apply presence-aware checks to all actual optional consumed fields,
including offsets, indices, URI/extension/schemaUri fields. Add paired absent,
null, correct-type and wrong-type controls at these boundaries. `RawDocument`
retains only the checked borrowed root and its admitted source lifetime, not an
owned payload DOM or speculative facts. Opaque payload numbers are
not materialized merely to reach schemaUri. Ordinary owned-document consumers
keep their existing numeric variants and opaque marker-object behavior.

Every actual consumed unsigned-property path uses the same exact interpretation,
including accessor-count preflight, byteStride fallback, and direct numeric target
comparisons as well as `uint`/`field`/`off`/`at`. Core, quantization, meshopt,
restart and optional polygon counts/lengths/offsets/references are included;
currently uninspected scene/node fields gain no new semantic claim. Numeric
b3dm BATCH_LENGTH independently follows the pinned table schema's JSON Schema
2020-12 integer rule before uint32 admission, so `0.0`/`0e0` is not malformed.
Its current `as_u64` is a separately established static correction seam, not an
additional executed defect in the fourteen controls. Compare exact numbers to
the consuming domain before lossy host conversion: a valid huge accessor count
exceeds work admission as ResourceLimit, while an out-of-array reference is
InvalidInput. Final source must demonstrate this classification at every
consuming gate before lossy host conversion.

Where an accessor has neither sparse nor bufferView, preserve zero-initialized
values for integrity/index/attribute checks but apply the published arbitrary
declared min/max exemption. Declared arrays still require the applicable finite
number/component-shape checks. Stored binary accessor extrema still agree with
declared bounds, so an identical arbitrary bound on actual zero bytes remains
InvalidInput. Missing referenced buffer/BIN bytes remain distinct. Arbitrary
declared viewless bounds do not change decoded zero values and do not establish
rendered geometry or decoded-content containment.
Do not add a cross-component `min <= max` requirement in the exempt case:
the selected text allows any values and the selected schema has no such cross
constraint. Required POSITION arrays and applicable component interpretation
remain; sparse stays Unsupported and extension-supplied actual views retain
their applicable decoded-bound comparison.

## Selected A14 proof and its limits

The complete immutable
[nonauthor mechanism decision](../../bench/architecture_audit/c1_payload_integrity/prereq_review/2026-10-10-numeric-mechanism-decision.md),
SHA256 `85a79b46031443274d4d1e199271bbb3989ea4da08365dd11d2a4765bfd55d36`,
approves the corrected iterative raw-only precode representation. Corrected
proof manifest SHA256
`0026d03177e0fb66ec5fb7dcb372cd6aa7310d474af081163f72217bc333f4ec`
binds seven source/build/receipt files. The independently rerun frozen artifact
SHA256 `d5c53c7df6a41e61b056dc394e30cfc8377fcead377cd3e5349b94699941312d`
is 9,068,248 bytes. Its 22 finite exploratory plus 31 raw rows (53 total) reran
with exit zero and byte-identical stdout SHA256
`af6a57d2f1d7951cffcec805d51bc50936823ce30ca860ca417b4566d8e026f5`.
Additional assertions check equality/one-over bytes/depth/nodes. This author
verified retained identities and inspected receipts, without executing the probe.

The earlier 22+26 phase remains immutable historical source/log/receipt evidence.
Its mutable target executable was overwritten; it is not an independently
reverified original artifact. The reviewer found that raw skipping alone accepts
invalid surrogate pairing; the separately dated correction adds scalar-string
decoding. No correction is retroactively applied to the old phase.

The approved mechanism facts and source-derived bounds settle representation
selection, not production codec or whole-profile acceptance. Checked requested
capacities, slot arithmetic, real borrowed lifetimes, all numeric record gates,
causal failures and resource admission still require final source/evidence review.

## One selection, narrow consumed values

`validate::inspect` remains the only operation owner selecting
`ValidationLimits::default()`. It retains all fifteen serialized public fields,
unchanged values and names. It maps the six actually consumed payload/JSON
fields once into checked private values. No default construction, report type,
operation request, path or `ValidationFailure` import exists below that boundary.
Private probe callers construct explicit small values; no public limit option or
bypass is added.

| Private value | C1 origin | Existing admission meaning |
| --- | --- | --- |
| JSON bytes | `json_bytes = 8 MiB` | Bytes of each interpreted JSON slice, including chunk/table/string padding that is supplied to the parser |
| JSON depth | `json_depth = 64` | Root value depth zero; reject values deeper than the ceiling |
| JSON value nodes | `document_items = 65,536` | Root plus every scalar/container value; object keys are not separate value nodes |
| Payload/resource bytes | `member_bytes = 64 MiB` | Outer payload bytes, sum of declared buffer lengths, and sum of actual buffer plus external-image bytes, each separately admitted |
| Document view/decode bytes | `document_decoded_bytes = 64 MiB` | Sum of all declared ordinary view bytes and meshopt output bytes, including overlapping/repeated views; individual data-URI decode admission uses this ceiling |
| Accessor components | `accessor_elements = 4,000,000` | Sum of `count * scalar_components` over every accessor, including unused/viewless accessors |

Use only `JsonLimits { bytes, depth, value_nodes }` and
`PayloadLimits { member_bytes, decoded_bytes, accessor_components, json }`, or
equivalent private owned values containing exactly those facts. These are
mandatory limits, not options. Check host conversion once when mapping byte/node
ceilings; retain checked count/offset/product/sum arithmetic at data-dependent
gates. A small conversion failure is a typed ResourceLimit, not truncation.

C1 passes a separate remaining component allowance from
`total_payload_elements = 16,000,000 - successfully charged components`.
The payload owner admits `min(per_payload_components, remaining_components)`
before resource callbacks or decode. Remaining allowance is not copied into a
new default or reset by nested b3dm/JSON calls. No generic meter, registry,
operation context or unused prepared-facts representation is introduced.

The other nine public fields stay with their actual C1/archive/hierarchy owners:
stored/source archive bytes, archive entries, central-directory bytes, hierarchy
visits/depth, references, total payload components and total bytes read. The
existing implicit presentation node/cache/serialized-byte budgets remain owned
by its expander; their use of the same numeric decoded ceiling does not combine
them with payload view accounting or JSON nodes.

## All actual JSON consumers

Use one shared JSON admission owner taking explicit `JsonLimits` and returning
a private checked **borrowed** root. Admit bytes, then root node/depth; obtain
one root RawValue with grammar/trailing-token checks. Traverse with an iterative
worklist of borrowed raw/depth pairs. For each container, check immediate unique
decoded keys and charge each child's logical node/depth **before** its RawValue
skip/retention. Process each charged child once, with no second node charge.
Drop the container decoder, scratch and key set before descending. Reverse
children into the LIFO worklist for left-to-right processing. Decode scalar
strings with a visitor returning unit to validate Unicode/surrogate pairing
without retaining scalar strings. Root skip alone does not establish that rule.

Malformed grammar, duplicate decoded keys, invalid Unicode, NaN syntax and
trailing tokens are InvalidInput; explicit byte/depth/node ceilings are
ResourceLimit. An exponent such as `1e400` is valid number grammar, not nonfinite
JSON syntax. Classification uses private typed failures/limit state, never
diagnostic text. Immediate-container key/child admission precedes descendants:
a duplicate root key wins over a depth-limited first child. This deliberate
compound-fault ordering is independently controlled; single-cause meaning and
valid equality boundaries remain. Limits apply per parse, not archive-wide JSON.

Ordinary C1 document consumers derive owned `Value` only **after** the same
grammar/Unicode/unique-key/depth/node admission. Keep current numeric variants;
no normalization changes conversion-report budget checks or implicit Value
equality/serialized-byte admission. If this adapter cannot represent an already
admitted document (e.g. `1e400`), return Unsupported finite owned-document
representation. Do not call valid source malformed or match serde error text.
Payload/table inspection consumes borrowed records and never constructs an owned
payload/table Value to obtain its unsigned fields or schemaUri. This one owner
and its representation adapter are not two JSON interpretations.

Let B be admitted bytes, N root-inclusive nodes and D inclusive root-zero depth.
Source-derived requested storage is O(B+N): one decoder scratch at a time,
source-bounded decoded keys/scalar scratch and bounded pending/immediate child
slots. The combined logical child/pending length is <=N; the observed 64-bit
raw/depth slot is 24 bytes. Check requested capacities/multiplications and
allocation failures; capacity growth/reallocation overlap is distinct from
logical lengths. The key set has node/header overhead as well as string bytes.
Root skip may scan/allocate nesting scratch before descendant depth admission;
never claim zero allocation or no scan before depth rejection. Sum of raw
substring lengths skipped is <=(D+1)*B; Unicode/key/container passes and duplicate
lookup costs are additional bounded work. These are representation bounds,
not exact CPU-byte counters, allocator overhead or total-process RSS promises.

| Current entry point | Actual document and retained consumer |
| --- | --- |
| `ArchiveResources::read_json` | Root/external tilesets and metadata `schemaUri` documents |
| `inspect` conversion report parse | `conversion.json`, retained-content/resource associations and existing report checks |
| `Check::prepare_tileset` resource callback | External JSON tileset schema validation during implicit presentation |
| `ExpansionBudget` parser → `expand_document` | External JSON tilesets interpreted by the expander |
| `ExpansionBudget` parser → subtree `parse` | JSON chunk of each binary subtree |
| Subtree `parse` metadata strings | Each JSON-encoded tile `extras` string; this is a real nested consumer |
| Payload JSON parse | glTF bytes or GLB JSON chunk, including embedded b3dm GLB |
| b3dm table parse | Feature JSON and nonempty batch JSON; each receives the same limits separately |

Current callback type `fn(&[u8])` cannot carry selected limits. The nonauthor
disposition selects a borrowed `&dyn Fn(&[u8]) -> Result<Value, crate::Error>`
passed explicitly to bounded expansion, recursive `expand_document`, subtree
`parse` and metadata extras. Keep `ExpansionBudget` owning presentation counters
only; store no capturing closure/lifetime in it. A producer wrapper supplies its
existing serde parser explicitly. Pass the same mapped `JsonLimits` through every
recursive/extras parse. Do not duplicate the parser or introduce a private
payload/JSON dependency on `validate`. The expander's existing presentation-limit
failure bridge is outside this payload/JSON extraction; the parser seam must not
add another validation dependency. Non-C1 producer/tool expander callers may keep their
separately selected existing parser behavior; their default is not an exception
for any C1 path. The bundled compile-time tileset schema is trusted program data,
not an archive JSON consumer; its existing schema compilation path is explicit
and does not justify bypassing untrusted documents.

## Ownership, causal failure and resolver admission

Use private crate module `content_integrity` with `json` and `payload` submodules,
replacing/removing `src/validate/json.rs` and `src/validate/payload.rs`.
Visibility is crate-private at most.
It must not import validation, archive traversal, URI discovery, run control,
jobs, publishers, reports or CLI/Python types. C1 chooses the payload format from
its name policy and passes a finite GLB/glTF/b3dm discriminator; the format owner
does not need a pathname. Moving the old files without removing defaults and
public-failure dependencies fails this contract.

Use private `FormatError` with InvalidInput/Unsupported/ResourceLimit and
`PayloadError<E> { Format(FormatError), Resolver(E) }`. `E` is only the
caller's causal error, not a configurable policy framework. JSON uses the same
three-way format failure. C1 maps format failures exactly once and unwraps its
resolver error unchanged. Real `io::Error` kinds, source payload and diagnostic
text survive, including actual InvalidData/UnexpectedEof I/O causes. Checked
byte exhaustion/range corruption is format InvalidInput. A limit-looking URI,
duplicate key or I/O message cannot select a different category.
The actual C1 implicit bridge must unwrap `crate::Error::Io` directly before
the retained generic conversion can reinterpret InvalidData/UnexpectedEof as
malformed archive bytes; `crate::Error::Validation` unwraps unchanged. Retain
typed archive ReadError adaptation for C1's real archive reads.

C1 retains percent-decoded archive-local URI authority, reference admission,
held-reader/source identity and total-read charging. A missing member/invalid
URI remains caller InvalidInput; remote authority remains Unsupported. The
private resolver consumes a raw resource URI and a maximum actual resource-byte
allowance, returns owned bytes or causal `E`, and does no path discovery itself.
C1 must admit member size against that supplied allowance and its aggregate
read budget **before** allocating/reading. Pass the current remaining actual
buffer/image allowance, not merely the original per-member ceiling, and verify
returned lengths as a defensive check. Repeated resource declarations charge
repeated actual bytes; no digest/URI deduplication changes that meaning.

All declared buffer lengths receive checked sum admission before the **first**
resource callback, alongside the all-accessor component plan. This intentionally
admits ResourceLimit before a callback error that previously could occur first;
independent compound-error controls must verify the declared order.
Base64 decode
and meshopt output allocation must be admitted before allocation/decode, with
checked arithmetic and padding/alignment conversions. Ordinary borrowed views
still charge view bytes although they allocate no decoded view. Meshopt's
aligned u32 backing and output-copy overlap are temporary allocations, not two
logical view charges. They require an explicit implementation-specific peak
bound and sensitive admission control, not a false 64 MiB whole-process claim.
The current complete-word output-copy loop can extend past its initial logical
capacity before truncation. The nonauthor review requires reworking that copy
to append only the final word's needed prefix, or an equally proved bounded
strategy. With a rounded aligned backing and exactly length-bounded byte output,
the requested simultaneous byte-storage bound is `2 * output_length + 3`,
excluding allocator overhead and retained buffers/views. The review's tiny
std-only probe independently observed old-copy capacities 8/12/20 for output
lengths 2/6/10 versus exact 2/6/10 after final-prefix copy. This demonstrates
capacity sensitivity, not codec/RSS acceptance. Admit checked rounded backing
and output sizes before allocating/decoding; final actual-owner proof remains.

Resource allowance/inline padded-base64 ordering has a concrete nonauthor
disposition. Retain `ceil(encoded_length/4)*3 <= decoded_bytes` individual
admission first; validate standard alphabet/canonical padding/trailing bits
without output allocation, derive exact actual bytes, then admit against actual
buffer/image remainder. Current upper-bound admission is conservative, whereas
`resource_bytes` is charged by actual decoded length. Passing remaining actual
allowance must not reject an admitted padded buffer just because an allocation
estimate exceeds its actual size. The implementation must prove a bounded
allocation strategy or exact validated decode-size preflight. Preserve all
fixed-profile admitted inputs and record any independently necessary error/order
correction before code; no newly tighter shared data-buffer/view pool is implied.
The existing allocating decoder requests at most actual bytes plus two for
valid padding, still within its independently admitted individual decoded
ceiling; it must not compare that estimate to the actual remainder. `AA==` with
remainder one and `AAA=` with remainder two are sensitive positives. An exact
output `decode_slice` strategy is also admissible only after actual allocation
and malformed-versus-limit order proof.

## Actual output consumers and accounting

Return only the actual structural-metadata schemaUri string, when present,
plus existing accessors/primitives/vertices/components counters and exclusions.
C1 confines/resolves the URI, admits its reference and reads the schema document,
merges exclusions and builds `PayloadReport`; it charges successful components
and caches the report by resolved payload member name. The former owned payload
document had only this string consumer; remove it rather than materialize opaque
numbers or return an unused borrowed document. This removes actual representation
cost/coupling while preserving schemaUri type/URI/reference behavior. No new
primitive extrema, scene/frame/wrapper facts, resource inventory roles or A2
consumer data are computed or returned.

| Quantity | Required meaning |
| --- | --- |
| `accessors_checked` | Every accessor validated, including unused and viewless |
| `elements_checked` | Every accessor's count times shape components; scalar-component admission/work allowance, not vertex count |
| `primitives_checked` | Every primitive in all mesh definitions, independent of scene use |
| `vertices` | Sum of full POSITION accessor counts per primitive; shared accessor reuse counts again per primitive, indices do not change this |
| C1 payload cache | Same resolved payload name yields one report and one successful aggregate component charge; distinct names remain distinct even if bytes match |
| C1 reads/references | Actual caller resource visits and reads, including repeated resource declarations and index/hash work; not inferred from returned counters |

Preserve all thirteen current check names and lexically ordered merged
`notInspected`: baseline decoded bounds/metadata/material/image/geometric-error
accuracy exclusions; payload scene/skin/animation/morph/rendered bounds and
metadata/feature exclusions; image decode, b3dm feature/batch semantics and
unknown optional extension names when applicable; implicit addressing and
availability exclusion. None of these counters establishes physical CPU reads,
total resident memory, semantic metadata validity or rendered containment.

## Concrete disposition and dependencies

| Component | Decision | Required evidence/dependency benefit |
| --- | --- | --- |
| C1 fixed request/report/URI/reference/source/read policy | Retain | Independent profile/counter/URI controls; no lower format imports of public report/error types |
| Checked framing/layout/scalar/index/meshopt algorithms | Conditional retain and rework ownership | Independent raw decoder/reference fixtures and corruption controls over the entire domain; historical parity alone insufficient |
| Integer-property and viewless min/max interpretation | Correct/rework at owner | Six independently demonstrated valid-input rejection defects; preserve fractional/mismatch negatives and exact lexical integrality before rounding |
| Payload default helper and ten default-selecting gates | Replace/remove | One mapped six-field profile reaches every consumed gate; no `caps()`/nested defaults remain |
| Validator-owned payload/JSON implementations | Replace their ownership/remove old copies | One private format owner and one parser; actual public-error/default dependency is eliminated, not only relocated |
| Owned recursive JSON admission/retained payload Value | Replace/remove | Corrected independently approved iterative borrowed owner, same logical ceilings/Unicode/unique keys, controlled compound ordering; only ordinary actual consumers derive Value after admission |
| Payload returned document | Remove/rework actual consumer | Return schemaUri+counters/exclusions; C1 retains schema resolution/read policy, no owned payload DOM or unused facts |
| Implicit parser callback | Rework narrowly | Carries selected JSON limits through external/subtree/extras calls; no validation import or duplicated C1 parser |
| Resource allocation/decode admission | Rework where proof requires | Resolver allowance reaches archive preflight; base64 exactness and meshopt temporary peak proved, no aggregate reset |
| Existing independent C1 corpus/official golden records | Retain as supplemental controls after oracle audit | Audit provenance and full-domain coverage; expand sensitive small-limit/cause controls |
| Unused future A2 facts/framework/options | Do not add | No current consumer or held-A2 authority |

Root owns Cargo/build/install/resource scheduling at two workers and nice 10.
The contract author runs no production edits, Cargo, producers, browser, heavy
probes or commits. A separate owner adjudicates the frozen contract/open items
before implementation. The
[separate prerequisite review](../../bench/architecture_audit/c1_payload_integrity/prereq_review/review.md)
records concrete A1–A12 architectural dispositions and tiny arithmetic/capacity
models; the immutable dated decision additionally approves corrected A14.
A13 is now cleared by the coordinator's immutable reader baseline record,
SHA256 `913f3bd19d4b981f4b66ff36c78ad156a8762853edc1e3e4be0c37e2546e7ead`,
and the separate immutable prerequisite verification, SHA256
`bfc8defb1dfd070bebbfee32c49d805a31b45c6f83552461bf5299c2094ea5c8`.
The accepted PR #152 head is `7cbc0884e79d7057d58e9a1fce79dd4d6e1c9440`;
the accepted develop merge and actual next-slice HEAD are
`0eebdeebf013591460e6ac77485ad91b7401aada`, tree
`58aadfdc1bb78c507d020ab91494dc83a3d646a3`, merged at
`2026-10-10T09:29:58Z`. The reviewer verified all 93 current production pins,
all 30 retained lossless remote records, completed exact-head runs, 21 applicable
successes, ten installed-wheel receipts over five wheel hashes and four official
Blender receipts (each 44/0/0/0, empty PATH). This author verified current
production pins/HEAD/tree and copied completed inputs; it did not run CI,
wheels or Blender. These records accept the bounded reader prerequisite only.

The frozen current contract binds copied immutable A13/A14 inputs, all fourteen
concrete dispositions and the selected ownership graph. Its separate nonauthor
incorporation decision is the final precode handshake. Root owns C1 bridges,
crate-module integration and the captured implicit parser callback; the JSON
owner owns shared JSON admission/adapter and private limits/error definitions;
the payload owner owns actual borrowed records, exact numeric consumption and
payload inspection. This split changes dependency authority and representation
cost, rather than moving the same policy coupling. A later distinct nonauthor
reviews final production source and evidence. Contract adjudication is an engineering gate within existing
authorization, not a new user approval flow.

## Independent acceptance plan

1. Author small analytic core/quantized/viewless/data-buffer/local glTF, GLB and
   modern b3dm positives independently of rusty-tiles producers. Include shared
   views/accessors, unused accessors, matrix final-padding and required meshopt
   placeholders; compare scalars/raw bounds/normalization with a separate
   little-endian reference decoder. Use pinned upstream meshopt golden bytes.
2. Pair each admission with exact/one-over explicit private limits. Cover payload
   bytes, declared/actual buffer sums, image remainder, borrowed/overlapping view
   sums, decoded meshopt bytes, individual/padded base64, components and JSON
   bytes/depth/root-inclusive nodes. Instrument resolver/decoder admission so an
   oversized case proves rejection before forbidden read/allocation/decode.
3. Exercise JSON sensitivity through every real C1 consumer: root/external
   tilesets, schemaUri, conversion report, payload chunks, each b3dm table,
   subtree chunks and embedded extras strings. Change only one mapped field;
   a nested default/reset or missing propagation must change the result.
4. Pair duplicate/trailing/invalid Unicode/NaN or malformed-exponent JSON and layout/range/role/index
   corruption with Unsupported valid outside-profile forms. Inject typed
   resolver failures, including InvalidData/UnexpectedEof and spoofing strings,
   and verify causal Io survives the C1/CLI/Python adapters. Probe multiple-error
   cases only against an explicitly adjudicated admission order. Rerun the
   fourteen selected-primary controls on final source and the selected mechanism's
   exact fractional/exponent range/overflow/nonnumeric/Unicode controls. Preserve
   owned ordinary-document variants/marker objects, typed representation
   Unsupported after raw admission, and immediate-container compound precedence.
   Integer acceptance cannot be decided from rounded f64 alone. Pair absent optional
   fields with present-null/wrong-type controls; absent/default positives must pass
   while present null retains its actual consumed type meaning.
5. Check two unique payloads at aggregate-component equality/one-over, repeated
   same-URI payload references, distinct aliases, repeated shared resources and
   repeated primitive POSITION use. Assert the justified full report, all
   counters/checks/exclusions and input immutability; distinguish physical work
   from logical successful charges.
6. Audit and run meaningful existing C1/archive/frontend integration controls,
   including historical missing-BIN negative and existing finite profile
   exclusions. Applicable CI, source-package and installed-wheel checks bind to
   exact source/artifact hashes; official glTF checks cover only their declared
   core/quantization domain, not meshopt/restart/b3dm/3TZ. Applicable official
   Blender checks are coordinator-owned integration evidence, not a new format
   oracle. Resource observations declare platform/peak/worker limits explicitly.

Before acceptance, settle every blocking question in
[adjudication.md](../../bench/architecture_audit/c1_payload_integrity/adjudication.md),
pin final source/artifact/fixture identities, remove the old interpretations,
and obtain the separate final-source nonauthor review. This slice does not close
#113/#121, authorize `main`, tagging or release, or settle #125/#126/#133's
broader geometry/metadata/implicit obligations.
