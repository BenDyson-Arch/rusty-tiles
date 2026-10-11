# P-IMPL: implicit layout without external-JSON content templates (v2)

Status: **v2 for PN's delta review**. It responds to the NO-GO in
`/tmp/pimpl-precode-review-20261011/review.md` (`16205c1c…`).
- [change-log.md](change-log.md) maps R-1 to R-14 to the changes. v1 is in [v1/](v1/).
- No production code was changed and no Cargo command was run.
- Base: `origin/develop` `f2ed077`. Every `src/` line reference is to that tree.
- I1 is pinned at v3.1 (`/tmp/rusty-tiles-i1-contract-20261011/contract-draft.md` `9674e60b…`;
  OD-1b withdrawn).
- Files and roles: [ownership-plan.md](ownership-plan.md). Choices: [open-decisions.md](open-decisions.md).
  Scripts and raw results: [evidence/v2/](evidence/v2/), with v1's in [evidence/](evidence/).

The user decided on 2026-10-11 that no point-cloud, vector or legacy `mesh-to-3tz` output may put
an external-JSON content template on an implicit root. The conditions are:
- refinement and picking stay correct in Cesium;
- bounds stay honest;
- load performance does not regress.

The coordinator added two conditions for v2:
- every JSON document must stay within the validator's per-document limits, at realistic sizes
  up to the vector default `maxTiles` of 100k;
- request counts must stay at or below today's.

## 1. Recommendation

**The layout is a bounded implicit forest whose explicit skeleton is split into explicit external
tileset documents.** Let S = 4 for both schemes.

- **Skeleton tiles.** A producer tile whose subtree has height ≥ S is an explicit tile.
- **Implicit roots.** A non-leaf child of a skeleton tile with height < S is an implicit root, and
  so is the root itself when the whole tree has height < S. Each has exactly one subtree:
  - `subtreeLevels == availableLevels == height + 1`;
  - `childSubtreeAvailability` is `{constant: 0}`;
  - every tile has a TILE_BOUNDING_BOX and TILE_GEOMETRIC_ERROR row.

  No client ever creates a placeholder whose volume is derived from the grid. That derived volume
  is the root cause, now proved causally by PN's ablation.
- **Documents (new in v2).** Explicit tiles are packed breadth-first into documents of at most
  16,384 value nodes, 2 MiB and 12 skeleton levels.
  - A skeleton tile that does not fit becomes a **link tile**: an explicit tile whose `content.uri`
    names an explicit external tileset document, with that tile as the document's root.
  - This is the ordinary core external-tileset feature on an *explicit* tile. It is not the banned
    form, which is a JSON *template* on an *implicit root*.
- **Bounds.** At most 32 levels, as a typed writer error, as today.

**Evidence summary:**
- **Scale.** Every document of every modelled case is ≤ 16,340 nodes (25% of the cap), ≤ 250 KB
  (3%) and JSON depth ≤ 31 (48%). That covers real vector up to 18.8k tiles with `lodLevels` 16,
  vector at 100k tiles with `lodLevels` 3 and 16 (real and full chains), and point octrees up to
  350k tiles.
- **Cesium, fixtures.** Every forest variant selects and picks 8/8 leaves, including the external-
  document variant `D`. The child-subtree layout `A` fails 8/8. A negative control `N` reproduces
  a schema-placement defect found in v2 (F-1).
- **Cesium, real data.** On the real LOD-chain vector cases (12.5k and 18.8k tiles), the vector
  perf case and `pc-deep`:
  - target leaves and picks are identical;
  - JSON plus subtree requests fall 2.4–46× (mf2-lod3 302 → 124; `pc-deep` 416 → 9);
  - total requests fall by 15–30%;
  - the forest's extra coarser selections are off-target and come from Cesium's explicit-tile
    child-bounds cull, the same as `--explicit` (§5.2).
- **Perf lane** (`bb7d054` scripts, ABBA ×2, CPUs 13,15, 24 runs). No gate problems.
  - Median load time: vector 2.00 s → 1.93 s, mf2-lod3 2.01 s → 1.94 s, `pc-deep` 16.54 s → 15.10 s.
  - Implicit-root-loading deep views (ABBA ×2): 25.85 s → 22.93 s.
  - Request bytes: +3.6% on vector perf, −29% on mf2-lod3 (§5.3).

## 2. Why external roots exist today

Unchanged from v1 and verified by PN.
- Cesium culls each child-subtree placeholder by its **derived** cell
  (`Implicit3DTileContent.js:1021-1068`, `Cesium3DTilesetTraversal.js:206-215`). Rows are used only
  for tiles inside a loaded subtree (`:638-669`).
- The writer chunks to external-JSON roots whenever a subtree-root row escapes its derived cell
  (`implicit/tileset.rs:284-308`; CHANGELOG `:28`, `ba4a195`).
- Today's validator refuses implicit roots on non-root explicit tiles (`validate.rs:1280-1284`).
- **Causal proof** (PN, `runs/fixtures-patched-A.log`). Patching only `makePlaceholderChildSubtree`
  to use the implicit root's volume turns fixture `A` from 0/8 to 8/8.
- Escape magnitudes are unchanged:
  - vector: kilometres, because the binary k-d tree is addressed by child ordinal;
  - `pc-deep`: µm, from f32 rounding padding;
  - `pc-perf-2000`: none (`evidence/samples-layout.json`).

## 3. Candidates

### 3.1 Layout (unchanged choice; wording corrected per R-9a)

| Candidate | Cesium 1.146 (executed) | Disposition |
| --- | --- | --- |
| (a) One implicit tree with child subtrees | 0/8 leaves (fixture `A`); causally the placeholder cull | Rejected |
| (b) Pad the grid until cells contain content | Impossible for vector; needs directed f32 rounding for points | Rejected |
| (d) Implicit roots with child subtrees under an explicit root | As (a) below level S | Rejected |
| (e1) One subtree for the whole tree | 8/8 (fixture `S`); dense availability: about 700 KB per stream for vector at 12 levels | The forest's own case when height < S |
| (e3) `--explicit` by default | 8/8 (fixture `E`); one JSON document per archive breaks the per-document caps at about 1.3k vector tiles | Rejected as default |
| **Forest, (c)+(d)** | 8/8 (`F`); 8/8 with external documents (`D`) | **Chosen** |

**Conformance.** The forest uses only:
- core explicit tiles;
- core external tilesets referenced from explicit tiles (core `:731-741`);
- implicit roots on any tile (IT `:34`, `:43`), meeting every root constraint (IT `:94-97`);
- tile metadata overrides (IT `:175`);
- constant-0 child availability, meaning "the tileset does not subdivide further" (IT `:314`).

**The rows keep the "more specific" dispute** (IT `:341`, Semantics `:145`; PD-7). Vector rows
exceed their derived cells by up to 4,094×. The forest removes the dispute's client consequence,
because no client ever needs a derived cell, but not the dispute itself.

### 3.2 Storing the explicit skeleton (new; R-1)

| Mechanism | Per-document limits (depth 64, 65,536 nodes, 8 MiB) | Up-front load | Requests (deep views) | Disposition |
| --- | --- | --- | --- | --- |
| One `tileset.json` (v1) | Fails from about 6.9k vector tiles (about 3k with chains); depth fails above about 33 levels | Grows with N | Fewest | Rejected |
| One `tileset.json`, larger S | Partial: chains above S stay explicit for any affordable S (100k at `lodLevels` 16: 55,559 explicit tiles at S = 8) | Grows with N | — | Rejected |
| **Explicit external documents, S = 4** | ≤ 25% of nodes, ≤ 3% of bytes, ≤ 48% of depth in every case (§5.1) | Bounded root document: 182 KB at 12.5k tiles, against 916 KB today | Below today in every real case (§5.2) | **Chosen** |
| Explicit external documents, S = 6 | Within limits | Larger root document (328 KB at 12.5k tiles) | About 4% fewer than S = 4 | Not chosen (PD-2) |
| Raise the I1 caps | Needs its own resource argument | — | — | Not needed |
| Accept a smaller domain (user decision) | — | — | — | Not needed for JSON limits (PD-12 covers archive-level caps) |

## 4. Contract (v2)

### 4.1 Representation

**Input and definitions.**
- The input is the producer's finished explicit hierarchy `H`. Each node has:
  - a box `B(n)` in its own frame;
  - an error `e(n)` and `refine(n)`;
  - extras;
  - 0..k payload contents;
  - a child slot (`extras.implicitChildIndex`, else the array index);
  - children with pure-translation transforms.
- `h(n)` = 0 for leaves, otherwise 1 + max over the children. `L = h(root) + 1`.
- `S = 4`. Document budgets are `DOC_NODES = 16,384`, `DOC_BYTES = 2 MiB` and `DOC_LEVELS = 12`.
  These are profile constants: there is no option, and no data-dependent alternative layout.

**Classification** (decided once, in the writer):
- If `L > 32`, a typed error.
- If `h(root) < S`, the root is the single implicit root.
- Otherwise:
  - a tile with `h ≥ S` is **skeleton**;
  - a skeleton tile's child with `h = 0` is an **explicit leaf**;
  - a child with `1 ≤ h < S` is an **implicit root**;
  - everything below an implicit root is **implicit**.

**Implicit root `r`.** `n` is its preorder index in H, written as literal decimal digits (R-7).
- **The explicit tile** carries:
  - the producer's `transform`, `boundingVolume.box = B(r)`, `geometricError = e(r)`, and
    `refine = refine(r)` (R-5);
  - `implicitTiling = {scheme, subtreeLevels: h(r)+1, availableLevels: h(r)+1, subtrees: {uri: "subtrees/r{n}-{level}-{x}-{y}[-{z}].subtree"}}`.
- **The slot list (R-4).** The **slot list of r** is the sorted, distinct `(content index, suffix)`
  pairs over the tiles of r.
  - It drives both r's templates, `implicit-content/r{n}-{level}-{x}-{y}[-{z}]-{slot}.{suffix}`,
    and its subtree's `contentAvailability` layers, one per entry, in the same order.
  - An empty list means no `content`/`contents` and no `contentAvailability` (IT `:299`).
  - Content headers are copied per slot.
- **One subtree at local `(0,…)`.**
  - Tile and content availability as `implicit.rs` writes today.
  - `childSubtreeAvailability {constant: 0}`.
  - A `rustyTile` row for every tile: `B(t)` translated by `δ(t)`, `e(t)`, and the extras without
    `implicitChildIndex`.
  - `δ(t)` is the sum of child translations from r down to t, accumulated parent-to-child in f64.
- **Payloads.** Implicit-tile payloads have `δ(t)` baked into the GLB scene roots as
  `(δ0, δ2, −δ1)` (`translate_content`, kept). `δ(r) = 0` gives a byte copy. A `.gltf` can occur
  only at r.

**Explicit tiles** (skeleton tiles and explicit leaves) are the producer's tile objects:
- box, error, refine, `transform`, extras without `implicitChildIndex` (R-8), and content URI;
- the payload bytes are not rewritten.

**Documents.**
- `tileset.json` holds the root.
- The writer packs breadth-first. Each skeleton tile is taken with its *unit*: its explicit leaves,
  its implicit-root tiles, and the retained source names they add (vector).
- Every frontier skeleton child is reserved at link cost.
- A child is accepted when the document stays within `DOC_NODES` value nodes (validator semantics:
  the root, containers and scalars) and `DOC_BYTES` of serialized bytes, and the child is at most
  `DOC_LEVELS` skeleton levels below the document root.
- A child that is not accepted becomes a **link tile**: `{boundingVolume: B(t), geometricError:
  e(t), refine, transform (producer's), content: {uri: "tileset-d{m}.json"}}`.
- Document m: `{asset, geometricError: e(t), extensionsUsed (as tileset.json), schema (P12), root:
  t without transform, extras (retained list, below)}`.
- Documents are archive-root members, so relative URIs inside them are unchanged.
- m counts from 1 in creation order.

**The schema (P12, F-1).**
- `tileset.json` carries the `rustyTilesImplicit` schema if any implicit root exists in any
  document. Cesium reads the schema only from the root (`Cesium3DTileset.js:2470`).
- Every document that holds an implicit root also carries it, so each document is self-describing
  for other readers.

**Retained sources (vector, `SourcePolicy::Retain`; R-3, PD-11 (b)).**
- A document that holds implicit roots lists, in its top-level `extras.rustyTilesSourceContents`,
  the raw `t/*` sources of every payload rewritten under those roots.
- The document budget counts these names.
- Point cloud, mesh and M2 (`Remove`) delete those sources instead.

### 4.2 Invariants

Each invariant is checked by A-2. A-3 checks it through I1 where I1 can.

- **P1, no JSON template** (restated). No implicit root's content template or available content
  names a `.json` resource. The only JSON tileset documents are `tileset.json` and the
  `tileset-d{m}.json` documents. Each of those is referenced by exactly one link tile's
  `content.uri`, and every link tile is an explicit tile.
- **P2, no placeholders.**
  - Every implicit root has `subtreeLevels == availableLevels ≤ S`, exactly one subtree member, and
    `childSubtreeAvailability {constant: 0}`.
  - No implicit tile references a document, and no subtree references another.
- **P3, rows everywhere.**
  - Every implicit tile has a row equal to the producer's `B(t)+δ(t)` and `e(t)`, bitwise as f64.
  - The level-0 row equals the root tile's box and error.
- **P4, distinct and closed names.** Prefixes are distinct per root and documents distinct per link.
  Every name a template produces for available content exists. Every written member is referenced
  by a tile, a template, a link, or a document's retained-source list (R-3).
- **P5, effective-tree equality.** Expanding every document and root yields exactly H:
  - tiles and parent/child relation;
  - errors, refine and extras (without `implicitChildIndex`);
  - world-frame boxes after composing transforms (A-4 states the arithmetic);
  - payloads equal to the producer's outside the GLB scene-root translation.

  A link tile and its document root count as one tile of H.
- **P6, coherence preserved.** No box is added, grown or shrunk. A link tile's box and error equal
  its document root's.
- **P7, determinism.** The bytes are a function of H and the profile constants.
- **P8, document bounds (new).** Every JSON document has at most 16,384 value nodes, at most 2 MiB,
  and JSON depth at most 32.
- **P9, level bound (new; R-2).** `L ≤ 32`.
- **P10, slots (R-4).** A root's layer count equals its template count, and both are zero for a
  root without content.
- **P11, no `implicitChildIndex`** in any written tile or row (R-8).
- **P12, schema (F-1).** `tileset.json` has the schema iff some implicit root exists. Every
  document holding an implicit root has it.

### 4.3 Supported domain

- **Producers:**
  - point cloud (OCTREE);
  - vector, including `--quantize` and `--meshopt` (QUADTREE);
  - legacy `mesh-to-3tz` (OCTREE);
  - the M2 small-model wrapper (one tile, ADD, `.gltf` with resources: a single one-level root).
- **Hierarchy:**
  - at most 8 (octree) or 4 (quadtree) children, with distinct slots;
  - translation-only child transforms (the root may carry any transform);
  - box volumes;
  - payload suffixes glb, gltf or b3dm (gltf only at an implicit root or an explicit tile);
  - L ≤ 32.
- **Validated domain.** P8 keeps every JSON document within the validator's limits at any H. The
  archive-level caps (`archive_entries`, `hierarchy_visits`, both 65,536) bound validated archives
  separately, in any layout (PD-12).

### 4.4 Failure semantics

Each of the following is a typed `Error::Data` from the writer, raised before publication.
Publication stays F0's staging and atomic completion.
- `L > 32` ("hierarchy has N levels; implicit output admits at most 32").
- A duplicate child slot, or a slot ≥ the branching factor.
- A non-translation child transform.
- A `refine` below an implicit root that differs from the root's (R-5).
- An unsupported content suffix.
- A missing or nonfinite box or error.
- A single unit larger than the document budget. This is unreachable for producer tiles: the
  largest measured unit is far below 16k nodes. It is a guard, not a fallback.
- `translate_content` failures.
- Checkpoint and cancellation, as today.

There is no tolerance constant and no data-dependent alternative layout.

### 4.5 What changes in each producer

| Producer | Change |
| --- | --- |
| Shared writer `implicit/tileset.rs` (PW) | `write_forest` replaces `write_tileset`, `write_tileset_recorded`, `write_chunks`, `chunk_document` and the escape test (`:284-308`); it writes the documents, links, roots and subtrees. `expand_tileset` follows explicit children, link tiles and documents, and expands every implicit root, with one shared budget (R-12d). It no longer follows JSON templates |
| Point cloud (PP) | `write_forest(…, Remove)`; `tree.members = output.members`; encoder `…-implicit-v3` |
| Vector (PV) | `write_forest(…, Retain)`. Every report statistic at `store.rs:1003-1036` comes from `ForestOutput.tiles`: `maximumTileBytes`, `primitiveReferences`, `uncompressedTileBytes`, `encodedTileBytes`, `maximumTilePrimitives`, `pointAggregation.contentTiles` (R-6). Reuse identity `implicit-forest-v3` |
| Legacy mesh `tile.rs:510-540` (PP) | `write_forest(…, Remove)`; stage `output.members` instead of hard-coded directories |
| M2 `tileset.rs:153-177` (PP) | **Its writer call changes** to `write_forest(…, Remove)` (R-12a); encoder string |

**`ForestOutput.tiles` (R-6).** One record per tile of H, in preorder, with role, written extras, and
`(member, bytes)` per content.
- **`encodedBytes` semantics:** for skeleton tiles and explicit leaves, the producer's value,
  because the payload is unchanged; for implicit tiles, the written byte length after baking, as
  today's writer sets it.
- Link tiles are not tiles of H and have no record.

**Retained, reworked and removed:**
- **Retained:** `translate_content`; `Subtree` encoding; the schema; `content_suffix`.
- **Reworked:** `collect`, which keeps children and computes heights and preorder ids.
- **Removed:** chunking, `derived_box` in the writer, `LEVELS`, `implicit-tileset-*`, and the
  hard-coded REPLACE.

### 4.6 The I1 interface (I1 v3.1)

- **Admission of explicit external documents: no I1 change.**
  - I1 admits implicit roots "on any explicit tile, including inside explicit external tilesets"
    (`contract-draft.md` `:150`).
  - It keeps today's explicit traversal for external tilesets (`:433-436`; `validate.rs:1364-1376`,
    `:1112-1126`).
  - The JSON-template exclusion is raised only in `admit_root` for an implicit root's templates
    (`:727`). A link tile is an explicit tile.
  - Limits: each document is read with C1's JSON limits. Each document costs 2 `hierarchy_visits`
    (Tileset and ExitTileset), and a link adds one explicit Node visit.
  - Depth: a link plus its document root adds 1 to `hierarchy_depth` (128). At L ≤ 32 with fewer
    than 4 document levels on any path, depth is at most about 36.
- **Change needed: the retained-source gate (R-3).** I1-B drops the `implicitTiling` condition at
  `validate.rs:1204` and honours `extras.rustyTilesSourceContents` on every tileset document
  (ownership plan, I1 asks 1).
- **Asks:** W11 witnesses (I1 asks 3) and link tiles in `budgets.tiles` (PD-13).

## 5. Executed evidence (v2)

**Model and runs.**
- The production writer is modelled by `evidence/v2/forest_model.py` (stdlib; independent of
  `src/`). It reads the producer's `--explicit` archive, so skeleton `extras` and `transform` are
  production-faithful (R-9b). `pc-deep` is read from today's implicit archive, which has no
  transforms.
- Every size below is from this model of the production writer, not from production.
- All browser runs used Cesium 1.146.0 (engine 26.4.0) and Chromium 153.0.8010.12 on SwiftShader.
  - Each ran with `taskset -c 13,15 nice -n 10`, one browser at a time. CPUs 13 and 15 are SMT
    threads of P-cores 6 and 7; their siblings are 12 and 14.
  - The session's base niceness is −4, so the recorded niceness is 6.
- Per-CPU busy time was sampled around every run (`*.cpuload`).
- Model and scale runs used E-cores 16,18.

### 5.1 Scale: every document within the limits (`evidence/v2/scale/`, `scale_table.py`)

S = 4 for both schemes. "Root doc" is `tileset.json`. Entries and visits use I1 v3.1's counting.

| Case | Tiles | L | Explicit | Roots | Links = docs−1 | Max nodes/doc | Max bytes/doc | Max JSON depth | Root doc (nodes / bytes) | Largest subtree | Entries | Visits |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| pc-deep | 2,233 | 5 | 1 | 8 | 0 | 258 | 4,021 | 6 | 258 / 4,021 | 38,496 | 2,245 | 2,243 |
| pc-perf-2000 | 428 | 5 | 1 | 8 | 0 | 258 | 4,198 | 6 | 258 / 4,198 | 11,208 | 440 | 438 |
| synth-pt-100k-b4 | 87,381 | 9 | 341 | 1,024 | 233 | 16,269 | 131,087 | 14 | 16,269 / 131,087 | 10,216 | 88,642 | 89,106 |
| synth-pt-100k-b8 | 37,449 | 6 | 9 | 64 | 0 | 3,042 | 31,228 | 8 | 3,042 / 31,228 | 64,928 | 37,517 | 37,515 |
| synth-pt-1m-b4 | 349,525 | 10 | 1,365 | 4,096 | 337 | 16,297 | 118,685 | 14 | 16,297 / 118,685 | 10,216 | 353,962 | 354,634 |
| synth-pt-1m-b8 | 299,593 | 7 | 73 | 512 | 24 | 16,138 | 162,461 | 10 | 16,138 / 162,461 | 64,928 | 300,133 | 300,179 |
| synth-vec-100k-lod16-full | 100,007 | 30 | 77,783 | 5,556 | 5,744 | 16,328 | 204,817 | 30 | 16,318 / 181,298 | 2,208 | 133,528 | 122,797 |
| synth-vec-100k-lod16-real | 100,325 | 27 | 25,219 | 13,852 | 1,500 | 16,340 | 249,779 | 31 | 16,330 / 181,598 | 5,984 | 189,933 | 118,679 |
| synth-vec-100k-lod3-full | 99,999 | 19 | 19,999 | 20,000 | 668 | 16,330 | 246,134 | 21 | 16,330 / 182,464 | 2,216 | 189,396 | 122,005 |
| synth-vec-100k-lod3-real | 99,860 | 20 | 15,119 | 10,490 | 188 | 16,330 | 229,213 | 23 | 16,330 / 182,337 | 4,704 | 193,466 | 110,916 |
| vec-mf2-lod16 | 18,778 | 25 | 4,748 | 2,547 | 290 | 16,294 | 198,125 | 31 | 16,294 / 180,495 | 4,984 | 35,646 | 22,197 |
| vec-mf2-lod3 | 12,530 | 17 | 1,230 | 1,225 | 188 | 16,294 | 181,949 | 20 | 16,294 / 181,949 | 4,656 | 25,244 | 14,321 |
| vec-perf | 547 | 12 | 48 | 45 | 0 | 5,456 | 103,805 | 21 | 5,456 / 103,805 | 6,064 | 1,092 | 594 |

Notes:
- The synthetic 100k vector cases copy real tile JSON objects from the real 12.5k/18.8k
  hierarchies, and real chain-length draws (`real`) or `lodLevels` chains on every leaf (`full`).
- Every case (S = 4, the chosen value) keeps every document at most:
  - 16,340 value nodes (≤ 25% of 65,536);
  - 249,779 bytes (≤ 3% of 8 MiB);
  - JSON depth 31 (≤ 48% of 64).
- The S = 6 and S = 8 rows are in `scale/`.
- `entries` and `visits` exceed 65,536 in every 100k case, in any layout (PD-12).

**Today's layout on the real deep vector cases** (`today-layout.json`):

| Case | Documents | Subtrees | Root document | Entries |
| --- | --- | --- | --- | --- |
| mf2-lod3 | 2,634 | 2,634 | 12,606 nodes, 916 KB | 30,326 |
| mf2-lod16 | 4,694 | 4,694 | 18,854 nodes, 1.37 MB | 46,942 |

Today's root document grows with N (the retained-source list), and reaches the node cap at about
65k tiles.

### 5.2 Cesium

**Fixtures** (`evidence/v2/cesium/`: `make_fixtures.py`, `run.py`, `fixtures-results.json`).

| Layout | Leaf selected | Leaf picked | Holes | Control | Requests (JSON / subtree / GLB) | Errors |
| --- | --- | --- | --- | --- | --- | --- |
| E | 8/8 | 8/8 | 0 | pass | 1 / 0 / 14 |  |
| A | 0/8 | 0/8 | 4 | pass | 1 / 5 / 8 |  |
| S | 8/8 | 8/8 | 0 | pass | 1 / 1 / 14 |  |
| X | 8/8 | 8/8 | 0 | pass | 5 / 5 / 14 |  |
| F | 8/8 | 8/8 | 0 | pass | 1 / 4 / 14 |  |
| D | 8/8 | 8/8 | 0 | pass | 3 / 4 / 14 |  |
| N | 0/8 | 0/8 | 0 | fail | 3 / 4 / 2 | tileFailed: Cannot read properties of undefined (reading 'classes') (×4) |

The fixtures run on CPUs 13 and 15, with probe `fb2d0fdb…`, runner `c3f41250…` and generator `578884d8…`.
- `A` fails 0/8 (4 holes). PN's patched-Cesium ablation proves the placeholder cull is the cause.
- `F` and `S` pass 8/8.
- `D` passes 8/8. It is the forest with the skeleton split into two explicit external documents,
  with implicit roots carrying their own `transform` inside them. It costs 3 JSON requests.
- `N` is the negative control for F-1: `D` without the schema in the root `tileset.json`. Every
  implicit tile load fails with "Cannot read properties of undefined (reading 'classes')", and 0/8
  leaves load. Cesium reads the metadata schema only from the root (`Cesium3DTileset.js:2470`).
  P12 makes this impossible.
- `E`, `S`, `X` and `F` reproduce v1 and PN's runs exactly.

**Real data, deep views** (`deep_runs.sh`, `cesium/run_real.py`, `compare_runs.py`, `deep/*.json`).
- 16 views per case, aimed at a real vertex of the target leaf (R-9c), 40 m above it.
- maximumScreenSpaceError 1e-4 (1 for `pc-deep`); the overview uses 16.
- Deterministic mode, 30 stable frames.
- Picks that are null on both sides are reported separately and never counted as agreement (R-9c).

| Case | Child-bounds cull | Target leaf selected (today / other) | Same selection | Other selects an ancestor | Same non-null pick (null=null separate) | Requests today (JSON / subtree / GLB) | Requests other | View seconds today / other |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mf2-lod16 (today vs forest4) | off | 14 / 14 of 16 | 16/16 | 0 (today coarser: 0) | 4 (null both: 12) | 248 / 248 / 773 | 57 / 62 / 773 | 29.21 / 24.25 |
| mf2-lod16 (today vs forest4) | on | 14 / 14 of 16 | 2/16 | 13 (today coarser: 0) | 4 (null both: 12) | 248 / 248 / 773 | 57 / 60 / 769 | 29.57 / 24.22 |
| mf2-lod3 (today vs forest4) | off | 6 / 6 of 16 | 16/16 | 0 (today coarser: 0) | 4 (null both: 12) | 151 / 151 / 518 | 45 / 79 / 518 | 25.66 / 22.68 |
| mf2-lod3 (today vs forest4) | on | 6 / 6 of 16 | 3/16 | 13 (today coarser: 0) | 4 (null both: 12) | 151 / 151 / 518 | 45 / 79 / 516 | 26.04 / 22.89 |
| mf2-lod3-explicit (today vs explicit) | on | 6 / 6 of 16 | 1/16 | 14 (today coarser: 0) | 4 (null both: 12) | 151 / 151 / 518 | 1 / 0 / 512 | 25.84 / 21.38 |
| mf2-lod3-explicit (today vs forest4) | on | 6 / 6 of 16 | 3/16 | 13 (today coarser: 0) | 4 (null both: 12) | 151 / 151 / 518 | 45 / 79 / 516 | 25.84 / 22.79 |
| mf2-lod3-s6 (today vs forest6) | off | 6 / 6 of 16 | 16/16 | 0 (today coarser: 0) | 4 (null both: 12) | 151 / 151 / 518 | 35 / 62 / 518 | 25.87 / 22.39 |
| mf2-lod3-s6 (today vs forest6) | on | 6 / 6 of 16 | 4/16 | 12 (today coarser: 0) | 4 (null both: 12) | 151 / 151 / 518 | 35 / 62 / 518 | 25.94 / 22.92 |
| pc-deep (today vs forest4) | on | 16 / 16 of 16 | 16/16 | 0 (today coarser: 0) | 16 (null both: 0) | 208 / 208 / 2209 | 1 / 8 / 2209 | 26.51 / 26.23 |
| vec-perf (today vs forest4) | off | 16 / 16 of 16 | 16/16 | 0 (today coarser: 0) | 2 (null both: 14) | 55 / 55 / 218 | 1 / 45 / 218 | 22.99 / 21.43 |
| vec-perf (today vs forest4) | on | 16 / 16 of 16 | 14/16 | 2 (today coarser: 0) | 2 (null both: 14) | 55 / 55 / 218 | 1 / 45 / 218 | 22.72 / 21.48 |

**Reading the table:**
- **Target behaviour is identical in every case.** Each view selects the same target-path tile in
  every layout (`deep/target-path.jsonl`), and every non-null pick is equal. Picks are mostly null in *both* layouts, because
  a centre pick on a thin line or point vertex misses at 3×3 px. Null=null is never counted as
  agreement.
- **The selection differences are Cesium's child-bounds cull on explicit tiles (R-14).**
  - With `cullWithChildrenBounds: false`, selections are identical in every view of every case
    (mf2-lod3 16/16 at S = 4 and S = 6, mf2-lod16, vec-perf).
  - With defaults, on the LOD-chain cases, the forest selects a strict ancestor of a today-selected
    tile in 13/16 views (mf2-lod3, mf2-lod16), always off the target path and with equal pixel
    coverage.
  - On vec-perf it is 2/16. On `pc-deep` it is 0/16: the skeleton is 1 tile.
  - Today never selects coarser.
- **`--explicit` does the same, more strongly** (`mf2-lod3-explicit-cb1`): it selects coarser than
  today in 14/16 views, and the forest matches `--explicit`'s selection in 13/16. The forest's
  behaviour is therefore today's explicit-tile behaviour, confined to the skeleton.
- **Requests and time.**
  - JSON plus subtree requests fall from 302 to 124 (mf2-lod3), 496 to 117 (mf2-lod16), 110 to 46
    (vec-perf) and 416 to 9 (`pc-deep`).
  - Content requests are equal, or slightly fewer where coarser tiles are selected.
  - Total view time is lower in every vector case (for example 26.0 s to 22.9 s on mf2-lod3) and
    equal on `pc-deep`.
- **S = 6 against S = 4** (mf2-lod3): 615 against 640 requests, at the same view time. The
  difference does not justify the larger root document (PD-2).
- **R-9f.** v1's "today did not settle in 90 s" `pc-deep` overview ran on contended CPUs 0 and 1. It
  is non-comparative and withdrawn. In v2's `pc-deep` run on CPUs 13 and 15, both layouts settle,
  and selections and picks match 16/16.

Where a target leaf is not selected, the selected tile on the leaf's path has a geometric error
of at most 5.7 µm. Its SSE at 40 m is below 1e-4, so Cesium correctly stops there. The selected
target-path tile is the same in every layout, in 16/16 views of every run (`target_path.py`,
`deep/target-path.jsonl`).

### 5.3 Perf lane, ABBA ×2 (`evidence/v2/perf_abba.sh`, `perf/*.json`, `*.cpuload`)

**Setup.**
- Lane scripts extracted from `bb7d054`: `cesium.py` `890f0159…` and `cesium.cjs` `23c78bbc…`.
  The run used `evidence/perf_lane_bb7d054/cesium_cpus.py`, which differs only in the affinity line,
  set to CPUs 13,15 because CPUs 0 and 1 carried another agent's load (CPU 0 at 46–93% busy across all 38 sampled v2 runs).
- One case per invocation, in the order base1, head1, head2, base2, base3, head3, head4, base4.
- `--baseline base1`, gate 1.25× with floors.
- Per-CPU busy was sampled around each invocation, including SMT siblings 12 and 14.
- **Cases.**
  - Base: today's develop-equivalent archives (`vec-perf.3tz` sample, `mf2-lod3-implicit.3tz`,
    `pc-deep.3tz`).
  - Head: the S = 4 forest models.

| Case | Run | Seconds to loaded | Requests (JSON / subtree / content) | Request bytes | Gate problems | Busy % on 13/15 (siblings 12/14) |
| --- | --- | --- | --- | --- | --- | --- |
| vector | base1 | 2.01 | 1 / 1 / 4 | 1,563,790 | none | 30.2/26.8 (0.5/1.9) |
| vector | head1 | 1.94 | 1 / 0 / 4 | 1,620,469 | none | 32.7/23.7 (1.4/2.2) |
| vector | head2 | 1.94 | 1 / 0 / 4 | 1,620,469 | none | 30.7/27.2 (2.3/1.4) |
| vector | base2 | 2.02 | 1 / 1 / 4 | 1,563,790 | none | 30.7/28.5 (2.5/1.7) |
| vector | base3 | 1.99 | 1 / 1 / 4 | 1,563,790 | none | 27.9/30.7 (0.3/0.5) |
| vector | head3 | 1.91 | 1 / 0 / 4 | 1,620,469 | none | 26.7/32.0 (1.7/1.2) |
| vector | head4 | 1.93 | 1 / 0 / 4 | 1,620,469 | none | 29.9/28.6 (1.7/0.9) |
| vector | base4 | 1.99 | 1 / 1 / 4 | 1,563,790 | none | 31.4/27.9 (0.8/0.8) |
| mf2-lod3 | base1 | 2.00 | 1 / 1 / 4 | 2,523,602 | none | 50.4/24.3 (1.2/3.7) |
| mf2-lod3 | head1 | 1.94 | 1 / 0 / 4 | 1,783,669 | none | 28.5/26.9 (2.7/1.6) |
| mf2-lod3 | head2 | 1.94 | 1 / 0 / 4 | 1,783,669 | none | 29.8/31.3 (3.5/3.2) |
| mf2-lod3 | base2 | 2.01 | 1 / 1 / 4 | 2,523,602 | none | 49.2/27.3 (2.3/7.1) |
| mf2-lod3 | base3 | 2.12 | 1 / 1 / 4 | 2,523,602 | none | 50.9/41.3 (19.6/23.3) |
| mf2-lod3 | head3 | 1.95 | 1 / 0 / 4 | 1,783,669 | none | 33.8/34.9 (3.4/3.1) |
| mf2-lod3 | head4 | 1.96 | 1 / 0 / 4 | 1,783,669 | none | 29.8/32.8 (5.6/1.6) |
| mf2-lod3 | base4 | 2.00 | 1 / 1 / 4 | 2,523,602 | none | 56.7/17.5 (2.3/13.3) |
| pc-deep | base1 | 16.52 | 208 / 208 / 2233 | 12,892,981 | none | 46.8/46.6 (11.0/9.5) |
| pc-deep | head1 | 15.33 | 1 / 8 / 2233 | 12,530,265 | none | 37.4/38.0 (2.4/2.0) |
| pc-deep | head2 | 15.03 | 1 / 8 / 2233 | 12,530,265 | none | 41.2/39.4 (2.4/2.8) |
| pc-deep | base2 | 16.99 | 208 / 208 / 2233 | 12,892,981 | none | 42.3/42.0 (8.6/9.0) |
| pc-deep | base3 | 16.30 | 208 / 208 / 2233 | 12,892,981 | none | 42.1/42.7 (5.2/6.1) |
| pc-deep | head3 | 14.78 | 1 / 8 / 2233 | 12,530,265 | none | 40.6/42.1 (3.5/2.8) |
| pc-deep | head4 | 15.17 | 1 / 8 / 2233 | 12,530,265 | none | 42.7/43.2 (9.7/9.3) |
| pc-deep | base4 | 16.57 | 208 / 208 / 2233 | 12,892,981 | none | 39.2/38.6 (3.5/3.4) |

| Case | Base median s (n) | Head median s (n) | Base range | Head range |
| --- | --- | --- | --- | --- |
| vector | 2.00 (4) | 1.93 (4) | 1.99–2.02 | 1.91–1.94 |
| mf2-lod3 | 2.01 (4) | 1.94 (4) | 2.00–2.12 | 1.94–1.96 |
| pc-deep | 16.54 (4) | 15.10 (4) | 16.30–16.99 | 14.78–15.33 |

- **No gate problems in any of the 24 runs.**
- The forest is within or below the base spread in every case.
- **Request bytes.**
  - vector: +56.7 KB (+3.6%; the root document is 104 KB against 41 KB + 5.7 KB);
  - mf2-lod3: −740 KB (−29%; today's root document is 916 KB);
  - `pc-deep`: −2.8%.
- **Foreign load on the siblings** was ≤ 11% except one base run (base3 mf2-lod3: 19.6/23.3%), which
  is also the slowest base run. Excluding it changes no conclusion.

**At the lane's default framing, neither vector case loads an implicit root or a document.** The
overview selects skeleton tiles. So the R-10 implicit-root case is timed separately: mf2-lod3 deep
views (16 views, MSSE 1e-4, child-bounds cull on, deterministic, CPUs 13,15), in ABBA ×2 by case
order (`deep_abba.sh`):

| Invocation | Order | Today: view s / requests (JSON/subtree/GLB) | Forest: view s / requests | Busy % 13/15 (12/14) |
| --- | --- | --- | --- | --- |
| round1-AB | today then forest4 | 25.90 / 151/151/518 | 23.03 / 45/79/516 | 16.0/15.4 (2.6/2.6) |
| round1-BA | forest4 then today | 25.80 / 151/151/518 | 22.88 / 45/79/516 | 14.7/14.7 (1.5/1.3) |
| round2-AB | today then forest4 | 25.73 / 151/151/518 | 22.98 / 45/79/516 | 15.5/15.9 (4.1/3.6) |
| round2-BA | forest4 then today | 25.93 / 151/151/518 | 22.88 / 45/79/516 | 16.4/16.1 (4.4/4.4) |
| median | | 25.85 s | 22.93 s | |

The forest is about 11% faster than today in every round. It loads 45 documents and 79 subtrees
against 151 + 151, with equal content requests (516 against 518).

## 6. Acceptance plan (v2)

Execution identities are bound per AGENTS.md: exact source heads, binary sha256, archive digests
and script hashes. Static observations, executed defects, profile choices and proof limits are
recorded separately.

- **A-1, writer unit tests** (`tests/implicit_forest.rs`, PW).
  - The v1 cases, plus:
    - an L = 32 chain, which succeeds and is A-3-validated;
    - an L = 33 chain, which raises the typed error (R-2);
    - a hierarchy whose skeleton forces at least 3 documents and a link two levels deep;
    - a single unit near the budget;
    - a root with no content (no templates, no layers);
    - two roots with different slot lists (R-4);
    - a `refine` change below a root (the typed error, R-5);
    - `.gltf` only at an implicit root (M2);
    - `implicitChildIndex` absent everywhere (R-8).
  - P1–P12 are asserted on the bytes. P5 is asserted against H.
- **A-2, independent forest checker** (PO, `bench/architecture_audit/implicit_layout/`).
  - Frozen before PO sees the writer diff.
  - Asserts P1–P12 on every W11 archive.
  - Sensitive controls, each of which must fail:
    - a JSON template on an implicit root;
    - a child-subtree bit;
    - a missing row;
    - a row differing by 1 ulp;
    - a repeated prefix;
    - an unreferenced member;
    - a retained source missing from its list;
    - a document over its node budget;
    - a link tile whose box differs from its document root's;
    - a missing root schema.
- **A-3, the I1 reader as the validator** (W11, merge gate).
  - Cases: every case in v1's A-3, plus:
    - the deep vector cases mf2-lod3 (12.5k tiles, LOD chains) and mf2-lod16;
    - a vector default case with at least 10k tiles;
    - the 32-level bound;
    - an output with external documents.
  - Expected for every case:
    - `checks` = develop's explicit-archive checks plus `"implicitAddressingAndAvailability"`
      immediately after `"hierarchyBounds"` (I1 `:705`);
    - `notInspected` = develop's minus `"implicitAddressingAndAvailability"` plus
      `"recordedBudgets.implicitTileExtras"`, and still containing `"decodedContentBounds"`,
      `"decodedContentBounds.implicitTiling"` and `"metadataSemantics"` (I1 `:713-720`);
    - `tiles` = |H| + link tiles (or |H| if I1 adopts PD-13);
    - `contentReferences` = emitted contents.
  - R records the full expected arrays per case before the run, and the run must match them
    exactly.
  - No Unsupported, no ResourceLimit.
- **A-4, differential** (develop `f2ed077` against the combined head, same inputs).
  - Expand both with A-2 and compare:
    - the same tile set and relation, errors, refine, and extras without `implicitChildIndex`.
      Rows are bitwise equal after expressing both in the top root frame: the box half-axes
      bitwise; the centres as `B.c + Σ translations`, summed parent-to-child in f64 (develop's
      global δ, and head's per-root δ composed with each implicit root's `transform`), with the
      summation order stated in the record;
    - payloads (R-11): the GLB BIN chunk byte-identical; the JSON chunk equal as parsed JSON
      except for the scene-root `translation` or `matrix`; and that translation equal to the
      composed δ under the same summation rule.
- **A-5, Cesium** (PO, ported from `evidence/v2/cesium/`).
  1. Fixture suite E/A/S/X/F/D/N. `A` must fail 8/8, `N` must fail (F-1 sensitivity), and
     `F`/`S`/`D` must pass 8/8.
  2. Deep-view A/B of develop against the head on vector perf, mf2-lod3, mf2-lod16, `pc-deep` and
     the mesh chunk case, retargeted on leaf geometry (R-9c), deterministic. Require:
     - the same target-path tile and the same non-null picks in every view;
     - with `cullWithChildrenBounds: false`, identical selections;
     - with defaults, differences only where the head selects an ancestor (R-14), with equal pixel
       coverage; the share of such views is reported.
  3. `tests/fixtures/implicit_levels.cjs` (PP) crosses implicit-root and document boundaries.
- **A-6, perf lane**, lane scripts extracted from `bb7d054` (R-10).
  - `process.py --base develop --head combined --repeat 3`, gate 1.25 with its floors. Every case
    is explained or fixed.
  - `cesium.py` (only the affinity line changed, if 0,1 are busy) in ABBA ×2 per case on one
    recorded CPU pair (topology recorded). The cases are:
    - the lane cases;
    - mf2-lod3 and `pc-deep`, which load implicit roots and documents.
  - Per-CPU busy is sampled around each invocation, siblings included. A run whose pair or
    siblings exceed 20% foreign busy is repeated.
  - The gate is `--baseline` against the first base run: requests (+2), `requestBytes` (+64 KiB),
    `secondsToLoaded` (+1 s), each at 1.25×. A flag is judged against the base-to-base spread of
    the same session.
- **A-7, repository gates.** As v1, plus CHANGELOG notes:
  - the reuse-identity bump rejects old caches with "run a fresh conversion" (`reuse.rs:285`);
  - default vector archives remain about 2× explicit ones (PD-11).

## 7. Proof limits and remaining gates

See [open-decisions.md](open-decisions.md), "Proof limits":
- SwiftShader timings;
- sampled views;
- one client;
- PD-7 wording;
- synthetic scale cases as size evidence only;
- C2 slice 3 owning decoded containment;
- PD-12 (archive-level caps) for the coordinator.

Nothing here claims release readiness.

## 8. Identities

| Item | Identity |
| --- | --- |
| Base | `origin/develop` `f2ed077eecf369375620c875b454d9df127c4b5f` |
| I1 | v3.1 `contract-draft.md` `9674e60b959eea0a4a985b6beec7a278ec79c08579f6c19e58f5a070aee03496` |
| Review | `review.md` `16205c1c554eb9ed058f1ccef1db998bae23af4266e9772c1a2ea43c34ca51ae` |
| Spec | `ImplicitTiling/README.adoc` `14a37a34…`; core `b841fade…`; Semantics `02ac07db…` |
| Cesium | `Implicit3DTileContent.js` `86109c4b…`, `Cesium3DTilesetTraversal.js` `6a2429b0…`, `Cesium3DTilesetBaseTraversal.js` `bcd98c47…`, `Cesium3DTileset.js` `04d08416…` |
| Producer binary | `/home/bend/.cache/c2-corpus/rusty-tiles-f2238cd` `369c3c81…` (the I1 receipt), used for the mf2 conversions (`evidence/v2/run_vector.sh`) |
| Inputs | Per-archive sha256 in each result file, and in `evidence/v2/inputs.json` |
| Scripts | sha256 in every result's receipt, and in `evidence/v2/SCRIPTS.sha256` |

## 9. Evidence index (v2)

| File | What it is |
| --- | --- |
| `evidence/v2/forest_model.py` | Forest and document model (measure and write modes, leaf-geometry views) |
| `evidence/v2/scale_runs.sh`, `scale/`, `scale_table.py` | §5.1 |
| `evidence/v2/run_vector.sh` | The mf2-lod3 and mf2-lod16 conversions (develop-equivalent binary, E-cores 17,19, `--jobs 2`) |
| `evidence/v2/write_forests.sh`, `today_map.py`, `implicit_today_map.py`, `explicit_map.py`, `parents_map.py` | Servable forests and URI→tile maps |
| `evidence/v2/cesium/` | Fixture generator and runner, `probe.cjs` (v2: separate overview SSE), `run_real.py`, results |
| `evidence/v2/deep_runs.sh`, `deep_runs_explicit.sh`, `compare_runs.py`, `target_path.py`, `deep/` | §5.2 deep views |
| `evidence/v2/deep_abba.sh`, `deep-abba/` | §5.3 timed implicit-root loading, ABBA ×2 |
| `evidence/v2/make_tables.py` | Renders every table in §5 from the result files |
| `evidence/v2/perf_abba.sh`, `cpustat.py`, `perf/` | §5.3 |
| `evidence/v2/today_layout.py`, `today-layout.json` | Today's layout on the deep vector cases |
| `evidence/v2/forests/` | Forest model summaries, plus the gzipped views, URI→tile maps and parent maps used by the deep runs. The servable forests and archives are scratch in `~/.cache/perf-lane/work/pimpl/v2/` (2.4 GB) and are regenerated by `run_vector.sh` and `write_forests.sh` |
| `evidence/v2/inputs.json`, `SCRIPTS.sha256` | Input and script identities |
| `evidence/superseded/` | v1's `real-vector-msse1-det.json` (R-9d) |
