# I1 contract change log

## v3 → v3.1 (N delta check DC-1 to DC-8; coordinator OD-9 and OD-10)

**Input.** `/tmp/i1-precode-review-20261011/delta/delta-check.md` (sha256 `abd37595…`, GO-WITH-CHANGES) and its probes
(`delta_attacks.*`, `content_index.out.json`, `uri_probe/`). N: "no further round needed if folded in as written".

**v3 is preserved in `v3/`** (`contract-draft.md` `13f8f468…`). Abbreviations: C = contract, OD = open decisions,
OP = ownership plan.

| Item | Change | Where |
| --- | --- | --- |
| **DC-1** (template length multiplies per-emission work) | Every content and subtree template is **normalized once at root admission**. Normalization follows `uri()` semantics, with variables as opaque digit runs. It is exact: normalized + digits = `uri(base, substituted raw)`. Per emission, digits are substituted into the normalized form; the raw template is never re-resolved. **Bound chosen: a static cap of 1,024 B on the normalized length** (Unsupported), not a byte charge. Reasons: the verdict is static and decided at admission, like JSON detection; it does not depend on availability or emission order; O mirrors it trivially; no second runtime counter; producer templates are about 30–60 B. Added: the escape guard in **every** segment ("escape adjacent to template variable") and **variable survival** (variables popped by `..` are Unsupported, so the reviewer's `{x}/..` construction cannot map every tile to one member). Template work enters the §2a work formula. New exit row X-15. Controls **R8** (the reviewer's construction: Unsupported within 5 s), **R8a** (padding with surviving variables: Accepted, the raw-per-emission mutant times out), **R8b** (1,024/1,025 B), C-7b, C-7c and C-7d. Mutants: "raw per emission", "no variable survival", "last-segment-only escape guard" | C: §3 "Templates: normalized once"; Supported domain; §2a "Walk", work formula; §7 table; precedence; Interface; X-15; Acceptance C/D/E |
| **DC-2** (detection bypass via `.`/`..`/empty segments) | Detection now runs on the **last segment of the normalized template** (OD-9). C-7 adds the four bypass forms, including a never-available slot. X-1 names the forms. Mutant "detect on the raw last segment" | C: §3 step 4; C-7; X-1; D |
| **DC-3** (content index checked, not reserved) | Step 10 makes `E` a **reservation** (`reference_visits += E` at admission). Emitted implicit contents use `ArchiveResources::reference_reserved`, so there is no double charge, and emitted ≠ `E` is InvalidState. `Σ_live E ≤ references` and "at most 262,144 index entries per validation" are now true; the memory statement is corrected. Exit row X-16. Control **R9** (K = 2,500 identical `Const1` slots: ResourceLimit at the first child subtree's step 10). Mutant "check only", observed through the R5b allocation probe | C: §2a step 10; §7 table and memory; Interface; Validator migration; X-16; R9; D. OP: B's files |
| **DC-4** (per-table class scans) | Classes are classified **once per `(schema document, class)`**, on first use: the Unsupported flag, tile or content use, interpreted descriptors, duplicate check and required list. Table admission is O(1 + its own properties). Added to the work formula. Control **R7c** (37,449 subtrees, a 32,668-property class: Accepted within 5 s). Mutant "per-table class scan" (timeout) | C: §5; §2a work formula; §7 memory; R7c; D |
| **DC-5** (template header `metadata`) | The Unsupported semantic set applies to the class of each content template header's `metadata`, at root admission, through the classification. Control C-12b; mutant "header class not classified"; X-6 | C: Supported domain; §5; precedence; Admitted (content header); C-12b; X-6 |
| **DC-6** (R7b infeasible under defaults) | R7b restated: K = 10,000 layers, each with its own overlapping 19.17 MB view. About 60,000 subtree JSON nodes and 20,000 tileset nodes, within default limits. `Scan` ≈ 192 GB gives ResourceLimit at step 2 within 5 s | C: R7b |
| **DC-7** (work-formula constant) | "At most four passes" replaced by an explicit pass list. Because each content view's length equals the tile view's, this gives **≤ 5 · Σ Scan bytes** (`5|tile| + 4Σ|content| + 2|child|`). Passes may be fused; bit work ≤ 8× bytes | C: §2a work formula |
| **DC-8** (minor) | (a) Property-table `count` has schema `minimum: 1`, so a subtree with an empty slot cannot carry `contentMetadata`. A table with `count` 0, or with `count` ≠ popcount, is **InvalidInput** (control D-10, X-9). (b) **R must confirm the R2b window lower bound by execution:** C1 refuses W2 at `document_decoded_bytes` 190,943 and accepts it at 190,944, with both receipts recorded before O freezes R2b. If the executed threshold differs, the contract number is updated | C: §5; D-10; X-9; R2b. OP: R tasks |
| **OD-9** (coordinator) | Decided: normalize once, and detect on the normalized last segment | OD: Decided; C: Settled decisions |
| **OD-10** (coordinator) | Decided: the scan charge goes to `total_bytes_read`, with its own message; no template charge (DC-1 uses the cap); R updates `docs/VALIDATION.md:121`; no `types.rs` fields | OD: Decided; C: §2a step 2, §7; OP: R files |
| DC-9 (N's OD-10 view) | Adopted as decided: shared counter, distinct message, documentation, and `Scan` sums recorded next to the R7, R7a, R7b and R7c timings | C: §7; OP: R tasks |
| Status | The N delta check gate is marked Met. A and O start on v3.1, and O freezes only on v3.1 text | C: header; OP: gates |


## v2 → v3 (resolving the precode review)

**Inputs.**
- Review `/tmp/i1-precode-review-20261011/review.md` (sha256 `92eee8f0cc79…`, GO-WITH-CHANGES) and
  its probes (`w2_caps.py`, `alias_budget.py`, `alias_budget.out.json`).
- Coordinator decisions on RC-3, RC-17, RC-18 and RC-19, and on CONTENT_URI and CONTENT_GROUP_ID
  (RC-1).

**Versions.**

| Version | Location | sha256 |
| --- | --- | --- |
| v1 | `v1/` | `contract-draft.md` `e7102d25…` |
| v2 | `v2/` | `contract-draft.md` `f3ba6a42…`, `open-decisions.md` `672165dc…`, `ownership-plan.md` `08e0844f…`, `producer-inventory.md` `bee3a079…` |
| v3 | top level | — |

**Abbreviations.** C = `contract-draft.md`, OD = `open-decisions.md`, OP = `ownership-plan.md`,
PI = `producer-inventory.md`. Section names refer to v3.

### Required changes

| RC | Change | Where |
| --- | --- | --- |
| **RC-1** | CONTENT_URI and CONTENT_GROUP_ID are **Unsupported**, in a single *Unsupported semantic set* applied to every class used by `tileMetadata` or `contentMetadata`. The set also covers the other TILE_* and CONTENT_* volume, height, transform and refine semantics, and TILE_BOUNDING_BOX or TILE_GEOMETRIC_ERROR in a content class. CONTENT_GROUP_ID is justified as refusing an override that I1 does not apply: the validator checks no group references. The test is applied at subtree property-table admission. Adds exit row X-6, controls C-12 (CONTENT_URI, including a `.json` value) and C-13, and mutants "ignore CONTENT_URI" and "ignore CONTENT_GROUP_ID" | C: Supported domain, Unsupported; §5; precedence; exit table X-6; Acceptance C and D. OD: Decided table |
| **RC-2** | New normative **§2a**: deduplicated availability sources `(kind, v, bits)`. A step order in which `Scan(s)` (Σ over distinct view sources) is charged to the `total_bytes_read` counter **before any bit pass**. One memoized pass per distinct source. **Closed forms** for `Const0`/`Const1` in every rule (levels, content ⇒ tile, child ⇒ tile, reservation). Child subtrees are admitted lazily in the walk, and `C` is never enumerated. A sparse content index of size `E ≤ references − reference_visits`, so `Const0` and empty layers are never consulted per tile. A normative **work formula**: bit passes ≤ 4·ΣScan ≤ 4·`total_bytes_read`; index O(ΣE ≤ references); per tile O(1 + entries + N). The §7 time statement is corrected. Controls **R7** (the reviewer's 20,000-alias construction: InvalidInput by the post-walk rule within 5 s), **R7a** (Accepted within 5 s), **R7b** (65,536 overlapping views: ResourceLimit before any pass) and A-13/A-14. Mutants "no memoization" (R7 timeout) and "per-object charge" (R7 ResourceLimit). Exit row X-12. Open choice of counter recorded as OD-10 | C: §2a (new), §7, Acceptance C/D/E, X-12. OD: OD-5, OD-10 |
| **RC-3** | Adopted uniform Unsupported for JSON templates. `RootAdmission::JsonTemplates`/`JsonRoot::check_level0`, the root-subtree pre-read and the extra precedence step are removed. The OD-1 argument is replaced with the `:90` reading (`content.uri` *is* the template). C-5 becomes Unsupported. The "level-0 JSON as Unsupported" mutant is dropped | C: Settled decisions; Supported domain; precedence; Interface (`admit_root`); C-5; D. OD: OD-1b withdrawn |
| **RC-4** | JSON detection redefined:<br>- strip `?` and `#`;<br>- take the last segment;<br>- a `%` escape overlapping a `{…}` variable counts as "can name JSON";<br>- otherwise percent-decode as `uri()` does and test `.json` case-insensitively.<br>The false "same rule as explicit content" claim is corrected: `:1365` is case-sensitive on the decoded name. C-7 gains `.js%6Fn` and `.j%7{x}on`. Mutant "case-sensitive or undecoded suffix". Recorded as OD-9 | C: §3 "JSON detection"; C-7; D. OD: OD-9 |
| **RC-5** | The "0x20-only padding" rule is removed. Bytes after the JSON value are admitted exactly when C1 `json::parse` admits them. Control **D-8** (`\n`-terminated JSON with zero padding: Accepted) and mutant "0x20-only" | C: §1 step 2; D-8; D |
| **RC-6** | **R2** now uses an O-authored **content-free synthetic**: QUADTREE `L = 1`, `A = 6`, branching 2, padded subtrees, live path of 6 subtrees, no payloads. **R2b** is kept on both the synthetic and W2, with a cap in `[190,944, 246,583]` taken from the reviewer's `w2_caps.py`. §7 now states that the live-path cap **shares `document_decoded_bytes` with C1's per-payload view budget**. Mutant "live path counts the current subtree only" is paired with R2; "cumulative" with R2b | C: §7 table; Acceptance E (R2, R2b); D |
| **RC-7** | The exact visit formula is in §7 (one per `Tileset`, `ExitTileset` and explicit `Node` task, plus `P_t` per subtree; W2 = 431); R1 uses 431/430. R3 states `A = L` (8 and 32) with child `Const0`. O's verdict domain excludes C1 payload integrity, and controls keep payloads valid except C-10 and R1b | C: §7; Acceptance A, E |
| **RC-8** | The exit-status table is **re-derived rule by rule** against `implicit/tileset.rs:662-1196` and `validate.rs:1203-1284`, with the unchanged rules listed. New rows: JSON-format subtrees, unknown scheme strings (X-3); `subtreeLevels > 32` (X-4); subtree `extensions` and content semantics (X-6); tolerance (X-10); R's own `V_R`/`e_R` against its parent (X-11); scan charge (X-12); already-written multi-node convert archives (X-1) | C: Exit-status table X-1 to X-14 |
| **RC-9** | "The tolerance is the same" is replaced with an explicit table:<br>- `V_0 ⊆ V_R`: absolute 1e-6 (`implicit/tileset.rs:761-765`, `bbox.rs:382-389`) → scaled 1e-6·(1+‖h‖) (`validate.rs:297`);<br>- `e_0`: strict (`:766`) → 1e-8·(1+e_R) (`validate.rs:1312`);<br>- R's parent: now also checked with `V_R`/`e_R`.<br>Exit rows X-10 and X-11; controls T-1 to T-3; mutant "absolute 1e-6 retained" | C: §4 tolerance table; X-10, X-11; T-1 to T-3 |
| **RC-10** | B removes only the validator's use (`validate.rs:56-62`, `:898`, `:1203-1263`, `:1600-1604`, test `:1758-1761`) and the `implicit.rs:20` re-export. **`implicit/tileset.rs` is untouched by I1**: the bounded expander stays as `expand_tileset`'s private implementation. The P-overlap sequencing for that file is removed | C: Validator migration step 3; Disposition. OP: file ownership, overlap check |
| **RC-11** | Each live subtree keeps typed facts only, and the parsed `Value` is dropped before descending. "Memory held" restated: live bytes ≤ `document_decoded_bytes`, plus rank tables ≤ ½ of that, plus ≤ 262,144 index entries, plus O(views + slots + columns) facts per live subtree | C: §1 "Typed facts only"; §7 "Memory held" |
| **RC-12** | (a) In the I1 PR, B registers **no `TileFact` below R** and no implicit C2 reference, and the rounded OD-3 box never enters `TileFact`. (b) Slice-3 obligation: a frozen-enum delta adding a "`Derived` present" reason (for example `derivedImplicitVolume`), or an exact `Cell` predicate; "stop excluding and register references" is no longer called sufficient. (c) Slice-3 obligation: amend C2 B3's 129-level bound (+1 per implicit root on the path) and the `SmallSpanList` capacity statement | C: Interface, "In the I1 PR" and "Obligations recorded for C2 slice 3" |
| **RC-13** | OD-4 is made exact: `recordedBudgets.implicitTileExtras` is added in **every archive with at least one implicit root**. Report-level `budgets.bytes`, `budgets.vertices` and `maxPoints` still apply per implicit tile (`validate.rs:1392-1407`) | C: "Exactly what the report says". OD: OD-4 |
| **RC-14** | "Reservation after allocation" is paired with **R5b** (a bitstream tile source over the remaining visits, with zero rank-table allocations probed). "Per-visit charging" is paired with **R1b** (a corrupt first payload in the last subtree: ResourceLimit versus InvalidInput). A-1 and A-2 are made single-defect with the co-adjustments specified (A-2 is now a facts control) | C: Acceptance C (A-1, A-2), D, E (R1b, R5b) |
| **RC-15** | The post-walk rule `:299` (content templates but no tile with content) is now InvalidInput, through `Walk::finish` and control C-14. Schema `minItems: 1` is enforced on the five subtree arrays (§1 step 3, control D-9). Exit row X-9 | C: §1, §3 "Post-walk", Interface, X-9, C-14, D-9 |
| **RC-16** | The parity seam and its tests live in `#[cfg(test)] mod tests` in `src/validate.rs`, as C2 does (`:1513-1531`, used at `:2746` and `:2847`). `tests/i1_implicit_read.rs` covers CLI and public API only | C: Acceptance A "Parity". OP: B's files |
| **RC-17** | **One combined integration PR containing P and I1.** Separate development branches are allowed. Why it is mandatory: a P layout with roots on explicit tiles is unreadable by today's validator (`:1280-1284`) and by `expand_tileset` (`store.rs:1006`), which P owns; and I1 alone would turn default outputs Unsupported. The gates and merge order are rewritten | OP: Start gates and merge order; roles (P). C: Status, Validator migration |
| **RC-18** | Perf lane pinned to `perf/lane` commit `bb7d0549b0f7…`, with git blob ids and sha256 for `process.py`, `corpus.py` and `README.md`. The modified worktree copy is not used. The gating amplified case requires a **base best wall of at least 0.4 s** (or the closest case recorded, with the gate on RSS). Before = the combined branch minus I1's validator commits | C: Performance obligation |
| **RC-19** | Convert retirement folded in. Gate: #159 (T1, carrying `retire/convert-to-implicit` `b59c7b3`/`4b35dd7`) merges before the combined PR, and B rebases on it. Removed: W8/W8u as producer witnesses, the OD "New: disposition" item, the `tests/convert_implicit.rs` references and the "refuses to publish" wording. Added: X-1 coverage of already-written multi-node archives (W8m: Unsupported) and single-node ones (W8s: Accepted). R rewrites #159's `docs/VALIDATION.md:143` sentence | C: X-1, witnesses W8s/W8m, Disposition (`VALIDATION.md`). OP: gates, file ownership. OD: Decided. PI: C1 rows |
| **RC-20** | Citations fixed: `:1366` → `:1365`; `containment_work_facts` → `:1513-1531`; `uri()` body → `:165-210`. **All primary-text lines are now raw line numbers of the pinned files.** ImplicitTiling was converted from stripped numbering, for example `:75` → `:96` and `:130` → `:182`. The convention is stated in the header. `Implicit3DTileContent.js:453` is now cited with its pin (`86109c4b…`, A2 `primary-sources.json`, byte copy) | C: header and all citations |

### Recommended changes (adopted)

| RC | Change | Where |
| --- | --- | --- |
| RC-21 | The `VisitedTile` parent is implied by path level. B keeps a level-indexed path stack, and there is no map keyed by walk index | C: Interface |
| RC-22 | `EffectiveVolume::DerivedRoot`, with a `derived-root` tag in O's records, so B-4 shows a facts change on level-0-only tilesets | C: §4, Interface, Acceptance A |
| RC-23 | Controls B-11 (a required interpreted property omitted: InvalidInput) and B-12 (an optional one omitted: falls back to `derived`). B-12 is also paired with the "error not halved" mutant | C: §5, Acceptance C, D |
| RC-24 | Template substitution happens only for emitted contents, so Σ template length is bounded by `references` | C: §2a "Walk" |

### Not changed by the review (confirmed by N)

- Owner placement.
- Symbolic `Derived` cells.
- Override by semantic.
- LSB-first availability, Morton order and rank.
- Live-path accounting (now with its shared-budget note).
- No new `ValidationLimits` fields.
- The report label and the counts.

## v1 → v2 (OD-1 user decision; summary)

- Strict reading. JSON templates moved from the terminal-occupancy admission (v1) to Unsupported, at
  the time with a level-0 InvalidInput exception, which v3 removes.
- The validator migration was gated on the producer layout change.
- The owner now drives the walk: no task variant, no root registry.
- v1's link-tile count change was removed.
- The per-root cumulative subtree cap became live-path accounting (measured at about 1,054 B per
  subtree on W2).
- OD-2 to OD-8 were re-checked.

## v3.2: coordinator rulings after A's implementation (2026-10-11)

1. **Subtree magic.** A member whose first non-whitespace byte is `{` (UTF-8 BOM allowed) is a JSON-format subtree and is Unsupported. Any other member not starting with `subt` is InvalidInput (D-4). A bad version is InvalidInput. This resolves the Supported-domain/D-4 conflict.
2. **Interface amendments**, accepted:
   - `admit_root(tile, base: &str, schema: Option<&ImplicitSchema>)`. `ImplicitSchema` is per schema document, which gives DC-4's once-per-(document, class) classification across roots.
   - `ImplicitLimits::new(live_cap, json)` carries the validation-wide OD-7 repeated-subtree set.
   - `Base = Resolved | Relative(member)` with a `Resolver` callback. The resolver charges exactly one reference and `bytes.len()` bytes, which the owner mirrors.
   - `VisitedTile.contents: Vec<(u16, String)>`.
   - Additions: `declares_implicit`, `ImplicitRoot::{header, slots, scheme, walk}`, `Walk::{next, finish}`, `ImplicitError::widen`, `Cell::rounded_for_hierarchy_bounds` (OD-3, labelled, `hierarchyBounds` only).
3. **Interpretations**, accepted:
   - `extensions` "anywhere" excludes values inside `extras`.
   - Table-level `offset`/`scale` on an interpreted column is Unsupported, as for class-level ones.
   - The subnormal derived-error rule applies at level ≥ 1; level 0 is `e_R`.
   - The escape guard is per segment, so a malformed escape such as `c%/{level}` is InvalidInput.
   - Implicit integer fields are read as JSON integers, so `8.0` and `1e1` are InvalidInput. This matches the previous expander.
   - A duplicated semantic of any kind in one class is InvalidInput.
   - Row values (finite, error ≥ 0) are checked eagerly at table admission.
   - OD-7 is validation-wide: an explicit tileset that references the same implicit-root external tileset twice is Unsupported.
