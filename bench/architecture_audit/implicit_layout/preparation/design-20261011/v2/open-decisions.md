# P-IMPL open decisions (v2)

v1 and the coordinator's v1 resolution are in `v1/`. Line references are to `origin/develop`
`f2ed077`. Evidence paths are relative to this directory.

## Decided

| ID | Decision | By |
| --- | --- | --- |
| U-1 | The producers stop writing external-JSON content templates on implicit roots, now | User, 2026-10-11 |
| I1 v3.1 | JSON content templates are uniformly Unsupported (OD-1). OD-1b is withdrawn. P lands with I1 in one combined PR (RC-17). `convert-to-implicit` is retired (#159). I1 asks for rows on every tile and distinct subtree prefixes | Coordinator (I1 `contract-draft.md` `9674e60b…`) |
| PD-1 | The bounded forest (unchanged; PN confirmed it causally) | Coordinator v1 resolution, reaffirmed by the v2 direction |
| PD-3, PD-6, PD-8, PD-9 | As in v1. PD-6 now states literal digits (R-7) | Coordinator v1 resolution |
| PD-5 | **Reversed** (R-2): one 32-level cap for every producer, as a typed writer error | Coordinator v2 direction |

## PD-1: the layout (unchanged choice; wording corrected per R-9a)

The bounded forest is the only candidate that passes all of the following:
- every executed Cesium probe;
- every producer's hierarchy shape;
- the validator's per-document JSON limits, once the skeleton is split into documents (PD-2).

**Conformance.**
- The forest uses only core explicit tiles, explicit external tilesets, and single-subtree implicit
  roots with rows. No disputed reading is needed for the layout.
- **The rows themselves keep the PD-7 dispute.** Vector rows exceed their derived cells by up to
  4,094×, which is hard to square with "more specific" in IT `:341` and Semantics `:145`.
- The forest removes the client consequence (no client ever derives a cell) but not the dispute.

## PD-2: how the explicit skeleton is stored, and S (re-decided per R-1)

**Recommendation: explicit external tileset documents, with S = 4 for both schemes.**
- A document holds skeleton tiles breadth-first from its root.
- A skeleton child whose unit would overflow the document's budget becomes a link tile:
  - its box, error, refine and producer `transform`;
  - `content.uri` = `tileset-d{n}.json`, an archive-root member, so relative URIs are unchanged;
  - the tile itself is the root of that external document, without the transform.
- A *unit* is a skeleton tile plus its explicit-leaf and implicit-root children, plus the retained
  source names those roots add to the document.
- Budgets: 16,384 value nodes (¼ of `document_items`), 2 MiB of serialized bytes (¼ of
  `json_bytes`), 12 skeleton levels per document. JSON depth ≤ 32 (½ of `json_depth`) is asserted;
  the largest measured is 31.

**Options evaluated** (evidence/v2/scale/, model of the production writer; design §5.1):

| Option | Result |
| --- | --- |
| One `tileset.json` at S = 4 (v1) | Breaks 65,536 nodes from about 6.9k vector tiles, or about 3k with chains (PN §4); depth 64 near 33 levels |
| One `tileset.json` at S = 6 or 8 (quadtree) | Partial. Chains above S are skeleton for any affordable S: at 100k tiles with `lodLevels` 16 and full chains, 55,559 explicit tiles remain at S = 8 |
| **Documents, S = 4** | Every document of every case ≤ 16,340 nodes, ≤ 250 KB and JSON depth ≤ 31. Fewest docs-plus-subtree requests per deep view on real data. The smallest root document (182 KB for 12.5k vector tiles, against 916 KB today) |
| Documents, S = 6 | Also within limits. The root document is larger (mf2-lod3: 328 KB), because more tiles sit in roots under it and their retained names. Deep views: 615 against 640 requests at S = 4. No load-time difference beyond noise (§5.2) |
| Documents, S = 8 (QUADTREE) or 5 (OCTREE) | Within the node limits, but the vector root document reaches 809 KB, and octree subtrees reach 515 KB (a 5-level full octree), loaded up front per root |
| Joint I1 cap change | Not needed |
| A smaller validated domain (user decision) | Not needed for the JSON limits. See PD-12 for the archive-level caps |

**Settling evidence.**
- `evidence/v2/scale/*.json`: 13 cases × 3 values of S.
- `evidence/v2/deep/*`: Cesium deep views, S = 4 and S = 6.
- `evidence/v2/perf/*`: the perf-lane ABBA at S = 4.

## PD-4: Cesium `cullWithChildrenBounds` on explicit tiles (re-measured at scale per R-14)

Cesium computes `checkChildrenWithinParent` only for tiles read from tileset JSON
(`Cesium3DTileset.js:2440-2441`). That covers the skeleton and every explicit external document. It
also covers every tile of an `--explicit` output.

On the LOD-chain vector cases, `today` (implicit tiles) and the forest disagree in many views, but:
- only in off-target tiles where the forest selects a strict ancestor;
- the target leaf (or its parent, which has a micrometre error) is the same in every view;
- non-null picks are equal;
- pixel coverage is equal.

Design §5.2 has the counts, with and without `cullWithChildrenBounds` and against `--explicit`.

**Recommendation.** Accept it as explicit-tile Cesium behaviour, equal to `--explicit` output.
Document it as a viewer-selection difference, not a bounds or content difference.

**Alternative.** Raise S to shrink the skeleton. This only partly helps: chains above S stay
explicit.

## PD-5: depth bound (reversed per R-2)

- The writer refuses any hierarchy with more than 32 levels with a typed `Error::Data` naming the
  bound. This holds for every producer.
- That equals today's effective bounds:
  - vector, through the writer's `level >= 31` refusal (`implicit/tileset.rs:32`);
  - point cloud, through `depth_limit` 31 (`point_cloud/tiles.rs:151`).
- So no output that works today is refused.
- JSON depth is bounded per document by the 12-skeleton-level rule, independently of L.
- Measured L: real vector mf2-lod16 = 25; synthetic 100k with `lodLevels` 16 and full chains = 30.

## PD-6: identifiers (R-7)

`r{n}` means the letter `r` followed by the decimal digits of the root's preorder index in H, written
literally at write time. I1 `:393` rejects any brace other than `{level}`, `{x}`, `{y}` and `{z}`.
Examples:
- `implicit-content/r17-{level}-{x}-{y}-0.glb`
- `subtrees/r17-{level}-{x}-{y}.subtree`

External documents are `tileset-d{m}.json` (m ≥ 1, breadth-first order of creation), at the archive
root.

## PD-7: the standards status of rows larger than their derived cells (unchanged; a proof limit)

See PD-1. Not blocking.

## PD-10 (new): document budgets

The budgets are 16,384 nodes, 2 MiB and 12 skeleton levels, with JSON depth ≤ 32 asserted.
- Smaller budgets mean more link requests.
- Larger budgets mean bigger first loads and less margin.

**Recommendation.** Use the values above. On real data they give:
- vector perf in 1 document;
- the 12.5k-tile vector in 189 documents, against 2,634 today;
- a root document of 182 KB or less.

**Settling evidence.** The A-6 perf lane on the combined head. If `requestBytes` or
`secondsToLoaded` is flagged, halve the node budget and rerun.

## PD-11 (new): retained vector sources, (a) or (b) (R-3)

**What they are.**
- `--reuse-tileset` copies unchanged content from the previous archive by its immutable name,
  `t/<sha256>.glb|b3dm`.
- It finds those names through `vector-build.json` `explicitRoot` and checks each hash
  (`reuse.rs:132-235`, `:340-390`).
- In the default implicit output, the tiles reference rewritten copies. The raw `t/*` stay only so
  that the archive can be a reuse baseline.
- README `:201` and VECTOR.md `:260-274` document reuse from the default implicit archive:
  "Reuse needs the same tiling mode (`--explicit` or the implicit default)".

**Options.**
- **(a) Stop retaining them.**
  - The default vector archive roughly halves. For mf2-lod3 the forest would keep its 12,527
    payload members (1,227 skeleton `t/*` and 11,300 rewritten implicit payloads) and drop the
    11,300 retained raw copies (`evidence/v2/scale/vec-mf2-lod3-S4-4.json`).
  - `--reuse-tileset` then works only from `--explicit` baselines. A default-mode baseline must be
    refused with a typed "previous archive is not a reuse baseline; rebuild with --explicit or
    without reuse" error.
  - This is a **user-facing change** to a documented feature.
- **(b, recommended) Keep them, and list them per document.**
  - The document that holds an implicit root lists that root's rewritten raw sources in its
    `extras.rustyTilesSourceContents`. The list is bounded by the document budget, and the budget
    counts it.
  - I1-B removes the implicit-root gate at `validate.rs:1204` and honours the list on every tileset
    document (ownership plan "I1 asks" 1).
  - The archive stays about 2× an explicit one, which is no worse than today.
  - Skeleton tiles reference their raw `t/*` directly, so they are not duplicated.

**Recommendation.** (b) in this slice. (a) is a user decision; if chosen, it is a PV-only change
plus a docs change.

## PD-12 (new; for the coordinator): validated domain against the archive-level caps

**The caps.**
- `archive_entries` is 65,536 and `hierarchy_visits` is 65,536 (`validate/types.rs:88`, `:91`).
  Both are C1-owned and kept by I1.
- They bound validated archives independently of the JSON documents.

**Measured per tile:**

| Layout | Entries per tile | Visits per tile |
| --- | --- | --- |
| Today (mf2-lod3) | 2.42 (30,326) | about 1.6 |
| Forest | 2.01 (25,244) | 1.14 (14,321) |

**Consequence.** Vector archives above about 32k tiles (entries) are refused in every layout.
Vector `maxTiles` defaults to 100k, so a default vector conversion between about 32k and 100k tiles
publishes an archive that `validate` refuses. That is true today, and the forest does not change it.

**Options.**
- Raise the caps with a resource argument (C1/I1).
- Lower the default `maxTiles`.
- Accept and document it.

This is not a P decision.

## PD-13 (new; for I1): link tiles in `budgets.tiles`

See the ownership plan, "I1 asks" 4.
- **Recommendation:** exclude external-document link tiles from the `budgets.tiles` comparison.
- **Alternative:** P writes the link count into `conversion.json`.

## Proof limits this design does not close

- **Real-browser, GPU and network timing.** SwiftShader on 2 pinned CPUs shows behaviour only.
  Timings are within-session A/B on one CPU pair.
- **Camera paths.** Selection equality is shown on the sampled views only:
  - 16 per real case (vector perf, mf2-lod3, mf2-lod16, `pc-deep`);
  - 8 fixture leaves per layout;
  - plus the overviews.
- **Standards wording.** PD-7.
- **Other clients.** No client other than Cesium 1.146.0 was executed. The forest's feature set
  (explicit tiles, explicit external tilesets, single-subtree implicit roots with rows) is the
  smallest a client needs, but that is not evidence for other clients.
- **Synthetic scale cases.** The 100k vector and 87k–350k point cases are shaped from the real
  hierarchies (vector tile JSON objects are copied from real tiles), not converted from real data.
  They are evidence for document sizes, not for Cesium behaviour.
- **Exact baked translations.** As in v1, A-4 states the comparison exactly. C2 slice 3 owns
  decoded containment.
