# Separate preimplementation adjudication

This is a nonauthor review of the original frozen contract SHA256
`88efd51e854bc02951cab30d1a0bd9a205e67a0ea41e458ea231953bdadac3a4`
against source `c8cbfb187de863ae21fe904275794894aa0a8381`. The contract author
is revising the contract separately; this record does not overwrite its initial
freeze, independent scope audit, or primary evidence. No production source was
edited, no Cargo/install/producer/browser workload was run in this lane, and no
candidate source acceptance is claimed. A tiny std-only rustc capacity probe and
independent Python arithmetic models were run at nice 10.

**Decision: architectural dispositions below are concrete, but production coding
of numeric interpretation remains gated on the actual serde mechanism proof.
The slice is not yet implementation-ready as a whole.** Archive prerequisite
identity must also be durably bound before the
coordinator records the final implementation-ready decision. Missing final-source
tests are acceptance obligations, not a reason to reinterpret old behavior as
the contract.

## Evidence and finite meaning

The separate primary lane's [receipt](../primary_controls/receipt.json), SHA256
`0b22cd29d29187e2518f5a852419abe88bd831be54ffd7658f2825e7fc497174`, and
[adjudication](../primary_controls/adjudication.json), SHA256
`d0868592655f023f01633a18411781b08ab59371f2bc7f1e26921c97207cbc64`, verify
published glTF 2.0.1 source revision
`8e798b02d254cea97659a333cfcb20875b62bdd4`, exact specification SHA256
`10aebb6a8362a155b196448912bf6250bf046f150431fa73c6cd657c44185aff`.
The fourteen tiny frozen-CLI controls establish six valid-form InvalidInput
misclassifications, with retained stored/viewless zero positives and sensitive
fractional/stored-bound negatives. Source and artifact identities are in that
receipt. This review read those records; it did not rerun or author them.

The exact-number models here independently show why f64 integrality is unsafe:
`3.0000000000000001`, `30000000000000001e-16`, and
`18446744073709551614.9` all appear integral as f64 but are fractional. Conversely,
`9007199254740993.0` is exactly integral and must not be silently rounded to a
different integer. Positive integer overflow, negative nonzero, fractional value,
finite JSON representation and host representability are different decisions.
The model's `range` is intentionally not a public failure classification.

## Gate dispositions

| Gate | Nonauthor disposition and owning requirement |
| --- | --- |
| A1 | Approve `JsonLimits { bytes, depth, value_nodes }` and `PayloadLimits { member_bytes, decoded_bytes, accessor_components, json }`, or equivalent exactly six facts. C1 selects its fifteen-field public profile once; checked host conversion occurs there. Remaining successful aggregate components is a separate argument, admitted with the per-payload ceiling before resource callbacks/decode. No A2 facts or generic meter. |
| A2 | Approve private three-way format failures plus `PayloadFailure<E>::Resolver(E)`. C1 unwraps resolver errors unchanged, with no text or io-kind inference. The existing blanket `From<crate::Error>` converts actual Io InvalidData/UnexpectedEof to InvalidInput; C1's implicit bridge must handle `crate::Error::Io` causally as Io before falling through. Existing typed archive ReadError adaptation is the valid C1 archive path. Inject both kinds plus misleading messages in direct owner and actual bridges. |
| A3 | Rework the narrow parser seam: pass a borrowed `&dyn Fn(&[u8]) -> Result<Value, crate::Error>` to bounded expansion and through `expand_document`, subtree `parse`, and each metadata extras parse. Keep budgets owning only presentation counters; no capturing closure stored in a fn pointer or required lifetime in the budget. A producer wrapper supplies its existing serde parser explicitly. The C1 closure captures its one mapped JsonLimits. This removes the need for an implicit import of validation or payload-limit policy. |
| A4 | Rework all declared buffer lengths into a checked sum preflight before the first resource callback, alongside existing all-accessor component planning. Each resolver receives `member_bytes - actual_resource_bytes`, and C1 compares catalog size and total-read allowance before allocation/read. Defensive returned-length checks remain. Include images and repeated resource declarations; do not deduplicate. This deliberately moves ResourceLimit before a callback that formerly could fail first; compound-error precedence must be explicitly tested. |
| A5 | Keep the existing conservative individual data-URI admission `ceil(encoded_len/4)*3 <= decoded_bytes`, independently of actual buffer/image allowance. Validate canonical standard base64 size/alphabet/padding/trailing bits without output allocation, derive exact actual bytes, and admit those against the remaining actual allowance. The existing Engine::decode allocates the conservative maximum; valid padding gives requested temporary output <= actual+2, still <= individual decoded ceiling. Do not compare its allocation estimate to actual remainder and reject AA== at remainder1 or AAA= at remainder2. An exact-sized decode_slice strategy is also valid if source proof demonstrates its actual allocation. Preserve malformed-vs-limit order explicitly. |
| A6 | Retain logical sum of ordinary view lengths including borrowed/overlapping views, and each meshopt logical output once. Correct full-word output copy before truncate: it can grow Vec capacity beyond requested length. Copy only final-word remaining prefix, or allocate exactly initialized output bytes. The std-only probe demonstrates capacities 8/12/20 for old copies at lengths2/6/10 versus exact2/6/10. Admit checked rounded backing `4*ceil(L/4)` and exact copy L before decoder/output allocation; requested simultaneous byte-storage bound is <=2L+3. This excludes allocator overhead, retained buffers/views and RSS. No extra logical view charge or new shared buffer/view pool. |
| A7 | Retain all-accessor scalar component counts, full POSITION count per primitive, and successful cache/aggregate charge keyed by resolved payload member name. Read/reference visits remain actual caller work. Independently authored two-payload/equality/one-over and repeated name/distinct alias/shared resource/shared POSITION controls remain final acceptance obligations. |
| A8 | Approve root-inclusive values, keys excluded, root depth0, ceiling inclusive, separately per parse. For `{"a":[0]}` nodes3 and depths0/1/2; depth2/nodes3 equality succeeds, depth1/nodes2 fails. Duplicate keys and magic-key lookalikes are ordinary malformed/object data, never resource category selectors. Test each table/chunk/extras invocation with independently small changed limits. |
| A9 | Close precode primary revalidation: published core pin and exact cached-byte identity are independently verified by primary_controls; the dated pinned-reference-verification fetch receipt retains five selected texts plus three b3dm dependencies. This review independently verified all eight retained length/hash identities and read the relevant rules. Golden byte comparison binds upstream compressed/expected literals to retained fixture bytes; it is static provenance, not new codec execution. Historical citations alone were insufficient; the new durable primary receipt settles that missing provenance. |
| A10 | Correct the six demonstrated defects; do not grandfather or declare an unsupported syntax subset. Every consumed core/quantization/meshopt/restart unsigned property must use the same exact integer-token rule, including preflight count, byteStride, mode/target/index references; no fallback `as_u64` that silently defaults. Numeric b3dm BATCH_LENGTH has its own JSON Schema integer authority and must use exact math before uint32 admission. Nonzero fractional lexemes remain InvalidInput even when f64 rounds integral. Viewless nonsparse bounds comparison is exempt; retain required POSITION min/max, shape, numeric finiteness and applicable component-type interpretation. Do not add min<=max in this exempt case: the selected text says any values, the selected schema has no cross-ordering condition, and the old source has no separate ordering check. Stored/extension-supplied views retain actual equality, and sparse remains Unsupported. Actual zero values still drive attribute/index/restart checks. |
| A11 | Retain trusted bundled schema compilation as program-data exception, not archive data. Root/external tilesets, schemaUri documents, conversion.json, external-expansion schema validation, expansion external JSON, subtree JSON, extras strings, payload JSON and each nonempty b3dm JSON table all use mapped limits. Producer/tool wrappers' separate selection is not an untrusted C1 bypass. |
| A12 | Retain full admitted profile: local glTF/GLB/modern b3dm, both base64 media types, viewless zeros, quantization, omitted final matrix padding, meshopt NONE and required placeholders, optional opaque extras/extensions, existing polygon references and finite modes/restart exclusions. Preserve thirteen checks and lexical exclusions. Audit independent existing corpus before relying on it; no rendered geometry, wrapper metadata semantics, unused A2 facts or closure claims. |
| A13 | Archive reader accepted exact head/rebase identity is coordinator-owned and still must be recorded here. Final source and artifacts require a distinct nonauthor review plus applicable coordinator-owned integration/CI/wheel/official-Blender checks. This precode review accepts neither prerequisite nor implementation, and does not clear full A2's literal external-template normative hold or authorize main/tag/release/#113/#121 closure. |

## Exact-number mechanism: precise remaining precode gate

Local serde_json 1.0.151 source establishes that `arbitrary_precision` stores
number strings and exposes Number::as_str, but deserialize_any delivers
non-native numbers via a synthetic map whose key is
`$serde_json::private::Number`. The old bounded visitor would treat that as an
object and an extra value node. Copying upstream magic-key recognition can
misinterpret an actual opaque extras object with that key. Merely enabling the
feature, or changing uint to `as_f64().fract() == 0`, is rejected.

A concrete candidate requiring a tiny actual-source proof is the existing Value
DOM with arbitrary_precision plus raw_value. At the bounded Seed boundary count
the logical value first, borrow RawValue from the immutable input, and intercept
lexically numeric tokens to construct Number directly from that token. For other
values apply the existing bounded visitor to the borrowed raw slice. Children
repeat interception, so synthetic numeric maps do not enter visit_map and
ordinary magic-key objects remain objects. Exact unsigned interpretation uses
the sign/significand/exponent string: zero is zero including -0, nonzero negative
and nonzero fractional values are invalid; trailing-zero cancellation precedes
checked range comparison, and no power-of-ten or arbitrary integer expansion is
allocated. Large exponents are bounded/saturated by token length and the actual
needed range, with zero handled before exponent magnitude. Number string bytes
remain bounded by the supplied JSON slice. No sidecar AST, path registry or
mathematical geometry framework is needed.

RawValue's local ignore_value implementation scans iteratively using scratch
stack, before the full visitor discovers nested logical depth/nodes. Record its
temporary storage bound under JSON byte admission and repeated-scan work bound
under maximum admitted depth; do not call it a zero-work or zero-allocation
parse. Review whether the candidate has the smallest justified representation
and whether globally enabled features affect other actual serde consumers.

Before implementing the affected rule, coordinator-owned small serde proof must
show: exact Number strings survive 3.0/3e0/tiny fractions/u64 boundary and large
integer tokens; duplicate/magic-key objects remain objects; numeric scalars cost
one node; equality/one-over depth/node limits remain typed; trailing/malformed/
nonfinite syntax fails; unknown fields/opaque extras survive; and borrowed raw
dispatch does not introduce an uncontrolled allocation/recursion path. Choose
explicit host/token-range dispositions at their consuming owner: e.g. a valid
count exceeding component work is ResourceLimit, while an exact out-of-array
reference is InvalidInput. Do not silently classify a mathematically valid
representation as malformed simply because f64 or host conversion lost it.
Actual mechanism proof and this disposition must enter the revised freeze.

The full consumed unsigned-number inventory includes `uint`/`field`/`off`/`at`
and three bypasses (`BATCH_LENGTH` at payload.rs304, accessor preflight count at
394, byteStride fallback at647). It also includes direct Value comparisons with
integer constants in polygon index view target799, attribute view target871,
and ordinary index view target955. Valid decimal/exponent targets must not pass
the first uint admission only to fail these comparisons. Declared buffer/view/
accessor/meshopt lengths, offsets, counts, stride, componentType, primitive mode,
material/attribute/indices/texture/sampler references and polygon count/reference
fields use that same exact consumed rule. Scene/node fields currently left
uninspected must remain uninspected rather than becoming new claims.

## Final acceptance remains separate

Small explicit-limit controls need instrumentation for no resolver/decode/output
allocation beyond admission; the existing fixed public profile alone cannot
exercise those private seams. Include aggregate remaining reset detection,
padded inline boundary cases, all three implicit contexts, numeric grammar at
every consumed unsigned path, arbitrary viewless bounds plus shape/finite controls,
actual causal Io kinds, full reports/counters/exclusions and immutable inputs.
Existing tests and old digests are supplemental candidates; fourteen primary
controls and the finite models here do not establish universal format support.
