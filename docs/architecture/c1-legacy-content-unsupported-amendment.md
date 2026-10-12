# C1 amendment H7: legacy 3D Tiles 1.0 content is Unsupported

**Status: PROPOSED (2026-10-13).** This amendment describes the validator after the H7
production change, which has not landed. Until it does, `develop` still admits modern b3dm v1
as the C1 contract describes. The independent oracles and controls already encode this
amendment (see Acceptance). This note records no acceptance; the nonauthor audit is still
pending.

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

1. **L0, existence.** It looks the member up in the archive catalog. No content bytes are
   read. A missing member is InvalidInput, as before.
2. **L1, suffix.** It takes the final suffix of the resolved member name (`rsplit_once('.')`
   on the whole name) and lowercases it as ASCII. If the suffix is `b3dm`, `i3dm`, `pnts` or
   `cmpt`, the payload is **Unsupported**:
   `legacy FMT content is unsupported (glTF 2.0 content only): NAME`. No byte of the member is
   read for payload inspection.
3. **Other suffixes.** Any suffix other than `glb` or `gltf` is Unsupported with the unchanged
   message "payload format outside GLB/local glTF 2.0". This check now also runs before the
   read.
4. **Read.** The existing bounded member read runs unchanged.
5. **L2, magic.** If the first four bytes are `b3dm`, `i3dm`, `pnts` or `cmpt`, the payload is
   **Unsupported**:
   `legacy FMT content is unsupported (glTF 2.0 content only): NAME starts with FMT magic`.
   Only those four bytes are examined. Valid glTF cannot match: a GLB starts with `glTF`, and
   a glTF JSON document cannot start with these bytes.
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
  - then, per reference: tile checks, including "content bounds escape tile";
  - then the member lookup;
  - then L0, L1, the read and L2.

  Implicit T runs at root admission. C2 runs only after C1 succeeds, so a legacy verdict
  beats every `notInspected` reason and every Outside.

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

## Acceptance (independent side, this pass)

- **Controls.** [bench/architecture_audit/legacy_unsupported](../../bench/architecture_audit/legacy_unsupported/README.md)
  holds 36 generated controls (44.6 KB) with declared H7 and pre-H7 expectations, a CLI and
  oracle checker, and 11 oracle mutants (all killed).
- **C2 oracle and O-implicit.** Both oracles classify legacy content as Unsupported (C1 and I1
  phases respectively).
- **C1 corpus oracle.** `tests/c1_validation_oracle.py` expects `unsupported` for its 10 b3dm
  controls and adds 8 legacy controls.

The production change must also pass the production mutants PM1-PM22 listed in the H7 plan,
which is kept outside the repository at
`~/.cache/rusty-tiles-evidence/h7-20261013/plan.md`.
