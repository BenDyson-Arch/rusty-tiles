# Nonauthor exact-number mechanism decision, 2026-10-10

**Approve precode selection of the corrected iterative raw_value-only borrowed
JSON admission mechanism and exact consumed unsigned-number interpretation.**
The actual pinned serde proof and source-derived bounds settle A14's
representation decision. The coordinator must bind this decision into the
revised frozen contract and settle A13's accepted archive-reader/rebase identity
before whole-slice production coding. This is not production implementation,
codec, full retained-profile, A2 or release acceptance.

## Independent evidence verification

The original mechanism manifest SHA256
`8981b67ce0f4ab1f15d794b32f2294e900d16a2a99b2e6021eadbddcd2899f0f`
has fifteen retained file hashes. This reviewer verified every file identity,
read the actual source and receipts, and confirmed its original JSON copy equals
the frozen source SHA256
`d9a7758368502adbb4c29c23c9c9bcbe53aad11ff571df7b1d54f916f3cdefe8`.
That phase preserves 22 finite-Value and 26 iterative-raw receipt rows. Its old
live binary was overwritten by the later probe; no independently verified
original-phase binary execution is claimed here.

The reviewer found that raw skipping did not validate Unicode surrogate pairing
in scalar strings. Local serde_json read.rs ignore_escape explicitly consumes
hex digits without validating those codepoints; the first iterative proof
therefore did not preserve the original string interpretation. The coordinator
fixed this in a separate dated phase, leaving the earlier records immutable.

The corrected manifest SHA256
`0026d03177e0fb66ec5fb7dcb372cd6aa7310d474af081163f72217bc333f4ec`
has seven retained file hashes, all independently verified. Corrected raw-owner
source SHA256 is
`2be33241cc109d80c509ebc2fb40ad70ba3a6bb478892ee8f318c17eb4d6cbc2`;
main proof source SHA256 is
`dbba5adac728dc25aa269d030f976e0aadc0ebf6fbe7e00d1fa5a90d6789dc29`.
Cargo.toml selects serde 1.0.229 and serde_json 1.0.151 with only float_roundtrip
and raw_value numeric-related features; no arbitrary_precision or global
normalization is used. The complete lockfile and coordinator build log are
retained. Probe String/prefix errors are presentation only: production must use
private typed three-way failures and typed limit state, never prefix matching.

The corrected frozen executable SHA256
`d5c53c7df6a41e61b056dc394e30cfc8377fcead377cd3e5349b94699941312d`
is 9,068,248 bytes. This reviewer independently checked the artifact hash and
reran it once at absolute nice 10 and affinity {0,1}. It completed successfully
in 0.325 seconds with empty stderr. Stdout was byte-identical to corrected
receipt SHA256
`af6a57d2f1d7951cffcec805d51bc50936823ce30ca860ca417b4566d8e026f5`.
The [rerun receipt](2026-10-10-numeric-proof-rerun-receipt.json) and full stdout
are separate records in this review namespace; no Cargo/build/producer/browser
or production codec ran in this lane.

The corrected proof retains 22 finite exploratory rows and 31 raw rows. Actual
assertions also check depth64 equality, depth65/100,000 failure, 65,536-node
equality and one-over, and explicit byte equality/one-over. Finite exact scalar
controls cover decimal/exponent integrality, cancellation, zero, negative,
u64 maximum/one-over, exact large native integer, and fractional lexemes whose
f64 representations appear integral. Raw controls add 1e400 exact overflow,
1e-400 fractional rejection, nonnumeric scalar types, escaped duplicate keys,
deep duplicates, malformed/trailing/NaN/exponent syntax, Unicode lone/bad/valid
surrogate strings, and an explicit compound-fault precedence control.

## Selected owner and meaningful corrections

Admit the explicit JSON byte ceiling first and root node/depth second. Obtain
one borrowed root RawValue with grammar/trailing-token admission. Traverse its
values iteratively using borrowed raw/depth work items. Each container checks
immediate unique decoded keys and charges each child node/depth in its seed
before borrowing/retaining that child. Do not charge that child again when
processing it. Object keys are not value nodes. Decode scalar strings with
deserialize_str and a visitor returning (), validating Unicode without retaining
owned scalar strings. Drop every container/scalar decoder before processing the
next queued value. Return a private checked borrowed document/root, not a new
exact-number AST or numeric marker object.

Immediate-container key/child admission precedes processing descendants;
children are processed left-to-right by reversing them into the LIFO worklist.
This deliberately differs from the old fully recursive failure precedence.
For a duplicate root key with a deeply limited first child, InvalidInput for
the immediate duplicate wins; the executed compound control proves this choice.
Byte/root admission and root grammar occur before that traversal. Other compound
cases follow the stated order, not diagnostic text or incidental old parity.

Ordinary actual C1 consumers may derive owned Value after this same validated
grammar/Unicode/depth/node owner. Keep their current numeric variants and
opaque object behavior. A failure to construct Value after successful admission
is an explicit Unsupported finite-owned-representation refusal, not malformed
source syntax. The proof demonstrates this for 1e400/-1e400 without error-text
matching. Numeric payload fields instead consume raw tokens and reach their
resource/domain gates even when they exceed f64. Unknown payload/extension-owned
numbers need not be materialized merely to obtain the one actual schemaUri
string consumer. The representation refusal must be disclosed as an adapter
profile choice; it is not universal JSON/glTF invalidity.

The payload owner gathers borrowed numeric properties once per actual record,
using the complete inventory in [raw-only-tradeoff.md](raw-only-tradeoff.md).
Arrays, references, buffer/view/accessor/meshopt counts/offsets/lengths/strides,
primitive mode and attributes, polygon references, image/texture/material
references, numeric b3dm BATCH_LENGTH and all old target/stride/preflight
bypasses use the same exact helper. BATCH_LENGTH has its own JSON Schema integer
authority and uint32 domain. Node/scene/animation/morph facts remain uninspected.
No global AP feature, parser numeric mode, number normalization, generic path
sidecar, unbounded bigint or second full DOM is justified.

Return only the currently consumed schemaUri string plus the existing checked
counters/exclusions. The old private owned payload document had only that one
downstream consumer. C1 retains URI authority, reference/resource admission and
schema-document reads; the format owner does not resolve schemaUri itself.
This is a concrete ownership/representation reduction, not new metadata facts.

## Exact helper proof and owning ranges

The helper's numeric-type guard rejects bool/null/string/array/object before
digit arithmetic. Raw admission establishes actual number grammar for the
remaining signed/digit tokens. Zero is checked before sign/exponent magnitude,
so -0 and arbitrarily scaled zero remain zero. Nonzero negatives are invalid
unsigned declarations. Let F be fraction digits and T trailing significand
zeros. After removing leading/trailing zeros, the mathematical integer scale
is exponent-F+T: negative scale means nonzero fraction; nonnegative scale
describes significant digits followed by zeros. Compare decimal digit length
before checked u64 accumulation, so exact native value and larger-than-u64
magnitude remain distinct results.

Exponent accumulation saturates at token_length+32 in the probe. This is sound
under its <=8 MiB token domain: beyond that positive cap the significant decimal
length exceeds u64's twenty digits; beyond the negative cap even all possible
trailing-zero cancellation leaves a nonzero fractional value. All-zero tokens
were handled first. The largest accumulator intermediate is below
10*(8 MiB+32)+9, so the probe's i64 arithmetic is bounded. Production must use
checked host/token conversions and arithmetic rather than assume arbitrary
private ceilings fit i64. No power expansion or token-proportional bigint is
allocated, and successful native accumulation adds at most nineteen/twenty
digits plus at most nineteen/twenty zeros.

Exact overflow is not an InvalidInput verdict on numeric syntax. A count or
declared length exceeding its component/byte work allowance is ResourceLimit;
an array/reference/actual byte range or enum/uint32 violation is its domain
failure. Those consuming gates must decide before native host conversion loses
the value. Fractional and nonzero negative unsigned values remain InvalidInput.
The probe proves exact classifications, not the future full production graph's
choice of every range/category/order. That mapping is specified here and in
the revised contract and is a final-source acceptance obligation.

## Source-derived space/work bounds

Let B be admitted source bytes, N admitted root-inclusive values, and D the
inclusive root-zero depth ceiling. Root RawValue skip may scan the whole source
and build its iterative nesting scratch before logical descendant admission.
It must not be described as zero allocation/work before depth rejection.
Each visited container decoder is dropped before descending, so only one
decoder scratch allocation remains live at a time; scalar string decoding also
has only its local temporary scratch. Raw source UTF-8 is checked by RawValue's
end_raw_buffering. Surrogate pairing requires the corrected scalar decode.

Raw skip scratch nesting entries and decoded scalar/key bytes are bounded by
source length. A container key set retains at most its immediate key count,
bounded by charged children plus one possible key preceding failed child
admission; aggregate decoded key bytes cannot exceed that container's source
bytes. The key set is dropped before child processing. It has O(N) node/header
overhead in addition to O(B) string storage, not an implied byte-free parse.

Pending and immediate-child lengths together are at most the already charged
unprocessed values, hence <=N. They store borrowed fat pointers and depth,
not copies of overlapping raw source bytes. On the observed 64-bit host one
raw/depth slot is 24 bytes. Vec capacity growth and possible temporary
reallocation overlap are separate from logical lengths; production must bound
checked requested capacities/slot multiplication and allocation failures.
The probe establishes the representation's O(N) slot bound, not exact future
allocator/RSS peaks. The same requirement applies to selected borrowed record
arrays; gather selected fixed numeric fields once, avoid an all-opaque-fields
map, and visit unbounded attribute-name pairs once rather than rescanning an
object once per key. Release temporary views when dependencies allow.

The sum of raw substring lengths skipped is <=(D+1)*B: each source byte occurs
in at most one raw value per visited depth, including the initial root scan.
Container punctuation/key traversal and scalar Unicode validation add bounded
source passes; UTF-8 validation, duplicate-key comparisons and native allocator
bookkeeping add implementation costs. BTreeSet duplicate lookup costs
O(keys*log(keys)) comparisons with source-bounded key bytes. Do not turn the
substring bound into an exact CPU-byte count, zero rescans, or a whole-process
memory claim. Incomplete/bad syntax and deep invalid input remain bounded by
the admitted B and actually visited depth, and the source proves no recursive
stack of raw-scan decoder capacities survives descent.

These are concrete bounded representation facts sufficient to select the
mechanism. Final production source still must demonstrate checked capacities,
borrowed lifetimes, each limit's propagation, causal typed failures, every
consumed numeric gate, sensitive no-forbidden-resource/decode admission controls,
and full admitted-profile/report semantics. A separate final-source nonauthor
review must inspect those implementations and executions. No such acceptance
is supplied by this mechanism decision.
