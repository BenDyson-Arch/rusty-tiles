# HLOD S2 sampled source error

Implementation contract for `hlod/{error,source_index}.rs`, based on D2/D4/S2
in `hlod-040-decision.md` on origin/develop. Policy v2 follows the coordinator's
client-speed decision: retain the 5% sampled-distance inflation and remove the
additive area-spacing allowance. Historical v1 receipts remain in the evidence
cache and git history; they do not describe this producer.

## Invariants and supported domain

- Measure captured source triangles in `[TileKey.first, TileKey.end)`, in the same
  local metre frame as the final decoded proxy. Child G and leaf encoding errors
  are floors, rather than recursively accumulated source estimates.
- Each direction requires `max(2048, ceil(200000 * tile_source_count /
  depth_source_count))` area samples. Each occupied encoded depth therefore has
  at least 200,000 samples per direction. Scheduling is independent of jobs,
  cache eviction, sort spill and the admitted memory budget. Empty/unsupported
  depths, including depth >=32, return a typed resource refusal before indexing.
- SplitMix64 jitter chooses equal cumulative-area strata, then uniform square-root
  barycentric samples. Source controls cover boundary vertices, at least one
  representative per component, and sliver vertices/longest-edge midpoints.
  Connectivity is computed on bounded 256-triangle patches. Every true source
  boundary is also a patch boundary; every global edge-connected component has
  at least one patch component. Artificial patch boundaries/components add
  conservative checks without a resident global connectivity graph. For either
  partially clipped endpoint block, all corners are conservatively retained.
  Degenerate/sliver vertices survive regardless of their area.
- Indexed proxy vertices and undirected edges are checked once per tile. Already
  computed source-nearest centroid distances used for representative IDs are
  included as additional proxy controls without repeating their queries.
- Both directed area/control distributions record count, mean, p95, p99 and max.
  Quantiles are upper endpoints of log2 bins (32/octave, <=2.19% relative slack).
  Counts, means and maxima are actual samples. q is the maximum p99 across all
  four distributions and remains diagnostic; D10 refinement calibration is off.
- With M the maximum of both directions including controls, publish
  `G = max(children G, 1.05*M)`. The recorded margin is `0.05*M`. The prototype's
  ~1% independent exceedance (2.8369 versus 2.8106 m) motivates 5% inflation.
  This is a heuristic sampling margin, **not** a covering radius, confidence
  interval or Hausdorff certificate. Narrow unsampled extrema may still exceed G.
- S1 collapses single-coarse-child chains before building: only the deepest
  encoded tile is measured; it is published at the top cell of the chain. There is
  no duplicate measurement cache. `encodedDepth` keys the seed, schedule and depth
  G summaries, while `depth` keys publication. Sampling totals omit pass-through
  cells, preserving 200,000 samples at every occupied encoded depth. Content-less
  cells below the publication retain that tile's G through delivery.
- Cached-area endpoint roundoff is corrected only within 1,024 machine epsilons
  of the total area; a larger incomplete schedule returns a typed refusal. This
  numerical selection guard adds no geometric-error margin.
- Incomplete/nonfinite sampling is a typed refusal. Delivery requires v2 policy
  and summaries for HLOD manifests and validates counts, occupied encoded-depth
  totals, encoded-tile seed, distributions, margin, q and parent ordering. Historical H17 rows remain readable
  until S6 removes that producer. No simplifier estimate is a fallback.

## Decoded bytes and S3 handoff

Prepare the lattice once for centroid identity and encoding. The encoder carries
its actual face order through material grouping. Its existing GLB readback checks
that every published corner is bit-identical to the position prepared for that
indexed vertex and that every representative matches. Measurement consumes the
indexed positions reconstructed from that GLB readback, not the pre-encode
position array. A sensitive control changes only the expected measured position
while preserving valid GLB bytes and must fail before publication.

`SourceIndex::build(captured, check)` streams compact source triangles and fine
BVHs into capture-owned disk sequences. Compact records contain original f32
positions, rank, source provenance, expanded ordinal, material, patch control
mask and cached area. BVH nodes sum those areas once at construction. A tile's
area sums whole subtrees and scans at most two clipped endpoint blocks.

Each admitted job calls `SourceIndex::query(captured, TileKey)`. At most 8,191
trimmed upper nodes are resident. A complete hierarchy releases its disk-node
cache and admits up to 192 fine blocks; a deeper hierarchy retains a bounded
16,384-entry node cache and 64 fine blocks. Caches use FxHash and FIFO eviction;
nearest arithmetic never relies on hash iteration order. Traversal carries each
already-read child node on its stack, rather than visiting it again on pop.
All bounds and triangle candidates clip to the descendant source range.

Queries return source rank, barycentrics and squared distance. Ties use source
provenance (node/mesh/primitive/triangle), expanded ordinal, then rank. Prior
actual witnesses only seed an upper distance; full nearest search still runs.
`primitive(hit)` uses the compact witness directly, avoiding a full-row disk read
per centroid. `triangle(hit)` reads the original `FullTriangle` when S3 actually
needs UVs, colours, normals, flags, material or provenance. The captured owner is
available through the query. S3 separately charges its image cache.

Coarse IDs use the nearest source primitive at the final decoded centroid and
are duplicated across each face's corners. S3 can consume `CoarseTile`'s indexed
decoded positions, triangles, locked mask and representatives, plus versioned
G/q summaries. Chart spacing is `max(G/16, 1e-5 m)`; charts must preserve decoded
positions and representatives. No facing/material/donor-distance gate or
certificate is part of this nearest-surface API.

## Resource ownership and acceptance

The HLOD plan charges 8 MiB query workspace and 4 MiB measurement workspace per
worker, plus 2 MiB fixed construction workspace. Histograms, bounded patch
connectivity, source/proxy buffers, coarse BVH, unique edge/vertex controls and
centroid checks fit these allowances. Held tiles use S1’s exact vector-capacity accounting; no summary cache remains.
Query checks use a running total of retained fine-block capacities,
actual vector capacities and conservative hash capacities plus replacement slack;
a change that exceeds the allowance refuses instead of allocating silently.
Disk allowance is `160 * source_triangles + 1 MiB`, including larger compact
records/area nodes, added to S1's incidence envelope. Memory-backed scratch is
charged through the same S1 budget. Source index files have capture ownership
and disappear on success, refusal and cancellation.

Revised-S1 incidence is admitted before S2. Exact groups hold at most one item
per leaf; the stage charges three times that count for vector capacity and
reallocation. Hosts use the largest guest tolerance and the cube rounding floor;
double padding covers the predicate’s independent axial and perpendicular
extensions. Dense cells spill and split before bounded chunked scans, without a
density refusal. Their allowance is 256 bytes/source triangle with a 64 MiB floor.
A shared prospective charge additionally covers retained candidate bytes, full
probe and row-sort spill generations, and actual live dense spills. Every output
burst and dense write must fit the declared interface-stage envelope first.
Both recursive partition scans check cancellation every 256 records, even when
every candidate is pruned. The final stream envelope includes candidates and
the overlap allowance too.

Known analytic maxima cover tilted/offset planes and a polygonal cone against
its flat disc base. A triangular hole in a proxy ring has its maximum strictly
inside one source triangle; only area samples discover it. A closed tetrahedron
checks the actual source control path and unique proxy edges/vertices. Additional
controls cover tiny distant components, degenerates, extreme 1e8:1 slivers,
unsupported scheduling depth, collapsed-chain scheduling, encoded tile identity
and child floors. Resident and forced-paged nearest queries compare with exhaustive arithmetic through clipped
block ranges and preserve original donor rows. Cancellation checks occur within
256-sample batches, including zero-area surfaces. Compensated means remain finite.

Final crop receipts pin input/binary/production source, independent decoder and
oracle hashes. Independent seed 20261025 uses 200,000 samples/direction at cuts
0/2/4/6 with terminal leaves. These compare each cut's sampled maximum with its
maximum G; they do not independently certify every tile's own source range.
Full-model, appearance and integrated speed gates remain S8/S3 obligations.
CPU diagnostics use process CPU for index construction and summed worker CPU for
representatives and each directed pass. Collapsed cells incur no measurement CPU. CPU counters stay outside
deterministic publication metadata.

All local builds/tests/bakes run under a user systemd scope with `MemoryMax=10G`
and `MemorySwapMax=0`, at idle priority. HLOD crop bakes additionally use the
required measured slot 2 and explicit `--memory-budget 8GiB`.
