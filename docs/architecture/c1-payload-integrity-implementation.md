# C1 payload and JSON integrity ownership

The bounded [contract](c1-payload-integrity-contract.md) was independently
approved before implementation. This slice replaces current C1's payload DOM
and recursive JSON admission with private `content_integrity` owners. It does
not implement A2 or certify decoded scene, image, material or metadata semantics.

`validate` selects the existing fifteen-field profile and maps its six relevant
facts once to checked JSON/payload limits. It retains archive/name/URI/source
identity, reference visits, read accounting, successful resolved-name caching,
aggregate component charging and public report/error policy. The private
`inspect_selected` entry permits independently supplied finite limits through
that actual C1 operation; the public entry still selects its existing profile.
Implicit traversal receives the same selected JSON parser for external tilesets,
subtrees and nested serialized metadata extras. Producer parsing keeps its own
existing policy.

The JSON owner admits borrowed source with an iterative container walk. It
counts root-inclusive values, enforces root-zero depth and validates unique
decoded keys and Unicode before interpretation. Children are charged before
their raw-value skip. It drops each container decoder/key set before descending.
Payload records consume borrowed fields with explicit absent/null distinction;
unsigned fields use exact decimal/exponent interpretation without conversion
through binary floating point. Opaque valid numeric tokens remain raw. Ordinary
document consumers derive `Value` only after the same admission; a valid number
that cannot be represented there is Unsupported rather than malformed JSON.

Payload plans retain admitted accessor shapes/counts/references, admit declared
buffers before resolver reads, and pass the actual remaining buffer/image
allowance to C1's archive reader. Actual/declared buffer storage and logical
decoded views keep separate pools. Base64 retains its individual conservative
guard, then validates canonical padding/unused bits and allocates the exact
admitted output. Meshopt admits checked aligned backing plus the real output
copy before decoding. POSITION use counts vertices per primitive while accessor
components charge once per payload; successful resolved names charge once in
C1's aggregate owner.

The existing finite GLB/local glTF/modern b3dm, accessor/quantization/matrix,
supported primitive, meshopt NONE and producer profiles remain within scope.
Undefined primitive modes are invalid; defined modes outside the selected
profile are Unsupported. Viewless nonsparse accessors retain zero semantics.
FLOAT accessor bounds round to finite f32 as allowed by the pinned primary
format; stored extrema are compared, while viewless declarations are exempt
from decoded-extrema equality. No new lexical f32 or cross-min/max rule is
invented. Payload results expose only the existing counters/exclusions and
the actual optional structural-metadata schemaUri; C1 owns its URI resolution.

The old `validate/json.rs` and payload-owned document result are removed. Format
owners import no validation request, archive, path, job, default profile or
publication policy. Resolver failures retain their typed cause, including
actual I/O errors through recursive implicit parsing.

## Independent evidence and limits

The independently authored [production controls](../../bench/architecture_audit/c1_payload_integrity/production_controls/README.md)
retain the original 205 literal archives, eleven private sensitive tests and
pinned primary sources. The original execution remains 204 passes and one
invalid positive-oracle failure. A [dated polygon adjudication](../../bench/architecture_audit/c1_payload_integrity/production_controls/polygon_correction/2026-10-10/adjudication.md)
preserves that failure, corrects two polygon fixtures against the primary
indexed-TRIANGLES requirements, and adds five sensitive controls. The corrected
205 retains 203 original case/archive records and 77 complete positive reports.
A separate additive
[context lane](../../bench/architecture_audit/c1_payload_integrity/production_controls/context_controls/2026-10-10/README.md)
supplies twelve dominating JSON consumers and five actual C1 cache/aggregate
pairs. The final portable library checkpoint `2ebfb74` passed 365 tests; captured
logs record all 82 actual context calls, with 41 complete-report admissions and
41 typed ResourceLimit failures. Earlier source/execution phases and the aborted
fixture-packaging build remain distinct historical records. Exact local
portable/native CLI artifacts each passed all 210 corrected/supplemental
controls, all 135 original C1 cases, 45 archive controls and 20 resource runs.
Producer framing independently passed eight portable and six native b3dm
members; this is framing evidence, not geometry or byte equivalence. Native
all-targets and Python-binding Clippy passed with warnings denied. Cargo's
source-package inventory includes all 267 selected required source/acceptance
paths; that check does not compile or certify an installed source archive.

The [45 lossless local records](../../bench/architecture_audit/c1_payload_integrity/candidate_evidence/local-2ebfb74/index.json)
bind 97 production and 179 selected acceptance inputs, separate unit/portable/
native executables, raw CLI streams and actual selected-limit outcomes. Linux
resource observations measured 63,172–71,348 KiB maximum RSS for portable and
84,308–90,556 KiB for native, with sampled descriptor maxima seven and six.
These finite observations are not universal process-memory limits. Fresh
independent source/artifact acceptance and exact final-head CI, installed-wheel
and official Blender checks remain separate merge gates.

Admission establishes explicit finite source/node/depth/component/read/decode
ceilings and checked requested allocations, not whole-process recoverable OOM
or universal RSS. Serde internals, decoded keys/string scratch, ordinary Value
and library collections retain source-bounded infallible allocations. The
borrowed raw-root scan and substring work remain within the contract's source
and selected-depth bounds; arbitrary huge-depth owned materialization is not
promised. No full A2 template/frame/metadata/lifecycle acceptance, parent issue
closure, main promotion, tag or release follows from this slice.
