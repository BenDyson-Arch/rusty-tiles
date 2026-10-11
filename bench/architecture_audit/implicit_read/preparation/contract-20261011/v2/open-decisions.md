# I1 open decisions (v2)

v1 is preserved in `v1/open-decisions.md`. The coordinator records each resolution before the
precode review signs off. Line references are to `f2ed077`.

## OD-1: JSON content templates on implicit roots (DECIDED by the user)

**Decision.** The producers change now. I1 adopts the strict reading:

- An implicit root whose content template, in any slot, can name a JSON resource is
  **Unsupported**. "Can name" means the template path ends in `.json`, ASCII case-insensitive,
  checked statically at root admission.
- No validity verdict is given (contract, "OD-1 resolution").
- The validator migration merges after the producer layout change, or together with it
  (ownership-plan start gates).

**Recorded consequence.** Until that change lands, these validate as Unsupported:
- default point and vector outputs above the chunking threshold;
- deep legacy `mesh-to-3tz` output;
- multi-node `convert-to-implicit` output;
- the #151 fixtures.

`convert-to-implicit` then cannot publish multi-node output.

## OD-1b: InvalidInput for a JSON slot available at level 0 (reviewer to confirm)

**The argument.** The text mandates a verdict in this one case:
- `ImplicitTiling/README.adoc` `:75`: "The `content.uri` shall not point to an external
  tileset", stated for the implicit root tile.
- `:130`: "`level` is 0 for the implicit root tile".
- `:198-201`: content availability selects which tiles have a resource at the template URI.

So an available level-0 JSON slot is the implicit root tile pointing to an external tileset. That
is a direct "shall not" violation. For level ≥ 1, no text speaks, and the official validator has a
TODO at this rule (A2 format audit). Those cases stay Unsupported.

**Recommendation.** InvalidInput for level 0.

**Cost.**
- The root subtree must be admitted before the Unsupported result is returned.
- One extra precedence step.
- Malformed root subtrees in such tilesets become InvalidInput.

**Alternative.** Uniform Unsupported with no subtree read. That is the coordinator's literal
wording, and it changes one rule and controls C-4 and C-5.

**Settling evidence.** The reviewer's reading of `:75` together with `:130`.

## OD-2: owner placement (unchanged; strengthened)

**Recommendation.** `src/content_integrity/implicit.rs`, depending only on `content_integrity`.
`Derived` cells stay symbolic.

**What changed.** With JSON templates excluded, the owner can own the whole depth-first walk
(`Walk::next`). The validator needs no new task variant and no root registry. That further reduces
the reasons to place the owner under `validate/`.

**Settling evidence.** As before: the review confirms that no owner invariant needs exact
evaluation.

## OD-3: `hierarchyBounds` for `Derived` cells (unchanged)

**Recommendation.** A labelled rounded evaluation, used only for that tolerance check and never
in C2 facts.

**Note.** Dropping JSON templates does not affect this.

**Request to the design lane.** Keep TILE_BOUNDING_BOX rows on every tile. Every producer does
today (`noRow = 0`), so this only preserves the property. Then `Derived` stays third-party-only.

## OD-4: per-tile recorded budgets that read `extras` (unchanged; now also a design-lane input)

**Recommendation.** Drop the comparison and report `notInspected:
"recordedBudgets.implicitTileExtras"`.

**What changed.** The producer layout change may rewrite the vector rows anyway. The coordinator
should tell the design lane that I1 treats the `extras` STRING as opaque. If the lane wants
validator-checked per-tile budgets, it must carry them in a form with an agreed application
profile. That is a separate decision; it is not I1's.

## OD-5: no new `ValidationLimits` fields (revised: live-path accounting)

**What changed.** Removing JSON links collapses each large output into one implicit root. v1's
cumulative per-root subtree-byte cap therefore becomes binding:

- Measured `pc-perf-2000`: 234 subtrees, 246,584 B, about 1,054 B each.
- 64 MiB is about 63,700 such subtrees, which is the scale of the 65,536-visit cap.

**Revised recommendation.**
- Charge **live-path** subtree bytes against `document_decoded_bytes`: the subtrees on the current
  depth-first path, released when done.
- At most `⌈32/L⌉` subtrees (8 for `L = 4`) are live.
- The quantity is deterministic, and O mirrors it.
- Still no new public field.

**Settling evidence.**
- R2 and R2b at-cap controls.
- R measures the amplified 50,000–65,000-tile case on the new layout to confirm that it is bounded
  by visits, not bytes.

## OD-6: external subtree buffers (unchanged)

**Recommendation.** Admit them: resolved relative to the subtree, data URIs refused, charged to
member and live-path bytes.

**Note.** Not affected by OD-1.

**Settling evidence.** Control D-6.

## OD-7: repeated subtree members (narrowed)

**What changed.** Without JSON links, each implicit root owns its subtree template. A repeat can
now arise only from:
- (a) a template collision within one root, when variables are not separated, for example
  `{level}{x}{y}`;
- (b) two implicit roots, on explicit tiles or in different documents, sharing a template and
  coordinates.

**Recommendation.** Unsupported, unchanged.

**Settling evidence.** The design lane must give each new implicit root a distinct subtree prefix.
Today's `write_chunks` already prefixes by coordinates (`implicit/tileset.rs:432-455`). This is a
start-gate check on W11.

## OD-8: availability tightness (unchanged)

**Recommendation.** Admit `availableLevels` declared larger than the actual depth, with no label.

## New: disposition of `convert-to-implicit` before I1 merges (coordinator or user)

**The problem.** Every multi-node output places a JSON slot in the implicit root's `contents`
(`convert_implicit.rs:404-421`). That is the A2 representation, and it is now Unsupported. The
operation validates its own candidate (`:205`), so it fails. This was already true of realistic
sources, which convert refuses as "irregular" (executed). In practice convert works only for
single-tile and tiny regular inputs.

**Options.**
- (a) Retire the operation for 0.4.0 (G2).
- (b) Restrict it to single-node conversions until A2 is redesigned.
- (c) Re-plan A2 on the new producer layout.

**Recommendation.** (b) as a stopgap if retiring is not yet decided. It must be decided before I1
merges, because these tests currently assert multi-node success:
- `tests/convert_implicit.rs:238`, `:349`, `:536-571`, `:573`, which will need re-verification;
- possibly `bindings/python/tests/test_api.py:985` (a small LAS input, so it may be single-tile;
  R checks).

## Already decided (listed for challenge)

- **Placeholder and level-0 volumes.** Both are kept; the 1e-6 placeholder check moves to
  `hierarchyBounds`.
- **Implicit roots on any explicit tile.** Admitted.
- **The 1.0 `3DTILES_implicit_tiling` extension.** Unsupported.
- **Level caps.** Profile caps of 32.
- **Role by suffix.** Content role comes from the URI suffix, as for explicit content. A `.glb`
  template whose bytes are tileset JSON fails C1 decoding (InvalidInput).
