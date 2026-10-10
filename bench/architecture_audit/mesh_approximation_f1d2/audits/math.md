# F1d2 mathematical audit and settled proof representation

Preimplementation audit against coordinator-supplied base
`3b5703231bf3991c2e37caef99989214308cc08d`. Read `AGENTS.md`,
`docs/architecture/README.md`, F1d1's approximation contract and
`src/mesh_archive/approximation.rs`. The source hash and executed focused controls
are in [results.json](../math_probes/results.json). No Cargo or Git command was
run. This is a mathematical design decision and baseline source inspection,
not acceptance of a future implementation or a release claim.

## Required invariant and smallest representation

The domain remains nonempty finite decoded f32 triangle supports, each coordinate
in [-1,000,000, 1,000,000] local metres, compared independently per source region
in both directions. Multiplicity, winding, normal fields and topology are not
part of the metric. Every face, including points and segments, participates.

Use a private pure certificate owner, below candidate preparation. A patch owns
three barycentric corner rows `[u32; 3]` against one original f32 source face,
each row nonnegative with sum D=2^24, plus depth <=24. The root rows are the three
scaled basis vectors. An address is exact; a float midpoint is never its geometry.
The integer patch is the only authoritative source support representation.
Intervals reconstructed from those weights are temporary arithmetic evidence.
The producer has no need to retain a transcript or expose new public tuning knobs.

For rows a,b,c, compute ab=(a+b)/2, bc=(b+c)/2, ca=(c+a)/2 componentwise using
checked sums. The four children are `(a,ab,ca)`, `(ab,b,bc)`, `(ca,bc,c)`,
`(ab,bc,ca)`. At depth d, every integer coordinate is divisible by 2^(24-d);
therefore splitting at d<24 always uses exact integer halves. Each componentwise
sum is <=2D and fits u32. Child weights remain nonnegative with sum D. The
canonical child order and exhaustive target-face order make executions reproducible.

Use DFS, accepting a patch when its best complete-patch bound is <= the requested
budget, otherwise splitting it. No source face succeeds until all its pending
patches succeed. Across multiple faces and both directions the bound is the max
of all accepted-leaf bounds. This remains a conservative measured certificate;
it is not an estimate of the smallest possible bound. Retain the existing declared
positive root budget semantics separately from this actual bound.

## Complete coverage and convexity proof

In the canonical simplex, each corner child is the region where its corresponding
root barycentric coordinate is >=1/2; the centre child contains the region where
all three coordinates are <=1/2. Their union is the whole simplex, with overlaps
only on boundaries. Applying the source face's affine map gives exact parent
coverage even when that map collapses the simplex to a line or a point. Adaptive
acceptance yields a finite complete subtree: every rejected internal node has all
four children, and no accepted leaf has descendants. Consequently accepted leaves
cover every point of the original triangle, not just its vertices or samples.

For any one patch with exact corners p_i, choose three explicit witnesses q_i in
the same target triangle T. For x=sum(lambda_i*p_i), lambda_i>=0 and sum lambda_i=1,
y=sum(lambda_i*q_i) belongs to T. Then

`||x-y|| <= sum(lambda_i*||p_i-q_i||) <= max_i ||p_i-q_i||`.

Thus the largest certified corner residual bounds the entire patch to T. Taking
the minimum over target triangles is sound because each candidate separately
certifies the whole patch. Taking the maximum over a complete patch cover and
all source faces bounds directed surface distance. The maximum of the two
directed results bounds symmetric Hausdorff distance. Choosing a different target
face for each individual corner without this single-face requirement is unsound:
the interpolated witnesses need not belong to the nonconvex target union.

Uniform four-way subdivision is also sound, but consumes equal depth everywhere
and grows as 4^d even when most patches already certify. Longest-edge binary
splitting is sound with exact barycentric addresses, but adds edge selection,
ties and a more involved shrinkage argument. One witness plus a certified patch
radius is a simpler complete-cover bound, but is looser and still needs exact
source addresses and interval radius arithmetic. Exact overlay/clipping would
support zero-error coincident triangulations, but brings a substantially larger
robust predicate/intersection foundation. Threshold-guided four-way subdivision
is the smallest useful replacement for the current one-face-per-source-face bound.

## Arithmetic, witnesses and refusal meaning

Retain explicit target weights summing exactly to D=2^24. Target vertices and
edge/interior projections propose witnesses; projection is untrusted and never
establishes simplex membership or a distance guarantee. Validate integer weights
before interval reconstruction. Source corner coordinates now require interval
reconstruction from their original face as well. Interval subtraction must enclose
both intervals: `[source_low-target_high, source_high-target_low]`, with outward
rounding at each subtraction. The maximum absolute residual endpoint, outward
squaring, sum and final square root give the certified norm upper bound.

The previous exact-f32 source point signature cannot be used for a rounded patch
corner. A concrete admitted mixed-magnitude control shows why: averaging x=1e6
and x=2^-149 in f64 drops the contribution 2^-150. For the edge from `(1e6,0)`
to `(2^-149,1)`, the rounded midpoint shifts inward, and the four rounded children
omit the exact original edge midpoint. The failure is tiny but disproves exact
coverage. Integer barycentric addresses represent that midpoint exactly; intervals
then contain it. At depth <=24 products of f32 significands and integer weights
need at most 49 significant bits, and lie well inside f64's exponent range.
Still retain explicit outward product/sum operations for a straightforward
auditable enclosure rather than rely on informal exactness assumptions.

Identical original complete faces may retain the exact-zero convex-hull shortcut
for equal corner sets, including permutations/repetitions and degeneracy. A
floating midpoint matching a target vertex does not justify exact zero. Do not
add arbitrary epsilons, welding, coordinate clamps or skipped invalid queries.
General collinear or differently triangulated coincident supports may certify
only a positive bound, or fail a tiny budget. That is permitted by this finite
proof profile and must not be described as positive true Hausdorff error.

Compare the outward norm bound directly with the positive finite requested
budget. Squaring an arbitrary budget can underflow or overflow; a rounded or
outward squared threshold can admit distances above it. Nonfinite interval
arithmetic or malformed internal weights/indices are invalid state. A legal
candidate that cannot certify within the fixed work/depth profile is Unsupported:
the message must say that the bounded certificate could not prove the requested
maximum, not that the true error exceeds it. No lower bound is computed here.

## Finite convergence limits

With exact closest witnesses and unlimited subdivision, a patch of diameter d
has a one-target-face upper bound <= h+d, where h is the true directed Hausdorff
distance: pick one patch point p, a closest target point q in some T, and use q
for all three corners. Therefore uniform midpoint subdivision tends to h and
proves any budget strictly greater than h in finitely many levels. This is a
feasibility theorem for ideal witnesses and unbounded depth, not a universal
success theorem for the actual proposer or finite profile. Equality h=budget
does not itself imply finite acceptance.

Each four-way split halves the geometric diameter. The admitted coordinate box
has diameter <=2,000,000*sqrt(3). At depth24 terminal patch diameter is therefore
<=0.20648m; for ordinary metre-scale geometry it is much smaller. Depth12 instead
permits terminal diameters up to ~845.73m, and offers no simpler coverage proof.
Depth24 was selected because it is exactly the maximum supported by the chosen
u32 denominator and adds little DFS memory. The work cap still prevents a large
uniform tree from being explored.

A fixed target lattice independently limits precision. The exactly interior point
x=1 on segment [0,1e6] has true distance zero, yet its nearest D=2^24 witness is
x=17e6/D and residual 3481/262144 ~=0.013278961m. Source splitting cannot remove
that residual. An ideal dyadic approximation to continuous barycentric weights
has coordinate error controlled by triangle size/D, but the actual f64 projection
proposer has no proved global accuracy bound for ill-conditioned faces. Outward
interval widths create a further rounding floor. Do not claim that more source
subdivision guarantees arbitrarily small error, that every feasible proxy passes,
or that failure establishes a violation. Larger denominator, exact point queries,
overlay proofs and exact rational predicates are later changes needing their own
evidence; none is required for this finite tightening slice.

## Work and resource accounting

Retain checked base admission `2*sum(original_region_faces*proxy_region_faces)`
<=16,777,216 because the chosen exhaustive policy evaluates every root against
every target before splitting. This is a mandatory baseline, not an upper bound
on adaptive work. Charge one unit for each actual `(source patch,target face)`
evaluation, including equal-face fast paths. Check the global limit before the
next evaluation, including across regions/directions; never overrun then report
a smaller count. On success report consumed actual tests separately from the
root baseline. Equal-face short circuit may skip arithmetic within a test but
must not skip accounting under exhaustive scans.

For one root, full depth d would evaluate `(4^(d+1)-1)/3` patches. The finite work
ceiling, rather than this impossible worst-case allocation, bounds computation.
DFS processes source roots sequentially and needs at most 1+3*24=73 live patches.
This bounds certificate scratch, not the resident captured source/proxy arrays
or total process RSS. Cancellation checkpoints occur before work and at most
64 evaluations apart, with a final check before accepting the complete result.
The geometry/math owner knows no archive paths, jobs, CLI or serialization.

## Baseline dispositions

| Existing component | Disposition and basis |
| --- | --- |
| Region-independent bidirectional support metric | Retain; convexity and union proofs apply to that domain. |
| `directed` one whole source face to one target face | Replace with complete adaptive patch coverage; baseline planar control proves avoidable looseness. |
| `face_squared_bound` exact original corner-set shortcut | Retain only for original exact supports; do not apply to rounded reconstructed patch corners. |
| Exact-f32 `point_squared_bound` input | Rework into source coordinate interval plus untrusted representative for proposals. |
| `dyadic` integer membership / vertex-edge-interior proposals | Retain as explicit feasible witnesses, not as accurate closest queries; precision limits above. |
| `witness_squared_bound` source point subtraction | Replace subtraction with interval-versus-interval enclosure; retain outward norm pattern after independent controls. |
| Base `admit_pairs` | Rework ownership/name/report meaning; retain checked compulsory admission under exhaustive scans. |
| `performed == comparison_pairs` assertion | Remove; actual adaptive work exceeds root baseline. |
| Single module mixing candidate source semantics and proof math | Rework into a small private pure certificate module with validated finite supports/work profile. |
| Existing regression suite | Retain as controls only after its oracles are classified; it cannot establish complete adaptive coverage. |

## Executed controls and remaining acceptance obligations

[exact_controls.py](../math_probes/exact_controls.py) executed successfully with
6544 exact rational canonical-simplex coverage points, 100 random depth24 paths
checking integer sums/divisibility/area, and 1000 exact convexity controls. It
also establishes the admitted rounded-midpoint support defect, the 0.013278961m
target-lattice floor, and the interior-hole control: all corners of an 8m right
triangle lie on its boundary target, while its centroid has distance squared
32/9 from that target. These controls illustrate the analytic derivation; finite
points do not substitute for the coverage proof and this is not executed Rust
acceptance. Source/probe hashes are pinned in the results file.

Independent final-artifact acceptance can reconstruct a canonical complete patch
cover at the producer's reported bound C using exact rational closest-point
projections against decoded f32 target faces. Its complete-patch corner bounds
are <= every producer explicit-witness bound, so it should stop on each producer
accepted patch or an ancestor. The checker must compare squared rational distances
to exact C^2, preserve degeneracies, enforce complete coverage and independently
track limits. A checker depth greater than24 may be useful diagnostically but
must not conceal a production coverage failure. It need not consume a production
proof transcript. An old whole-face reference can exceed the new valid tighter
bound and is no longer a valid acceptance lower reference.

Before retaining final source, execute independent mixed-magnitude interval
enclosure checks, alternate/coincident triangulation controls, actual reduced
proxies with a tighter bound, interior hole/spike controls, degenerate points and
segments, depth/work boundary controls, and cancellation/accounting checks.
The source must separately prove its canonical split, interval enclosure,
checked work and accepted-leaf aggregation. Final build/artifact identities and
nonauthor review remain required; this audit author does not grant acceptance.

The analogous subdivision strategy is described in the primary
[CGAL bounded Hausdorff distance manual](https://doc.cgal.org/6.2/Polygon_mesh_processing/index.html).
CGAL's implementation is not used as a proof of this Rust arithmetic or profile;
the coverage, witness and finite-limit derivation above is specific to this slice.
