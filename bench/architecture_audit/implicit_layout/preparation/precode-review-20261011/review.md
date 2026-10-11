# P-IMPL precode review: the bounded implicit forest

Reviewer: PN (fresh, nonauthor). Date: 2026-10-11.
Package under review: `/tmp/rusty-tiles-pimpl-design-20261011/`. `SHA256SUMS` was verified OK:
- `design.md` `24bf76ec…`
- `open-decisions.md` `6e6e2950…`
- `ownership-plan.md` `22effb0d…`

Sources:
- develop `f2ed077eecf3…` (`/tmp/rusty-tiles-c2`, `git show origin/develop:`);
- Cesium 1.146.0 engine sources, whose sha256 values match design §8;
- spec copies `ImplicitTiling/README.adoc` `14a37a34…`, core `b841fade…` and Semantics `02ac07db…`, which match §8;
- I1 contract v3.1 `contract-draft.md` `9674e60b…`, which matches `SHA256SUMS.v3.1`.

No cargo was run and nothing in the repo was edited. All of my output is under
`/tmp/pimpl-precode-review-20261011/`.

**CPUs.** CPUs 0 and 1 were running another agent's perf-lane Chromium (81–85% busy during my runs), so I did
not use them. Every browser run of mine used `taskset -c 13,15 nice -n 10`, one browser at a time. The SMT
siblings 12 and 14 were 1–7% busy during the timed runs (`runs/perf/*.cpuload`).

## Verdict: **NO-GO for the design as adopted**

The core choice holds up. The bounded forest is the right layout:
- The root-cause claim is verified from source, and I proved it causally (§1).
- The layout is conformant (§2).
- Every Cesium result I re-ran reproduced exactly (§3).

Two of the adopted decisions do not hold, however:
- **PD-2**: S = 4 for both schemes, with no size bound.
- **PD-5**: a depth limit of 64.

Under those decisions the producers would write `tileset.json` documents that the validator in this same
combined PR refuses (I1 v3.1 inherits the C1 JSON limits), for realistic vector sizes and depths that work
today. There is also an unspecified I1/P interface (retained vector sources) that makes every deep vector
forest InvalidInput.

These issues change the writer's classification rule and possibly its interface (`ForestOutput`, the set of
JSON documents). Coding the writer now would put the work at risk of being redone.

**Path to GO.**
1. Amend the design for R-1 to R-3 and record the coordinator's re-decision of PD-2 and PD-5.
2. I review the delta only.

R-4 to R-14 can go into the same amendment or be closed during implementation. The point-cloud, mesh and
I1-reader parts of the plan are otherwise sound.

## 1. The root-cause claim: verified, with a causal ablation

**Source reading** (Cesium 1.146.0, hashes match):
- `makePlaceholderChildSubtree` (`Implicit3DTileContent.js:1021-1068`) builds each child-subtree placeholder
  from `deriveBoundingVolume(...)`. Its geometric error ignores metadata.
- `updateTileVisibility` (`Cesium3DTilesetTraversal.js:206-215`) first culls a placeholder by its own volume.
  Only a visible placeholder recurses, through `children[0]`.
- `canTraverse` (`:56-66`) always traverses visible implicit and external content.
- Loaded tiles use the TILE_BOUNDING_BOX row (`deriveChildTile` → `getTileBoundingVolume`, `:638-669`).
- `getContentBoundingVolume` (`:696-714`) is undefined unless CONTENT_* semantics exist.
- `canUpdateHeights` is irrelevant for box rows.

**Every derived-cell path is closed by the forest.**
- (a) Placeholders appear only when `childSubtreeAvailability` has 1 bits at `isBottomOfSubtree()`. Constant 0
  removes them.
- (b) A derived volume is used only when a row is missing. P3 puts a row on every tile.
- (c) The implicit root's own placeholder is an explicit tile, so it is culled by the explicit `B(r)`. Its
  `children[0]` is the level-0 tile, whose row equals `B(r)` (P3).
- (d) `meetsScreenSpaceErrorEarly` returns false under an implicit parent. Under ADD it uses the child's own
  (row) box.

I found no other consumer of derived cells in the selection or pick path.

**Causal ablation (new).**
- I copied the `CesiumUnminified` build and patched only `makePlaceholderChildSubtree` so that a placeholder
  takes the implicit root volume instead of the derived cell (`cesium-patched/Cesium.js`).
- Unpatched, the unminified build fails fixture `A` exactly as the minified one does: 0/8 leaves, 4 holes, and
  the control passes.
- Patched, fixture `A` passes **8/8 selected and 8/8 picked**.
- So the 0/8 failure is attributable to the placeholder cull, and to nothing else. This is stronger than the
  single control view in the design.
- Files: `runs/fixtures-unmin-A.log` and `runs/fixtures-patched-A.log`.
- The JSON receipts were not written, because of a path bug in my wrapper. The logs hold every per-view
  result.

**Sensitivity of `F`.** In `F`, leaf rows escape their local derived cells. In the level-3 y-split, the
quadtree child cell is the x-half of the y-lower half. So `F` passing is not trivial.

## 2. Spec conformance

These points conform, with citations:
- Implicit roots on any tile: IT `:34` and `:43` ("may be added to any tile … multiple tiles").
- The root constraints (IT `:94-97`): no `children`, no `metadata`, no external-tileset `content.uri`, no content
  `boundingVolume`. Producer content objects carry only `uri` and `extensions` (checked on
  `vec-perf-explicit` and `pc-deep-explicit`), so copying the header is safe.
- `subtreeLevels == availableLevels`: allowed. Neither the schema (`tile.implicitTiling.schema.json`) nor the
  text relates the two.
- A child availability of constant 0 means "the tileset does not subdivide further" (IT `:314`, subtree
  schema). `childSubtreeAvailability` is still required (IT `:399`) and is present.
- Templates include `{level}`, `{x}` and `{y}` (and `{z}` for octrees) (IT `:231`). The template coordinates
  are local to each implicit root, which matches "subtree root's global level" within that implicit tileset.
- `transform` on an implicit root is not forbidden.
- Spatial coherence (core `:749-756`) concerns actual volumes, and P6 carries the producer's enclosure over.

**Overstatement (R-9a).** §3 and PD-1 say the forest is conformant "without any disputed reading". That
contradicts PD-7:
- Vector rows remain up to 4,094× their derived cells.
- IT `:341` and Semantics `:145` call row volumes "more specific … than is implicitly calculated".
- The forest removes the *client consequence* of the dispute, not the dispute itself.

PD-7's wording is the correct one. §3 and PD-1 must match it.

Nothing else in the forest relies on ambiguous text.

## 3. Evidence soundness and my reruns

**Reproduced exactly:**
- The fixtures regenerate byte-identically (`diff -r` against `fx/`).
- The script sha256 values match the receipts.
- Fixture suite E/A/S/X/F on CPUs 13,15 (`runs/fixtures-stock.log`): 8/8, 0/8 (4 holes), 8/8, 8/8, 8/8, and the
  control passes everywhere. These are the same selections and picks as `results.json`.
- The forest models of `vec-perf` and `pc-deep` regenerate byte-identically to the ones in
  `~/.cache/perf-lane/work/pimpl/*-forest-s4` (`diff -rq`).
- The `views.json` hashes match the receipts.

**The vector deep-view A/B, re-run with the shipped scripts** (`runs/real-vector-msse1-det-rerun.json`):
- 10/12 same selection and 12/12 same pick. The only differences are `6-47-0` against `5-23-0` in
  `leaf-8-174-0` and `leaf-8-222-0`. This matches the design.
- The authoritative `real-vector-msse1-det.json` records probe `ffe8ff0c…` and runner `5bafb6fa…`. Neither
  matches the shipped `probe.cjs` (`32a85eda…`) or `run_real.py` (`8c8850a0…`). The result now stands
  reproduced with the shipped scripts, but the file must be replaced (R-9d).

**The perf lane, ABBA, CPUs 13,15** (`runs/perf/{base1,forest1,forest2,base2}.json`), with lane scripts
`890f0159`/`23c78bbc` (= `bb7d054`):

| Case | base1 | forest1 | forest2 | base2 | Requests base → forest | Bytes base → forest |
| --- | --- | --- | --- | --- | --- | --- |
| vector | 2.08 s | 1.99 s | 1.93 s | 2.01 s | 6 → 5 | 1,563,790 → 1,595,269 |
| pc-deep | 15.98 s | 14.48 s | 14.91 s | 15.58 s | 2,649 → 2,242 | 12,892,981 → 12,530,286 |

- No gate problems.
- Every screenshot is byte-identical to the design's (`28db29fc…` and `15f7a2b7…`).
- The design's §5.3 conclusions hold on this corpus.

**Are the CPUs acceptable?**
- On this i7-12700KF, CPUs 0 and 1 are the two SMT threads of one P-core. CPUs 13 and 15 sit on *two
  different* P-cores (6 and 7).
- So r2's absolute times come from a different (stronger) environment than the lane's 0,1 baseline.
- They are valid only as within-session A/B on one pair, which is how r2 used them: base, S4, S6, base, all on
  13,15.
- The deviation is acceptable for model evidence, provided it is stated. A-6 must control for it (R-10).

**Evidence weaknesses that must be corrected (R-9):**
- The vector deep views never reach their target leaves, in **either** layout:
  - `leafSelected` is 0/12 for both (3 to 6 tiles selected, levels 3–8);
  - 7 of the "12/12 same picks" are null = null.
  The vector equality evidence is real but shallow. Deep leaf behaviour rests on fixture `F` alone. Fix the
  view targeting, for example by aiming at the leaf's geometry and not its box centre.
- `pc-deep` is strong: 12/12 identical sets of 22 to 972 selected tiles. But the "today did not settle in 90 s"
  overview ran on CPUs 0,1 (receipt) in the same period as the contended r1 runs. It is not a performance
  comparison and must not read as one.
- At the lane's default framing, the forest vector case loads **0 subtrees** (1 JSON + 4 skeleton GLBs). The
  vector Cesium gate therefore measures only `tileset.json` size and parse. It never exercises an implicit root
  (R-10).
- The 78,333 B `tileset.json` is model-only. `relayout_forest.py` drops skeleton `extras` and `transform`
  (production keeps both: `vec-perf-explicit` has a transform and about 250 B of extras on every tile).
  - Production for vector perf is about 95–115 KB, not 78 KB.
  - 39.7 KB of the model's file is `rustyTilesSourceContents` (544 names).
- Stale names:
  - `evidence/perf-cesium-forest-s4.json` and `-s6.json` (PD-2, §6 A-6) do not exist; the files are
    `perf-cesium-r2-*`;
  - the r1 receipts point at a missing `perf-cesium-base.json`;
  - the receipts show `nice` 6, not 10.

## 4. Bounds and limits: the blocking part

Measured and derived; the scaling model is `agent-bounds/scripts/scale.py`.

**The I1 limits on `tileset.json`.**
- The validator reads `tileset.json` with the content JSON limits: depth 64, 65,536 value nodes, 8 MiB
  (`validate.rs:864-872`, `validate/types.rs:85,98`).
- I1 v3.1 keeps them (contract `:1001-1008`).
- An explicit vector tile costs about 50 value nodes and about 658 B (`vec-perf-explicit`: 27,577 nodes,
  360,064 B, 547 tiles).
- So `--explicit` vector output already fails validation above roughly 1.3k tiles today.

**The forest moves the default vector mode toward that wall.**
- Value nodes ≈ N·(1 + 50·f), where f is the share of tiles written to JSON (skeleton plus implicit roots).
- `vec-perf` measures f = 93/547 = 0.17. That gives the 65,536-node cap at about **6.9k tiles**.
- Today's chunked layout keeps `tileset.json` at about 1 node per tile (the source list) and reaches about 65k
  tiles before any cap.

**LOD chains make it worse** (`store.rs:595-612`; the default `lodLevels` is 3, and up to 16 is allowed).
- A full-detail leaf gets a single-child chain of up to `lodLevels` coarse tiles. Every k-d node above it then
  has h ≥ 4, so it is skeleton.
- With full chains, f ≈ 0.4 and the cap is reached at about 3k tiles.
- With `lodLevels` 16, chain tiles above height S are themselves skeleton, for **any** S the quadtree can
  afford: 17 levels is (4^17−1)/3 bits, about 716 MB per stream.
- At the default `maxTiles` of 100,000, the estimates are:
  - about 13–33 MB of `tileset.json`, against about 7.3 MB today;
  - with `lodLevels` 16, about 221 MB, close to `--explicit`'s 234 MB.
- Cesium parses all of it and builds every explicit tile at load.

**Depth.**
- JSON depth = 2·(L−S) + 5 for L levels. This matches the measured 21 at L = 12.
- At S = 4, depth 64 allows L ≤ 33.
- PD-5 (64 levels for points) and the design's own A-1 40-level chain (JSON depth ≈ 77) therefore produce
  outputs that the combined PR refuses.
- The forest also deletes vector's only effective depth bound: the writer's `level >= 31` check
  (`implicit/tileset.rs:32`). `store.rs:587` alone admits 64 split levels plus the LOD levels.
- serde_json's default recursion limit of 128 (`expand_tileset` re-reads) is reached near L = 65.

**What is fine:**
- Point octrees are bushy: `pc-deep` has 1 skeleton tile and 8 roots, and a 4,042 B `tileset.json`.
- Every root holds at least 2 tiles, because of PD-3. No root is a tiny one-tile subtree. Measured root sizes
  for vector are {3:1, 4:12, 7:4, 15:28}.
- The forest writes fewer files than today in every measured case.
- The largest octree availability stream is 585 bits. I1's per-subtree capacity is 4,096.
- Names are about 50 B, against the 1,024 B cap.
- Recursion to 64 levels in `collect` and `tiles.rs` is fine.
- Boxes do not degenerate: the half-size floor is 1e-6.

**A partial mitigation, measured on the model.** S = 8 for QUADTREE on `vec-perf` gives:
- 11 explicit tiles, 12 roots and a 49.7 KB model `tileset.json`;
- a largest subtree of 23,480 B and 223,032 B of subtrees in total.

This cuts f several-fold for trees without chains, but cannot absorb long LOD chains (above).

## 5. Compatibility with I1 v3.1

**Admitted with no Unsupported:**
- The template syntax, if `n` is written as literal digits.
- The 1,024 B cap (measured on the normalized template, about 50 B).
- `.json` detection (last segment of the normalized template; `content_suffix` allows only glb, gltf and
  b3dm).
- No CONTENT_URI semantics or escape-guard triggers.
- `rustyTile` rows: FLOAT64[12] and FLOAT64, opaque STRING extras. I1 never compares rows with derived cells
  (`:464`, `:477`).
- Constant-0 child availability, constant-1 tile availability.
- One distinct subtree per root (OD-7).
- Repeated payloads (`:437`).
- Non-root implicit roots (X-8, C-8).
- b3dm and local `.gltf`.

**Not admitted:**
- **R-3, retained sources.** Today `validate.rs:1204` reads `rustyTilesSourceContents` only if
  `doc.root.implicitTiling` exists. The I1 migration moves `:1206-1217` "unchanged" (`:680-681`, `:778`) and
  says nothing about the gate. A deep vector forest has an explicit root, so its retained `t/*` sources fall to
  "unreferenced archive entries" (`validate.rs:1675`), which is InvalidInput.
- **R-1 and R-2.** These are the JSON limits in §4. A deep or large forest is a ResourceLimit failure.
- **R-4, per-root content layers.** Today `Subtree::new` gets the global slot count (`tileset.rs:359`, `:367`). I1
  requires the layer count to equal the template count, with no array when there are no templates
  (`:273-274`), plus a post-walk content rule (`:438-440`).
- **A-3's criterion is vacuous.** No I1 label names JSON templates (a JSON template is a failure). Assert the
  exact `checks` and `notInspected` sets and the tile count (`:705`, `:713-720`) (R-13).
- The design pins "I1 v3", and OD-1b is withdrawn in v3.1. Pin v3.1 with its sha256 (R-9g).

## 6. Ownership, acceptance and sequencing

The split is mostly clean and disjoint. Sequencing as one combined PR is correct and necessary:
- today's `validate.rs:1280-1284` refuses nested roots;
- I1 makes today's templates Unsupported.

PO's checks are frozen before PO sees the writer diff, which is good.

Defects:
- The M2 contradiction, the fixture overlap and the expander budget (R-12).
- A-6's measurement controls (R-10).
- A-4's payload equality is not well defined (R-11). `translate_content` re-encodes the GLB JSON chunk, and
  develop bakes the global δ while head bakes a per-root δ.

## Required changes

### Blocking: before any writer code; amendment plus a PN delta review

**R-1. Bound `tileset.json` against I1's JSON limits and the load-regression condition (re-decide PD-2).**
- Quantify f(N, H, `lodLevels`) for each producer.
- Choose a mechanism that keeps **every JSON document** within depth 64, 65,536 value nodes and 8 MiB. It must
  also keep the up-front Cesium load no worse than today, for vector defaults up to `maxTiles` 100,000 at
  `lodLevels` 3 and 16.
- Candidates:
  - **Per-scheme S.** QUADTREE 8 is measured above. It is partial, because chains defeat it.
  - **Chunking the skeleton into explicit external tilesets.** This is a core feature, not the banned form (a
    JSON template on an implicit root). I1 must confirm it admits them, and P1 must be restated.
  - **A joint I1 change to the `tileset.json` caps.** It needs its own resource-safety argument.
  - **A recorded user decision** that accepts a smaller validated domain, with a typed writer error.
- Add to A-3, A-5 and A-6:
  - a default vector case with at least 10k tiles that has LOD chains;
  - a `lodLevels` 16 case.

**R-2. Reverse PD-5.**
- Keep an explicit implicit-mode bound of L ≤ 32 levels (`level ≤ 31`) for **both** point and vector, or
  whatever bound R-1 derives from 2(L−S)+5 ≤ 64. It must be a typed `Error::Data` in the writer.
- Replace A-1's "40-level chain succeeds" with:
  - the bound succeeds and A-3 validates it;
  - bound + 1 fails with the typed error.

**R-3. Specify the retained-source gate jointly with I1-B.**
- `rustyTilesSourceContents` must be honoured whenever it is present, or whenever any implicit root exists, not
  only on an implicit document root.
- State what the forest lists: only the rewritten sources, or all of them.
- P4's "no unreferenced member" and A-2's "unreferenced member" control must exempt listed retained sources.
  Otherwise every vector archive trips them.

### Required in the amendment or during implementation

**R-4. One per-root slot list.** It drives both the root's templates and that subtree's content layers. A root
with no content gets no templates and no layers. Today's global `slot_keys.len()` must not leak.

**R-5. Refine.** Add a §4.4 typed error for a hierarchy whose `refine` differs below an implicit root.
Otherwise P5's "identical refine" is false. The small-model wrapper is ADD, and today's writer hard-codes
REPLACE in its chunk documents (`tileset.rs:453`).

**R-6. `ForestOutput.tiles` covers every tile of H** (skeleton, explicit leaves and implicit), not
"available tiles".
- The vector report computes these over every node (`store.rs:1010-1036`):
  - `primitiveReferences`, `uncompressedTileBytes`, `encodedTileBytes`, `maximumTilePrimitives`,
    `pointAggregation.contentTiles` and `maximumTileBytes`.
- State the semantics of `encodedBytes` for skeleton tiles, which are not rewritten.

**R-7. PD-6 notation.** `n` is written as literal decimal digits at write time. I1 `:393` rejects any other
brace.

**R-8. Strip `extras.implicitChildIndex` from skeleton and explicit-leaf tiles**, as `collect` already does for
rows. Otherwise it leaks into `tileset.json`.

**R-9. Evidence corrections:**
- (a) Align §3 and PD-1 with PD-7: the dispute remains, but its client consequence does not.
- (b) Label every `tileset.json` size as model-only, give the production estimate, and fix the model to keep
  skeleton `extras` and `transform`.
- (c) Disclose that the vector deep views reach no leaf in either layout, and retarget them in A-5.2.
- (d) Replace `real-vector-msse1-det.json` with a run of the shipped scripts. Mine is in `runs/`.
- (e) Fix the stale file names (PD-2, A-6, r1 baseline) and the nice level.
- (f) Mark the `pc-deep` "today did not settle" overview as contended and non-comparative.
- (g) Pin I1 v3.1 with its sha256, and note that OD-1b is withdrawn.

**R-10. A-6 measurement controls:**
- Run base and head on the same CPU pair in ABBA order, at least 2 rounds.
- Sample per-CPU busy time, including the SMT siblings, around each Cesium case. `foreignCpuSeconds` exists only
  in `process.py`.
- Record the topology of the pair.
- Add a vector case that loads implicit roots, for example the `run_real` views timed, or an overview at
  maximumScreenSpaceError 1. The default framing loads none.
- Pin the lane commit that is actually used. The working tree at `/tmp/rusty-tiles-perf` already differs from
  `bb7d054` (`--cpus`, budgets).

**R-11. A-4 payload equality, defined precisely:**
- the GLB BIN chunk is byte-identical;
- the JSON chunk is equal as parsed JSON, except for the scene-root `translation` or `matrix`;
- each δ is compared after composing it with the implicit root's `transform`;
- the f64 summation order is stated.

**R-12. Ownership:**
- (a) M2 (`src/tileset.rs:153-177`) must change its writer call, not "only the encoder string", because
  `write_tileset` is removed.
- (b) A-5.3 edits `tests/fixtures/implicit_levels.cjs`, which belongs to PP. Assign it to one owner.
- (c) After I1-B removes the re-exports on `implicit.rs:20`, PW must delete `expand_tileset_bounded` and
  `ExpansionBudget`, or keep them used inside the reworked `expand_tileset`. That avoids dead code under
  `-D warnings`.
- (d) One `ExpansionBudget` is shared across all roots of a forest, not one per root.

**R-13. Acceptance for I1:**
- Replace A-3's "`notInspected` must not name JSON templates" with the exact expected `checks` and
  `notInspected` sets and the tile count (contract `:705`, `:713-720`).
- Ask I1 for a W11 or C-8 witness of a translated non-root implicit root (`transform`). Only fixture `F`
  exercises one; the real-data model roots carry no transform.

**R-14. PD-4 at scale.** The children-bounds cull now applies to the whole skeleton. A-5.2 must include an
LOD-chain vector case and report the share of views that select a coarser tile.

### Notes (no change required)
- Content headers are taken once per slot (`tileset.rs:271-282`). This is today's behaviour, and I1 treats
  headers as opaque.
- A `.gltf` can sit only at an implicit root (`translate_content` parses GLB). A-1's `.gltf` case must sit
  there.
- The vector reuse-identity bump rejects old caches with a "run a fresh conversion" error (`reuse.rs:285`). The
  CHANGELOG should say so.

## Files produced

All under `/tmp/pimpl-precode-review-20261011/`:
- `evidence-copy/`: the replay copy.
- `fx-regen/`: the regenerated fixtures, identical to the design's.
- `models/`: the forest models, identical at S = 4, plus S = 6, 8 and 12.
- `cesium-unmin/`, `cesium-patched/`: the ablation builds.
- `scripts/`: `run_ablation.py`, `abba.sh`, `cpustat.py`, `cpuload.py`.
- `runs/`: the fixture logs, the ablation logs, the vector deep-view rerun and the ABBA perf JSON with CPU
  load.
- `agent-i1/`, `agent-bounds/`: the delegated cross-check scripts. I verified the findings I relied on
  myself: the `:1204` gate, `json_depth` and `document_items`, the node and depth counts, LOD chains and the
  M2 call site.
