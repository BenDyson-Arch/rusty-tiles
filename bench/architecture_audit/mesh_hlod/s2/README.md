# S2 decoded source-error evidence

Candidate production source: `16b58964ff2d65f41e549c6fe8ad641a5b67404f`, based on
S1 `88036137d2b7676dffeb2ef20ffa2b32af5b4dcf`. Contract and resource ownership:
[`hlod-s2-error.md`](../../../../docs/architecture/hlod-s2-error.md).
The final receipt commit adds the independent archive preparation and this record;
it does not change the candidate production source. Executions are bound by
`execution-identity.json` to the release binary and both input hashes.

Bulk evidence root:
`/home/bend/.cache/rusty-tiles-evidence/hlod-s2-20261024/`.
[`evidence-sha256.txt`](evidence-sha256.txt) pins the execution artifacts by SHA-256.
The bulk `SHA256SUMS` also inventories receipts/logs, including input/binary identity,
compiler, environment, source-aware audits, publication/leaf identity and resource
observations. Published archives are retained there by their recorded hashes.
Scratch belongs only to `/home/bend/scratch/rusty-tiles-scratch/hlod-s2/` and is
removed after the campaign. No 2048 run or browser work is part of this slice.

## Method and limits

D2 samples each final decoded tile against captured source in its descendant
range, in both directions. There are at least 2,048 area samples/tile and
200,000/occupied depth/direction, deterministic tile-seeded area strata, plus
component/boundary/sliver controls. For sampled maximum M, larger area A and
scheduled count N, margin is `max(.05*M, .5*sqrt(A/N))`; published G is
`max(M + margin, children G)`. This is a sampling heuristic, not certification.
The spacing term follows the distance field's 1-Lipschitz property at the sample
scale; it is not a proven covering radius. The 5% term exceeds the prototype's
~1% independent maximum exceedance. q is diagnostic only.

The source index uses capture-owned disk sequences and bounded tile-local BVH
caches. Each admitted worker is charged 8 MiB for source queries and 4 MiB for
measurement; fixed construction is charged 2 MiB. Cache capacities are checked
against their allowance. Scratch adds `128*N + 1 MiB`. Original captured donor
rows remain available to S3 through rank/barycentric nearest witnesses.

## Executed results

Both inputs have 788,425 source triangles, 156 exact leaves and 63 parents.

| input | whole CPU-s | coarse CPU-s (per Mtri) | S2 CPU-s (per Mtri) | RSS GiB | wall s |
| --- | ---: | ---: | ---: | ---: | ---: |
| tex3 | 36 | 18.038 (22.88) | 16.113 (20.44) | 1.11 | 25.9 |
| crop3 | 27 | 21.022 (26.66) | 18.554 (23.53) | 0.22 | 23.2 |

**S2 misses the proposed 5 CPU-s/Mtri allocation.** The current coarse stage is
below 30 CPU-s/Mtri in both crops and above the 20 stretch target. Neither
observation accepts the complete future coarse stage including S3/S4.

Both inputs publish the same G and sampled maxima (metres):

| depth | parents | maximum G | maximum sampled M | area samples/direction |
| --- | ---: | ---: | ---: | ---: |
| 0 | 1 | 1.429247317 | 1.360462914 | 200000 |
| 1 | 1 | 1.429247317 | 1.361187920 | 200000 |
| 2 | 1 | 1.429247317 | 1.361187920 | 200000 |
| 3 | 1 | 1.429247317 | 1.361187920 | 200000 |
| 4 | 8 | 0.276515808 | 0.255958554 | 200003 |
| 5 | 24 | 0.088683418 | 0.067290299 | 200013 |
| 6 | 23 | 0.028292967 | 0.011981022 | 200014 |
| 7 | 4 | 0.004494989 | 0.002236699 | 200002 |

Independent decoded cut results are identical for tex3/crop3 (metres, pooled
400,000 samples/cut; raw receipts also retain each direction):

| cut | triangles | mean | p95 | p99 | max | maximum published G | exceedance |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| 0 | 8182 | 0.019074406 | 0.050062537 | 0.071019990 | 1.332443906 | 1.429247317 | False |
| 2 | 8182 | 0.019126394 | 0.050171281 | 0.071090215 | 1.340423161 | 1.429247317 | False |
| 4 | 65510 | 0.004710033 | 0.016959218 | 0.025751583 | 0.201128774 | 0.276515808 | False |
| 6 | 651656 | 0.000081813 | 0.000166384 | 0.001596799 | 0.009414346 | 0.028292967 | False |

The cropped prototype has root mean/p95/max 0.013265/0.036122/0.390762 m.
At cuts 2 and 4 its maxima are 0.169848 and 0.029096 m; it terminates at depth
5, with essentially zero leaf distance. Those cuts differ from this ownership
tree (and its root has 15,970 triangles versus 8,182 here). This comparison
records the geometry difference; it is not an equal-budget quality claim.

The 250 ms scratch monitor observed 736,925,294 logical bytes (739,094,528
allocated) beneath the declared 6 GiB campaign bound; this is a sampled
observation, separately from the producer's exact scratch admission counters.
All 219 decoded members are at most 8,192 triangles. Both archives have zero
leaf-byte differences and monotone errors. Independent root mean/p95/max meet
the stated crop quality thresholds, with no sampled G exceedance in the four
audited cuts. Sampling does not rule out other extrema.

The independent audits used CPUs 4,5 at idle priority, with two numerical
workers: crop3 14.503 CPU-s / 20.401 wall s; tex3 14.216 CPU-s / 18.304 wall s.
This cost is separate from all bake and S2 CPU figures.

Validation: clippy `--all-targets -- -D warnings`; `cargo test --lib mesh_large`
(322 passed, 6 ignored); `h26_mesh_core`, `h26_mesh_frontends`, `golden`,
`output_digests` (24 passed, 1 ignored recipe exporter); formatting/tree budgets.
Tests use the requested Cargo debug settings plus test opt-level 2, two test
threads and CPU affinity 4,5. Analytic maxima match within 1e-12 and G is inside
the documented margin. Jobs/budget determinism, exact-minimum admission and
one-below refusal/cleanup passed.

Measurements use balanced, release, `--lod hlod --memory-budget 8GiB --jobs 2`,
idle scheduling, and the required `RT_MEASURE_SLOT=2` wrapper (CPUs
`0-3,16-19`). Whole CPU comes from user+sys; S2 reports index process CPU plus
worker-thread CPU, never worker wall multiplied by jobs. Slot 2 is the requested
loaded correctness lane, not a quiet hardware performance acceptance lane.
These crops establish no full-model, appearance, transfer or integrated S8 pass.
The S2 diagnostic allocation is 5 CPU-s/Mtri; a miss remains explicit future
performance work. The current coarse stage excludes S3/S4 appearance/codecs.

## Independent reference and sensitive controls

`unpack.py` reads the published explicit crop archive, verifies contiguous source
ranges, monotone errors and its complete GLB inventory, then compares ordered leaf
SHA-256s with S1's captured leaf inventory. That inventory was independently
matched by S1 to develop `cda7ca6110c714f8eaceea04df8f5945eb6443c8`; this campaign
checks S2 against those same bytes. It does not repeat the historical H17 bake.

`decode_geometry.mjs` uses the separate JavaScript meshoptimizer decoder on those
published GLBs and the source. `audit.py` uses numpy seed 20261025 and
point_cloud_utils closest-point queries: 200,000 samples/direction at cuts 0, 2,
4, 6, carrying terminal leaves forward. It does not reuse Rust nearest arithmetic,
producer samples or producer facts. The JS oracle deliberately supports only the
fixtures' identity/translation/scale node domain; archive preparation supports
these explicit crops. General scene/forest auditing belongs to S5.

Example (all commands run at idle priority):

```sh
python3 -I unpack.py out.3tz STAGE BASELINE_LEAVES.json identity.json
node decode_geometry.mjs INPUT.glb STAGE DECODED
OMP_NUM_THREADS=2 OPENBLAS_NUM_THREADS=2 python audit.py DECODED audit.json
```

The prototype comparison pins `hlod-final-{tex3,crop3}/quality.json` from the
bake-off evidence. Its root has 15,970 triangles versus this tree's 8,182; depth
and ownership differ. Its cut values are context, not a matched geometry regression
baseline. Independent exceedances, if any, are reported in the cut receipts.

Analytic tests independently know tilted/offset plane maxima (.25/.50 m), the
unit sphere cap's sagitta (`1 - .8f32`, about .20 m), a distant degenerate point
(sqrt(109)), and a 1e8:1 sliver's interior zero distance and 2 m normal offset.
Controls attain the analytic maxima within 1e-12 and G covers each within the
stated margin. Different seeds preserve the analytic maxima. The source query
suite compares resident and forced-paged caches through partial block ranges;
the sliver analytic control does not reuse closest-triangle arithmetic.

Sensitive delivery controls understate G, counts, q or margin, remove all S2
metadata, and reduce internally consistent non-root totals below 200,000. All
refuse before publication. Tests also interrupt inside both positive/zero-area
sample loops, verify exact depth schedules, child/leaf floors, finite summaries,
compensated means, jobs/cache/budget determinism, exact budget admission and
one-below refusal/cleanup. Existing H16 tests cover explicit/implicit, external
and synthetic error propagation; the planner preserves G unchanged.

## Review and remaining gates

A first independent review found four P2 issues: sliver cancellation, removing S2
metadata bypassing validation, incomplete non-root depth totals, and unbounded
sample-loop cancellation latency. All were corrected with sensitive controls.
A fresh second source review found no actionable correctness issue in the
optimized resident/disk queries and cache charges. The final source/evidence
review is recorded in the bulk receipt. Reviews are static inspection, separate
from the executions above.

This intermediate preserves S1 geometry and leaf encoding, changes coarse
representative IDs to decoded-centroid nearest-source primitives, and publishes
measured G plus summaries. S3 can consume the bounded source-query API, original
source attributes/material/provenance, and chart spacing `max(G/16, 1e-5 m)`;
its image cache needs its own budget charge. Full-model gates and appearance
remain S8/S3 obligations. Revised S1 integration is conditional on publication
of its revised base; see the final PR for the exact base used.
