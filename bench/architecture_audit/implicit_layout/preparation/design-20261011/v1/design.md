# P-IMPL: implicit layout without external-JSON content templates

Status: **design for a fresh nonauthor precode review**. No production code changes in this design,
and no Cargo command was run. Base: `origin/develop` `f2ed077`, whose tree is identical to
`/tmp/rusty-tiles-c2` `3563d53`. Every `src/` line reference is to that tree.

P lands together with the I1 validator migration in one combined integration PR (I1 v3, RC-17).
Files and roles are in [ownership-plan.md](ownership-plan.md); choices still open are in
[open-decisions.md](open-decisions.md); scripts and raw results are in [evidence/](evidence/).

The user decided on 2026-10-11 to change the producers now. No point-cloud, vector or legacy
`mesh-to-3tz` output may put an external-JSON content template on an implicit root. The
conditions are:
- refinement and picking stay correct in Cesium;
- bounds stay honest, including oversized padded boxes;
- load performance does not regress.

## 1. Recommendation in one paragraph

Replace both of today's layouts with a **bounded implicit forest**. Let S = 4.
- **Skeleton tiles.** Every producer tile whose subtree has height ≥ S stays an ordinary explicit
  tile in `tileset.json`.
- **Implicit roots.** Every non-leaf child of a skeleton tile with height < S becomes an implicit
  root. So does the root itself when the whole tree has height < S.
- **One subtree per implicit root.** Each implicit root has exactly one subtree:
  - `subtreeLevels == availableLevels == height + 1 ≤ S`;
  - `childSubtreeAvailability` is `{constant: 0}`;
  - every available tile has a TILE_BOUNDING_BOX and TILE_GEOMETRIC_ERROR row.
- **No placeholders.** Child subtrees never exist, so no client ever creates a placeholder whose
  volume would have to be derived from the regular grid. That derived volume is exactly what makes
  Cesium 1.146 cull content today.
- **No external documents.** No external tileset JSON is written, so the I1 JSON-template question
  disappears for producers.

Executed Cesium 1.146 evidence:
- On hand-made fixtures, this layout loads, selects and picks every leaf at close range.
- The single-tree child-subtree alternative fails all 8 leaf views.
- On real producer data (vector perf, pc-deep), the forest produces the same selections and picks
  as today's layout, with no JSON requests and fewer total requests (§5).

## 2. Why external roots exist today (settled from source, history and execution)

**The writer and the commit that introduced it.**
- The writer chooses between two layouts (`implicit/tileset.rs:284-308`). The comment says:
  "Metadata may enlarge a tile beyond its regular cell. Cesium must cull an unloaded child subtree
  using that cell, including during a narrow pick. External tileset roots expose the actual
  boundary box before that cull."
- Whenever any tile at a level that is a positive multiple of `LEVELS = 4` has a row escaping its
  derived cell (by `Obb::contains`, tolerance 1e-6), the writer uses `write_chunks` (`:473-602`).
- This came in with `ba4a195` "Enable default implicit tiling for native converters". CHANGELOG
  `:28` says: "Bounds that exceed regular subtree cells use external implicit tileset roots so
  Cesium can refine and pick them".
- No executed Cesium failure was retained. The rationale was source analysis, and the browser
  fixture `tests/fixtures/implicit_levels.cjs` checks only that levels are reached.

**A second constraint: today's validator.**
- It refuses an implicit root on any non-root explicit tile ("nested implicit roots require an
  external tileset JSON", `validate.rs:1280-1284`).
- It expands only a document-root implicit root (`implicit/tileset.rs:673`).
- So external JSON was also the only layout today's validator accepted for multiple roots. I1
  removes this constraint (I1 X-8, C-8).

**The Cesium behaviour, verified in 1.146.0 source** (engine 26.4.0, sha256 in §8):
- `Implicit3DTileContent.js:296-338` (`deriveImplicitChildren`). At the bottom level of a subtree,
  each available child subtree becomes a placeholder tile made by `makePlaceholderChildSubtree`
  (`:1021-1068`). Its bounding volume is `deriveBoundingVolume(...)`, which is the **derived
  cell**, because the child subtree's metadata is not loaded yet. Its geometric error is also
  derived ("Ignore tile metadata … since the child subtree's metadata hasn't been loaded yet").
- Tiles inside a loaded subtree use the TILE_BOUNDING_BOX row when present (`deriveChildTile` →
  `getTileBoundingVolume`, `:638-669`). Derived cells are used only when no row exists, and for
  placeholders.
- `Cesium3DTilesetTraversal.js:206-215` (`updateTileVisibility`). A placeholder is first culled by
  its own (derived) volume. Only a visible placeholder recurses into its loaded root.
- `Cesium3DTilesetTraversal.canTraverse` (`:56-66`) always traverses visible placeholders.
- So the subtree under a placeholder is requested and rendered only when the **derived cell**
  intersects the view or pick frustum. Content of an oversized subtree root that lies outside its
  cell is invisible and unpickable whenever the cell itself is out of view.
- **Executed.** Fixture `A` (§5.1) reproduces this. All 8 leaf views fail. The single control view
  inside the derived cell passes. Fixture `S` (identical rows, no child subtrees) passes all 8.

**Which outputs actually escape** (`evidence/samples-layout.json`, independent stdlib expander):

| Output | Tiles / levels | Rows outside the derived cell | Child-subtree roots unsafe under one tree | Largest escape |
| --- | --- | --- | --- | --- |
| `pc-perf-2000` | 428 / 5 | 0 | 0 of 233 | — |
| `pc-deep` | 2,233 / 5 | 2 per level 2–4 | 2 of 1,648 | 5.5 µm (3.5e-7 of the cell) |
| `vec-perf` = perf-lane `vector` | 547 / 12 | every non-root row | 272 of 272 | 13.4 km (4,094× the cell) |
| perf `point-cloud`, `mesh` | 9 / 2 | 0 | 0 | — |

- **Point clouds** partition on the midpoint octree of the tight AABB (`point_cloud/tiles.rs:160-205`).
  Rows escape only where f32-rounding padding (`:140`) crosses an internal cell face near a
  point.
- **Vector** builds a binary median k-d tree with LOD chains (`store.rs:586-751`). It is addressed
  by child ordinal on the x bit (`implicit/tileset.rs:113-124`, slot at `:119-121`), so derived cells are slivers
  unrelated to content. This is the finite profile already adopted in
  `docs/architecture/implicit-rewrite-contract.md:30-47`: rows are authoritative, and coordinates
  are addresses.

So the external roots exist to keep Cesium from culling placeholders by their derived cells. For
vector this is decisive; for point clouds it is micrometric.

## 3. Candidates, judged

The primary texts are pinned at `4d781014`, with the hashes in `probes/receipt.json` of the I1
directory. "IT" below means `ImplicitTiling/README.adoc`, and "core" means
`specification/README.adoc`.

| Candidate | Spec conformance | Cesium 1.146 (executed) | Bounds honesty | Requests (real data) | Implementation |
| --- | --- | --- | --- | --- | --- |
| **(a)** One implicit tree, child subtrees only | Conformant (IT `:243`, `:314`) | **Fails** whenever a child-subtree root's row leaves its derived cell. Fixture `A`: **0 of 8** close leaf views select or pick the leaf. 4 views show the coarse level-1 tile; 4 show a hole (contentless routing ancestor). The control view inside the derived cell passes (§5.1) | Honest rows, but Cesium ignores them at placeholders | `pc-deep`: 1,649 subtrees (one per level-4 tile); vector: 273 | Smallest diff (delete the chunk branch), but wrong for vector (and pc-deep in µm) |
| **(b)** Pad the grid so cells contain the content | Conformant | Would pass where containment holds | Only point clouds can approach it, and the f32 rounding pad still crosses internal faces. A guarantee needs directed rounding (a payload encoding change). Impossible for vector (whole features cross any grid) and for mesh partitions | as (a) | Encoding change per producer; vector repartitioning. Rejected for this slice |
| **(c)+(d)** **Bounded forest**: explicit skeleton, single-subtree implicit roots (recommended) | Conformant without any disputed reading: implicit roots on any tile (IT `:34`, `:43`); root constraints (IT `:94-97`) all met; overrides (IT `:175`); child subtrees absent (IT `:314`, "the tileset does not subdivide further"); templates carry level/x/y[/z] (IT `:231`) | **Passes**. Fixtures `F` and `S`: 8 of 8 leaves selected and picked, no holes. Real data matches today's selection and picks (§5.2) | Rows are the producer's boxes, used verbatim. No client derives a cell | vector: 1 JSON + 45 subtrees in total (today 85 + 85). `pc-deep`: 1 + 8 (today 208 + 208) | One writer replaces two layouts; payload rewriting shrinks to tiles below implicit roots |
| **(d) alone** Several implicit roots under an explicit root, each with child subtrees | Conformant | Same failure as (a) below level S | as (a) | as (a) | No benefit |
| **(e1)** One subtree for the whole tree | Conformant | Passes (fixture `S`) | Honest | Dense availability: vector depth 12 needs 5.6 Mbit per stream (≈700 KB ×2) loaded up front | This is the forest's own case when height < S |
| **(e2)** Explicit external tilesets to chunk a large skeleton | Conformant (core `:731-741`) | Not needed | Honest | +1 JSON per chunk | Only if `tileset.json` growth is ever flagged (PD-2) |
| **(e3)** `--explicit` by default | Conformant | Passes (fixture `E`) | Honest | vector `tileset.json` 360 KB against 41 KB today; overview 2.67 s against 2.16 s (perf baseline) | A product default change; point `--explicit` uses a different (binary) hierarchy |
| **(e4)** Tight volumes on placeholders via tile `metadata` | Forbidden on implicit roots (IT `:95`); placeholders have no JSON | — | — | — | Not possible |

**Why not (a) "always"**, when `pc-perf-2000` already does it?
- Its rows happen to lie strictly inside every cell, but nothing in the point partition guarantees
  that (`pc-deep` escapes).
- (a) is wrong by kilometres for vector.
- It also puts each sparse level-4 tile in its own subtree file: 233 of 234 subtrees in
  `pc-perf-2000` hold one tile.

## 4. Contract

### 4.1 Representation

The input is a producer's finished explicit hierarchy `H`, as the producers already build it. Each
node `n` has:
- a box `B(n)` in its own frame;
- error `e(n)`;
- refine;
- 0..k payload contents (`.glb`, `.gltf`, `.b3dm`);
- a child slot (`extras.implicitChildIndex`, or the array index; `implicit/tileset.rs:113-124`);
- children with pure-translation transforms.

Definitions:
- `h(n)` = 0 for a leaf, otherwise 1 + max over the children.
- `S` = `MAX_IMPLICIT_LEVELS` = 4. This is one profile constant, replacing `LEVELS` (PD-2).

Classification, decided once in the writer:
- If `h(root) < S`, the root is the single implicit root.
- Otherwise a tile with `h ≥ S` is **skeleton**. A skeleton tile's child `c` is:
  - an **explicit leaf** if `h(c) = 0` (PD-3);
  - an **implicit root** if `1 ≤ h(c) < S`;
  - **skeleton** if `h(c) ≥ S`.
- Every tile below an implicit root is an **implicit tile** of that root.

Output (the writer's manifest):
- **Skeleton tile or explicit leaf.** The producer's tile is emitted unchanged:
  - box, error, refine and extras;
  - its translation `transform`;
  - its content URI and bytes, which are not rewritten.
- **Implicit root `r` with id `n`**, its preorder index (PD-6). The explicit tile carries:
  - the producer's `transform`, `boundingVolume.box = B(r)`, `geometricError = e(r)` and `refine`;
  - `implicitTiling = {scheme, subtreeLevels: h(r)+1, availableLevels: h(r)+1, subtrees: {uri: "subtrees/r{n}-{level}-{x}-{y}[-{z}].subtree"}}`;
  - one payload template per content slot, `implicit-content/r{n}-{level}-{x}-{y}[-{z}]-{slot}.{ext}`.
    The producer's content header is kept (vector `3DTILES_content_gltf_vector`). Templates are
    omitted entirely when no tile of `r` has content (IT `:299`).
- **The subtree of `r`.** Exactly one file, at local `(0,0,0[,0])`:
  - tile and content availability, with LSB-first Morton order and constant streams when uniform,
    as `implicit.rs` writes today;
  - `childSubtreeAvailability: {constant: 0}`;
  - a `rustyTile` property table with one row per available tile:
    - `boundingBox` = `B(t)` translated by `δ(t)`, the sum of the translations from `r` down to `t`;
    - `geometricError = e(t)`;
    - `extras`, with `encodedBytes` updated to the written bytes as today.
- **Implicit tile payloads.** Rewritten with `δ(t)` baked into the GLB scene roots
  (`translate_content`, kept). `δ(r) = 0`, so the root's own payload is a byte copy.
- **`schema`.** Present in `tileset.json` iff at least one implicit root exists.
  `rustyTilesSourceContents` is kept for vector reuse.

### 4.2 Invariants (each is checked by the acceptance oracle, §6)

- **P1, no JSON.** The archive's only tileset JSON document is `tileset.json`. No content template,
  and no content URI anywhere, ends in `.json` (I1 OD-1).
- **P2, no placeholders.** Every implicit root has `subtreeLevels == availableLevels ≤ S`, exactly
  one subtree member, and `childSubtreeAvailability == {constant: 0}`. Therefore no subtree
  references another subtree, and no conforming client needs a derived cell for any culling
  decision.
- **P3, rows everywhere** (I1 OD-3).
  - Every available implicit tile has a row.
  - Its TILE_BOUNDING_BOX equals, as f64 bits, the producer's `B(t) + δ(t)` in `r`'s frame, and
    its TILE_GEOMETRIC_ERROR equals `e(t)`.
  - The level-0 row equals the implicit root's `boundingVolume.box` and `geometricError`.
- **P4, distinct prefixes.** Subtree and content names of different roots are distinct (I1 OD-7).
  Every name the templates produce for an available tile exists. No written member is
  unreferenced.
- **P5, effective-tree equality.** Expanding the output (explicit tiles plus every implicit root)
  yields exactly `H`:
  - the same tiles and parent/child relation (child order by slot);
  - world-frame boxes equal up to the documented translation recomposition;
  - identical errors, refine and extras;
  - payloads equal to the producer's bytes, apart from scene-root translations, for implicit tiles.
- **P6, coherence preserved.** The writer adds no box and shrinks none. The producer's enclosure
  (`rowsNotEnclosingChildRows = 0` in every probe) carries over unchanged.
- **P7, determinism.** For a given `H`, the output bytes are a function of `H` alone: preorder ids,
  sorted slots, and the subtree encoding of `implicit.rs`.

### 4.3 Supported domain

- **Producers:**
  - `point-cloud` (OCTREE);
  - `vector`, including `--quantize` and `--meshopt` (QUADTREE);
  - legacy `mesh-to-3tz` (OCTREE);
  - the small-model wrapper `implicit_glb_job` (one tile, ADD, `.gltf` plus sibling resources:
    one single-level implicit root, unchanged in meaning).
- **Hierarchy:**
  - at most 8 (octree) or 4 (quadtree) children, with distinct slots;
  - translation-only child transforms;
  - box volumes;
  - payload suffixes glb, gltf or b3dm;
  - any number of content slots;
  - any depth up to the producers' own limits.
  The 31-level global-coordinate limit (`implicit/tileset.rs:32`) disappears, because coordinates
  are local and below S (PD-5).
- **Not in the domain:**
  - `convert-to-implicit`, retired by #159;
  - third-party implicit input (I1's concern);
  - regions and spheres;
  - child subtrees;
  - external buffers;
  - JSON subtrees.

### 4.4 Failure semantics

Each of the following is a typed `Error::Data` from the writer, raised before any member is
published. Publication remains the job's (F0) staging and atomic completion.
- A duplicate child slot or a slot ≥ the branching factor (today's messages, kept).
- A non-translation child transform at or below an implicit root.
  - Skeleton tiles also keep the translation-only rule: producers emit only translations, and
    relaxing it is a separate decision.
- An unsupported content suffix.
- A missing or nonfinite box or error.
- A level-0 row that does not equal the root box. This is an internal assertion, unreachable by
  construction.
- `translate_content` failures (truncated b3dm, GLB without scenes), as today.
- Checkpoint and cancellation, propagated unchanged: the writer calls `checkpoint()` per tile and
  per member, as today.

There is no data-dependent layout choice, no fallback, and no tolerance constant. The `Obb::contains`
1e-6 test and `derived_box` leave the writer.

### 4.5 What changes in each producer

| Producer | Change | Retained | Removed |
| --- | --- | --- | --- |
| Shared writer `implicit/tileset.rs` | `write_forest` (ownership plan) replaces `write_tileset`, `write_tileset_recorded`, `write_chunks`, `chunk_document` and the escape test (`:284-308`). It returns `ForestOutput` (members, per-tile written stats). `expand_tileset` walks explicit children and expands each implicit root; it no longer follows JSON templates (PD-8) | `collect` (reworked to keep children and compute heights), `translate_content`, `Subtree` encoding (`implicit.rs`), the schema, `content_suffix` | Chunking, `derived_box` in the writer, `LEVELS`, the global 31-level limit, the `implicit-tileset-*` naming |
| Point cloud | Call `write_forest(..., SourcePolicy::Remove)`; `tree.members = output.members`, which now includes skeleton `t/{i}.glb`; encoder `rusty-tiles-native-las-implicit-v3`; depth limit 64 (PD-5) | The partitioning, sampling and boxes | Only sources rewritten into `implicit-content/` are deleted |
| Vector | Call `write_forest(..., SourcePolicy::Retain)`; compute `maximumTileBytes` and the sums from `ForestOutput.tiles`, not from `expand_tileset` (`store.rs:1003-1013`); reuse tiling identity `implicit-forest-v3` (`reuse.rs:269`) | Reuse, the `t/*` retention, report fields | The display re-read through `expand_tileset` |
| Legacy mesh `tile.rs:510-540` | Stage from `ForestOutput.members` instead of the hard-coded `implicit-content`, `subtrees` and `implicit-tileset-*` names; encoder `-implicit-v3` | Mesh hierarchy | The `implicit-tileset-` staging loop |
| Small-model `tileset.rs:153-177` | Only the encoder string. The `.gltf` resources stay in `implicit-content/` because the ids are name prefixes (PD-6) | Everything else | — |

The deterministic output digests and the "`--explicit` preserves the earlier output bytes" note in
CHANGELOG `:28` are not compatibility obligations (AGENTS.md). The CHANGELOG line is rewritten
by R.

## 5. Executed evidence

Everything is under `evidence/`, run with `taskset -c 0,1 nice -n 10`, one Chromium at a time, in
Cesium 1.146.0, with Chromium 153.0.8010.12 on SwiftShader.

### 5.1 Hand-made fixtures (`evidence/cesium/`: `make_fixtures.py`, `run.py`, `probe.cjs`, `results.json`)

**The hierarchy.** One logical hierarchy of 15 tiles over 4 levels, a binary k-d split with x-bit
child addressing like vector. Every non-root row escapes its derived cell. Tile `(1,1)` is a
contentless routing tile. It is written in five layouts:
- `E`, explicit;
- `A`, one tree with `subtreeLevels 2` and child subtrees at level 2;
- `S`, one subtree with 4 levels;
- `X`, today's chunked external-JSON form;
- `F`, the forest with S = 2, whose implicit roots carry their own translation `transform`.

**The views.** One close top-down view per leaf (8). Each view targets the point of the leaf that
is farthest from the derived cell of the leaf's level-2 ancestor, 421–1,693 m away. There is one
control view inside that cell. maximumScreenSpaceError is 16.

| Layout | Leaf selected | Leaf picked at the centre | Holes (coverage < 0.5) | Control | Requests to load everything (JSON / subtree / GLB) |
| --- | --- | --- | --- | --- | --- |
| `E` explicit | 8/8 | 8/8 | 0 | pass | 1 / 0 / 14 |
| **`A` child subtrees** | **0/8** | **0/8** | **4** | **pass** | 1 / 5 / 8 |
| `S` single subtree | 8/8 | 8/8 | 0 | pass | 1 / 1 / 14 |
| `X` today | 8/8 | 8/8 | 0 | pass | 5 / 5 / 14 |
| **`F` forest** | **8/8** | **8/8** | **0** | pass | **1 / 4 / 14** |

- **`A`'s failure is isolated to the placeholder cull.** `A` and `S` carry identical rows and
  payloads, and differ only in subtree partitioning. The `A` control view passes exactly where the
  derived cell is in view.
- **What `A` shows instead.** Under a content parent it shows the level-1 coarse tile, and picks
  return that tile. Under the routing parent it shows nothing, and picks return nothing.

### 5.2 Real producer data, today against the forest model (`relayout_forest.py`, `cesium/run_real.py`)

**The model.** `relayout_forest.py` rewrites today's archive into the forest layout. The tiles,
rows, errors and payload bytes are kept exactly. Because today's payloads are already baked into the
top-root frame, the model needs no intermediate transforms. This is a layout model, not the
production writer.

**The probe.** It reads selected and picked content and maps each URI back to its global tile key.
- 12 close leaf views, plus the overview.
- maximumScreenSpaceError 1, to force deep traversal.
- Deterministic mode: no foveated or dynamic SSE, no progressive resolution, no request culling
  while moving; 30 stable frames.

| Data | Same overview selection | Same view selection | Same pick | Requests (JSON / subtree / GLB) |
| --- | --- | --- | --- | --- |
| vector perf, S = 4 | yes | 10/12; **12/12 with `cullWithChildrenBounds: false`** | 12/12 | today 17 / 17 / 50; forest **1 / 13 / 50** |
| `pc-deep`, S = 4 | today did not settle in 90 s (959 requests); forest settled in 69.5 s with all 1,953 leaves | 12/12 | 12/12 | today 188 / 188 / 1,747; forest **1 / 8 / 2,233** (all loaded at the overview) |

**The 2 vector views that differ.** Both select ancestor `5-23-0` in the forest and `6-47-0` today,
with identical pixel coverage. The cause is Cesium's child-bounds cull:
- `checkChildrenWithinParent` is computed only for tiles read from tileset JSON
  (`Cesium3DTileset.js:2440-2441`), so it now applies to skeleton tiles.
- Disabling it makes all 12 views identical.
- `--explicit` output behaves the same way today (PD-4).

### 5.3 Perf-lane Cesium gate at `bb7d054` (`evidence/perf_lane_bb7d054/`, `evidence/perf-cesium-*.json`)

**The cases.** Base is today's archives. Head is the forest model of the same archives, with
payload bytes unchanged. The gate is the lane's own `--baseline` rule (1.25×, with floors). The
order was base, then forest S = 4, then forest S = 6, then base again.

**A deviation from the requested `taskset -c 0,1`.** The pinned `cesium.py` hard-codes CPUs 0 and 1.
- The first run there (`r1-contended-*.json`) shared CPUs 0 and 1 with another agent's Chromium,
  at host load 15.
  - Base `pc-deep` timed out at 120 s.
  - The forest `pc-deep` page did not even load in 30 s.
  - Forest vector took 44.6 s, against 4.5 s for base, with the same 5–6 requests and a
    byte-identical screenshot.
  - That run measured contention, not the layout.
- The authoritative run (`perf-cesium-r2-*.json`) used `cesium_cpus.py`. It is the pinned script
  with only the affinity line changed: CPUs 13 and 15 (two CPUs, nice 10, idle). The diff is shown
  by `diff cesium.py cesium_cpus.py`.

| Case | Layout | Loaded at (s) | Requests (JSON / subtree / content) | Request bytes | Gate | Screenshot sha256 |
| --- | --- | --- | --- | --- | --- | --- |
| vector | today (base 1 / base 2) | 2.02 / 2.57 | 6 (1 / 1 / 4) | 1,563,790 | — | `28db29fc…` |
| vector | forest S = 4 | **1.93** | **5** (1 / 0 / 4) | 1,595,269 (+2.0%) | pass | `28db29fc…` (identical) |
| vector | forest S = 6 | 2.09 | 5 (1 / 0 / 4) | 1,573,294 (+0.6%) | pass | `28db29fc…` (identical) |
| pc-deep | today (base 1 / base 2) | 15.65 / 16.95 | 2,649 (208 / 208 / 2,233) | 12,892,981 | — | `15f7a2b7…` |
| pc-deep | forest S = 4 | **16.02** | **2,242** (1 / 8 / 2,233) | 12,530,286 (−2.8%) | pass | `15f7a2b7…` (identical) |

- At the default framing, both layouts render byte-identical canvases.
- The forest never makes more requests than today, and needs no external JSON.
- The vector `tileset.json` grows from 41 KB to 78 KB at S = 4 (56 KB at S = 6). That is 2% of the
  request bytes at the overview: below the gate, and paid once.
- Load times are within the base-to-base spread: 2.02–2.57 s for vector and 15.65–16.95 s for
  pc-deep.

This is model evidence. A-6 repeats it on the production binaries.

## 6. Acceptance plan

Execution identities are bound per AGENTS.md:
- exact source heads;
- binary sha256;
- archive digests;
- script hashes.

Static observations, executed defects, profile choices and proof limits are recorded separately.

- **A-1, writer unit tests** (`tests/implicit_forest.rs`, PW). Synthetic hierarchies:
  - `h < S`, giving one root;
  - `h = S`;
  - `h = 2S + 1`;
  - quadtree slot gaps;
  - octree slot 7 only;
  - routing tiles inside and at implicit roots;
  - two content slots;
  - an implicit root with no content (templates omitted);
  - `.gltf` with sibling resources (M2);
  - b3dm.

  For each, assert P1–P4 and P7 directly on the bytes, and P5 against `H` by an in-test expander.
  Negative cases cover each §4.4 error. **A 40-level chain** (PD-5) must succeed.
- **A-2, independent forest checker** (PO, `bench/architecture_audit/implicit_layout/`).
  - It is stdlib Python, imports nothing from `src/`, and is frozen before PO sees the writer diff.
  - It is derived from `evidence/layout_probe.py`, extended to expand implicit roots on explicit
    tiles.
  - It asserts P1–P6 on every W11 archive.
  - Sensitive controls, each of which must be rejected:
    - a JSON template;
    - a child-subtree bit;
    - a missing row;
    - a row differing by 1 ulp;
    - a repeated prefix;
    - an unreferenced member.
- **A-3, the I1 reader as the validator** (W11, merge gate). `rusty-tiles validate` on the
  combined head must return Accepted, with production equal to I1's oracle O, for:
  - point default on the C2-corpus `cloud.las` with `--maxPoints 40` (pc-deep) and with
    `--maxPoints 2000` on the perf `points.las`;
  - perf-lane `point-cloud`;
  - vector perf default, `--quantize` and `--meshopt`;
  - legacy `mesh-to-3tz`, both default and `--maxTriangles 2` (the `native_implicit.rs` chunk
    case);
  - the small-model wrapper.

  `notInspected` must not name JSON templates. No Unsupported is allowed.
- **A-4, the differential on the same inputs.** Run develop (`f2ed077`) against the combined head.
  The A-2 expansion of both outputs must give:
  - the same tile set, relation, errors, refine and extras;
  - rows equal in world frame after composing translations. The exact comparison is stated in the
    record: box half-axes bitwise equal; centres compared after recomposition, with the f64 sum
    order documented;
  - payload bytes equal outside the GLB scene-root translation.
- **A-5, Cesium behaviour** (PO, ported from `evidence/cesium/`).
  1. Rerun the five-layout fixture suite as a standing sensitivity check. `A` must keep failing,
     and `F`/`S` must pass.
  2. Run the deep-view A/B (`run_real.py`) of develop against the head on vector perf, pc-deep
     and the mesh chunk case, in deterministic mode. Require:
     - identical selection and picks with `cullWithChildrenBounds: false`;
     - with defaults, only the PD-4 kind of difference, with equal pixel coverage.
  3. Update `tests/fixtures/implicit_levels.cjs` so it crosses implicit-root boundaries instead
     of external-root ones.
- **A-6, the perf lane at `bb7d054`**, gate 25%. These run on R's builds, serially.
  - `process.py --base develop --head combined --repeat 3`. The lane flags a case when head/base
    exceeds 1.25 and the absolute growth exceeds a floor: wall 0.1 s, RSS 16 MiB, output bytes 0.
    Expectations:
    - vector output bytes fall (skeleton tiles are no longer copied);
    - point and mesh outputs are equal or a little smaller (fewer subtrees);
    - the perf point and mesh cases (2 levels) stay one implicit root, with only member names
      changing.
    Any flag needs `--explain` with a measured reason, or a fix.
  - `cesium.py` on every lane case with `--baseline` develop. The gate is requests (+2 floor),
    `requestBytes` (+64 KiB) and `secondsToLoaded` (+1 s), all at 1.25×.
  - Explained extra cases on the same harness:
    - `pc-deep`, because the lane corpus has no deep point case;
    - the vector `--quantize` and `--meshopt` outputs.
  - §5.3 gives the expected result from the model.
  - Before every timed run, check that the pinned CPUs are free of foreign load (README
    `foreignCpuSeconds`). If they are not, use another idle pair and record that, as §5.3 had to.
- **A-7, repository gates.** Existing tests are reworked by their owners: `native_implicit.rs`,
  `native_point_cloud.rs`, `fidelity.rs`, `mesh_tile.rs`, `mesh_crs.rs`, and the
  `p1_point_oracle.py` walker. Then CI, the installed wheel and official Blender checks. P's
  nonauthor review (PN) inspects the final source and this evidence.

A green existing suite does not establish P1–P7. A-2, A-3 and A-5 do.

## 7. Proof limits and remaining gates

See [open-decisions.md](open-decisions.md), "Proof limits". In brief:
- SwiftShader timings only;
- sampled views only;
- no client other than Cesium 1.146;
- the wording of "more specific" for oversized rows (PD-7) stays a standards proof limit, now
  without any client consequence;
- C2 slice 3 still owns decoded containment.

Nothing here claims release readiness.

## 8. Identities

| Item | Identity |
| --- | --- |
| Base | `origin/develop` `f2ed077eecf369375620c875b454d9df127c4b5f` |
| Spec `ImplicitTiling/README.adoc` | `14a37a34…`; core `README.adoc` `b841fade…`; `Semantics/README.adoc` `02ac07db…` |
| Cesium `Implicit3DTileContent.js` | `86109c4b82bc0ab209d82df1eb0d6e72e362c3d348fb212b1d0963388f620311` |
| `Cesium3DTilesetTraversal.js` | `6a2429b04d8782071981615bf938f6cb3163fca545b39eff954c1a40cc575b0e` |
| `Cesium3DTilesetBaseTraversal.js` | `bcd98c4737587756b717f99cb26d0315cab2b7766272b8a15ef37b8b5a57d154` |
| `Cesium3DTileset.js` | `04d08416e3ad721f6fea557853eec861f06eee299a845d61657dec1cbfacb48d` |
| Inputs | `pc-deep.3tz` `162db1e4…` (I1 samples). The perf-lane baseline `vector.3tz` was regenerated by another lane run during this work: `ac0c0cf9…` was the forest-model input, `7c619ed7…` the base Cesium runs, and `30015971…` the final probe. Every member is byte-identical except the volatile `conversion.json`, which Cesium never fetches; `tileset.json` is `fcf00e32…` in all three. Per-archive sha256 values are in `evidence/*-layout.json` and the perf JSON |
| Scripts | sha256 recorded inside each results file (`receipt`) |

## 9. Evidence index and replay

Python is stdlib only, run as `python3 -I`. Browsers ran one at a time. Large scratch output is in
`~/.cache/perf-lane/work/pimpl/`, which is not a deliverable.

| File | What it is |
| --- | --- |
| `evidence/layout_probe.py` | Independent expander for today's implicit archives (both layouts). Reports row-against-derived-cell escapes, enclosure, and the costs of (a) and of the forest as a function of S |
| `evidence/samples-layout.json` | The probe on the I1 samples `pc-perf-2000`, `pc-deep` and `vec-perf` |
| `evidence/perf-baseline-layout.json` | The probe on the perf-lane baseline `point-cloud`, `vector` and `mesh` |
| `evidence/relayout_forest.py`, `relayout-summary.jsonl` | Forest model of real archives (S = 4 and S = 6), plus the leaf views and the URI→tile-key maps |
| `evidence/cesium/make_fixtures.py`, `run.py`, `probe.cjs`, `results.json` | §5.1 fixtures (`E`, `A`, `S`, `X`, `F`) and their results; the probe sha256 `32a85eda…` is in the receipt. `fx/` holds the generated fixtures |
| `evidence/cesium/run_real.py`, `real-*.json` | §5.2 deep-view A/B. Authoritative: `real-vector-msse1-det.json` (defaults), `real-vector-msse1-det-nochildbounds.json`, `real-pc-deep-msse1-det.json`. Preliminary, non-deterministic, kept for transparency: `real-vector.json` (SSE 16; the views never left the top levels) and `real-vector-msse1.json` |
| `evidence/perf_lane_bb7d054/` | Lane scripts extracted from `bb7d054`, plus `cesium_cpus.py` (affinity override only) |
| `evidence/perf-cesium-r2-*.json` | §5.3 authoritative gate runs. The `r1-contended-*.json` files are the contended CPU 0/1 run, which is not authoritative |

Replay (from `evidence/`):

```sh
python3 -I layout_probe.py samples-layout.json $I1/samples/{pc-perf-2000,pc-deep,vec-perf}.3tz
python3 -I cesium/make_fixtures.py cesium/fx
taskset -c 0,1 nice -n 10 python3 -I cesium/run.py cesium/fx cesium/results.json
python3 -I relayout_forest.py IN.3tz OUT_DIR 4 12
PIMPL_MSSE=1 PIMPL_DETERMINISTIC=1 PIMPL_STABLE=30 [PIMPL_CHILD_BOUNDS=0] \
  taskset -c 0,1 nice -n 10 python3 -I cesium/run_real.py OUT_DIR-views.json OUT.json \
  today=UNPACKED_TODAY:today forest4=OUT_DIR:forest
PIMPL_CPUS=13,15 nice -n 10 python3 -I perf_lane_bb7d054/cesium_cpus.py --timeout 300 \
  --work W --out OUT.json [--baseline BASE.json] vector=... pc-deep=...
```
