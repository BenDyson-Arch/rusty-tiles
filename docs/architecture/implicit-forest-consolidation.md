# Implicit forest consolidation (mesh delivery layout)

Status: design and implementation on `feat/consolidate-implicit-forest` (from `develop`
`41af7f6e`). Full-audit tier: this changes the published layout of `rusty-tiles mesh`. It
amends the roles and partition of the [H16 delivery contract](h16-delivery-contract.md). The
shared writer (`src/implicit/tileset.rs`), the other producers (vector, point cloud) and
[P-IMPL](p-impl-layout-contract.md) for those producers are unchanged. Evidence and the
simulator are in
[bench/architecture_audit/implicit_forest_consolidation/](../../bench/architecture_audit/implicit_forest_consolidation/README.md).

## Problem

A cold CesiumJS load of the 2048-texture `mgal_detail` output (2,806 tiles) made 102 tile
requests: 70 GLBs, 25 subtrees and 7 tileset JSONs
(`~/.cache/rusty-tiles-evidence/load-profile-20261023/m2048-cold.json`). A content GLB of an
implicit root can be requested only after that root's subtree, and the subtree only after
the external document that holds the root. The deepest content therefore sat five dependent
round trips from the start: `tileset.json`, an external document, a nested external
document, a subtree, then the GLB. On a 150–250 ms link with six HTTP/1.1 connections, each
round trip is paid in full. The owner wants native Cesium behaviour, so the fix is in the
tiler output.

## Method

`simulate_first_view.py` rebuilds every tile of an output (documents, implicit subtrees,
boxes, errors and content sizes). It then replays CesiumJS's base traversal
(`Cesium3DTilesetBaseTraversal.js` with `skipLevelOfDetail` false and maximum
screen-space error 16) from the preview's opening camera (`preview/index.html` `frame()`:
heading −0.8, pitch −0.65, range 3r; 1600×1000; fov 60°). A resource defined by a document
or subtree that loads on round trip *k* can be requested on round trip *k* + 1, and
`tileset.json` is round trip 1. **Dependency depth** is the last round trip of the first
view.

**Calibration.** On the 2048 output the simulator requests exactly the CDP profile's
102 URLs, with no extra or missing URL. The same code can re-lay the same logical tree under
another rule, so options are compared on identical tiles. Re-laying with today's rule
reproduces the parsed output: 200 roots (48/81/71 at 2/3/4 levels), 7 documents and the same
102 requests. The `rtt200` column below is a rough six-connection schedule at 200 ms per
request with zero transfer time. It is a guide, not a measurement.

## Why there are 200 roots

The cause is the explicit/implicit split rule acting on a mixed-depth, sparse tree. It is not
a subtree size cap: `subtreeLevels == availableLevels` and no root has child subtrees. It is
not per-branch availability as such either.

- **Mixed-depth leaves.** H1 stops splitting a cell once it fits the leaf budget, so leaves
  sit at depths 2 to 9 (2048: 3/33/63/160/601/751/316/32 leaves at depths 2–9). The tree is
  sparse, with a median of 3 children per internal tile (tex3: 2).
- **The split rule (H16 roles, P-IMPL §4, S = 4).** Every tile of height ≥ 4 is explicit,
  and *every* non-leaf child of height 1–3 of such a tile is its own implicit root with one
  subtree. The 2048 tree has 50 skeleton tiles carrying 0–8 such children each, which gives
  200 roots. They are small, with a median of 3.5, 9 and 21 tiles at heights 1, 2 and 3,
  and median subtrees of 2.3 KB. tex3 (219 tiles, height 8) has 5 skeleton tiles and 15
  roots (2/6/7 at heights 1/2/3).
- **Why roots are not merged today.** P-IMPL §2 rules out child subtrees: Cesium culls a
  child-subtree placeholder by its derived grid cell, and H16 boxes are unions of children,
  not grid cells. So a merged tree must be one dense subtree, whose availability grows 8×
  per level.
- **Why there are 7 documents.** H16's partition (`plan::LIMITS`: 32 objects, 1,024 records,
  8 levels) moves a skeleton tile into its own document as soon as its fragment passes
  32 objects. That gives 13–30 KB documents, nested two deep, while the whole skeleton fits
  in 7.6k JSON value nodes (writer cap 16,384).

## Options (2048 output, first view)

| Option | Requests (JSON / subtree / GLB) | Depth | Constraints and cost |
| --- | --- | --- | --- |
| Today (S = 4, 32/1024/8) | 102 (7 / 25 / 70) | 5 | — |
| (a) Inline roots of ≤ 2 levels, today's partition | 97 (9 / 18 / 70) | 5 | More objects split more documents (10) |
| (a) Inline ≤ 3 levels, today's partition | 97 (16 / 11 / 70) | 5 | 24 documents |
| (a) Inline all (fully explicit), today's partition | 92 (22 / 0 / 70) | 6 | 38 documents; JSON 120 → 876 KB |
| (a) Inline all, one document | 71 (1 / 0 / 70) | 2 | 64.6k value nodes, 868 KB: over the writer's 16,384-node cap and at the validator's 65,536; does not scale |
| (b) Merge siblings, S = 5 | 107 (4 / 33 / 70) | 4 | Parent content moves behind a subtree |
| (b) S = 6 | 100 (2 / 28 / 70) | 4 | Dense availability of up to 6 levels; first-view metadata bytes 2.5× |
| (b) S = 7 | 90 (2 / 18 / 70) | 4 | 7-level subtrees of 75 KB of availability each; 1.0 MB of metadata in the first view |
| (c) Raise `subtreeLevels` | 102 | 5 | No-op: every root already has one subtree covering all its levels |
| (d) Pack documents to the budget (480 objects, 4,096 records, 12 levels) | 96 (1 / 25 / 70) | 3 | One 120 KB `tileset.json` (7.6k nodes) |
| (d) + inline roots of ≤ 2 levels | 89 (1 / 18 / 70) | 3 | 156 KB, 10.7k nodes |
| **(d) + inline while the budget allows (recommended)** | **87 (1 / 16 / 70)** | **3** | 164 KB, 11.4k nodes; 146 roots remain |

(b) is feasible by addressing, because H1 is an octree by address and packed and synthetic
leaves are ordinary slots. But it trades subtrees for larger subtrees. It also puts the
merged parent's own (H17 proxy) content behind a subtree, and S = 5 is worse than today.
Inlining with today's partition does not help, because extra objects split extra documents,
which are dependent hops too. Packing documents is what removes the hops. Inlining then
removes subtree requests where the document has room.

**JSON growth against parse cost.** Explicit tiles cost about 25 value nodes and 350–550 bytes
each. Implicit tiles cost nothing in JSON, plus 104 bytes of metadata rows in their subtree.
The recommendation grows the 2048 JSON from 120 KB in 7 documents to 164 KB in one, and
tex3's from 8.8 KB to 66 KB. CesiumJS parses JSON at hundreds of MB/s, and building the
tile objects is linear. This is well under one round trip, and the budget caps every
document at about 12.5k nodes regardless of input size.

## Recommendation

For the mesh delivery planner (`src/mesh_large/delivery/plan.rs`) only:

1. **Inline shallow trees while the document has room.** A non-leaf child of height 1 to 3 of
   a skeleton tile (today always an implicit root) starts as an **inline tree**: explicit
   tiles at `content/f<first>-d<depth>.glb`. So does a whole tree of height below 4. The
   skeleton rule (height ≥ 4) and explicit leaves are unchanged.
2. **Pack documents to a budget.** When a skeleton tile closes and its fragment exceeds a cap,
   the planner (`Builder::fit`) takes one step at a time:
   1. if tile objects are over, it turns the largest explicit shallow tree anywhere in the
      fragment into an implicit root (most tiles, then greatest height, then lowest rank);
   2. if levels are over, it takes the child that reaches deepest (lowest slot on ties):
      a shallow tree is demoted, and a skeleton child is moved into its own external
      document;
   3. if objects (with no shallow tree left) or records are over, it moves the inline
      skeleton child that contributes most (lowest slot on ties) into its own external
      document;
   4. if a cap is still exceeded, which only records can be, the tile itself becomes an
      external document, as today.

   The caps are per document: 480 tile objects (at most 26 value nodes each, so
   ≤ 12,480 + document overhead < 16,384), 4,096 records and 12 tile levels (deepest member
   at JSON depth 26 < 32). A fragment held after a close is within the caps, so the
   planner's resident bound has the same form as today. The measured `peakHeldRecords` is
   unchanged: 4,680 and 4,687 for 2^12 and 2^18 leaves.
   The root `tileset.json` never splits, so it can hold up to 1 + 8 · 585 = 4,681 records
   (eight implicit roots of four levels under the root), past the 4,096 records cap.
3. Implicit roots keep today's form exactly: one subtree, `subtreeLevels == availableLevels
   == height + 1`, constant-0 child availability, and a metadata row per tile.

**Expected first view.** 2048 output: 102 → 87 requests (JSON 7 → 1, subtrees 25 → 16,
GLBs 70), depth 5 → 3, and 54 of 70 GLBs requestable on round trip 2. tex3 and crop3 (fresh bakes of develop and
this branch): 22 → 15 requests (subtrees 7 → 0), depth 3 → 2, fully explicit in one 66 KB
`tileset.json`. The simulator's prediction from develop's output matched the branch
output exactly. Larger outputs keep implicit roots wherever documents are full, so the
archive-level gains shrink towards (d)'s, but the documents stay full and the JSON chain
stays short.

**Archive size.** Members fall: up to one subtree per inlined root, and fewer documents.
JSON grows by about 25 value nodes per inlined tile. On the 2048 output, 54 subtrees
(about 120 KB) are replaced by about 44 KB of JSON, against a 1.15 GB archive.

### Format and validator rules touched

- **H16 contract, roles.** "Its other children (height 1 to 3) are implicit roots" and "a
  whole tree of height below 4 is one implicit root" become budget-dependent: inline
  explicit trees, or implicit roots when demoted. Member names follow the role
  (`content/…` explicit, `implicit-content/…` implicit).
- **H16 contract, partition.** The constants change from 32/1,024/8 to 480/4,096/12, and
  the rule changes from "a fragment over a limit becomes a document" to the three steps
  above. The document bound changes from `1 + 8·objects` to `objects`.
- **Output digests.** Every mesh output's JSON, subtree and member set changes. Content GLB
  bytes do not.
- **Validator: no rule changes.** The output uses only constructs it already admits:
  explicit tiles with explicit children and content (core); implicit roots on explicit
  tiles (I1); and external documents on explicit tiles (C1). Containment (C2) sees the
  same boxes. The relevant caps, which are unchanged, are per-document `json_bytes`
  8 MiB, `json_depth` 64 and 65,536 value nodes (documents stay below 12.5k nodes), plus
  `hierarchy_visits` 2^18 (one visit per tile, as today).
- **Consumer behaviour.** Cesium selects the same tiles with the same boxes and errors. The
  simulator reaches the same GLB set in every layout (70 of 70 on the 2048 output).

### Not changed

H8/H17 payloads (byte-identical; only member names of inlined tiles change), boxes, errors,
placement, the 3TZ writer, the shared `ForestWriter`, and the vector and point-cloud
layouts.
