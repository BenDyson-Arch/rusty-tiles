# P-IMPL open decisions

Line references are to `origin/develop` `f2ed077`. Evidence paths are relative to this directory.
Each item has a recommendation and the evidence that would settle it. The coordinator records
the resolution before the precode review signs off.

## Decided before this design

| ID | Decision | By |
| --- | --- | --- |
| U-1 | The producers stop writing external-JSON content templates on implicit roots, now | User, 2026-10-11 |
| I1-OD-1/1b | The I1 reader treats every JSON content template as Unsupported. P must therefore leave no `.json` template in any point-cloud, vector or legacy `mesh-to-3tz` output | Coordinator (I1 v3) |
| I1-RC-17 | P and the I1 validator migration land in one combined integration PR | Coordinator (I1 v3) |
| I1-RC-19 | `convert-to-implicit` is retired by #159, so it is not a P producer | Coordinator (I1 v3) |
| I1 asks | Rows on every tile (I1 OD-3). Distinct subtree prefixes per root (I1 OD-7) | I1 v3 |

## PD-1: the layout (recommendation: the bounded forest)

**Options** (design.md §3 has the full comparison):

- (a) One implicit tree with child subtrees.
- (b) Pad the grid until cells contain the content.
- (c)+(d) **The bounded forest.** An explicit skeleton, with single-subtree implicit roots.
- (e1) One subtree for the whole tree.
- (e3) Explicit output by default.

**Recommendation: the bounded forest.**
- It is the only candidate that passes all of the following:
  - every Cesium probe (fixtures `F` and `S`; real data with identical selection);
  - the spec text with no reading in dispute;
  - every producer's hierarchy shape.
- Candidate (a) fails 8 of 8 executed leaf views (`A`).

**Settled by:** the executed evidence in `evidence/` (already done), and the precode review.

## PD-2: the value of S, the maximum levels per implicit root (recommendation: 4 for both schemes)

**Background.** S trades skeleton tiles, which go into `tileset.json`, against the number of
subtrees and their size. Measured on the real outputs (`evidence/samples-layout.json`,
`evidence/relayout-summary.jsonl`):

| Output | S | Explicit tiles | Implicit roots | `tileset.json` | Subtree bytes |
| --- | --- | --- | --- | --- | --- |
| vector perf (547 tiles, 12 levels) | 4 | 48 | 45 | 78,333 B | 210,280 B |
| vector perf | 6 | 19 | 20 | 56,358 B | 205,320 B |
| pc-deep (2,233 tiles, 5 levels) | 4 | 1 | 8 | 4,042 B | 252,504 B |
| pc-perf-2000 (428 tiles, 5 levels) | 4 | 1 | 8 | — | ≈54 KB (est.) |
| perf point cloud, mesh (9 tiles, 2 levels) | any | 0 | 1 | as today | as today |

For comparison, today's layout writes:
- vector perf: 85 documents (141,973 B) and 85 subtrees (287,424 B);
- pc-deep: 208 documents and 208 subtrees.

**Options.**
- 4 for both schemes. This is the smallest change from today's `LEVELS = 4`. The largest octree
  availability stream is 585 bits.
- A per-scheme constant: OCTREE 4, QUADTREE 6. Vector `tileset.json` shrinks by 22 KB, but this
  adds a second constant with no measured load benefit.

**Recommendation.** S = 4 as one named constant. Revisit it only if the combined-PR perf lane
flags `requestBytes` on a vector case.

**Settling evidence.** The perf-lane Cesium A/B at S = 4 against develop
(`evidence/perf-cesium-forest-s4.json`). If a gate flags, rerun at S = 6
(`evidence/perf-cesium-forest-s6.json` records the S = 6 model).

## PD-3: leaf children of skeleton tiles (recommendation: keep them explicit)

A childless child of a skeleton tile could become a one-tile implicit root. That costs one subtree
request and saves about 300 B of `tileset.json`.

**Recommendation.** Keep them explicit. This affects only skeleton leaves: 2 tiles in vector perf,
0 in pc-deep.

## PD-4: Cesium `cullWithChildrenBounds` on skeleton tiles (recommendation: accept, document)

**Observation.** Cesium computes `checkChildrenWithinParent` only for tiles it reads from tileset
JSON (`Cesium3DTileset.js:2440-2441`). Skeleton tiles are therefore subject to the
children-bounds cull, and implicit tiles are not.

On vector perf at maximumScreenSpaceError 1 in deterministic mode:
- 2 of 12 deep views select the coarser ancestor `5-23-0` instead of `6-47-0`. Pixel coverage is
  identical.
- With `cullWithChildrenBounds: false`, all 12 views select identically.
- Evidence: `evidence/cesium/real-vector-msse1-det*.json`.

Every `--explicit` output already has this behaviour. It is consistent with REPLACE semantics,
because children replace the parent.

**Recommendation.** Accept it. Document it as a viewer-selection difference, not a bounds
difference.

**Alternatives.**
- Raise S so the skeleton is smaller. At S = 6 there are 19 explicit tiles.
- Accept no explicit tiles. That is impossible for deep trees without child subtrees.

## PD-5: point-cloud implicit depth limit (recommendation: one limit, 64)

**Background.** `point_cloud/tiles.rs:151` caps implicit partitions at 31 levels and explicit ones at
64, because the writer refuses global implicit levels from 31 up (`implicit/tileset.rs:32`). The forest
uses coordinates local to each root, of at most S levels, so the 31-level cap no longer has an
owner.

**Recommendation.** Use one limit of 64 for both. If the reviewer prefers the smallest diff, keep
31 and record why.

**Settling evidence.** A writer unit test with a 40-level chain (A-3 in design.md).

## PD-6: identifiers for implicit roots (recommendation: preorder index in file-name prefixes)

**Recommendation.** Use `implicit-content/r{n}-{level}-{x}-{y}[-{z}]-{slot}.{ext}` and
`subtrees/r{n}-{level}-{x}-{y}[-{z}].subtree`, where `n` is the root's preorder index in the
producer hierarchy.
- They are file-name prefixes, not subdirectories. The legacy small-model `.gltf` keeps its sibling
  resources in `implicit-content/` (`tileset.rs:153-177`).
- The names are deterministic and distinct per root (I1 OD-7).

**Alternative.** The root's global slot path. It becomes unbounded in length for 64-level vector
trees.

## PD-7: the standards status of rows larger than their derived cells (recorded, not blocking)

**Background.**
- `ImplicitTiling/README.adoc:175` allows overrides "if desired".
- `:341` and `Semantics/README.adoc:145` call TILE_BOUNDING_BOX "a more specific bounding volume
  … than is implicitly calculated".
- Vector rows exceed their derived cells by up to 4,094× (`evidence/samples-layout.json`), because
  the vector hierarchy is a binary k-d tree addressed by child ordinal. The rewrite contract
  already adopted this as a finite profile inference: rows are authoritative and coordinates are
  addresses (`docs/architecture/implicit-rewrite-contract.md:30-47`).

**The forest's effect.** No client ever needs a derived cell, because each tile has a row and
there are no placeholders. The ambiguity therefore no longer changes viewer behaviour. The
standards wording remains a proof limit.

**Option, not recommended in this slice.** Move vector onto a true spatial quadtree. That is a
vector partitioning change, not a layout change.

## PD-8: what `implicit::expand_tileset` becomes (recommendation: P reworks it; removal stays outside)

**Background.**
- `expand_tileset` is public (`implicit.rs:19`). Tests use it to read outputs.
- It expands only a document-root implicit root (`implicit/tileset.rs:673`), so it would
  return the forest's skeleton unexpanded.

**Recommendation.** P makes it walk explicit children and expand every tile that carries
`implicitTiling`. P also deletes its JSON-template chain handling, because no producer writes that
any more and I1 makes it Unsupported. Removing the function stays outside both slices (I1
ownership plan).

**Alternative.** Point every test at the I1 reader. The reader is not public, and that would couple
P's tests to B's interface.

## PD-9: whether skeleton content leaves its original path (recommendation: no)

Skeleton tiles keep the producer's content URI and bytes (`t/{n}.glb` for point and mesh, the
vector content hash). Only tiles inside an implicit root are rewritten into `implicit-content/`.
- Vector retains its sources anyway (`retain_sources = true`).
- Point and mesh must therefore stop deleting the sources of skeleton tiles, and must publish them
  from the writer's member list.

## Proof limits this design does not close

- **Real-browser, GPU and network timing.** SwiftShader timings show behaviour only.
- **Every camera path.** Selection equality is shown on the sampled views only: 12 vector,
  12 pc-deep, 8 fixture leaves, plus the overview.
- **Standards wording.** PD-7.
- **Cesium versions other than 1.146.0, and other clients.** The bounded forest relies only on core
  explicit tiles and on single-subtree implicit roots that have rows. That is the smallest feature
  set a client needs, but no other client was executed.
- **Exact equivalence of baked translations.** The production writer rebases baked translations
  onto each implicit root, not onto the global root. The f64 sums in GLB node translations
  therefore differ from today's in rounding order. The differential check (A-4) has to state its
  comparison exactly; C2 slice 3 owns decoded containment.
