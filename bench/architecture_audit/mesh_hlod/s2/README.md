# S2 decoded source-error evidence — policy v2

Measured production commit: `30c07bf70b0f52996cdccbc949f035f46d70d83a`, based on S1
`88036137d2b7676dffeb2ef20ffa2b32af5b4dcf`. Contract:
[`hlod-s2-error.md`](../../../../docs/architecture/hlod-s2-error.md).
The later receipt commit changes documentation/hash receipts only. Production,
input, binary and oracle identities are pinned separately.

Bulk evidence root:
`/home/bend/.cache/rusty-tiles-evidence/hlod-s2-v2-20261024/`.
[`evidence-sha256.txt`](evidence-sha256.txt) pins immutable execution artifacts.
`execution-identity.json` records the release binary, inputs, Cargo settings,
profile, jobs, scope and measured lane. `production-source-sha256.json` pins every
production path. Archives remain in the evidence cache by hash. Prior v1 evidence
is historical at `../hlod-s2-20261024/`, measured source `16b58964`; its area-spacing
margin and old performance figures are superseded here.

## Method and resource contract

The coordinator's client-speed decision removes additive area-spacing inflation:
`G = max(children G, 1.05*M)`, recorded margin `0.05*M`. M is the maximum of
source→proxy and proxy→source samples/controls on final decoded triangles in the
descendant source range. This is sampled error, not certification. At least 2,048
area samples/tile and 200,000/occupied parent depth/direction remain mandatory.

Version 2 reuses an immediate child's actual measured summary only when source
range, complete published GLB SHA-256 and sufficient sample count agree. Actual
seed/count and original `reusedFromDepth` are retained; recompute the child/leaf
floor. The published inventories have two such identical parent/child pairs in
each crop (depths 1→2 and 2→3); `reuse-eligibility.json` records their exact gates.

Source controls cover bounded 256-triangle patch boundaries, a vertex per patch
component and sliver vertices/midpoints. Every true source boundary/component is
covered; artificial patch boundaries/components retain extra conservative checks.
This avoids a global resident connectivity graph. Clipped endpoint blocks retain
all corners. FxHash sets deduplicate checks without changing distance/tie meaning.
The root source controls fall from 533,100 to 264,116. Proxy vertices/undirected
edges are checked once: root queries fall from 30,472 to 16,953; 8,182 already
computed representative centroid hits are additional controls with no repeat query.

Compact records include material and cached area. Centroid primitive selection
avoids full-row disk reads; source donor rows stay available to S3 on demand.
BVH nodes cache summed area; each tile scans only clipped endpoint blocks for area.
Traversal carries already-read children, and cache charging keeps a running
fine-block capacity total. Charges remain 8 MiB/query + 4 MiB/measurement/worker,
2 MiB fixed construction, with `160*N + 1 MiB` disk allowance. Fine caches admit
192 blocks with a complete resident hierarchy or 64 with the bounded paged cache.
The existing held-tile slack includes the fixed cached summary/hash.

The lattice is prepared once. `encode::check` decodes the actual GLB, verifies
face mapping and representative IDs, asserts every corner bit-identical to the
prepared indexed position, and returns readback positions for measurement.
A control corrupting only the expected measured position must refuse publication.
Depth >=32 and genuinely incomplete schedules are typed resource refusals;
roundoff correction is bounded and adds no geometric-error margin.

## Executed crop results

Each input has 788,425 source triangles, 156 exact leaves and 63 parents.

| input | whole CPU-s | current coarse CPU-s (/Mtri) | S2 CPU-s (/Mtri) | RSS GiB | scope peak bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| tex3 | 28 | 10.786 (13.68) | 8.929 (11.33) | 1.12 | 1,565,429,760 |
| crop3 | 19 | 14.476 (18.36) | 12.131 (15.39) | 0.22 | 530,731,008 |

**The proposed 5 CPU-s/Mtri S2 allocation is still missed.** Prior figures were
20.44/23.53 CPU-s/Mtri (tex3/crop3); current values are 11.33/15.39. These measured
slot-2 observations are not quiet-lane performance acceptance. Current coarse
cost excludes future S3/S4 appearance/codecs and establishes no integrated S8 pass.
All bake scopes have `MemoryMax=10G`, `MemorySwapMax=0`, and zero max/OOM events.
Bakes use idle priority, release, balanced, `--jobs 2 --lod hlod --memory-budget 8GiB`.

Both inputs publish identical maximum errors (metres):

| depth | parents | maximum G | maximum sampled M | area samples/direction |
| --- | ---: | ---: | ---: | ---: |
| 0 | 1 | 1.429247317 | 1.360462914 | 200000 |
| 1 | 1 | 1.429247317 | 1.361187920 | 200000 |
| 2 | 1 | 1.429247317 | 1.361187920 | 200000 |
| 3 | 1 | 1.429247317 | 1.361187920 | 200000 |
| 4 | 8 | 0.268756482 | 0.255958554 | 200003 |
| 5 | 24 | 0.070643549 | 0.067279570 | 200013 |
| 6 | 23 | 0.011989462 | 0.011418535 | 200014 |
| 7 | 4 | 0.002348534 | 0.002236699 | 200002 |

The independent decoded cut results are identical for both inputs (metres,
400,000 pooled samples/cut; each direction is also retained):

| cut | mean | p95 | p99 | max | maximum G | max > cut max G |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| 0 | 0.019074406 | 0.050062537 | 0.071019990 | 1.332443906 | 1.429247317 | False |
| 2 | 0.019126394 | 0.050171281 | 0.071090215 | 1.340423161 | 1.429247317 | False |
| 4 | 0.004710033 | 0.016959218 | 0.025751583 | 0.201128774 | 0.268756482 | False |
| 6 | 0.000081813 | 0.000166384 | 0.001596799 | 0.009414346 | 0.011989462 | False |

Independent audit cost is separate: crop3 12.207 CPU-s / 8.893 wall s;
tex3 12.093 CPU-s / 8.699 wall s, pinned to CPUs 4,5 with two numerical workers.
The audit scope is also capped at 10 GiB with swap disabled and zero limit/OOM events.

All 219 published GLBs in each archive are byte-identical to v1, including every
leaf and coarse position/ID. G and summaries change in publication metadata.
Ordered leaves also match S1's inventory verified against develop `cda7ca61`.
`identity-check.json` verifies source ranges, monotone G and complete inventory;
`geometry-unchanged.json` pins the comparison with the retained v1 receipts.
Every decoded member remains at most 8,192 triangles. The cropped prototype had
15,970 root triangles versus 8,182 here, with different ownership and essentially
exact prototype leaves. Its root mean/p95/max .013265/.036122/.390762 m and cut-2/4
maxima .169848/.029096 m are context, not an equal-budget regression baseline.

The 250 ms bake/audit scratch monitor observed 555,212,091 logical bytes
(556,888,064 allocated), below the declared 6 GiB campaign bound. Unit-test
fixture scratch is separate. Published archives were preserved by hash and the
owned campaign scratch root was deleted.

## Checks, independent reference and limits

Clippy `--all-targets -- -D warnings` passed. `cargo test --lib mesh_large` passed
328 tests (6 evidence-only ignores); `h26_mesh_core`, `h26_mesh_frontends`, `golden`
and `output_digests` passed 24 (1 ignored recipe exporter). Test compilation/runs
were in a capped scope: peak 5,738,549,248 bytes, swap disabled, zero max/OOM events.
Cargo uses the requested target/jobs/debug settings plus test opt-level 2, two
test threads and two-CPU test affinity. Focused HLOD tests additionally exercised
requested jobs 1/4 and admitted budget differences.

Analytic plane maxima .25/.50 m and polygonal cone height `1-.8f32` match within
1e-12. A triangular proxy hole has known inradius `2/(1+sqrt(5))` strictly inside
one source face: its source controls see zero; area samples find the interior
maximum within 5%, and G covers it. A closed tetrahedron exercises source controls
(one component vertex) and proxy controls (four vertices, six edges). Grid controls
skip interior vertices; sliver/degenerate/distant-component controls remain.
Tests cover reuse facts/floors, unsupported depth, incomplete schedules, cancellation,
malformed/removed summaries, occupied depth counts, cache/range correctness,
job/budget determinism, exact-minimum admission and one-below refusal/cleanup.

`unpack.py`, `decode_geometry.mjs` and `audit.py` are independent of producer
nearest arithmetic and sampling. They read the actual explicit crop archive,
use separate JS meshoptimizer decode, numpy seed 20261025 and point_cloud_utils
closest points, 200,000 samples/direction at cuts 0/2/4/6 with terminal leaves.
The oracle supports these identity/translation/scale explicit crops; general
scene/forest validation belongs to S5. Cut comparisons use each cut's maximum G,
not every tile's own G/range. No source-distance certificate is claimed.

Final independent review and scratch cleanup are recorded in the bulk receipt.
Revised S1 integration remains conditional on publication of that branch revision;
the tested base is explicit above. S3 can use the source query/donor API, original
attributes/material/provenance, representative IDs and versioned G/q summaries,
with chart spacing `max(G/16, 1e-5 m)` and its own image-cache charge. Full-model,
appearance, transfer and integrated performance acceptance remain S8 obligations.
