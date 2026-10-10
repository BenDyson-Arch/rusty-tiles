# Generated-vector compression codec preparation audit

Preparation only. Current C1 acceptance remains the merge gate; full A2 remains held. No production edits, Git, Cargo, codec executions, resource probes or issue writes occurred.

Coordinator-provided checkout identity: `2ebfb749`; exact inspected file hashes are in [audit.json](audit.json). All files written by this lane are under this temporary `codec` directory.

## Recommendation and concrete consumer

After C1 acceptance, prepare one private byte/view compression plan and one caller-owned standalone replacement operation. The vector producer already consumes `compress_document` under its existing Attempt; it must use the same plan without invoking the standalone operation. Historical Python calls the hidden worker before wrapping fill GLBs. This is an actual operation migration, not unused A2 facts.

## Full source domain

**Unquantized positions** — `src/vector/pipeline/encoding.rs:172-186; tests/fixtures/vector_oracle/vector.py:72-81`. FLOAT VEC3, count N, 12-byte records, exact authored f32 bits. No vertex reorder or requantization.

**Quantized positions** — `src/vector/pipeline/encoding.rs:227-266; tests/fixtures/vector_oracle/vector.py:85-112`. normalized UNSIGNED_SHORT VEC3, parent byteStride 8, three components plus two padding bytes; KHR_mesh_quantization required; authored node translation/scale preserved.

**Feature IDs** — `src/vector/pipeline/encoding.rs:188-203`. UNSIGNED_SHORT SCALAR in four-byte padded records when items <=65536; FLOAT SCALAR four-byte records otherwise, with upstream exact-ID ceiling 16777217. Preserve EXT_mesh_features attribute-set/propertyTable association, not merely accessor bytes.

**Topology** — `src/vector/pipeline/encoding.rs:84-169,202`. POINTS/LINE_STRIP/TRIANGLES modes 0/3/4; U32 scalar index streams; line restart u32::MAX under required pinned KHR_mesh_primitive_restart draft; polygon indicesOffsets, loopIndices, loopIndicesOffsets and loop restart values under EXT_mesh_polygon draft. ATTRIBUTES/NONE is byte-wise lossless; do not reinterpret these as triangle-codec inputs.

**Fill outputs** — `src/vector/pipeline/encoding.rs:205-214; tests/fixtures/vector_oracle/vector_parallel.py:88-96`. Ordinary TRIANGLES GLB with exact KHR_materials_unlit/doubleSided/PBR factors, EXT_mesh_features and structural metadata. Do not exclude fill-only content; wrapping happens after codec.

**Scalar/raw metadata** — `src/metadata.rs:276-346`. BOOLEAN bitsets; signed INT64 and FLOAT64 little-endian columns; UTF8 STRING values and UINT32 stringOffsets; missing noData sentinels/class definitions; source IDs/layers and aggregation table/class names. All metadata-only views raw and aligned to eight bytes. Raw UINT64 preservation required by format fixtures even though reviewed property-table writer only emits INT64 for integral properties.

**Generated array/list meaning** — `src/vector/source_fields.rs:48-89; src/vector/pipeline/source_native.rs:371-376,678-732; tests/fixtures/vector_oracle/vector_source.py:183,231`. Explicit list_fields=json turns arrays (including heterogeneous arrays, exact integers, order and null) into JSON-encoded STRING values. Preserve exact UTF8 content/stringOffsets and ordinary noData semantics. Do not silently classify this currently generated array use as unsupported.

**Structural array metadata** — `src/metadata.rs:151-165`. Public PropertyTableProperty can refer to arrayOffsets and stringOffsets; current encode_property_table does not author native structural arrays. Raw view and raw JSON preservation can cover independently authored variable/fixed array tables, but must not be claimed as accepted array authoring or semantic validation without independent controls.

**Source bytes/layout** — `src/glb.rs:328-355,361-395; tests/fixtures/vector_oracle/vector.py:58-81`. Reviewed builders create one embedded buffer, append eight-byte-aligned distinct views and distinct accessor-backed stream views; generated view objects have no pre-existing meshopt extension. Sparse accessors, interleaved/shared views, extra buffers/external resources and unknown GLB chunks are not produced by these builders.

## Primary invariants and claim boundaries

Pinned primary bytes and retrieval/hash receipts are saved in [primary/primary-receipt.json](primary/primary-receipt.json). glTF core, meshopt and quantization are kept distinct from pinned feature/metadata/polygon/restart draft profiles.

`glTF-core.adoc:521-529,650`: Integer schema values may use decimal/exponent notation but must have exact zero fraction; references must point to actual elements. Ordinary as_u64 after floating conversion is not authoritative source interpretation.

`glTF-core.adoc:886-898,1124-1141`: A view ranges over its selected buffer; accessor offset/effective stride/count/window fit that view. Shared vertex attributes require parent byteStride. Shared references are not intrinsically invalid. Index and vertex attribute data cannot share the same view.

`EXT_meshopt_compression.md:73-87,120-147`: Codec is defined at whole-view level. Extension byteLength/byteStride/count/ranges and parent stride must agree; ATTRIBUTES stride divisible by four <=256; NONE preserves decoded bytes. Placeholder fallback buffer is large enough, index >=1 in GLB, has only compressed-view parent references, and extension is required when fallback has no source.

`KHR_mesh_quantization.md:49-115`: Normalized U16 POSITION, original min/max and node dequantization transform keep their meaning; required extension declaration remains. Compression is not a new quantizer or error estimate.

`EXT_mesh_features.md:60-74,162-182`: Feature accessor is SCALAR/non-normalized; set index names _FEATURE_ID_n; table association and count preserved. FLOAT exact-ID implementation guidance explains source-generated f32 IDs; raw codec preservation does not certify upstream source attribution.

`EXT_mesh_polygon.md:29-111; KHR_mesh_primitive_restart-draft.md:27-87`: Polygon offsets/loop bindings and restart topology remain authored and ordered. These references are explicitly pinned drafts, not broad ratified glTF claims.

`EXT_structural_metadata.md:229-279; 3d-metadata.adoc:561-610`: Table count/class/property associations and little-endian scalar/string/array bytes remain; bufferView component-size alignment holds, including eight-byte 64-bit columns. Inference: retaining every referenced raw view byte and referenced view ID preserves admitted table storage without reauthoring metadata.

## Small consumed representation

**owner:** One private immutable JSON/BIN codec owner, caller-selected limits, no paths/RunControl/Attempt/discovery/report defaults.

**plan:** A consumed plan assigning each actual physical view exactly once: checked source range/selector; either raw copy or whole-view ATTRIBUTES/NONE stream with chosen stride and count; checked output/fallback layout and encoded bound. Keep accessor windows/roles distinct from meshopt physical-record count. No generic IR/format registry or returned unused geometry facts.

**critical count rule:** Do not copy current accessor.count * chosen_stride == view.byteLength as a universal rule. Meshopt count belongs to whole-view stream. Repeated compatible references need no repeated compression; conflicting last-accessor overwrite is replaced by explicit once-per-view planning.

**raw transform:** Use admitted RawValue and targeted output serialization preserving untouched numeric lexemes/subtrees. Only buffers, bufferView source/layout and relevant EXT_meshopt bookkeeping/extensionsUsed/Required change. Do not parse arbitrary opaque metadata/extras/node values into Value and assume float_roundtrip preserves them.

**producer integration:** An already-authored producer Value may be serialized once to the same admitted raw planner, then emitted with changed layout. This is a representation option needing cost settlement; do not build two independent interpretations or assume a general raw mutation engine is needed. No extra run and no source-path public validator invocation.

**C1 dependency:** Pending C1 json::admit exposes CheckedDocument/raw structural admission; json::parse deliberately uses a finite owned representation. Exact numeric helper is currently payload-private, not a reusable public API. After C1 acceptance, expose/move only the actual consumed private exact unsigned interpretation, if source review justifies reuse. This audit does not authorize changes to current C1 or claim helper retention acceptance.

## Explicit profile dispositions

**Currently generated streams/metadata/fills/lists:** Mandatory retain full supported generated domain, conditional on independent acceptance; no convenience cap or shape exclusion may silently remove it.

**Repeated same-view references:** Plan once and preserve every reference; validate role/offset windows independently. No per-accessor last writer ownership.

**Interleaved or partial-window views:** Precode decision still required. Whole-view physical stride/count can support byte-preserving layouts; current generated profile has no interleaving. Valid cases outside a declared finite whole-view stream profile must be Unsupported, not declared malformed solely for differing accessor counts. Do not broaden to stride<=256 merely because primary permits it without resource/consumer evidence.

**Metadata and accessor roles sharing a view:** Inventory actual roles before transformation. Never compress metadata-only views by inferring an accessor role accidentally; incompatible index/vertex role sharing is independently malformed under core, while unsupported other sharing gets an explicit profile disposition. Do not rewrite table associations.

**Source buffer selector/external resource:** All retained generated views select embedded buffer zero. Check selector and actual source ranges before any rewrite. Additional/external/data-URI buffers are not discovered or silently treated as BIN; declare Unsupported for valid excluded shape or InvalidInput for nonexistent/out-of-range reference.

**Already meshopt-compressed input:** Current helper must not read virtual fallback parent bytes as source. Reviewed standalone caller feeds uncompressed GLB and producer supplies precompression bytes. Recommend explicit valid-but-Unsupported initial operation disposition unless a separately proved idempotent/decoded plan is selected; this choice must be documented before implementation, not silently attributed to old behavior.

**Extensions and extras:** Known generated root/primitive extension objects and metadata/extras stay raw; extension declarations remain. A compressed view must merge meshopt bookkeeping rather than overwrite unrelated fields. Unknown buffer/view-affecting extensions and unknown required semantics need explicit Unsupported before rewrite; do not silently strip them. Known KHR_materials_unlit and generated metadata/topology/quantization/restart uses are mandatory.

**Unknown GLB chunks:** Core permits unknown chunks to be ignored by readers, not a lossless rewrite to silently delete potentially referenced chunks. Generated sources have none. Preserving them or refusing a valid excluded representation is an explicit precode disposition, not generic malformed-input acceptance.

## Retain / rework / replace / remove

**Retain**

- Accepted F0 primitives are dependencies, not new codec acceptance.
- Lossless ATTRIBUTES/NONE concept and eight-byte metadata placement after controls.
- Current generated vector scalar/topology/metadata/list meaning and intrinsic reuse dependency invalidation.
- Existing meaningful raw-byte roundtrip regression only as supplemental evidence; encoder/decoder share meshopt lineage.

**Rework**

- Per-view ownership, checked exact source integer interpretation and source-buffer selector/ranges.
- Shared/partial/interleaved role dispositions and source view/window checks.
- Bounded output/fallback plan and targeted raw serialization.
- Known extension/raw metadata preservation and producer/private codec integration.

**Replace**

- Last-accessor-wins BTreeMap interpretation with one physical-view plan.
- Standalone Value numeric roundtrip with preservation of untouched raw values.
- Unchecked encoder bound and allocation arithmetic with admitted checked requested-storage plan.

**Remove after migration**

- Old codec classification/transform implementation once both real callers migrate; no duplicated ownership.
- Public compress_file direct-write path/module export and hidden adapter protocol are coordinator/lifecycle removal gates, not changes performed by this lane.

## Encoded size and working-storage facts

These are static source derivations, not measured resource acceptance. Resource ceilings and source/publication semantics are assigned to the separate lifecycle/coordinator lane.

**actual codec pin:** Cargo.lock meshopt 0.6.2 checksum e01e77ead21976b3a9f01ec1724f766923da74f0726364e2b0f425658935d71a; actual vendored source hash in source pins.

**current stream bound:** For N records and S in {4,8,12}, B(N,S)=1+ceil(N/256)*S*(S/4+260)+32, derived from vertexcodec.cpp:1784-1803. Compute ceil by quotient/remainder, not unchecked N+255. All operations, accumulated output and isize/u32 host/container conversions must be checked.

**output bin bound:** Conservative U <= sum(raw_view_lengths)+sum(B_i)+7*view_count; include final align4 padding explicitly. This bound reserves full worst-case capacity per encoded stream and does not assume compression shrinks data.

**virtual fallback:** Sum eight-byte-aligned decoded compressed view lengths with checked arithmetic; placeholder size is virtual address-space metadata, not permission to allocate it or decode it redundantly.

**glb bound:** T=28+align4(output_json_bytes)+align4(output_bin_bytes), u32 GLB header/chunk limits checked before materialization.

**unresolved:** Output JSON delta representation/ceiling and actual requested capacities, per-payload/profile limits versus producer aggregate work, and whether direct serialization to owned staging avoids a BIN copy must be settled before code. Do not expose a memory-limit promise based on sum of logical lengths or C1 constants.

Simultaneous ownership layers:

- Captured input bytes and raw JSON borrowing
- Bounded structural-admission scratch, role/reference containers and consumed plan
- Codec output BIN including worst-case reservation for current stream
- Targeted output JSON bookkeeping/raw serialization
- Final GLB copying BIN under current encode_glb; producer original Value+optional raw serialization can coexist

## Typed causes

- Malformed framing, invalid selected-buffer/accessor/view references/windows or nonintegral source integer => InvalidInput.
- Valid excluded format/extension/layout/owned representation => Unsupported with explicit profile cause.
- Consumed checked work/bytes/count/host/container/allocation ceiling => ResourceLimit distinct from excluded semantics.
- Codec encode failure => typed codec/format cause; required resolver/publication I/O belongs to caller, no string matching.
- Existing runtime mapping lacks ResourceLimit; coordinator must settle causal presentation if resource request exposed, not collapse resource cause into Unsupported.

## Independent sensitive acceptance plan

**G1 — Authored unquantized, normalized/padded U16 and padded U16/f32 feature streams.** Hand-authored raw BIN record bytes, independently decode output via pinned external decoder, compare full bytes including padding; compare accessor/raw nodes/extension associations. Nonzero padding makes missing padding detectable.

**G2 — POINTS, multiline restart, polygon with exterior+hole/multiple polygons and fill-only GLB.** Explicit expected U32 arrays incl u32::MAX, independent raw topology walk, polygon and feature/table associations. Corrupt offset/restart/feature-table link without changing harmless byte counts must fail the oracle.

**G3 — 64-bit numeric/bitset/UTF8/null/list/string metadata.** Signed INT64 extrema and >2^53+1, raw UINT64 max fixture, FLOAT64 authored bits, BOOLEAN byte crossing, Unicode/empty UTF8 strings, noData collision-sensitive values, array-as-STRING with heterogeneous integers/null/order. Independently read table refs/count/class and binary offsets; alter raw byte, stringOffset or schema noData/table link as controls.

**G4 — Structural variable/fixed arrays and shared metadata references.** Independently authored values/arrayOffsets/stringOffsets with exact expected rows and raw bytes. Establish preservation coverage separately from actual writer source profile. Controls swap view IDs or array/string offsets.

**G5 — Per-view interpretation sensitivity.** Compatible repeated references, differing accessor offsets/counts, valid interleaving/partial window, contradictory stride/layout, mixed index/vertex role, missing/out-of-range view and nonzero actual source-buffer selector. Assert chosen declared supported/unsupported/malformed result, not old implementation parity.

**G6 — Already compressed/fallback/extension/chunk decisions.** No actual parent fallback buffer for compressed view; virtual buffer with offsets unlike actual compressed BIN; altered extensions and unknown chunks. Prove no re-reading virtual ranges or silent loss. Test compressed extension declarations and fallback reference graph independently.

**G7 — Raw numeric/JSON interpretation.** Decimal oracle over exact source tokens: integer 4/4.0/4e0, invalid fractional near-boundary token that rounds integral, host/count overflow, huge zero exponent, duplicate decoded key, Unicode surrogate, and untouched extras/schema 9007199254740993.0 or exponent beyond finite Value. Compare preserved untouched raw numeric tokens, never Value equality as oracle.

**G8 — Output/accounting boundaries.** Small explicitly selected caps exact/one-over, alignment boundaries, many shared refs but one encoded view, codec-bound vs actual output, incompressible input increasing size. Separately verify checked B/formula against actual pinned API/encoded cases and output/fallback reference dimensions; model arithmetic sensitivity controls before big probes.

**G9 — Both real integrations.** Portable/native real vector generation with meshopt toggled, quantized/unquantized/list/fill/profile combinations and standalone hidden-worker migration. Decode against independently authored semantic expectations, not Python/native agreement or common decoder helpers. Check reuse invalidation when actual codec source changes.

Evidence limitations:

- A pinned meshopt JS/WASM decoder is a separately executed reference but may share upstream implementation lineage with Rust meshopt; retain canonical compressed/expected vectors and raw independently authored bytes, document the limitation.
- C1 production payload inspector may establish admitted integrity after independent acceptance; it does not verify table/feature meanings, world bounds, geometry approximation or source fidelity.
- Historical Python vector writer has design lineage with native writer and invokes this same compressor; parity is supplemental.
- No controls or production codec probes were executed by this preparation audit.

## Required precode settlement

- Current C1 exact-source independent acceptance/merge before borrowing owners.
- Settle finite full generated source profile plus shared/interleaved/existing-compressed/extensions/chunk dispositions; no silent source-use loss.
- Select one small actual raw plan/output representation and numeric helper boundary; account producer serialization.
- Settle consuming resource ceilings, exact requested capacities/output bounds and typed ResourceLimit mapping with lifecycle owner.
- Author sensitive independent raw/semantic/control fixtures and specify final independent acceptance.

This preparation resolves no implicit-template standards claim, parent closure or release gate. Detailed facts, exact source hashes, source roles and resume pointers are in [audit.json](audit.json).
