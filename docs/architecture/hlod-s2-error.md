# HLOD S2 sampled source error

Implementation contract for `hlod/{error,source_index}.rs`, following D2/D4/S2 and
the D1 exact-seams and omission amendments in `hlod-040-decision.md`. Policy v2 keeps
the 5% sampled-distance inflation and has no additive area-spacing allowance.

## Measurement

- Measure captured source triangles in `[TileKey.first, TileKey.end)`, in the local
  metre frame of the final decoded proxy. Child G and leaf encoding errors are floors,
  not recursively accumulated source estimates.
- Each direction takes `max(2048, ceil(200000 * tile_source / depth_source))` area
  samples, where `depth_source` sums the source ranges of every encoded tile at that
  depth. A depth whose tiles are all published therefore has at least 200,000 samples
  per direction. Scheduling is independent of jobs, cache eviction, spill and budget.
  Depth >=32 or an empty schedule is a typed resource refusal.
- SplitMix64 jitter chooses equal cumulative-area strata, then square-root
  barycentric samples. Source controls cover boundary vertices, one vertex per
  component and sliver vertices/longest-edge midpoints, computed on bounded
  256-triangle patches (a conservative superset of true boundaries/components). Both
  clipped endpoint blocks keep every corner. Indexed proxy vertices and edges are
  checked once; centroid distances from representative selection are added controls.
- Each directed area/control distribution records count, mean, p95, p99 and max.
  Quantiles are upper log2-bin endpoints (32/octave). q, the maximum p99, is
  diagnostic only; D10 calibration is off.
- With M the maximum of both directions, controls included, publish
  `G = max(children G, 1.05*M)`. The 5% margin is motivated by the prototype's ~1%
  independent exceedance. It is **not** a covering radius, confidence interval or
  Hausdorff certificate: narrow unsampled extrema may still exceed G.
- S1 collapses single-coarse-child chains: only the encoded tile is measured, and
  `encodedDepth` keys its seed, schedule and depth summaries while `depth` keys
  publication.
- **Omitted cells (D1):** a parent whose exact interface cannot fit the cap, and every
  ancestor, publishes no content. Its job returns before any S2 work, so nothing is
  sampled, stored or allocated for it. A published parent's children are all
  published, so its G is at least theirs. Delivery gives each content-less cell
  `max(children G, 16 × diagonal)`. At a depth with omissions, the published tiles
  keep the depth's sampling density over the source they cover; delivery checks
  `samples >= required(sum of published ranges, depth_source)`. Without omissions it
  requires the published ranges to cover the whole schedule (200,000 samples).
- Incomplete or nonfinite sampling is a typed refusal, never zero error. Delivery
  validates the policy, per-row share, depth totals, encoded-tile seed, distributions,
  margin, q and parent ordering. H17 rows stay readable until S6.

## Bounded nearest search

Every production nearest query (representatives, source→proxy, proxy→source) is a
best-first search with a per-query budget `B = NEAREST_STEPS = 1024` node visits plus
triangle tests (`source_index::Budget`, the pattern of S3's donor search). A search
without a candidate first takes a greedy root-to-leaf seed outside the budget, so it
always has a finite witness: one fine block (at most 256 triangles) on the paged
index, one leaf (at most 4) on a resident surface. Hints from the previous winner
seed the next query. A query therefore costs O(B + depth + block) whatever the
source. Queries per tile are bounded by its controls (at most three per source
triangle in range, and per proxy vertex/edge) plus its samples, so the walk's nearest
work is O((n·h + P·C)·B) for n source triangles, height h, P tiles and cap C. On exhaustion the best distance
found is used. It is never below the exact distance, so M and G only grow
(conservative); a representative may then be a near, not the nearest, primitive.
Each row records `nearestSearch` (searches, exhausted, seeded, max steps), summed in
`hlod.errorMeasurement.nearest`. The exact searches remain as test oracles only.

The round-3 review adversary (R parallel oblique triangles whose boxes all contain
every query, all distances positive) took Θ(Q·R) exact work: 1,048,607 `closest`
calls for 32 queries at R = 32,768. The committed test
`nearest_searches_are_bounded_on_oblique_adversaries` reproduces that count and
bounds the budgeted search at `Q(B+1)` calls on both paths, independent of R.
B is a tuning constant, chosen by measurement. The tex3/crop3 searches take at most 501
steps, so none exhausts at 1024. On the adversarial chords input (2,048 random chord
slivers across two ribbon blocks, every box overlapping almost every query) a release bake
spends 28.0/17.6/9.6/5.0 S2 CPU-s at B = 4096/1024/512/256. G is unchanged down to 512 and
grows 30% at 256. The worst case per bake is about `B × (2 × samples + controls)` steps,
independent of the source's spatial structure.

Library tests that do not exercise S2's measurement run a reduced schedule (4,096 per
depth, 64 per tile) and B = 64 through `Settings`. Delivery outside library tests admits
only the production schedule (`Schedule::admitted`), and each row records its schedule.

## Decoded bytes and S3 handoff

The lattice is prepared once for centroid identity and encoding. The encoder carries
its face order through material grouping; GLB readback checks every corner bit for
bit against the prepared position and every representative row, and measurement
consumes the positions read back. A control that only changes the expected position
must fail before publication.

`SourceIndex` holds compact source triangles (original f32 positions, rank,
provenance, expanded ordinal, material, control mask, cached area) and fine BVHs in
two disk sequences; a tile's area sums whole subtrees and scans at most two clipped
blocks. `SourceIndex::query(captured, TileKey)` keeps at most 8,191 trimmed upper
nodes resident, plus a bounded FIFO node cache and fine-block cache (FxHash, never
iterated for results). All candidates clip to the descendant source range; ties use
provenance, expanded ordinal, then rank. S3 consumes `nearest_budget`, `surface`,
`area`, `primitive` and `triangle` (the original `FullTriangle` for donors), the
decoded `CoarseTile`, its representatives and G; chart spacing is
`max(G/16, 1e-5 m)` and must keep decoded positions and representatives.

## Resources

- **Scratch:** the index is the walk scratch component `s2-source-index`, admitted
  through `ScratchLedger::admit` for `160 * source_triangles + 1 MiB` before its first
  write. Every block and node write is charged to the lease first; a write past the
  admission is refused. Memory-backed scratch charges it to memory in the plan. S2
  writes no other scratch. Files are capture-owned and removed on success, refusal
  and cancellation.
- **Memory:** each parent job is charged 8 MiB query caches, 4 MiB measurement
  workspace and the search frontiers (`SEARCH_BYTES`, at most `2B+1` entries each);
  2 MiB fixed construction. Query caches check actual capacities and refuse past
  their allowance.
- **CPU:** index construction uses process CPU; representatives and each directed
  pass sum worker-thread CPU. CPU stays out of deterministic metadata.

Builds, tests and bakes run in a user systemd scope with `MemoryMax=10G`,
`MemorySwapMax=0`, at idle priority; crop bakes use `--memory-budget 8GiB --jobs 2`.
