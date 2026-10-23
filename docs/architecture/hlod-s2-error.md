# HLOD S2 sampled source error

Implementation contract for `hlod/{error,source_index}.rs`, based on the approved
D2/D4/S2 direction in `hlod-040-decision.md` on origin/develop. This branch retains
S1 geometry, leaf encoding, lattice and complete-interface ownership. It replaces
S1's meshopt estimate as the published error and child-derived representatives.
It retains H16 bounds/forest planning and strengthens its input validation.

## Invariants and supported domain

- Measure the captured source triangles in `[TileKey.first, TileKey.end)`, in the
  same local metre frame as the final decoded proxy. No recursive child estimates
  enter the distance measurement; child G and leaf encoding errors are floors.
- Each direction has `max(2048, ceil(200000 * tile_source_count /
  depth_source_count))` area samples. Each occupied parent depth therefore has at
  least 200,000 samples per direction. Cost scales with occupied depths/tiles and
  source controls, never an unbounded requested spatial density. Scheduling is
  independent of jobs, cache eviction, sort spill and the admitted memory budget.
- A versioned tile seed drives SplitMix64 jitter in equal cumulative-area strata,
  followed by uniform square-root barycentric sampling in the selected triangle.
  Source controls visit every vertex, including zero-area components; duplicate
  vertices are suppressed only within bounded 256-triangle windows. Slivers
  (area <= longest squared edge * 1e-4) and degenerates add their longest-edge
  midpoint. Decoded proxy controls visit vertices and every edge midpoint.
  These controls include every component without retaining a connectivity graph.
- Both area distributions and control distributions record sample count, mean,
  p95, p99 and max. Quantiles are upper endpoints of fixed log2 bins (32/octave,
  <=2.19% relative bin slack); counts, means and maxima are the actual samples.
  q is the maximum p99 over both area/control sets in both directions. G is
  published unchanged; D10 refinement calibration is not enabled.
- Let M be the maximum over the two directions including controls, A the larger
  surface area and N the scheduled area count per direction. Publish
  `G = max(child G, M + max(0.05*M, 0.5*sqrt(A/N)))`. Half the characteristic
  area spacing accounts for the 1-Lipschitz distance field at the sample scale;
  the 5% term exceeds the prototype's ~1% independent maximum exceedance
  (2.8369 versus 2.8106 m). This is a heuristic safety margin, **not a covering
  radius, confidence interval or Hausdorff certificate**. Narrow unsampled
  extrema may still exceed G and independent exceedances must be reported.
- Empty, incomplete or nonfinite sampling is a typed refusal. Delivery rejects
  malformed counts/distributions, inconsistent policy/M/margin/q, understated G
  and a G below the actual children. Historical H17 rows remain readable until
  their producer is removed in S6. There is no fallback to a simplifier estimate.

## Source query and S3 handoff

`SourceIndex::build(captured, check)` writes compact triangle blocks and their
fine BVHs plus a balanced hierarchy into capture-owned disk sequences. It streams
256 triangles at a time, with O(log N) construction state. Each admitted job calls
`SourceIndex::query(captured, TileKey)` and holds a bounded hierarchy of
at most 8,191 tile-local hierarchy nodes. Unary paths outside the tile's rank
range are removed once. A complete resident hierarchy releases its paged-node
cache and admits up to 240 fine blocks; a deeper hierarchy instead keeps a
16,384-entry FIFO node cache and 96 fine blocks. The query clips both hierarchy
and triangle candidates to the descendant rank range. It returns an unrestricted
nearest surface witness: source rank, barycentrics and squared distance. Ties use
source provenance (node/mesh/primitive/triangle), expanded ordinal, then rank.

`SourceQuery::triangle(hit)` reads the original `FullTriangle` only when needed:
UVs, colours, normals, flags, material and provenance are available to S3 without
keeping an uncharged source mesh. `surface()` streams compact original positions
in deterministic block-BVH order and clips by stored source rank; S2's source
samples use this stream instead of repeatedly decoding captured attributes.
Coherent queries may seed the traversal with the last actual triangle witness
as an upper distance; the full nearest search and source-order tie rule still run. `primitive(hit)` supplies D4's representative
row. Coarse IDs are selected at final decoded centroids and duplicated per corner
by the existing encoder. S3 owns and separately charges its image cache.

S3 can use `CoarseTile`'s indexed decoded positions, triangles and locked mask;
`TileError.geometric_error`/the persisted G sets chart spacing
`max(G/16, 1e-5 m)`. `HlodRow.measurement` is the versioned G/q/directed summary.
Charting must preserve the decoded positions and representative IDs. No H17
facing restrictions, primitive gates, donor-distance gates or certificates are
part of this API.

## Resource ownership and acceptance

The HLOD planner adds 8 MiB query-cache allowance and 4 MiB measurement workspace
per admitted job, plus 2 MiB construction allowance to the fixed stage charge.
Histograms, source/proxy stream buffers, the proxy BVH, controls, index I/O and
centroid identity queries fit inside those charges; none grows with the tile's
source range. The cache checks actual vector capacities and conservative hash
capacities against the 8 MiB allowance, including replacement/allocation slack,
and refuses if that charge is exceeded. Fine BVH leaves hold at most four
triangles. Construction and the unbounded-source fallback remain streamed. The disk allowance is `128 * source_triangles + 1 MiB`, added to
the existing incidence scratch envelope. It includes compact source records,
fine and upper BVH nodes; capture scratch removes it on success/refusal/cancel.
Memory-backed scratch is charged through the same S1 budget resolution.

Analytic tests use known tilted/offset plane distances and a sphere-cap sagitta,
plus tiny distant points, collinear/degenerate faces and boundary maxima. Disk
queries are compared with exhaustive nearest-triangle arithmetic through block
boundaries. Existing S1 tests exercise job/budget determinism, exact-budget
admission, one-below refusal, cancellation and decoded interface/leaf identity.
Sensitive delivery controls alter G, sample counts, q and margin, remove the S2
metadata entirely, and lower internally consistent non-root depth totals. The
manifest discriminator prevents historical-row fallback for HLOD directories;
terminal iterator errors propagate before publication. Resident and forced-paged
queries are compared with the same exhaustive reference, including compact-stream
rank coverage. Extreme 1e8:1 slivers have a separate analytic distance control,
using cross-product barycentrics to preserve their narrow interior dimension.
Cancellation is checked inside each 256-sample batch, including zero-area surfaces.
Compensated accumulation keeps constant-distance sample means stable. Crop bakes pin
binary/input hashes; source-aware audits and CPU cost are reported separately.
The full-model quality, speed and memory gates remain S8 obligations.

CPU diagnostics separate source-index construction, representative queries, proxy
BVH preparation, source-area scans and both directed query passes. The index uses
a process CPU clock before workers start; tile shares sum worker-thread CPU
clocks. CPU counters are excluded from deterministic manifests; wall time is never
substituted for CPU. Final crop receipts and independent source-aware cut results
are recorded separately from preliminary profiling runs.
