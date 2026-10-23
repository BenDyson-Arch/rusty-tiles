# H16 streamed hierarchy and 3TZ delivery

Status: implemented on `ml/h16-delivery` (PR #192), light process (tests, one
nonauthor review, perf/memory evidence, consumer proof, CI). This slice publishes
H1's tile tree ([H1 contract](h1-capture-tree-contract.md)) and H8's full-detail
leaves (#187) as one Tiles 1.1 implicit forest in a `.3tz`, through the shared
typed serializer of #191/#198 (`src/tileset_node.rs`, `src/implicit/tileset.rs`,
`src/implicit/tileset/stream.rs`). Parents carry no content unless the leaf
directory has H17 coarse tiles (`proxyManifest`), which are then published as
their content ([H17 contract](h17-lod-contract.md)). The public entry is
`rusty-tiles mesh` / `mesh_to_tileset` (H26); the leaf-directory steps are
crate-internal. Evidence: `~/.cache/rusty-tiles-evidence/h16-20261013/`
([acceptance record](../../bench/architecture_audit/mesh_large_h16/README.md)).

## Input boundary

H16 consumes H8's published leaf directory unchanged: `manifest.json`
(`kind: full_detail_leaf_payloads`, schema 1, totals), `leaves.jsonl` (one row
per leaf, in rank order) and `leaves/<n>.glb`. A row carries the H1 identity
(`cellDepth`, `cell`, `pathLength`, `path`, `lastOctant`, `packed`, `depth`), the
rank range `[first, end)`, H8's outward `geometryError`, and the payload's byte
length and SHA-256. Rows are checked as H1 defines them: valid octant digits,
synthetic run octants only below a depth-21 cell, depth at most 31, `packed`
exactly when an unpacked-address leaf covers more than its first octant, ranks
contiguous from 0, a synthetic run leaf or the root covering only its own
slot, and the totals equal to `manifest.json`. Violations are `InvalidInput`. H8's tree, lattice and payload bytes are never changed, split or
rebuilt. A row of more than 8,192 triangles is `Unsupported` (see Consumer
property tables).

Leaf bounds come from each GLB's JSON chunk only. H8 writes one scene-root node
per primitive with at most `translation` and `scale`, and POSITION accessors
(`FLOAT`, or unnormalized `UNSIGNED_SHORT` codes with `KHR_mesh_quantization`)
whose `min`/`max` bound the stored values. Each decoded coordinate is `t + s·c`;
products and sums are rounded outward with exact FMA/TwoSum residuals. The box
is then taken from glTF y-up to the tile frame's z-up as `(x, y, z) -> (x, -z,
y)`, the conversion C2 applies exactly once. A node with any other field is
refused. Each payload is read and hashed against its row when it is staged (a
mismatch is `InvalidInput`), and the forest writer checks the staged member
again when it publishes it.

## Hierarchy reconstruction

An H1 address is the `cell_depth` spatial octants of `cell` followed by the
`path_len` synthetic octants of `path`, most significant first; a digit is the
implicit slot `x + 2y + 4z`. The H1 cell tree opens a cell at every depth on the
way down and a run hierarchy a node at every run length, so every proper prefix
of a leaf address is an internal tile and the leaf rows alone determine the
tree. A packed leaf occupies its FIRST octant only; its later octants are
covered metadata (`lastOctant` in its extras) and the next sibling must take a
later slot. No phantom siblings are created and no payload is duplicated.

The rows are streamed with a stack of at most 32 open tiles (`plan::Builder`).
A tile closes when the next leaf leaves its subtree; preorder and octant order
are checked as tiles open. Global depth is the address length, so synthetic run
depths beyond 21 are ordinary levels. Coordinates are three u32 axes, never a
combined Morton key. A tile's key is H1's `(first rank, depth)`.

## Roles, partition and the serializer API

Roles follow P-IMPL §4 with `S = 4`, as amended by the
[implicit forest consolidation](implicit-forest-consolidation.md). A closed tile of height 4 or
more is an explicit skeleton tile; its leaf children are explicit leaves. Its other
children (height 1 to 3) are *shallow trees*: written as explicit tiles while their
document has room, otherwise demoted to implicit roots, each one subtree with
`subtreeLevels == availableLevels == height + 1` and no child subtrees. A whole
tree of height below 4 is likewise explicit tiles at `tileset.json`, or one implicit root
when it exceeds the caps; a one-leaf tree is an explicit root leaf.

H16 is the first caller of the streamed API (#198), not of the whole-tree
`write_forest_records`:

- `ForestWriter::new(tileset.json document, staging, OCTREE)
  .with_max_implicit_levels(4)`;
- each document is a tree of `ForestTile`s: `ForestTile::explicit(record,
  children)` for skeleton tiles and explicit leaves, `ForestTile::named(
  ImplicitNames::Templates { contents, subtree }, TileTree::with_slots(..))
  .planned(Availability)` for implicit roots, and `ForestTile::Link` for a
  document written earlier;
- external documents are written children first by `ForestWriter::external` as
  tiles close; `tileset.json` last by `ForestWriter::root`.

Partition (`plan::LIMITS`, amended by the consolidation): caps of 480 tile objects,
4,096 tile records and 12 tile levels per fragment and so per document. When a skeleton
tile closes over a cap (`Builder::fit`): while objects are over, the largest explicit
shallow tree anywhere in its fragment is demoted to an implicit root (most tiles, then
greatest height, then lowest rank); while levels are over, the child reaching deepest is
demoted (shallow tree) or moved (skeleton tile); while objects or records are over, the
inline skeleton child contributing most (lowest slot on ties) is written as the external
document `tileset-f<first>-d<depth>.json` and replaced by its link. If records alone are
still over, the tile itself becomes a document, as before. Every document therefore holds
at most 480 tile objects (at most 26 JSON value nodes each) and 12 tile levels (JSON depth
26), within the writer's 16,384 nodes, 2 MiB and depth 32, which it still enforces. (Until
the consolidation, a fragment over 32 objects, 1,024 records or 8 levels became a document at
once, giving documents of at most `1 + 8·32` objects and the 13–30 KB nested documents of
the 2048 output.)

External documents sit at the archive root. Content, template and link URIs
resolve relative to the document that holds them and the writer emits
archive-root member names, so this slice also makes `ForestWriter::external`
refuse a document name containing `/`.

## Addressing, members and availability

Implicit roots use caller templates:
`implicit-content/f<first>-d<depth>-{level}-{x}-{y}-{z}.glb` and
`subtrees/f<first>-d<depth>-{level}-{x}-{y}-{z}.subtree`. Explicit leaves are
`content/f<first>-d<depth>.glb`. Child slots are explicit
(`TileTree::with_slots`, the H1 digit). The planned `Availability` is computed
from each tile's H1 axes relative to its root, sorted level-major and Morton
within a level, independently of the slots from which the writer derives its
own; the writer refuses any positional difference before writing. Every
available tile has a metadata row (FLOAT64 box and error, the existing tile
schema) and leaves are the only content.

Payloads are hard-linked (copied only without hard links) from the leaf
directory into a staging folder at their FINAL member names, so the writer keeps
them in place: no raw source is ever a different name, `ChunkOutput.
implicit_sources` must be empty (checked), and #198's caveat that `check_sources`
does not see implicit members of earlier documents cannot apply, because no
source name is reused. No payload needs a rebase: every tile shares the dataset
frame. This slice adds the missing #198 test that a payload staged at its final
member but needing a rebase is refused and leaves the staging directory as it
was.

## Bounds, errors and placement

A tile box is the f64 centre of its bounds and outward half lengths; its closed
extent, rounded outward, is what its parent encloses, so boxes nest and every
decoded vertex lies in every ancestor's box. Internal boxes are the union of the
children's extents, never H1's centroid cells. Leaf error is H8's outward
correspondence bound. An empty parent's error is `max(16 · box diagonal,
children's errors)`: CesiumJS (default maximum screen-space error 16) refines it
until the whole box projects below about one pixel, so leaves are reached at
any distance. This is a traversal policy for empty tiles, not a proxy
certificate; with full-detail-only output a far view loads every visible leaf
until H17 supplies certified coarse content. The tileset error is
`max(1 m, root error, root diagonal)`.

Placement is local metres with one rigid anchor/orientation/offset matrix on
the `tileset.json` root, from the existing placement boundary
(`mesh_archive::ResolvedPlacement`; WGS84 anchor, ENU unit quaternion, source
y-up offset; no CRS conversion). No other tile has a `transform`, payloads keep
H8's frame and no RTC is introduced. Under the merged C2 frame chain the root
placement transform enters no containment predicate, so C2 checks containment
in the dataset frame only; it does not cover rotated boxes, RTC or the
placement itself, and none is claimed. The placement is checked by a unit test
(the written matrix equals the boundary's exactly, on the root only) and by the
consumer proof.

## Consumer property tables (CesiumJS 8192 limit)

CesiumJS 1.146 builds one GPU structural-metadata texture per property table,
whose width is the table's row count, and refuses tables wider than its 8,192
maximum texture size (the H4 diagnosis,
`~/.cache/rusty-tiles-evidence/h4-property-budget-20261013/diagnosis.md`); it
resolves picks through one table per model. An H8 leaf has a `source_triangle` table with one
row per triangle and a `source_primitive` table with at most as many rows.

**Decision: cap the leaf size.** Every profile's leaf budget `L` is 8,192
(`config::CONSUMER_TABLE_ROWS`; `fast` was 32,768 and `balanced` 16,384), so
both tables of every leaf fit, and the H8 writer refuses a wider table as a
broken plan (`InvalidState`, not a retryable budget). Exact per-triangle
provenance stays in the leaf's own tables and Cesium picks it per model. The
alternatives were rejected:

- Splitting a leaf into several contents costs the same requests and draw calls
  as smaller leaves, but every part shares one bounding volume (no culling or
  refinement benefit), atlases would have to be split or duplicated, and H8's
  frozen leaves would be re-cut after planning.
- A coarser table with per-triangle IDs resolved elsewhere breaks per-triangle
  picking in the viewer, needs a sidecar resolver, and does not bound the
  primitive table: a leaf of 32,768 triangles may touch more than 8,192
  primitives.

The cost is up to four times as many leaves for `fast` and twice for
`balanced`; profiles still differ in texture pages and encoding. H16 refuses a
leaf directory from an older producer with wider leaves.

## Streaming, memory and determinism

Two passes stream the rows through the same builder. The first counts: exact
members per document, document count, the root error the `tileset.json`
document needs. Nothing is staged. The second stages and writes one document at
a time and must reproduce the first pass's shape and member count exactly.
Resident hierarchy state is the open stack plus inline fragments within the
partition limits: at most 32 frames of 8 children each within the records cap,
reported as `peakHeldRecords`: 4,680 and 4,687 for 4,096 and 262,144 leaves, the same
before and after the consolidation (the 2,097,152-leaf figure, 4,687, predates it).
Measured H16 peak RSS is 24.3, 27.5, 37.2 and 48.4 MiB for synthetic trees of
4,096, 32,768, 262,144 and 2,097,152 leaves (the last 2.4 M tiles, 522
documents, 2.1 M members), and 20.8 MiB for the real 10 M-triangle mesh. What
still grows is the index sorter's in-memory run (capped at 16 MiB) and the
serializer's one name per explicit leaf and per document (#198's deliberate
residue; explicit leaves are a small fraction of all leaves).

The archive is written after `tileset.json`: `tileset.json` first, the other
members in their deterministic production order, the index last. Index rows
(MD5, offset) are external-sorted in scratch; central-directory records are
spooled to a scratch file; members are copied with one 64 KiB buffer. The ZIP
records are the shared `archive3tz` primitives (`local_header`,
`complete_entry`, `central_header`, `end_records`), factored out unchanged.

Output depends only on the leaf directory and placement: not on jobs, run size,
fan-in or scratch/work location (H8 already makes leaf bytes independent of
them). Tests compare whole archives across work locations, and across H8 jobs
1 and 2 with separate scratch.

## Member count and the 3TZ ceiling

Members are `tileset.json`, external documents, explicit and implicit payloads,
subtrees, and the index. The ceiling is the codec's 178,956,970 TOTAL members,
index included (`archive3tz::MEMBER_CEILING`, the validator's), so the index
holds `24·(N - 1)` bytes. Refusal is `Unsupported` and happens before anything
is staged: first a lower bound (leaves + 2) from `manifest.json`, then the exact
planned count. The exact count itself is admitted. Arithmetic overflow and
scratch exhaustion are `ResourceLimit`; a leaf directory that contradicts itself
is `InvalidInput`; a writer refusal of a planned document or a plan/write
mismatch is `InvalidState`; cancellation keeps its kind. A failed run removes
its staging and publishes nothing.

## Validator acceptance and limits

The merged validator (C2 slice 3 with the H11 fast path) accepts the output with
`decodedContentContainment` for untextured leaves, on the synthetic mixed trees
(packed leaves, depth-24 synthetic runs, external documents) and on real H8
leaves. PNG (`lossless`) leaves are not recorded as validated. Two limits are recorded, not fixed here
(validator changes need the full process):

- **KTX2 leaves.** H8's lossy profiles emit KTX2 textures and correctly list
  `KHR_texture_basisu` in `extensionsRequired`. The merged C1 payload profile
  admits only four required extensions, so the validator refuses such archives
  as `Unsupported` ("required extension KHR_texture_basisu"). A test pins this.
  Admitting it is a validator follow-up.
- **Scale.** The validator's fixed limits refuse far smaller archives than H16
  writes: its 1 GiB archive-byte limit refuses the real mesh's 2,806 tiles at
  1.40 GB, and its 2^18 hierarchy-visit limit refuses a 299,603-tile synthetic
  forest. Validating these, and milestone 4's 2^22-tile delivery gate, needs the
  validator-scale slices (VS-A to VS-D: catalog, traversal/C2, limits, scale
  acceptance). H16's own planning streams them.

## Acceptance

Tests (`src/mesh_large/delivery/tests.rs` and the M1/H8 additions):

- mixed tree: roles, explicit and implicit members, slots and subtree
  availability read back by an independent subtree-bitstream reader and compared
  with the generator's H1 addresses; payload bytes unchanged; validator accepts;
- determinism across work locations for both a one-document and a
  multi-document partition, and across H8 jobs 1 and 2 on real leaves;
- ceiling: early and exact refusals, shown to precede staging (a missing work
  folder fails only the admitted count), exact count admitted;
- malformed input: preorder, octant order (including a packed overlap), rank
  gap, payload length, a same-length payload change found while staging
  (nothing published, no work files left), a 8,193-triangle row, totals
  mismatch, bad addresses (depth 32, cell range, depth sum) and a run leaf
  claiming more than its slot;
- the streamed archive equals `archive3tz::serialize` byte for byte at 70,000
  members (ZIP64 counts) with the index sorted through spilled runs;
- root forms (one leaf, explicit short tree, one implicit root over the caps) and the
  exact rigid placement;
- consolidation: single-level shallow trees, the deepest shallow tree at and one tile
  beyond the object cap, and mixed explicit-leaf, explicit-tree, implicit-root and link
  siblings; every forest checked against the generator's roles, availability and the
  per-document caps;
- planner memory bounded by the partition at 2^18 leaves;
- 8,192/8,193 table rows at the H8 writer; every real leaf's tables within it;
- outward z-up GLB bounds and the rotated-node refusal;
- M1: external documents outside the root refused; final-member rebase refused.

Real mesh, consumer proof and performance figures are in the acceptance record.

## Follow-ups

H17 coarse content and certificates; the C1 `KHR_texture_basisu` admission; the
validator-scale slices; H8 throughput (`ml/h8-throughput`) and its texture
gates; the documented CesiumJS sRGB/ASTC colour limit; retuning the partition
constants with perf evidence; a bounded-memory replacement for the serializer's
explicit-name set if explicit leaves dominate at scale.
