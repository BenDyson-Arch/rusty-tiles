# Separate whole-slice preimplementation decision, 2026-10-10

**Approve implementation of the bounded C1 payload/JSON foundation under the
completed frozen contract. All A1–A14 prerequisite choices are settled.** This
decision supplies the final incorporation handshake; it accepts no production
implementation, codec, full C1 conformance, A2 change or release. Reviewer:
`/root/payload_prereq_review`, separate from the contract and implementation
authors. No production edit, Cargo, install, producer or heavy execution was
performed in this review.

The reviewed contract SHA256 is
`3cb5ad341a73cdda77a9dd4a1925cf9800af6e37e49b902eb6db0c4f0c3ee1e3`.
Its immutable copy is
`history/20261010-implementation-ready/c1-payload-integrity-contract.md`.
The completed `freeze-20261010-a14-selection.json` SHA256 is
`44c45ca15958e1068ecaf62349902c720fe93bd8dacd849ca4f4b08919dff8c9`.
The reviewer independently verified all eleven current preparation files,
twelve immutable own snapshot files and sixty-six copied foreign input files,
including every recorded foreign length. The current and immutable contract
copies match. The freeze records readiness as false at its author's earlier
pre-review moment; this new decision establishes readiness without rewriting
that historical observation.

The actual implementation baseline is accepted develop merge
`0eebdeebf013591460e6ac77485ad91b7401aada`, complete tree
`58aadfdc1bb78c507d020ab91494dc83a3d646a3`. Actual worktree HEAD and all 93
production hashes match the independently accepted reader source. Historical
C1 source inventory and original primary-control executable retain their own
earlier identities; this decision does not rebind them to the new baseline.

## Concrete prerequisite dispositions

| Gate | Settled decision and basis |
| --- | --- |
| A1 | Map exactly the six consumed JSON/payload facts once at C1's operation boundary. Separate remaining aggregate components admits the minimum before resource/decode work. Public fifteen-field reporting and other owners stay with C1; no unused policy bag or meter. Actual source/caller inventory establishes this narrow surface. |
| A2 | Share private `FormatError` with InvalidInput/Unsupported/ResourceLimit and `PayloadError<E>::Format/Resolver`. C1 maps format failures once and unwraps causal resolver errors. The actual implicit `crate::Error::Io` bridge must precede generic conversion; InvalidData/UnexpectedEof are real Io when caused by the resolver. Source inspection establishes the current conversion hazard. |
| A3 | Pass the selected borrowed `&dyn Fn` explicitly through bounded expansion, recursive external documents, subtree JSON and JSON-encoded extras. Presentation budget retains counters only. The source map proves all three real implicit contexts and the existing function-pointer capture limitation. |
| A4 | Preflight all declared buffer sums and all accessor components before the first resolver. Pass remaining actual buffer/image allowance; caller archive size/read-budget checks precede allocation/read, followed by defensive returned-length checks. Repeated declarations still charge repeated bytes. ResourceLimit-before-callback precedence is deliberately selected. |
| A5 | Preserve individual conservative base64 decoded admission, then canonical alphabet/padding/trailing-bit validation and exact actual-size admission against the resource remainder. `AA==` at remainder one and `AAA=` at remainder two remain admitted. Independent arithmetic/grammar models and current decoder inspection establish the distinction between estimate, actual charge and bounded temporary allocation. |
| A6 | Charge all ordinary/overlapping view bytes and meshopt output once logically. Rework the full-word copy's final prefix; the tiny std-only probe demonstrates old capacities 8/12/20 versus exact 2/6/10 for lengths 2/6/10. Rounded backing plus bounded output requests at most `2*L+3` bytes, with checked arithmetic; this is separate from allocator overhead, other retained data and RSS. |
| A7 | Retain all-accessor scalar component counts, full POSITION counts per primitive, and successful aggregate charging/cache by resolved payload member name. Distinct aliases and repeated resource reads retain their meaning. Actual caller/counter inspection establishes these consumers; no physical-work or deduplication claim follows. |
| A8 | Retain per-parse inclusive byte/node/depth ceilings, root node and root depth zero, keys excluded. `{"a":[0]}` costs three nodes/depth two. Charge children before raw skip/retention and do not charge them twice. Immediate-container duplicate/child admission precedes descendants; the corrected frozen proof controls the deliberate compound precedence and exact boundaries. |
| A9 | Close selected-primary provenance: exact published glTF 2.0.1 source and accessor schema, five freshly pinned references and three b3dm dependencies were independently checked/read. Upstream 85/48-byte meshoptimizer golden identity is established. These facts do not certify a decoder execution or universal format coverage. |
| A10 | Correct the six independently executed valid-input rejection defects. Exact integer interpretation covers every inventoried consumed path, including preflight count, stride fallback and direct target comparisons. Numeric b3dm BATCH_LENGTH has its own schema integer authority and uint32 gate. Viewless nonsparse bounds retain required POSITION shape/finite/component meaning while skipping decoded equality and any invented cross `min<=max` rule; stored bounds and decoded zero integrity remain checked. |
| A11 | Trusted bundled schema and separately selected non-C1 expander parser are explicit exceptions only. Every untrusted actual C1 JSON consumer, including schemaUri and nested implicit extras, still uses the selected shared owner. Source inventory establishes these boundaries. |
| A12 | Preserve the full admitted C1 profile, thirteen checks, counters and exclusions: core modes, local glTF/GLB/modern b3dm, data buffers, viewless values, quantization/matrix padding, meshopt NONE/fallback roles, draft restart and optional polygon references. Conditional algorithm retention requires later independent execution. No narrower A2 geometry domain, unused facts or new semantics are selected. |
| A13 | Clear accepted-reader/rebase prerequisite. The reviewer verified current 93 production pins, actual/reviewed/merged tree equality, all 30 retained lossless remote records, exact-head completed runs and 21 successes. Ten installed-wheel and four official Blender receipts each retain 44 tests, zero failures/errors/skips and empty PATH. This verifies the accepted prerequisite's retained evidence, not a new payload implementation. |
| A14 | Select corrected iterative raw_value-only borrowed admission and exact native-u64/Overflow consumed numbers. Source/actual pinned serde inspection, 53 independently rerun frozen controls and mathematical/storage/work reasoning settle the mechanism. Scalar Unicode validation, presence-aware fields and one decoder at a time are mandatory. No global AP/normalization/marker/path sidecar/full payload DOM. Ordinary Value derives after shared admission and truthfully refuses an unrepresentable valid document as Unsupported. |

## Selected representation and owner obligations

The approved private graph is `content_integrity::{json,payload}`. Remove the old
validator-owned implementations after migration. C1 owns one profile selection,
format-name choice, URI/reference/read policy, causal adapters and report/cache
assembly. The JSON owner provides shared admitted borrowed roots, the ordinary
owned adapter and narrow private limits/errors. The payload owner consumes
presence-aware borrowed records and exact numbers, immutable bytes and a caller
resolver. It returns only the actual schemaUri string plus existing counters
and exclusions. This removes the old document's one-consumer representation
cost and public validation/default coupling; it supplies no new metadata facts.

Each selected numeric property is fetched once per real record. Visit arbitrary
attribute-name/reference pairs once; do not rescan their entire object per key.
Absent optional fields preserve their defaults; present null is retained as a
present raw value and gets its actual consumed type outcome. A derived
`Option<&RawValue>` must not erase that distinction. The final contract includes
paired absent/null/correct-type/wrong-type controls for numeric fields, optional
references/enums and URI/extension/schemaUri paths.

Exact overflow remains mathematical range information. Huge valid counts or
declared lengths exceed their work ceilings as ResourceLimit, while invalid
actual references/ranges, enums and uint32 BATCH_LENGTH return InvalidInput.
Fractional/nonzero-negative unsigned values remain InvalidInput before rounding.
Opaque payload numbers are not materialized just to reach schemaUri. Ordinary
document numeric variants, marker-lookalike objects, implicit equality and
conversion-report behavior retain their justified existing meaning.

The separately approved [numeric mechanism decision](2026-10-10-numeric-mechanism-decision.md)
binds exact helper arithmetic and O(B+N) representation storage. Root raw skip
may scan and allocate source-bounded nesting scratch before descendant logical
admission. One decoder survives at a time; pending/immediate logical lengths are
bounded by admitted nodes. Raw-substring skipped lengths total at most
`(D+1)*B`, with additional bounded key/Unicode/lookup work. Requested capacities,
reallocation overlap, host arithmetic and actual record storage require final
source proof. These bounds are not a total-process RAM guarantee.

## Remaining production acceptance

No policy, representation or prerequisite identity question remains open before
coding. The following are obligations of the implemented source and its later
fresh separate nonauthor acceptance; they are not waived by this decision:

1. Verify checked capacities, host/token arithmetic, borrowed lifetimes and all
   exact consumed range/category gates, including presence/null handling. Prove
   old copies/defaults and public validation imports are removed from the private
   format owner.
2. Exercise exact/one-over limits through every real ordinary, payload/table and
   implicit JSON consumer. Instrument resolver/allocation/decode admission,
   canonical padded-base64 positives and meshopt final-copy backing/output
   capacity; test declared compound ordering and causal Io through adapters.
3. Rerun the fourteen selected-primary controls and corrected exact-number,
   Unicode, duplicate, marker-object, representation-refusal and compound cases
   on final production source. Independently verify full core/quantized/matrix/
   viewless/data/meshopt/restart/b3dm/polygon domain controls, raw scalar/bounds
   oracles and sensitive corruption negatives. Historical regression agreement
   is supplemental.
4. Verify aggregate components, same-name cache, distinct aliases, shared
   accessors/resources, schemaUri policy and full reports/counters/exclusions.
   Retain immutable inputs and truthful physical/resource observation limits.
5. Bind final source/artifact/fixture hashes and obtain fresh distinct nonauthor
   final-source acceptance plus applicable CI, installed-wheel and official
   Blender integration before an accepted develop merge. The coordinator alone
   owns sustained Cargo/install/resource work with the agreed two-worker/nice10
   scheduling.

The two historical mutable foreign-document pin exceptions and overwritten
earlier exploratory executable remain explicitly disclosed; their old original
bytes/artifact are not newly verified. Completed dated decisions and copied
inputs bind this approval. Full A2 literal-template hold, broader geometry and
metadata claims, #113/#121, main/tag/publication/release gates remain unchanged.
