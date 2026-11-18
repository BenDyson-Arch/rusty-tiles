# Certified publication and S2 construction error

## Publication contract

Let S be the union of the **closed** source triangles in the retained H1 rank
interval [first, end), using their original pre-weld positions and including
any disconnected or degenerate faces. Let P be the union
of the closed triangles drawn by the encoded GLB, after decoding indices,
positions and local node transforms. The coordinate frame is captured local
binary32 metres. Survey accuracy, upstream placement and shading are separate
quantities. The producer bounds both suprema of Euclidean point-to-surface
distance. The published G is the maximum of this certificate, the S2
construction floor, the applicable texture floor and the greatest published
child G. An omitted cell keeps its existing refinement-only metadata.

The source count selects one comparison. At most 131,072 original faces use
S directly. Larger counts use the complete union C of the delivered child
frontier. The worker verifies contiguous nonempty source and leaf intervals
through the parent's end. In either case the target is the final GLB surface.
Readback checks a bijection of faces and bit-identical positions at all three
corners. A held coarse tile keeps the read-back coordinates when they differ
from its construction positions. A held leaf uses the canonical coordinates
checked by its frozen lattice encoder, including aligned primitive rebasing.

For each directed comparison, cover each domain triangle by closed panels.
Each accepted panel chooses one actual target triangle and a target point for
each of its three corners. The three points have integer barycentric weights
summing to 2²². Interpolating them with a panel point's barycentric coordinates
keeps the witness in that target triangle. The norm of the residual is at most
the largest of the three corner residuals. This argument includes segments and
points. Four midpoint subtriangles cover a split panel. A bounded nearest
search (768 BVH node visits per query), depth limit seven and a per-direction
split allowance of `max(65536, 256 × max(domain faces, target faces))` guide
the price. At a work limit the current panel receives its complete corner
price. Run this comparison in each direction.

For an original-source comparison, take the larger directional upper bound.
For a child comparison let L bound both directions between P and C, and E be
the greatest child's geometric certificate. Each point in the union of the
original child ranges has a point within E in the matching decoded child.
The converse holds for every point in C. Applying the triangle inequality
in each direction yields H(S,P) ≤ E + L. Round the sum upward. At a leaf,
pair corresponding corners and use the outward maximum encoding displacement
with the same barycentric argument. Direct comparisons reset accumulated
slack. The private stage row records both local directions, the reference type
and size, and E. Its consumer checks the certificate arithmetic and source
count before delivering G as binary64 in explicit or implicit metadata.

### Opt-in source welding

A direct reference uses the original mapped binary32 corners of every assigned
rank before the source weld, including triangles collapsed or dropped during
simplification. The weld's sorted move pass writes 12 bytes per genuinely
moved expanded vertex. A rank bitmap and prefix counts index that file, and
primitive offsets distinguish primitive instances. The retained RAM charge
covers 16 bytes per 64 expanded vertices plus one sentinel pair, eight bytes
per primitive offset including its sentinel, and 4 KiB for the file. The
capture scratch reservation conservatively assumes every considered vertex
moves. A run with no moved vertices uses the ordinary source index directly.

S2 construction and appearance use the welded input. Each prepared leaf
primitive pairs original corners by their authored IDs with the final decoded
coordinates verified by the leaf writer. The maximum outward corner residual
covers its closed faces in both directions. The walk uses the separate
construction error for geometry decisions and the original-source certificate
for publication and induction. Delivery uses these values directly. The weld
audit still reports the largest move, collapsed and coincident faces and their
reach. Its job-wide additive charge is zero.

### Numerical enclosure

The pricing operations use finite IEEE binary32 coordinates, IEEE binary64
round-to-nearest, and disabled contraction. The kernel checks the arithmetic
mode and format. Write M for the largest absolute input coordinate and
u = 2⁻⁵³. Seven rounded midpoint levels cost at most 8uM per coordinate.
The three dyadic weights are exact. The target interpolation costs at most
6uM per coordinate, the subtraction 3uM, and norm evaluation adds less than
12uM. The total norm error is below 50uM. All nonzero pricing coordinates
are on a grid at least as coarse as 2⁻¹⁷¹, and squared norms of nonzero
residuals are within binary64's normal range. The terminal directional price
adds 512uM and rounds up. The BVH and approximate projection only select a
target face and propose weights. Clamping and integer snapping establish
simplex membership before pricing. A nonfinite bound, invalid ownership,
allocation failure or cancellation stops publication.

### Admission and progress

One admitted worker reserves 512 bytes per possible reference or target face
plus 8 MiB. The target count uses the proxy cap. When the complete input fits
the direct cutoff, its face count bounds every reference. For larger inputs
the reference count is the greater of 131,072 and the maximum bounded child
frontier. The cutoff limits the original-source contribution to 64 MiB per
worker and gives the same choice across worker counts and memory budgets. The two directional BVHs run sequentially.
`certificate_workspace_bytes` identifies that term in the HLOD memory plan.
Held tiles account for up to 12 extra bytes per vertex (36 per triangle)
when readback differs from construction. Such overrides and their scalar
error use the existing held-tile spill lease, including a 9-byte header
extension. The certifier creates no separate scratch sequence. Progress
reports completed domain faces across workers and a 250 ms heartbeat where
the worker polls. Source iteration, BVH construction and panel descent check
the same cancellation and observer errors. Where a per-thread CPU clock is
available, `certification_cpu_by_depth` sums preparation and both cover passes
by encoded depth, separately from S2 construction and texture baking.

Border relief and chord changes precede measurement of the delivered geometry.
The geometric certificate therefore prices their surface displacement. The
seam contract still checks common interface identity and mixed-depth gaps.

## S2 construction measurement

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
- With M the maximum sampled distance in either direction, controls included,
  construction uses `max(children construction G, 1.05 × M)`. Narrow
  unsampled extrema can exceed this floor. Final publication applies the
  continuous certificate above.
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
  margin, q and parent ordering. A coarse staging directory must carry the new
  surface-policy declaration and certificates before delivery. An existing
  delivered archive retains the checks supported by its own declarations.

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
found is used. It is never below the exact distance, so a sampled distance can grow; a representative may then be a near, not the nearest, primitive.
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
`max(1.25·G/16, 1e-5 m)` (D5, amended 2026-10-30) and must keep decoded positions and representatives.

## Resources

- **Scratch:** the index is the walk scratch component `s2-source-index`, admitted
  through `ScratchLedger::admit` for `160 * source_triangles + 1 MiB` before its first
  write. Every block and node write is charged to the lease first; a write past the
  admission is refused. Memory-backed scratch charges it to memory in the plan. S2
  writes no other scratch. Files are capture-owned and removed on success, refusal
  and cancellation.
- **Memory:** each parent job is charged 16 MiB query caches, 4 MiB measurement
  workspace and the search frontiers (`SEARCH_BYTES`, at most `2B+1` entries each);
  2 MiB fixed construction. Query caches check actual capacities and refuse past
  their allowance.
  S1b's internal budget tuning retains 384 fine blocks when the upper hierarchy
  is completely resident; the paged path retains 64 blocks. These fixed budgets
  do not change B, sampling, nearest-search order or source-scale bounds.
- **CPU:** index construction uses process CPU; representatives and each directed
  pass sum worker-thread CPU. CPU stays out of deterministic metadata.

Builds, tests and bakes run in a user systemd scope with `MemoryMax=10G`,
`MemorySwapMax=0`, at idle priority; crop bakes use `--memory-budget 8GiB --jobs 2`.
