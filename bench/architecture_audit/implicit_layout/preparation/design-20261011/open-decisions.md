# P-IMPL open decisions (v3)

v1 (with the coordinator's v1 resolution) and v2 are in `v1/` and `v2/`. v3 records the user's decisions on PD-11 to PD-13. Line references are to `origin/develop`
`f2ed077`. Evidence paths are relative to this directory.

## Decided

| ID | Decision | By |
| --- | --- | --- |
| U-1 | The producers stop writing external-JSON content templates on implicit roots, now | User, 2026-10-11 |
| I1 v3.1 | JSON content templates are uniformly Unsupported (OD-1). OD-1b is withdrawn. P lands with I1 in one combined PR (RC-17). `convert-to-implicit` is retired (#159). I1 asks for rows on every tile and distinct subtree prefixes | Coordinator (I1 `contract-draft.md` `9674e60b…`) |
| PD-1 | The bounded forest (unchanged; PN confirmed it causally) | Coordinator v1 resolution, reaffirmed by the v2 direction |
| PD-3, PD-6, PD-8, PD-9 | As in v1. PD-6 now states literal digits (R-7) | Coordinator v1 resolution |
| PD-5 | **Reversed** (R-2): one 32-level cap for every producer, as a typed writer error | Coordinator v2 direction |
| PD-11 | **No retained sources.** Reuse reads the archive's published payloads (design §4.7). Old archives that carry the list keep validating through I1's unchanged gate | User, 2026-10-11 |
| PD-12 | The validator's entry and visit caps are raised by a separate validator slice owned by the coordinator, so that default vector `maxTiles` 100k archives validate. P supplies per-tile counts | User, 2026-10-11 |
| PD-13 | I1 excludes link tiles from the `budgets.tiles` comparison | User, 2026-10-11 (accepted) |

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

## PD-11 (decided): no retained vector sources; reuse from published payloads

**Decision (user).**
- `--reuse-tileset` exists to apply a diff (for example a GPKG changeset) to an earlier archive.
- Default archives carry no duplicate raw sources (`t/<sha256>` copies and
  `rustyTilesSourceContents`).
- Reuse reads the archive's actual tile payloads.

**Design (design.md §4.7, evidence §5.4).**
- The writer's payload rebase becomes reversible: only the root node object's text changes. The
  writer returns a per-payload record (δ, raw root node text, raw JSON chunk length, raw sha256).
- `vector-build.json` stores those records in a `payloads` map.
- Reuse restores exact raw bytes, checks the sha256, and stages them privately.
- The match identity, the granularity and what gets re-encoded are unchanged.
- Measured exactness: 14,703 of 14,703 production payloads, including `--quantize`, `--meshopt`
  and both.
- Measured benefit on realistic edits: §5.4.

**Validation of older archives (user's lean: yes; recommendation: yes, with no I1 change).**
- I1 moves today's gate (`validate.rs:1204-1217`) unchanged, so pre-v3 single-tree default
  archives with the list keep validating.
- Pre-v3 *chunked* archives are Unsupported under I1 for their JSON templates in any case.
- v2's ask to honour the list on every document is withdrawn.

**Ownership.** PV, except `rebase_payload` and `restore_payload`, which are PW's (ownership plan).

**Recorded for the vector owner (outside P).**
- GeoJSON features without a top-level `id` get ordinal source ids, so an insertion or deletion
  defeats reuse. At 12.5k tiles that makes reuse slower than a fresh build (§5.4).
- GPKG fids are stable. A GeoJSON id, or a documented requirement, would fix GeoJSON.

## PD-12 (decided): validator caps raised in a separate slice; P's inputs

The coordinator's validator slice raises `archive_entries` and `hierarchy_visits` so that default
vector `maxTiles` 100k archives validate. P's inputs, v3 layout without retained sources
(design §5.1):

| Output | Entries per tile | Visits per tile | At 100k vector tiles (entries / visits) |
| --- | --- | --- | --- |
| Vector, real (547 to 18,778 tiles) | 1.08–1.15 | 1.09–1.18 | — |
| Vector, synthetic 100k (`lodLevels` 3 and 16, real and full chains) | 1.11–1.20 | 1.11–1.23 | 110,523–120,336 / 110,916–122,797 |
| Point octrees | 1.00–1.02 | 1.00–1.02 | — |

- 2^17 (131,072) admits every modelled 100k vector case with at least 6% margin.
- 2^18 (262,144) admits them with at least 2.1× margin.
- Today's layout needs about 2.4 entries per tile, so it would need about 2^18 for 100k.

## PD-13 (decided): link tiles in `budgets.tiles`

- I1 excludes tiles whose only content is an external tileset document from the `budgets.tiles`
  comparison (`validate.rs:1662`). The report's `tiles` counter keeps them.
- This is an I1 interface item. P records nothing extra.

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
