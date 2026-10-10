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
pairs. The actual final local execution source `e4ec3cc` passed 367 portable
library tests and captured all 82 actual context calls: 41 complete-report
admissions and 41 typed ResourceLimit failures. Native all-targets and
Python-binding Clippy passed with warnings denied.

A separately adjudicated control found an inherited false InvalidInput on the
earlier `2ebfb74` candidate: an unused tightly packed SCALAR accessor over a
decoded meshopt view was incorrectly required to match the codec record stride.
The [primary-backed decision](../../bench/architecture_audit/c1_payload_integrity/final_source_review/2026-10-10-meshopt-stride-adjudication/decision.md)
settles codec grouping separately from accessor layout. The correction removes
only that equality check and its unused tracking. Defined parent stride
agreement, decoded length, ordinary ranges/alignment/finite values and budgets
remain. Independent additive [six-case stride controls](../../bench/architecture_audit/c1_payload_integrity/production_controls/stride_controls/2026-10-10/README.md)
and a [separate unmasked parent-stride negative](../../bench/architecture_audit/c1_payload_integrity/production_controls/stride_parent_control/2026-10-10/README.md)
retain literal upstream encoded/decoded bytes and complete positive reports.
The old executable fails the two sensitive packed-accessor positives; both
fresh corrected executables pass all seven controls. Two private tests also
exercise actual component/decode equality and one-less limits.

The final frozen portable/native CLI artifacts each pass 210 corrected and
supplemental payload controls plus the seven stride controls, all 135 original
C1 cases, 45 archive controls and 20 resource runs. Producer framing independently
passes eight portable and six native b3dm members; this is framing evidence,
not geometry or byte equivalence. Linux observations measured 63,272–71,456 KiB
maximum RSS for portable and 84,164–90,272 KiB for native, with sampled descriptor
maxima six each. These finite observations are not universal process-memory
limits. Actual `cargo package --locked -p rusty-tiles` at `e4ec3cc` compiled the
extracted 1,965-member source archive. Its SHA256 is
`6a63ab5ec1e769418c56ed8e15cd57c739f6ee2a9e8c724fccdc78b932a2f3c0`.
All 1,961 ordinary members are byte-identical to committed source. The original
manifest remains exact; root-package lock normalization prunes only eight
Python-only package tables and leaves all 288 retained tables identical.

The [82 lossless final records](../../bench/architecture_audit/c1_payload_integrity/candidate_evidence/local-e4ec3cc/index.json)
bind 97 production and 237 selected acceptance inputs, separate unit/portable/
native artifacts, actual logs, raw CLI streams and source archive compilation.
They also preserve the earlier corrected library parser stop, test-only Clippy
failure, sensitive old-binary failures and a pre-case executable-copy permission
failure. Restoring the copied executables' execute bits changes no artifact
bytes; the successful integration run uses a separate evidence directory.

Earlier [45 local records](../../bench/architecture_audit/c1_payload_integrity/candidate_evidence/local-2ebfb74/index.json),
[local review](../../bench/architecture_audit/c1_payload_integrity/final_source_review/2026-10-10-final-checkpoint/review.md)
and [source-package compilation](../../bench/architecture_audit/c1_payload_integrity/candidate_evidence/2026-10-10-source-compilation/index.json)
retain their original `2ebfb74`/`2ef8db1` source identities. They do not establish
acceptance of this corrected candidate. Fresh corrected-source
[independent review](../../bench/architecture_audit/c1_payload_integrity/final_source_review/2026-10-10-stride-final-e4ec3cc/review.md)
and exact final-head remote CI, installed-wheel and official Blender acceptance
are separate merge gates.

Admission establishes explicit finite source/node/depth/component/read/decode
ceilings and checked requested allocations, not whole-process recoverable OOM
or universal RSS. Serde internals, decoded keys/string scratch, ordinary Value
and library collections retain source-bounded infallible allocations. The
borrowed raw-root scan and substring work remain within the contract's source
and selected-depth bounds; arbitrary huge-depth owned materialization is not
promised. No full A2 template/frame/metadata/lifecycle acceptance, parent issue
closure, main promotion, tag or release follows from this slice.
