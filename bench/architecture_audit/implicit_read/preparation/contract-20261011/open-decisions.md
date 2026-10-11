# I1 open decisions (v3.1)

v1 and v2 are preserved in `v1/` and `v2/`. Review RC references point to
`/tmp/i1-precode-review-20261011/review.md`. Line references are to `f2ed077`; primary-text line
numbers are raw.

## Decided

| ID | Decision | By |
| --- | --- | --- |
| OD-1 | JSON content templates are excluded by profile, and the producers stop writing them | User |
| OD-1b | **Withdrawn.** All JSON templates are **uniformly Unsupported**: no level-0 InvalidInput, no root-subtree pre-read. `:90` makes `content.uri` the template, so the text gives no level-0-only rule (RC-3) | Coordinator, accepting RC-3 |
| Sequencing | I1's validator migration and P land in **one combined integration PR**. They may be developed on separate branches (RC-17) | Coordinator |
| Convert | `convert-to-implicit` is **retired**: `retire/convert-to-implicit` `b59c7b3`/`4b35dd7`, being folded into PR #159 (T1). It merges before the combined PR (RC-19). The v2 "disposition" item is closed | Coordinator |
| Content semantics | CONTENT_URI and CONTENT_GROUP_ID are **Unsupported**, alongside the bounding and height semantics (RC-1) | Coordinator |
| Perf pin | Lane commit `bb7d054`; the amplified case needs a base of at least 0.4 s (RC-18) | Coordinator |
| **OD-9** | **Normalize each template once at root admission** (exact, because variables become decimal digits), and detect `.json` (ASCII case-insensitive) on the last segment of the normalized template. This covers DC-2's bypasses, and the verdict stays static and exact. Added with it: the escape guard in every segment, the variable-survival rule and the 1 KiB normalized-length cap (DC-1). Contract §3 | Coordinator (v3.1) |
| **OD-10** | The availability scan charge goes to `total_bytes_read`, with its own message (`availability scan exceeds total_bytes_read work limit`). DC-1 uses a length cap, so no template bytes are charged. R updates `docs/VALIDATION.md:121`; `types.rs` gets no new fields | Coordinator (v3.1) |

## Resolved by the coordinator (2026-10-11): all recommendations OD-2 to OD-8 adopted as written

OD-2 owner at `src/content_integrity/implicit.rs` owning the walk; OD-3 labelled rounded evaluation confined to the legacy `hierarchyBounds` tolerance check; OD-4 per-tile extras comparison dropped for implicit tiles with the `recordedBudgets.implicitTileExtras` label; OD-5 no new `ValidationLimits` fields; OD-6 external subtree buffers admitted (data URIs refused); OD-7 repeated subtree members Unsupported; OD-8 looser `availableLevels` admitted. OD-9 and OD-10 were decided earlier (normalized last-segment `.json` detection; scan charge on `total_bytes_read`).

## Original recommendations (kept for the record)

### OD-2: owner placement (unchanged)

**Recommendation.** `src/content_integrity/implicit.rs`, owning the depth-first walk. `Derived`
cells stay symbolic.

**Settling evidence.** The delta check confirms that no owner invariant needs exact evaluation.

### OD-3: `hierarchyBounds` for `Derived` cells (unchanged)

**Recommendation.** A labelled rounded evaluation, used only for this tolerance check and never
in C2 facts (RC-12a).

**Ask of P.** Keep TILE_BOUNDING_BOX rows on every tile.

**Settling evidence.** An O control where the tolerance slack dominates.

### OD-4: per-tile extras comparison (made exact; RC-13)

**Recommendation.** Drop it for implicit tiles. `notInspected` gains
`"recordedBudgets.implicitTileExtras"` in **every archive with at least one implicit root**.

Report-level `budgets.bytes`, `budgets.vertices` and `maxPoints` still apply per implicit tile
(`validate.rs:1392-1407`). Explicit tiles keep the extras comparison (`:1408-1415`).

**Alternative.** A declared application profile that reads a STRING property named `extras`.
This reintroduces name dependence.

**Settling evidence.** No acceptance record or control relies on the implicit comparison (grep of
`tests/`: none).

### OD-5: no new `ValidationLimits` fields (revised again)

**Recommendation.** The following share existing fields:

| Quantity | Existing field |
| --- | --- |
| Live-path subtree bytes | `document_decoded_bytes`, also used by C1 as the per-payload view budget, applied independently (RC-6) |
| Availability scan bytes | `total_bytes_read`, shared with member reads (RC-2) |
| Content entries | `references` |
| Tile visits | `hierarchy_visits`, by reservation |

**Risk.** Overloading these fields must be documented in `docs/VALIDATION.md`.

**Settling evidence.**
- R measures the amplified case in P's layout and reports which cap binds first.
- R7 and R7b show that the scan charge refuses only adversarial aliasing.
- If a real P output approaches a cap, add one named field instead and record the API break.

### OD-6: external subtree buffers (unchanged)

**Recommendation.** Admit them. Data URIs are refused. They are charged to member, live-path and
scan bytes.

**Settling evidence.** Control D-6.

### OD-7: repeated subtree members (unchanged)

**Recommendation.** Unsupported.

**Ask of P.** Use distinct subtree prefixes per root.

**Settling evidence.** W11.

### OD-8: availability tightness (unchanged)

**Recommendation.** Admit a looser `availableLevels`.

### OD-9, OD-10

Decided in v3.1 (see the Decided table). The v3 text is preserved in `v3/open-decisions.md`.

## Withdrawn since v2

- OD-1b (level-0 InvalidInput).
- "New: convert-to-implicit disposition".
- v2's per-root cumulative subtree cap (already replaced in v2).
