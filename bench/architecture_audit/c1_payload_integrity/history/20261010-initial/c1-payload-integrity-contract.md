# C1 payload integrity and explicit JSON admission

Status: contract candidate for **preimplementation nonauthor adjudication**.
No production implementation or new execution acceptance is recorded here.
Source inventory baseline: `c8cbfb187de863ae21fe904275794894aa0a8381`.
The archive-reader prerequisite must be independently accepted and this slice
rebased onto its accepted exact head before implementation/merge acceptance.

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
Their existing citations and evidence are provenance, not a new read or run.

A locally cached primary glTF text was read for alignment, raw min/max and
normalization rules; its SHA is recorded, but its source revision has not been
verified against the selected published pin. It also exposes two important
exceptions: integer-valued decimal/exponent property syntax, and arbitrary
min/max where neither sparse nor bufferView exists. Current strict integer
parsing and zero-value bound comparisons are static observations. Adjudication
must verify the selected source and decide a truthful existing finite-profile
description or an independently justified correction. Independently established
InvalidInput misclassifications or missing declared-bound exemptions must be
corrected at their owner; legacy tests cannot veto the correction. A valid form
excluded by an explicitly chosen finite profile is Unsupported, never malformed.
This document does not assert executed defects or authorize silent domain changes.

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

Use one JSON parser taking explicit `JsonLimits`, returning owned `Value` and a
private causal format failure. It rejects duplicate keys, malformed/nonfinite
JSON and trailing tokens as InvalidInput; explicit byte/depth/value-node ceilings
as ResourceLimit. Classification comes from typed limit state, never diagnostic
text. Unknown JSON fields remain in the returned document. Count/check a value
before retaining it; JSON byte ceilings bound string/key storage separately.
Budgets apply independently to each parse invocation, not archive-wide JSON work.

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

Current callback type `fn(&[u8])` cannot carry selected limits. Rework only the
internal bounded-expansion parser seam to accept an explicit parser context or
borrowed capturing callback. Pass the same mapped `JsonLimits` through every
recursive/extras parse. Do not duplicate the parser or introduce a private
payload/JSON dependency on `validate`. The expander's existing presentation-limit
failure bridge is outside this payload/JSON extraction; the parser seam must not
add another validation dependency. Non-C1 producer/tool expander callers may keep their
separately selected existing parser behavior; their default is not an exception
for any C1 path. The bundled compile-time tileset schema is trusted program data,
not an archive JSON consumer; its existing schema compilation path is explicit
and does not justify bypassing untrusted documents.

## Ownership, causal failure and resolver admission

Choose a private root `payload_integrity` owner with its private JSON helper
(final file layout may be flat or nested). Visibility is crate-private at most.
It must not import validation, archive traversal, URI discovery, run control,
jobs, publishers, reports or CLI/Python types. C1 chooses the payload format from
its name policy and passes a finite GLB/glTF/b3dm discriminator; the format owner
does not need a pathname. Moving the old files without removing defaults and
public-failure dependencies fails this contract.

Use private three-way format failures and an explicit resolver-failure arm, for
example `PayloadFailure<E> { Format(IntegrityFailure), Resolver(E) }` where
`IntegrityFailure` is InvalidInput/Unsupported/ResourceLimit. `E` is only the
caller's causal error, not a configurable policy framework. JSON uses the same
three-way format failure. C1 maps format failures exactly once and unwraps its
resolver error unchanged. Real `io::Error` kinds, source payload and diagnostic
text survive, including actual InvalidData/UnexpectedEof I/O causes. Checked
byte exhaustion/range corruption is format InvalidInput. A limit-looking URI,
duplicate key or I/O message cannot select a different category.

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

Declared buffer-length admission precedes resource acquisition. Base64 decode
and meshopt output allocation must be admitted before allocation/decode, with
checked arithmetic and padding/alignment conversions. Ordinary borrowed views
still charge view bytes although they allocate no decoded view. Meshopt's
aligned u32 backing and output-copy overlap are temporary allocations, not two
logical view charges. They require an explicit implementation-specific peak
bound and sensitive admission control, not a false 64 MiB whole-process claim.

Resource allowance/inline padded-base64 ordering is a critical adjudication
item. Current `ceil(encoded_length/4)*3` admission is conservative, whereas
`resource_bytes` is charged by actual decoded length. Passing remaining actual
allowance must not reject an admitted padded buffer just because an allocation
estimate exceeds its actual size. The implementation must prove a bounded
allocation strategy or exact validated decode-size preflight. Preserve all
fixed-profile admitted inputs and record any independently necessary error/order
correction before code; no newly tighter shared data-buffer/view pool is implied.

## Actual output consumers and accounting

Return only the existing owned document, accessors/primitives/vertices/components
counters and exclusions. C1 reads structural metadata `schemaUri` from that
document, merges exclusions and builds `PayloadReport`; it charges successful
components and caches the report by resolved payload member name. No new
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
| Payload default helper and ten default-selecting gates | Replace/remove | One mapped six-field profile reaches every consumed gate; no `caps()`/nested defaults remain |
| Validator-owned payload/JSON implementations | Replace their ownership/remove old copies | One private format owner and one parser; actual public-error/default dependency is eliminated, not only relocated |
| JSON unique-key/value-node/depth visitor | Conditional retain with explicit limits/private errors | Independently specified JSON counter convention and exact/one-over controls; no message-based classification |
| Implicit parser callback | Rework narrowly | Carries selected JSON limits through external/subtree/extras calls; no validation import or duplicated C1 parser |
| Resource allocation/decode admission | Rework where proof requires | Resolver allowance reaches archive preflight; base64 exactness and meshopt temporary peak proved, no aggregate reset |
| Existing independent C1 corpus/official golden records | Retain as supplemental controls after oracle audit | Audit provenance and full-domain coverage; expand sensitive small-limit/cause controls |
| Unused future A2 facts/framework/options | Do not add | No current consumer or held-A2 authority |

Root owns Cargo/build/install/resource scheduling at two workers and nice 10.
The contract author runs no production edits, Cargo, producers, browser, heavy
probes or commits. A separate owner adjudicates the frozen contract/open items
before implementation. A later distinct nonauthor reviews final source and
evidence. Contract adjudication is an engineering gate within existing
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
4. Pair malformed duplicate/trailing/nonfinite JSON and layout/range/role/index
   corruption with Unsupported valid outside-profile forms. Inject typed
   resolver failures, including InvalidData/UnexpectedEof and spoofing strings,
   and verify causal Io survives the C1/CLI/Python adapters. Probe multiple-error
   cases only against an explicitly adjudicated admission order.
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
