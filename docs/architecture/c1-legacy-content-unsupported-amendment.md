# C1 amendment H7: legacy 3D Tiles 1.0 content is Unsupported

**Status: in effect (2026-10-13).** The H7 production change implements this amendment.
Legacy 3D Tiles 1.0 content is no longer admitted; modern b3dm v1 framing, as the C1 contract
describes it, is no longer inspected. The coordinator's nonauthor audit is recorded with the
pull request.

It amends [c1-payload-integrity-contract.md](c1-payload-integrity-contract.md). That file stays
byte-identical (sha256 `3cb5ad341a73cdda77a9dd4a1925cf9800af6e37e49b902eb6db0c4f0c3ee1e3`)
because the C1 production-control driver pins it through `referencePins`. The
[WB-A amendment](c1-json-admission-wb-a-amendment.md) is handled the same way. Where the two
texts disagree on payload formats, this note governs.

## Decision

The user decided on glTF-only content: 3D Tiles 1.1 with glTF 2.0 payloads. The validator
reports legacy 3D Tiles 1.0 content (b3dm, i3dm, pnts and cmpt) as **Unsupported**, never as
InvalidInput, and never inspects it partially. All producers already emit only GLB (#185).

## Rule

`Check::payload` handles every payload reference: explicit content, implicit content,
`rustyTilesSourceContents` and `conversion.json` `retainedContentUris`. It applies these steps
in order:

1. **L0, existence.** Every caller has already established that the member exists, before
   `Check::payload`: explicit content, `rustyTilesSourceContents` and `retainedContentUris`
   through the reference check ("missing archive entry"), implicit content through the
   catalog lookup for the tile byte count. No content bytes are read. A missing member is
   InvalidInput, as before.
2. **L1, suffix.** It takes the final suffix of the resolved member name (`rsplit_once('.')`
   on the whole name) and lowercases it as ASCII. If the suffix is `b3dm`, `i3dm`, `pnts` or
   `cmpt`, the payload is **Unsupported**:
   `legacy FMT content is unsupported (glTF 2.0 content only): NAME`. No byte of the member is
   read for payload inspection.
3. **Read.** The existing bounded member read runs unchanged.
4. **L2, magic.** If the first four bytes are `b3dm`, `i3dm`, `pnts` or `cmpt`, the payload is
   **Unsupported**:
   `legacy FMT content is unsupported (glTF 2.0 content only): NAME starts with FMT magic`.
   Only those four bytes are examined. Valid glTF cannot match: a GLB starts with `glTF`, and
   a glTF JSON document cannot start with these bytes. Magic wins over every non-legacy
   suffix, including `.glb`, `.gltf` and unknown suffixes such as `.bin`.
5. **Other suffixes.** Any remaining suffix other than `glb` or `gltf` is Unsupported with the
   unchanged message "payload format outside GLB/local glTF 2.0". It stays after the read, as
   before, so that L2 can name legacy bytes under an unknown suffix.
6. **Inspection.** Ordinary GLB or glTF inspection follows, unchanged.

**T, implicit content templates.** This step belongs to I1 root admission and amends
[I1 §3](i1-implicit-read-contract.md). Step 4b comes after JSON detection. If the last segment
of the normalized **content** template ends in `.b3dm`, `.i3dm`, `.pnts` or `.cmpt` (ASCII,
case-insensitive), the template is Unsupported:
`implicit: unsupported legacy FMT content template in DOCUMENT`. No subtree is read. The
subtree template is exempt.

`FMT` is always the lowercase format name. Error categories map as before: Rust
`ValidationFailure::Unsupported`, CLI `unsupported` with exit status 2, Python
`UnsupportedError`.

## Invariants

- **No partial inspection.** Nothing after offset 3 of a legacy payload is interpreted: no
  version, length, table, `BATCH_LENGTH`, `RTC_CENTER`, embedded GLB or cmpt inner tile.
  Valid, corrupt, truncated and future-version legacy payloads all give the same verdict.
  A cmpt that wraps a GLB is one Unsupported cmpt.
- **No new reads or limits.** L1 and T read nothing. L2 inspects four bytes of a buffer that
  the existing read already holds. Archive admission still reads, CRC-checks and hashes every
  member first, as before. For a legacy-suffix member, the payload re-read that used to come
  before the generic Unsupported no longer happens.
- **glTF unaffected.** Every all-glTF archive keeps its exact report.
- **Order.** The first failure in traversal order still wins:
  - archive admission;
  - then, per reference: L0, the reference check (a missing member is InvalidInput "missing
    archive entry: NAME", so it precedes every later check, including the content volume);
  - then the remaining tile checks, including "content bounds escape tile";
  - then the member lookup for the tile byte count;
  - then L1, the read and L2.

  Implicit T runs at root admission. C2 runs only after C1 succeeds, so a legacy verdict
  beats every `notInspected` reason and every Outside.

## Report changes

- The C2 reason `rtcCenter` (`decodedContentBounds.rtcCenter`) is removed. Its only carrier, a
  b3dm feature table, is never admitted, so the frozen enum has 14 names (C2 contract, H7
  amendment). The `CESIUM_RTC` glTF extension keeps reporting `gltfExtension`.
- C2-WB `J(u)` counts the glTF JSON or GLB JSON chunk only; the b3dm table terms are gone.
- Work accounting: a legacy verdict fails the validation in C1, so no C2 reference, unit or
  `UriFacts` row is recorded. For a legacy-suffix member the payload re-read (and its share of
  `total_bytes_read`) no longer happens, which can only lower that counter. Every other
  counter is unchanged.

## Superseded C1 text

The C1 contract's references to modern b3dm v1 framing are superseded for validation:

- lines 31, 56 and 59 (domain and pinned b3dm references);
- line 137 (`BATCH_LENGTH`) and line 217 (nested b3dm JSON calls);
- lines 280-281 (b3dm JSON consumers);
- line 305 (the GLB/glTF/b3dm discriminator);
- line 398 (b3dm feature and batch semantics in `notInspected`);
- lines 455, 465 and 489 (b3dm acceptance obligations).

The JSON admission rules, limits and outcome categories are otherwise unchanged. The
`notInspected` label "b3dm feature and batch table semantics" is no longer produced.

## Exit-status changes

| ID | Before | After | Inputs |
| --- | --- | --- | --- |
| X-H7-1 | success | Unsupported | any archive that references a valid b3dm, including b3dm with `RTC_CENTER` (formerly success with `notInspected` `decodedContentBounds.rtcCenter`) |
| X-H7-2 | InvalidInput | Unsupported | malformed b3dm; `.glb` or `.gltf` members that hold legacy bytes; legacy implicit templates whose subtree defect was previously reported first |
| X-H7-3 | success | Unsupported | single-node `convert-to-implicit` archives whose `retainedContentUris`, or tilesets whose `rustyTilesSourceContents`, name b3dm |
| X-H7-4 | Unsupported (generic message) | Unsupported (legacy message) | `.i3dm`, `.pnts` and `.cmpt` content |

## Acceptance

- **Production.** `Check::payload` (`src/validate.rs`) is the single classifier for the four
  callers; `payload::legacy_suffix` and `payload::legacy_magic` (`src/content_integrity/payload.rs`)
  implement L1 and L2, and `Template::normalize` step 4b (`src/content_integrity/implicit.rs`)
  implements T. `PayloadKind::B3dm`, `unwrap_b3dm`, `records::FeatureTable`,
  `ContentExclusions::rtc_center` and the b3dm `notInspected` label are deleted.
- **Controls.** [bench/architecture_audit/legacy_unsupported](../../bench/architecture_audit/legacy_unsupported/README.md)
  holds 36 generated controls (44.6 KB) with declared H7 and pre-H7 expectations, a CLI and
  oracle checker, and 11 oracle mutants (all killed). The production CLI passes 36/36.
- **C2 oracle and O-implicit.** Both oracles classify legacy content as Unsupported (C1 and I1
  phases respectively).
- **C1 corpus oracle.** `tests/c1_validation_oracle.py` expects `unsupported` for its 10 b3dm
  controls, has 8 legacy-format controls, and 17 controls for the C1-only paths: legacy magic
  under `.bin`, a `.b3dm` holding a GLB, a missing `.b3dm`, the content-volume check before
  classification, `retainedContentUris` (present and missing), `rustyTilesSourceContents`,
  three implicit content templates, b3dm bytes under a `.glb` template, the exempt subtree
  template (with an implicit baseline), traversal order in both directions (b3dm before and
  after a truncated GLB), a b3dm referenced twice and a b3dm inside an external tileset.
  Legacy cases check message tokens, in Python and in `tests/c1_validate.rs` (cargo test).
- **Frozen payload corpus.** The 12 b3dm controls of the corrected-205 bundle expect
  `unsupported` through the additive
  [legacy-unsupported amendment](../../bench/architecture_audit/c1_payload_integrity/production_controls/legacy_unsupported_amendment/2026-10-13/README.md);
  the three b3dm compiled context controls expect Unsupported for all 18 runs
  ([C2-PIN-AMENDMENT](../../bench/architecture_audit/c1_payload_integrity/production_controls/context_controls/2026-10-10/C2-PIN-AMENDMENT.md)).
- **Production mutants.** The 22 production mutants of the H7 plan each flip a committed
  control and the clean tree flips none; the record is in the pull request.
