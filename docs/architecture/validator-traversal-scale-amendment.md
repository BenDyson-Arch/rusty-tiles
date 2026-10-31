# Validator traversal and C2 at scale: amendment VS-B

Status: author amendment for validator-scale slice VS-B, written before the code. The
nonauthor final-source review and the coordinator's gate are still pending. Nothing here
records acceptance.

This note amends the traversal parts of the C1 validator
([c1-payload-integrity-contract.md](c1-payload-integrity-contract.md), which stays
byte-identical because the C1 production-control driver pins its hash), the
[C2 contract](c2-decoded-containment-contract.md) (phase 1, phase 2 and the slice-3
amendment), the [I1 contract](i1-implicit-read-contract.md) (OD-7 and §7) and the
[external-tileset memo amendment](c1-external-tileset-memo-amendment.md) (EM-7, EM-8). If
the texts disagree, this note governs. Design:
`~/.cache/rusty-tiles-evidence/validator-scale-design-20261013/design.md` (§1.2 M8–M15, §2
W6–W11 and W14, §3, §4.2 A-I1-2 and A-I1-3, §4.3 A-C2-1 and A-C2-2) with the coordinator's
decisions beside it. VS-A ([catalog amendment](c1-archive-catalog-scale-amendment.md)) is the
base. VS-C owns the limit surface (opt-in volume caps, `--max-volume`, Python `max_volume`,
`storedBytes`, cancellation); VS-D owns the scale generators and the frozen κ.

## Defect

After VS-A the traversal still held, per validation:

- one 376-byte C2 `TileFact` per visited tile, one reference record per payload reference,
  one content volume per content `boundingVolume`, and one `(String, UriFacts)` row plus a map
  entry and a count row per inspected payload URI;
- every metadata schema built during the run;
- the OD-7 subtree names;
- every external-document summary of the memo, whose events indexed the C2 arena;
- every distinct attacker-named glTF extension in `notInspected`;
- the unvisited siblings of every live document on the path, with no bound across documents,
  and a copy of the document's member name per pending tile.

All of these grow with tiles, references or payloads. Phase 2 re-read every reference after
traversal from the stored list. The tileset schema's `uniqueItems` on `children` re-hashed
or re-compared every child subtree at every level (`O(bytes × depth)`). The work budgets were
fixed numbers, so an archive's legitimate work could exceed them: the H16 tiler output hit the
2^18 visit cap.

## Terms

- **S**: the admitted stored member bytes (VS-A), fixed before any budgeted work.
- **Path stack**: the C2 tile facts of the current traversal path, root first.
- **Rank**: a member's catalog position (VS-A).
- **Facts record**: the 199-byte per-URI record of C2 facts, the vertex count, the H23
  replay receipt (VS-B-11) and a CRC-32 of both, keyed by rank.
- **In domain**: no exclusion and no precheck stop so far.

## Invariants

**VS-B-1. Result identity.** For every archive and every `ValidationLimits`, validation returns
the result it returned before this slice, with these exceptions only:

1. the default budgets scale with S (VS-B-2), so an archive that exceeded a fixed budget may
   now pass, and `limits` reports the effective values;
2. the new per-item limit `liveDocumentBytes` (VS-B-7);
3. the I1 availability index tables are charged to the live-path limit (VS-B-8);
4. `notInspected` keeps at most 64 glTF extension labels of at most 256 bytes
   (VS-B-9).

Everything else is unchanged: the report (`tiles`, `contentReferences`, `entries`,
`checks`, `notInspected`, `payloads`), every failure category and message and its point in
the traversal, the C2 units, bytes and decoded charges, the C2 outcome and its message.

**VS-B-2. Budgets (A-C1-1, A-I1-1, A-C2-1).** `inspect` (the CLI, Python and `validate_3tz`
path) uses `max(default, κ·S)` for the seven amplification budgets:

| Budget | Default (floor) | κ·S |
| --- | --- | --- |
| `hierarchyVisits` | 2^18 | S/8 |
| `references` | 2^20 | S/8 |
| `totalBytesRead` | 2^31 | 4·S |
| `totalPayloadElements` | 16,000,000 | 16,000,000 + 130·S (additive, 2026-10-31) |
| `containmentWorkUnits` | 2^29 | 128·S |
| `containmentBytesRead` | 2^31 | 4·S |
| `containmentDecodedBytes` | 2^31 | 16·S |

The values are resolved once, after catalog admission and before the hash pass, and the report's
`limits` holds them. Every archive with `S ≤ 1,000,000` bytes keeps exactly today's values; the
crossovers are 1 MB (elements), 2 MiB (visits), 4 MiB (units), 8 MiB (references) and 512 MiB
(bytes). Charges are per use, never per distinct item, and nothing found during traversal
changes a budget, so the budgets cannot amplify (design §3.2). An explicit `ValidationLimits`
(`inspect_selected`, the in-crate controls) is absolute, as before. The per-item limits and the
absolute volume caps (`archiveStoredBytes`, `sourceArchiveBytes`, `archiveEntries`,
`centralDirectoryBytes`) are unchanged; VS-C makes the volume caps opt-in.

**VS-B-3. Path stack (A-C2-2.1).** The C2 tile arena is replaced by the path stack. A tile
registered with parent at stack position `p` truncates the stack to `p + 1` and is pushed at
`p + 1`; the root has position 0. Traversal is depth-first pre-order, so when a tile is
registered its ancestors are exactly the stack below it, and a reference (inline, deferred
after an external tileset, implicit, or replayed from a summary) always names a tile that is on
the stack. Each level chain is built from the stack in the same order as from the arena, with
the same transforms, volumes, external-content volumes and roles. Each entry keeps its chain
bits (C2-WB §3.3), its depth for the message, and the owner tag of VS-B-6.

**VS-B-4. Phase 2 after admission, from a stream log (A-C2-2.2–3, amended after audit).**
The reference list and the content-volume list are removed. A reference ORs its chain, content
and URI exclusion bits as before; while in domain it builds its level chain once and runs the
precheck step. Phase 2 still runs only after whole-archive admission (C1 success, no exclusion,
no precheck stop), as the old phase 2 and the H23 handoff require. Because no tile arena or
reference list remains, the traversal writes a **stream log** while the archive is in domain:
one record per tile registration (its input facts: parent position, depth, caller chain bits,
transform, volume, external-content volume) and one per reference whose precheck step passed
(its tile position, content volume and rank), in traversal order. The log stays in memory up to
32 MiB and then goes to an append-only scratch file; it is dropped at the first registration or
reference that finds the archive out of domain, since no stream will then run. After admission
the log is read once: tile records rebuild the path stack exactly as during traversal (VS-B-3),
and each reference record builds its level chain from it and is streamed with the old phase 2's
state: the phase-1 arena (or one reserved before the first reference, as before), one bitmap,
the corner credit accrued by `T(u)` per reference and the re-read counter against
`containmentBytesRead`. The first failure (Outside, allocation refusal, containment fault,
re-read failure) is the result. So the outcome precedence (R8) and every streamed byte are the
old phase 2's, and an archive that leaves the domain streams nothing. The audit measured the
first, inline design at 0.31 s to 3.9 s on the hostile `workLimit` archives; with the log they
take 0.17–0.19 s again.

*Why a log (the design's "spilled replay only where inline would break the memory bound").*
Streaming inline needs no storage but streams references whose result is discarded when the
archive later leaves the domain; keeping the references in memory would grow with the archive.
The log is the only bounded way to stream after admission: memory at most 32 MiB, disk at most
about 350 B per in-domain tile and 130 B per in-domain reference, which the visit and reference
budgets bound. Its scratch is not in the preflight estimate, because tiles are not known from
the end record; a full disk is a typed `io` failure.

**VS-B-5. Per-URI facts (M7, M8).** The per-URI table, its name map and the count rows are
replaced by a second bit per rank (inspected) and the facts record, written at the first
inspection and read at each later reference (C1 vertex count for the tile budget, C2 facts for
the exclusion bits and the precheck). The record holds the C2 facts exactly (`UriBounds`, `T`,
`Nodes` or overflow, `Idx`, `E`, `B`, `Dec`, `J`, `Res`, `Prim`, the maximum node depth, the
exclusion bits), the vertex count and the replay receipt (VS-B-11). The store is chosen from N
before traversal, like the catalog (CAT-5): rank-indexed pages of 64 records in memory when
`N·199 ≤ 100 MiB` (every archive within the default entry limit 2^19), otherwise a rank-indexed
scratch file read and written one record at a time, included in the scratch preflight estimate.
Every record carries a CRC-32 checked on read; a mismatch is `invalid_state` "validation
scratch: payload facts record R is corrupt", never a verdict. The forced-spill profile always uses the file. Repeat references are never
re-inspected and never re-charged (C1 semantics unchanged). Recomputing evicted facts was
rejected: it costs `Σ_refs size(u)`, which is not linear in S when more distinct payloads than
a cache holds are referenced in rotation.

**VS-B-6. Memo (EM-7, EM-8).** A summary's events carry the tile facts (transform, volume,
relative depth, parent) and the reference facts (rank, content volume) by value, because no
arena keeps the first placement's facts. A recorder maps a stack position to its own tiles
through an owner tag (recorder serial, local index) stored in the stack entry. Summaries and
the events of recorders in progress are charged to a memo budget of 64 MiB (event size plus the
document name). A recorder whose next event would exceed it becomes opaque and frees its events;
its document then gets no summary, so later placements are traversed in full (EM-6 fallback),
which gives the reference traversal's result by EM-1. A summary also records the largest
live-document charge of its expansion above the placement (VS-B-7), its own document and
replayed nested placements included; a placement uses it only if that charge fits the limit
beside the documents already live, so `liveDocumentBytes` gives the same verdict whether or not
the memo is used (EM-6 fallback otherwise). Summaries are never evicted. So the memo
is bounded, and with a full memo the work is still the logical charge of EM-2, bounded by the
budgets of VS-B-2.

**VS-B-7. Live documents (A-C1-4, design M14).** The new per-item limit `liveDocumentBytes`
(default 256 MiB) bounds `Σ (raw bytes + 128 · JSON value nodes)` over the tileset documents
being traversed in full (prepared, not yet exited), plus a `schemaUri` document kept for an
implicit root. The charge is made right after the document is parsed, before its schema check;
over the limit is ResourceLimit `live tileset documents exceed N bytes`; the charge is released
at the document's exit. Pending tiles share their document's name (`Rc<str>`) instead of copying
it. The `tileset.json` read for the vector build check is dropped before traversal.

**VS-B-8. I1 (A-I1-2, A-I1-3).**
- OD-7 is one bit per rank, owned by the validator's resolver. A `Base::Resolved` name is a
  subtree member; the resolver refuses one already resolved in this validation with Unsupported
  `repeated subtree member NAME` before any charge, then marks it. Meaning and detection point
  are unchanged.
- The availability index tables are charged to the live-path limit (`documentDecodedBytes`)
  after the content reservation (§2a step 10, including its host-range check) and before any
  allocation. With `T` the subtree's tile bit count, `P_t` its available tiles and `E` its
  reserved content entries, the charge is
  `4·⌈T/64⌉` when the tile availability is a bitstream (the rank table), plus, when `E > 0`,
  `4·(P_t + 1) + 2·E + 4·Σ_v popcount(v)`, the sum over the distinct bitstream sources `v`
  (distinct `(bufferView, bit count)` pairs; slots that alias one count it once) that
  `contentAvailability` uses (offsets, slots and the per-source positions). If the charge
  exceeds the limit minus the live bytes already on the path (the subtrees above, and this
  subtree's member and external buffers), the result is ResourceLimit `subtree availability
  index exceeds the live subtree byte limit`. Otherwise the charge joins the subtree's live
  bytes and is released with them, so a later read on the path sees a smaller maximum.
- Metadata schemas are built per preparation and live as long as their document's tiles; no
  validation-wide schema map.

**VS-B-9. `notInspected` labels (M13, W14).** glTF extension names come from the archive. At
most 64 distinct names are kept, the byte-smallest, each at most 256 bytes; any other is
dropped and the report gains `notInspectedTruncated: true`. The member is absent otherwise, so
reports without truncation are unchanged. The validator's own labels are always kept. Extension
labels are merged into the list before Contained removes `scene transforms`, as before.

**VS-B-10. Schema `uniqueItems` (W6).** The tileset schema is compiled with a `uniqueItems`
keyword of the same meaning and the same place in the evaluation order. It decides exactly what
the built-in decides: for at most 15 items, whether two items are equal under the schema
library's equality; for more items, whether two items are equal and also hash alike in the
library's value hash, which distinguishes `0` from `-0.0` (the library's own behaviour, kept).
Candidates are found by two keyed 64-bit structural hashes, computed once per array or object
of the document and memoized for one schema check, then confirmed by the exact comparisons. So
the check is linear in the document, and its failure message is the built-in's
`VALUE has non-unique elements`.

**VS-B-11. Stream step 1 by payload replay (H23; amends C2 "Stream algorithm" step 1).** The
first, full C1 inspection of a payload yields a private 64-byte receipt: the SHA-256 of the
member bytes (with the payload kind) and of the ordered transcript of its external buffer and
image resolutions (each entry `u64 URI length, URI, u64 byte length, bytes`, the full returned
bytes). It exists only after that inspection succeeded, and is stored in the payload's facts
record (VS-B-5), so a record is 199 bytes with its CRC. Step 1 of the stream is now: re-read the member and
replay it against that receipt (`payload::replay_scene`). Replay checks the member's byte
allowance, then its digest before interpreting any layout; resolves every external resource in
the original order under the original limits, charging the re-read to `containmentBytesRead` as
before; reconstructs the same views, accessors and scene graph with the same parser and
decoders; and compares the transcript digest before any consumer event. It skips only the
accessor-value pass (extrema, finiteness, restart and index-range predicates) and the KTX2
container inspection, which the receipt's admission already decided for the same bytes. The
units are unchanged: `E`, `B`, `Dec` and the rest are still charged per reference, so they bound
the replay (which does no more work than the full inspection plus linear hashing of bytes `B`
already counts).

- **Admission order.** A receipt is written only after its payload's full first admission
  succeeded, and replay runs only after whole-archive C1 admission with no exclusion and no
  precheck stop (VS-B-4), as the handoff requires. So replay never decides a C1 verdict.
- **Trust.** Receipts are read back only from this validation's own facts store, for a rank
  whose inspected bit it set, during the same validation. They are never decoded from archive
  bytes, reports or metadata, and never outlive the validation (the store is dropped before the
  report rows are ordered).
- **Persistence bound.** One receipt per inspected member, inside its record: resident at most
  `N·199` bytes when that fits 100 MiB (99.5 MiB at the default entry limit of 2^19), otherwise
  in the rank-indexed scratch file (`N·199` bytes of disk, one record read per reference).
- **Unchanged bytes.** For bytes identical to the first admission, replay produces the same
  consumer events, facts and failures as the full inspection did (the old step 1), so every
  verdict and message is unchanged.
- **Changed bytes (failure-order distinction, kept explicit).** If the archive changes during
  validation, changed bytes never reach a consumer event, but the failure is not always the one a
  full re-inspection would give. A changed member fails its digest check, `source archive
  changed during inspection`, before parsing. A changed external resource can first fail an
  existing size, range, layout, decoder or allocation gate in replay order, or a real I/O error
  of the resolver, before the transcript comparison; only otherwise is it the changed-source
  error. A full re-inspection could instead have failed a skipped value check. This is not an
  atomic snapshot of the archive; the final source-identity check after traversal is unchanged.

**Allocation ledger (amends C2 "Representation and memory").** Phase-1 and precheck facts are
the facts store (VS-B-5, receipts included), the path stack and the level chain; the tile arena,
reference list and per-URI table are gone. Phase 2 reserves its arena (0.56 MiB at the default
caps, separate from the phase-1 rotation arena) before the first vertex test of the first
streamed reference, and keeps one bitmap of at most `accessorElements/8` bytes. Replay allocates
the payload's views and decoded buffers as the first admission did (at most
`documentDecodedBytes` per payload, charged as `Dec`) and two SHA-256 states; it allocates no
per-accessor extrema. Every reservation is fallible; a refusal during the stream is recorded and
published as ResourceLimit at precedence 4, as before.

## Memory bound

Traversal memory that depends on the archive (design §1.4, `M_walk`) is now configuration only:

| Term | Bound |
| --- | --- |
| Bits per rank (used, inspected, subtree) | `24·⌈N/64⌉` B, 64 MiB at the member ceiling |
| Path stack and level chain | `(hierarchyDepth + 2)` entries of about 400 B each |
| C2 arenas, bitmap | two arenas of `(16·(nodeDepth + 1) + 32)` values (about 0.56 MiB each), bitmap ≤ `accessorElements/8` B |
| Facts (receipts included) | resident ≤ 100 MiB; spilled: one record |
| Stream log | resident ≤ 32 MiB; then scratch |
| Memo | ≤ 64 MiB plus a summary header per summarized document within it |
| Live documents | ≤ `liveDocumentBytes` (estimate: raw + 128 B per value node) |
| I1 live path | ≤ `documentDecodedBytes`, index tables included |
| Schema hash memo | ≤ 2 hashes per array or object of one document (≤ `documentItems`) |
| Extension labels | ≤ 64 × 256 B |
| One payload in flight | as C1 (`memberBytes`, `documentDecodedBytes`, accessor plans) |

The catalog, the payload rows and their sorts are VS-A's. Disk: the facts file is
`N·199` bytes, sparse where ranks have no payload; the stream log (VS-B-4).

## Work bound

Every traversal step is O(1) or linear in what it reads: a tile registration is O(1) (stack
truncation is amortized against pushes); a reference costs its level chain (charged
`8·(L+1)` units) at its precheck and again when streamed, as before; the stream log is written
and read once, sequentially; a facts access is one page or one positioned read; a summary
replay is O(tiles + references) of its expansion, each charged as visits and references; the
schema check is linear in the document; each live-document charge is linear in its document.
With the budgets of VS-B-2 the hostile classes (shared external tilesets, `Const1` subtrees,
shared buffers, repeated references, memo exhaustion) are linear in S with today's constants
as floors: at the floors, today's bounds hold unchanged.

## Report and API changes (breaking)

- `limits` holds the effective budgets (VS-B-2). `liveDocumentBytes` is enforced but not yet
  serialized: VS-C adds it to `limits` with the rest of the limit surface, so the pinned C1
  report bundles are amended once.
- `ValidationLimits::live_document_bytes` is a new public field.
- `ValidationReport::not_inspected_truncated` is new; JSON `notInspectedTruncated` appears only
  when true.
- Default validation may accept an archive that exceeded a fixed budget, and may refuse one for
  `liveDocumentBytes` or for its availability index tables.

## Acceptance plan

1. **Parity.** Every in-crate parity suite (C2 slice-1, C2-WB, H11, slice-3, S3-18 sweep,
   forest, I1, W corpus), the EM controls, the VS-A controls and every `validate::`,
   `archive3tz::` and `content_integrity::implicit` test in both catalog profiles, plus the C1
   CLI oracles, with 0 disagreements. The I1 oracle models the index-table charge from this
   text (separate author).
2. **Controls** (`src/validate/traversal_controls.rs`): outcome precedence with the stream
   after admission (Outside then a later exclusion, Outside then a later precheck stop,
   exclusion after a passing reference, an Outside behind a later C1 failure); the
   live-document verdict in both memo modes (the audit probe); the stream log resident and
   spilled; the path stack
   against deep, wide, deferred-reference, implicit and replayed placements; the facts store in
   both profiles; memo exhaustion against the reference traversal; `liveDocumentBytes` at the
   charge and one below; the I1 table charge at the cap and one below; the label cap; the
   `uniqueItems` keyword against the built-in on generated arrays (both sizes, `±0`, nested
   near-duplicates); budget resolution at each crossover.
3. **Sensitive mutants** of each invariant, each caught by at least one control, plus the H23
   payload mutants and the existing C2 production mutants re-run on the integrated source.
   The reference traversal of the controls streams after traversal with full re-inspection
   (the old phase 2), so every differential control also compares replay with it.
4. **Hostile archives** (`ext-amplification-20261013`): still bounded, with time and RSS.
5. **Performance**: mf2-lod3 and the perf-lane validate cases against VS-A, and the C2-WB
   accessor-heavy and byte-heavy classes in ns per unit with replay, against τ.
6. **Scale** (after the memory test on the host ends): peak RSS flat across tile counts up to
   the gate (d) archives and the 8.1 GB H16 archive, and time linear in tiles; run through the
   scale driver (volume caps raised, budgets scaled).

## Unresolved obligations

- κ is provisional until VS-D measures producer use against it (rule of design §3.2).
- The bits per rank are `3N/8` bytes, bounded by the format ceiling, not O(1) (OQ-1).
- `storedBytes` in the report, opt-in volume caps and cancellation are VS-C's. Closed by VS-C ([limits amendment](validator-limits-amendment.md)): `storedBytes`, `limitDerivation`, opt-in caps, `inspect_with` cancellation, and `liveDocumentBytes` serialized in `limits`.
