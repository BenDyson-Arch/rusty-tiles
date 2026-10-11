# I1 open decisions

This file lists only the choices that are actually open. Each one has a recommendation and the
evidence that would settle it. The coordinator records every resolution before the precode review
signs off. Line references are to `f2ed077`.

## OD-1: reading JSON content templates on implicit roots (coordinator decision; this is the A2 gate)

**The problem.**
- Pinned text `ImplicitTiling/README.adoc` `:75` says an implicit root's "`content.uri` shall not
  point to an external tileset".
- Point and vector defaults at realistic size put an external-JSON template in the root `contents`
  (W3, W5). So do multi-node `convert-to-implicit` outputs (W8) and the #151 fixtures.
- In every one of these, the JSON slot is available only at childless tiles at level ≥ 1.
- Today's validator accepts them. It also runs on `convert-to-implicit`'s publication path
  (`convert_implicit.rs:205`).

**Options.**

| Option | What it means | Consequence |
| --- | --- | --- |
| (a) Literal | Any JSON template on an implicit root is InvalidInput | Default producers and convert go red. The producers would need to switch to child subtrees, but the writer chunks deliberately for Cesium culling of unloaded subtrees (`implicit/tileset.rs:284-286`). That would become a producer slice ahead of I1 |
| (b) Terminal-occupancy reading **(recommended)** | Admit a JSON slot only where it is available at level ≥ 1, on a tile with no available implicit children and no other available content. JSON available at level 0 is InvalidInput | The report adds `notInspected: "implicitExternalContentTemplates"` whenever such a template exists. This states that the reading was applied without a standards verdict, so I1 makes no conformance claim either way |
| (c) Unsupported | Any JSON template is Unsupported | Default outputs return exit 2 from `validate`, and `convert-to-implicit` cannot publish |

**Settling evidence.**
- An authoritative clarification of `:75`: an issue on CesiumGS/3d-tiles, or the validator TODO at
  `TileValidator.ts` `7fa62c5f` resolved.
- Or a user or coordinator product decision recorded on #125 and #126.

**Recommendation.** Option (b), because it makes the uncertainty visible in the report. It does not
settle the A2 gate and must not be cited as settling it. If the gate is later resolved negatively,
the reader switches to (a) by changing one rule and its control C-4b. The producers then need their
own slice.

## OD-2: owner placement and dependency direction

**Options.**
- **Recommended:** `src/content_integrity/implicit.rs`, as the recommendation proposes. The format
  owner depends only on `content_integrity::{json, FormatError, JsonLimits}`. It returns symbolic
  `Cell`s and does no exact arithmetic.
- **Alternative:** `src/validate/implicit.rs`, if the authors want to use the C2 dyadic arithmetic.
  That requires making `containment::dyadic` `pub(crate)`, which touches `containment*`. It also
  couples the format reading to the validator.

**Settling evidence.** The precode review confirms that no I1 invariant needs exact evaluation
inside the owner. The draft has no such invariant: `Derived` is symbolic, and only C2 or the
legacy check evaluate it.

## OD-3: `hierarchyBounds` for `Derived` cells (third-party only)

**Background.** `contains()` (`validate.rs:318-358`) needs an `f64` box. Today the expander feeds it
`derived_box` in `f64` (`implicit/tileset.rs:403-418`).

**Options.**
- (a) A labelled rounded evaluation, with the same formula used for this legacy tolerance check
  only.
- (b) Skip `hierarchyBounds` whenever a `Derived` cell is involved, and add
  `notInspected: "hierarchyBounds.implicitDerived"`.
- (c) Exact containment through a new exact routine.

**Recommendation.** Option (a).
- The check is already tolerance-based (1e-6) and is slated for retirement by C2 D4.
- No producer reaches it, so behaviour stays equal to today.
- The rounded value must never enter C2 facts. A grep plus a unit test enforces this.

**Settling evidence.** An O control with a row child inside its exact derived parent cell, but
outside by 1 ulp of the rounded centre. It shows whether (a) can produce a false Outside within
the tolerance. Expectation: no, because the slack of `1e-6·(1+|h|)` dominates.

## OD-4: per-tile recorded budgets that read the opaque `extras` STRING (`validate.rs:1408-1415`)

**Background.** Today the validator compares the vector rows' `extras.encodedBytes` and
`extras.vertices` with the actual payloads. Vector rows carry these keys on 628 of 631 rows.
Treating `extras` as opaque removes that per-tile comparison for implicit tiles. Archive-level
`budgets` and `maxPoints` remain.

**Options.**
- (a) Drop it and report `notInspected: "recordedBudgets.implicitTileExtras"`. **(recommended)**
- (b) Keep a declared *application profile*: when a tile class has a STRING property *named*
  `extras`, parse it as JSON and compare. This reintroduces name dependence, but only for
  application data, never for semantics.

**Settling evidence.** Whether any acceptance record relies on the per-tile implicit comparison.
Search the C1/P1/V1 acceptance receipts for that failure, and check whether any control expects it.
None was found in `tests/` by grep. A2's metadata settlement also treats extras as application
data under its own mapping (`a2_metadata_settlement/audit.md:86-120`).

## OD-5: no new `ValidationLimits` fields

**Recommendation.** Express all I1 bounds through existing fields (contract §7). This avoids a
second public struct-literal break in the cycle after C2's five fields.

**Risk.** It overloads `document_decoded_bytes` with a per-root cumulative subtree-bytes meaning.
That must be documented in `docs/VALIDATION.md`.

**Settling evidence.** R measures the amplified case (contract, Performance, item 3) and confirms
that no real producer output approaches 64 MiB of subtrees per root. Probed maxima:

| Output | Bytes per root, max | Total |
| --- | --- | --- |
| `pc-deep` | 65 KB | 447 KB |
| `vec-perf` | 6.2 KB | 287 KB |

If a real output does approach the cap, add one named field instead and record the API break.

## OD-6: external subtree buffers

**Options.**
- **Recommended:** admit them, as recommendation invariant 1 asks. They are resolved relative to
  the subtree member, data URIs are refused, and the bytes are charged to the per-root cap.
- **Alternative:** Unsupported. That is a smaller surface, and no producer writes them.

**Settling evidence.** O control D-6 passing in both implementations. An unused external buffer
must also be handled by the unreferenced-entry rules. Decide whether declaring a buffer counts as
referencing its member: recommended yes, read it and charge it.

## OD-7: repeated subtree members

**The question.** The spec does not forbid two subtree addresses, or two implicit roots, resolving
to the same member. The expander caches by coordinates only.

**Options.**
- **Recommended:** Unsupported (profile). It keeps the reservation and per-root bytes simple and
  deterministic, and no producer repeats a member.
- **Alternative:** admit repeats, charging each read again. Cycles are impossible, because subtrees
  never reference documents.

**Settling evidence.** A survey of third-party implicit tilesets that share subtree files. None
are known to the author.

## OD-8: availability tightness

**Background.** `availableLevels` larger than the deepest available level is admitted in this
draft. Producers are always tight.

**Options.** Requiring equality would be stricter than the text supports: `:65` "How many levels
there are in the tree with available tiles" is descriptive.

**Recommendation.** Admit looser declarations, with no report label.

**Settling evidence.** A pinned official validator or Cesium behaviour on a loose declaration. This
is low priority.

## Already decided in the draft (listed so the review can challenge them)

- **Placeholder and level-0 volumes.** Both are kept. The 1e-6 placeholder check becomes a
  `hierarchyBounds` check.
- **Core external-tileset semantics.** Link tiles and external roots are separate tiles; they are
  not merged. This changes `tiles` and `contentReferences` for chunked outputs (contract, report).
- **Implicit roots on any explicit tile.** Admitted.
- **The 3D Tiles 1.0 `3DTILES_implicit_tiling` extension.** Unsupported.
- **Level and subtree limits.** `availableLevels` and `subtreeLevels` are capped at 32 as profile
  constants, so exceeding them is Unsupported.
