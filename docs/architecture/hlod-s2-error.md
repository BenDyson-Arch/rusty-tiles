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

### Complete directional covers

Each source point needs one target witness. The worker tries two geometrically
sufficient tests on each closed domain triangle or midpoint panel.

1. Choose one actual target triangle. Snap a target point for each panel corner
   to nonnegative barycentric coordinates summing to 2²² integer units. Affine
   interpolation stays on that triangle. The largest corner residual bounds the
   Euclidean residual on the entire panel, including a segment or a point.
2. Project along a coordinate axis and start in an actual target face whose
   footprint has positive-area overlap with the panel. Traverse equal 3D edges.
   Every edge crossing the open query must have a visited face on its opposite
   projected side. The visited footprints form a finite closed union. Away
   from projected vertices, a covered point in the open query has a neighborhood
   covered by a face interior or by two faces across an edge. The open triangle
   minus finitely many vertices is path-connected, so one overlap covers this
   whole set. Closure adds its edges and vertices. Within each overlapping
   face, bound the separation along the dropped axis at common projection.
   Subtracting the same affine function of the two retained coordinates leaves
   this separation unchanged. Extrema of each affine residual occur at corners.
   Intersecting projected bounding boxes can tighten the bound. Each height
   difference prices an actual **3D** witness. A fold, a hole, a degenerate
   projection, or the 1,024-face limit can prevent this test from certifying a
   panel. A split panel rounded to binary32 for projection also pays its
   largest actual corner displacement.

The construction floor is a search target. Its budgeted point searches can
overestimate nearest distance. A separate lower-bound observation excludes
*every* target leaf with an enclosing box or supporting half-space before it
can raise the target to 1.01 times that lower bound. A paid upper from an
already accepted panel can also guide the remaining work. A panel stops when
it has a complete upper under the target. Otherwise it can split four ways.
At eight levels or exhaustion of its assigned split credits, it publishes the
complete simplex upper. The extra level resolves narrow edge incursions on
long retessellated faces. The total credits for the range stay fixed. A work
cap can widen the result beyond 1%. The formal relative gap applies when
the outward-rounded upper meets the relative target. Every emitted value
comes from actual witnesses across the whole domain.

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

Pricing assumes finite IEEE binary32 coordinates, native binary64 evaluation
with round-to-nearest and disabled contraction. The build checks the formats
and evaluation precision. Each native entry checks the rounding mode.
Projected side tests first use a conservative determinant filter. An ambiguous
sign uses an error-free expansion of six binary32 products, each exactly
representable in binary64. A nonzero product cannot underflow binary64.

Let M be the maximum absolute coordinate of the target and the entire domain
range, and u = 2⁻⁵³. Eight midpoint levels introduce at most 9uM per
coordinate. Simplex interpolation and evaluation of its norm cost less than
50uM. For projection, all selected affine slopes have magnitude at most eight.
A difference of two coordinates costs up to 2uM. Multiplying it by one slope
adds at most another 16uM of rounding. The two products and two sums put a
single affine residual within 128uM. Differencing two residual intervals and
adding both coordinate-box slope corrections stays below 768uM. These are
absolute bounds and also cover cancellation. The returned directional price
adds 2,048uM and rounds upward, including norm evaluation and the optional
panel displacement. Lower-bound exclusions receive the scale of both surfaces
and downward padding. Invalid ownership, arithmetic or allocation, cancellation
and observer failure stop publication.

### Admission and progress

Each admitted tile reserves 512 bytes per possible reference or target face,
plus 8 MiB. The native layout check includes the two index lifetimes, triangle
arrays, BVH nodes, sorted keys, edge table and up to four visit maps per
direction. The scratch edge hash and radix buffers have disjoint lifetimes. Construction can
retain two candidate face streams before either native index is allocated.
The target uses the proxy cap. For larger inputs the reference charge uses
the greater of 131,072 faces and the maximum bounded child frontier. This rule
is independent of memory budget and worker count. Held tiles charge up to 36
additional bytes per triangle if readback differs from construction, inside
the existing held-tile spill lease. Certification adds no scratch file.

For a direct unwelded comparison, construction's mandatory S2 scan supplies
every original face. A rank map checks completeness and uniqueness. Existing
S2 hits can seed a search, and the certifier checks that witness anew on the
delivered coordinates. An active weld uses the original-position mapping
described above. The two directional indexes are immutable and run concurrently
on the operation's pool. Each direction uses up to four fixed contiguous
ranges, one per 8,192 faces rounded up; their split credits partition
`max(65536, 256 × max(domain faces, target faces))`. One worker uses the same
ranges and ordered maximum. Source reads, index construction and panel descent
poll cancellation. `hlod_certify` reports completed faces across both directions
and a 250 ms heartbeat where the worker polls; `certification_cpu_by_depth`
sums preparation, index and range thread times by encoded depth. The S2 timer
includes capturing original faces during its traversal.

### Checked mathematics

The Lean development in
[`bench/architecture_audit/mesh_hlod/s5/proof`](../../bench/architecture_audit/mesh_hlod/s5/proof)
proves the closed-panel and four-way cover, directed composition, the
two-sided depth induction and Hausdorff bound. It also proves the projected
seam and connected-cover arguments, the equality of axial separation and 3D
distance at common projection, affine corner and slope bounds, and the
conditional relative stopping bound. Its hypotheses include face identity
and range partition, genuine simplex witnesses or projected neighborhoods,
and numeric enclosures. The Lean theorems work in real arithmetic. The
decoder, floating-point implementation and its error budget are checked
separately. With Lean 4.36.0 and Mathlib at
`fc4f59e577feb1a4956671bc04ac18aaaf8cd842`, use a Mathlib checkout with its
dependencies built and set `proof` to the absolute repository proof directory.

```sh
out=$(mktemp -d)
lake env lean --root="$proof" -j 1 -DwarningAsError=true -o "$out/HonestG.olean" "$proof/HonestG.lean"
lake env sh -c 'LEAN_PATH="$1:$LEAN_PATH" lean --root="$2" -j 1 -DwarningAsError=true "$2/FastCover.lean"' sh "$out" "$proof"
```

Border relief and chord changes precede measurement of the delivered geometry.
The geometric certificate therefore prices their surface displacement. The
seam contract still checks common interface identity and mixed-depth gaps.

## S2 construction measurement

- Measure captured source triangles in `[TileKey.first, TileKey.end)`, in the local
  metre frame of the final decoded proxy. Child G and leaf encoding errors are floors in the construction calculation.
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
