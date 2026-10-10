# Independent archive prerequisite and A2 checkpoint review

2026-10-10. Separate nonauthor, preimplementation review. Baseline identity was
supplied by the coordinator; this lane used no Git, Cargo, installation, browser,
producer, conversion, contacts or production edits. Owned output is this directory
only. [reviewed-sources.json](reviewed-sources.json) pins the reviewed bytes and
distinguishes source inspection, checkpoint reading and receipt-binding checks.

**Verdict: the revised stored-archive prerequisite is implementation-ready in
its finite C1 scope.** The initial proposal needed descriptor extent/error and
extra-field/I/O decisions; the coordinator incorporated the concrete resolutions
below before this verdict. This does not accept current production or a future
implementation. Fresh separate final-source acceptance remains mandatory.

## Executed control and current source dispositions

I independently replayed the coordinator's tiny overlap driver against the exact
frozen portable binary into the new directory
`/tmp/rusty-tiles-archive-prereq-review-overlap-20261010`. Both ordinary and nested
physical-member cases exit 0. Binary, driver and both archive SHA256 values agree
exactly with the original receipt. [overlap-replay.json](overlap-replay.json)
records these executions. This establishes the physical admission gap, not a
generic ZIP specification violation, decoded geometry defect or allocation bug.

| Current component | Decision and evidence |
| --- | --- |
| `validate/admission.rs` | Rework under private archive ownership; remove the validator-owned decoder after integration. Retain checked envelope/ZIP64 arithmetic only through final independent tests. No physical extent table exists; descriptor success discards consumed length and uses central start as its boundary. |
| Extra-field parsing | Rework: current `zip64_sizes` skips all TLV framing/duplicate checks when no sentinel requests expansion. This is a static observed gap; this lane did not execute a malformed-extra production case. |
| `archive3tz::validate_open_3tz` | Rework to borrow the selected `Read + Seek` source. Current File clone, unbounded local index `Vec` API, and broad `by_name(...).is_err()` manifest classification cannot supply the new contract. Existing C1 admission bounds its actual index input; no executed unbounded C1 allocation claim follows. |
| C1 held-file/identity and whole-member reads | Retain for their existing scope. The source is opened once, admission precedes ZIP/index allocation, all members are fully streamed for hashing/CRC, and identity/length/mtime are checked afterward. These checks do not create an atomic snapshot against in-place mutation. |
| C1 report/payload/geometry policy | Keep separate. Current structural child-box policy and explicit uninspected geometry/metadata limits do not prove A2 decoded-content coherence. |

## Settled minimum contract

The codec owns no paths, file metadata, RunControl, converter or validation
report. A private five-field `ReadLimits` supplies archive size, central size,
entry count, member size and aggregate stored bytes. C1 maps its public limits
once. The codec accepts `Read + Seek` and the caller-known length; regular-file
and source-stability choices remain C1's. It does not need a public reader API,
generic role registry or archive resource framework.

The legacy standalone `validate_3tz` wrapper has ungated ZIP directory allocation
outside the bounded C1 profile. Preserve its path facade without inventing codec
defaults; its later admission/resource disposition remains open. This readiness
verdict covers the explicitly limited C1 caller, not every archive helper.

Disjointness covers local header/name/extra, stored data and the chosen descriptor,
including zero-byte members. Physical order may differ from central order;
adjacency and gaps are allowed. A nested ZIP payload with no separately indexed
interior member remains valid. Coherent nested/shared member records are
Unsupported by this profile; malformed names/fields/ranges are InvalidInput.
Central-directory/source boundary crossings remain malformed, as stated in the
finite contract. This is a declared profile rule, not universal ZIP conformance.

Each descriptor retains at most two fixed candidates for its selected 32/64-bit
width. Structural matching checks signature when present, CRC and both sizes.
After physical sorting, filter against the next coherent local-record start or
central start. No structural match or truncation is InvalidInput. Structural
matches that all cross another coherent local-record boundary are Unsupported.
One distinct fitting end wins; multiple distinct fitting ends are Unsupported.
CRC equal to the optional signature must not force signed interpretation. Exact
boundary equality is legal; padding does not force an extent to fill the gap.

Parse every extra TLV even without ZIP64 sentinels. Refuse malformed framing and
duplicate ZIP64 tags; preserve unknown well-framed tags as opaque. Consume required
ZIP64 values in sentinel order through one shared local/central implementation.
ZIP64 descriptor width and local placeholders need independent controls; successful
32-bit fixtures cannot establish their meaning.

Index allocation follows checked exact length `24 * (entry_count - 1)`, admitted
member length and host range. Read its admitted raw stored range and check CRC32
explicitly. CRC mismatch is typed malformed; underlying seek/read errors,
including injected InvalidData, remain Io. Only the typed missing-member variant
becomes MissingManifest. C1 maps private codec failures directly. Its remaining
payload ZipFile InvalidData cause classification is explicitly unresolved outside
this slice; do not claim all C1 I/O provenance is fixed.

Resource evidence must prove admission before allocation: source limit before
the bounded tail (at most 65,557 bytes), central count/bytes before extent-table
or ZIP-directory allocation, local fields/data/candidates before downstream
materialization, exact index length before its bytes. At most two descriptor
candidates per admitted entry give O(entries) storage; sorting is O(entries log
entries). Measure actual Rust layout and allocation scaling. Existing C1 defaults
are 1 GiB source/stored bytes, 16 MiB directory, 65,536 entries and 64 MiB/member;
these are domain limits, not whole-operation RSS or work-time guarantees.

## Frozen A2 checkpoint adjudication

The format audit and receipts honestly establish bounded materialization of two
five-root candidates, all six payload slots/four terminal links, exact frames,
and sensitive wrong-frame/occupancy controls. The actual client also loads
forbidden root JSON and ignores present TILE_TRANSFORM. Its tolerance and the
official validator's statically observed TODO do not prove an unavailable-root
JSON-template exemption. **Full production A2 stays HELD.** Public viewer
selection/picking remains unexecuted; the partial-implicit product is distinct
and has not been selected. Archive work neither selects that product nor shrinks
0.4.0 obligations.

The metadata/provenance checkpoint has 36 model cases, 28 pinned fixture
inspections (24 admitted/four unknown-extension refusals), and 11 inventory-model
cases. I independently replayed the latter at nice 10 with two-worker environment
caps; [inventory-replay.json](inventory-replay.json) equals its frozen receipt.
Trusted source roles, exact root snapshots, historical vector state and actual
original-payload inspection are sound ownership decisions. Arbitrary report
labels cannot authorize used-member exemptions. Public standalone C1 support for
the new A2 report/snapshot roles remains an integration gate; model GLB headers
and skeleton generated JSON are not production payload/schema acceptance.

The payload checkpoint records six exact-source cases per portable/native mode,
23 scene/content controls and six arithmetic controls: 23 selected primitive
instances, 54 own/ancestor checks and 2,487 model arithmetic steps. Source-selected
referenced extrema, diagonal positive T/S, exact normalization and local/ancestor
frames form a coherent finite design. The peak modeled unreduced width is 116
bits under 127-bit preflight. This does not accept a Rust i128/GCD implementation,
the proposed 257-unit accounting, 4 MiB fact storage, general glTF, resource
closure, global accuracy, source capture or A2 format. Extraction must preserve
C1 reports and caller-owned budgets; final independent acceptance is still needed.

## Mandatory implementation stop gate

Before production retention, use independently authored disjoint/adjacent/
reordered/nested records, opaque nested-ZIP control, duplicate offsets,
header/data/directory overlap, all classic/ZIP64 signed/unsigned descriptors,
CRC-signature ambiguity, malformed extras without sentinels, and exact/one-over
limits. Prove Cursor/held-file agreement, read/seek/InvalidData causes, truncation,
index corruption/missing-manifest separation and preallocation sensitivity.
Replay this CLI control before/after, meaningful C1/archive suites, and measured
extent storage under coordinator-owned builds. Bind final-source/artifact hashes
and applicable platform/wheel/CI evidence before develop merge. No A2 retention,
legacy removal, standards certification or release gate closes here.
