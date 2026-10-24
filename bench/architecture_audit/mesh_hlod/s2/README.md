# S2 decoded source-error evidence — revised S1 integration

Measured production commit: `560283e205dc50d73f454d68ad18096e9bcbc12c`. Integrated S1:
`85bdd4f6811804f76fd49e9e16276c5410a1b959` (including the owner's revised chain, incidence and
leaf-cache changes). Contract: [`hlod-s2-error.md`](../../../../docs/architecture/hlod-s2-error.md).
Later receipt commits change documentation/hash manifests only. The production
source manifest pins every executable path separately from inputs, binary and
oracle helpers. `execution-stages.json` distinguishes the historical pre-cancellation
focused/library checks from all final-source checks.

Bulk evidence root:
`/home/bend/.cache/rusty-tiles-evidence/hlod-s2-s1-final-20261024/`.
[`evidence-sha256.txt`](evidence-sha256.txt) pins the retained artifacts and both
archives. Prior v1, v2 and intermediate integration campaigns remain historical
in adjacent cache directories; their costs and geometry inventories are superseded
by this campaign. No revised-S1 integration condition remains.

## Contract and integration

Publish `G = max(children G, 1.05*M)`, where M is the maximum of both directed
area samples and controls against the descendant source range, measured on actual
decoded GLB positions. The 5% inflation is heuristic, motivated by the prototype's
roughly 1% independent sampled exceedance. It is not a source-distance certificate.
Every occupied encoded depth has at least 200,000 area samples/direction, with
2,048 minimum per tile; deterministic seeds and schedules use `encodedDepth`.
Publication `depth` can differ after S1 hoists the deepest tile over its identical
chain. Pass-through cells inherit that encoded tile's G. The duplicate-reuse
cache was removed: collapsed cells perform no S2 measurement.

Source controls cover patch boundaries, patch components and slivers in bounded
256-triangle patches. Proxy vertices and edges are unique; representative centroid
hits are reused as controls. Compact material/area records, cached subtree area,
FxHash and bounded nearest caches avoid repeated disk reads. The source index and
original donor attributes/provenance remain available to S3. S3 can use decoded
positions, representatives and G/q with spacing `max(G/16, 1e-5 m)`.

Prepare encoding once; GLB readback checks corner positions bit for bit and returns
the measured indexed positions. Unsupported depth >=32 returns a typed refusal.
Query/measurement charges are 8 MiB + 4 MiB per worker, 2 MiB fixed construction,
plus `160*N + 1 MiB` disk allowance, charged through S1 including memory-backed
scratch. Exact incidence groups charge per-leaf capacity and reallocation. Dense
joins spill/split/chunk; shared prospective accounting checks candidate, probe,
row-sort and dense output growth before writing. Broad-phase padding covers both
axial/perpendicular endpoint tolerance. Both recursive partition scans check
cancellation every 256 records, including pruned candidates.

## Fresh crop executions

Both inputs have 788,425 source triangles, 156 unchanged leaves and 61 encoded
parents (two fewer after chain collapse). No encoded tiles occupy depths 1–2.

| input | S2 CPU-s | S2 CPU-s/Mtri | current coarse CPU-s/Mtri | RSS GiB | bake scope peak bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| tex3 | 9.517036 | 12.071 | 14.895 | 1.10 | 1445101568 |
| crop3 | 9.291122 | 11.784 | 14.390 | 0.22 | 476303360 |

The proposed 5 CPU-s/Mtri S2 allocation is still missed. These slot-2 observations
are not quiet-lane acceptance. Current coarse cost excludes future S3/S4 work.
All commands use idle priority and user systemd scopes with MemoryMax=10G,
MemorySwapMax=0; retained cgroup receipts report zero limit/OOM events. Bakes use
release, balanced, jobs 2, `--lod hlod --memory-budget 8GiB`.

Both inputs publish the following identical maximum errors (metres), keyed by
encoded depth. Content-less publication cells retain their ancestor tile's G.

| encoded depth | encoded parents | max G | max sampled M | area samples/direction |
| --- | ---: | ---: | ---: | ---: |
| 0 | 1 | 1.42904584107 | 1.36099603912 | 200000 |
| 3 | 1 | 1.42904584107 | 1.36099603912 | 200000 |
| 4 | 8 | 0.268756481952 | 0.25595855424 | 200003 |
| 5 | 24 | 0.0706435486373 | 0.0672795701308 | 200013 |
| 6 | 23 | 0.0119894619624 | 0.0114185352023 | 200014 |
| 7 | 4 | 0.00234853407257 | 0.00223669911673 | 200002 |

The independent decoded cut audit uses numpy seed 20261025 and 200,000 samples per
direction, separate JS meshoptimizer decoding and point_cloud_utils nearest points.
Both crop results agree exactly. At physical cut 2, the content-less chain is
traversed to its first descendant content at depth 4; this differs from the
un-collapsed v2 cut 2 and is not an equal-depth quality comparison:

| cut | mean m | p95 m | sampled max m | cut maximum G m |
| --- | ---: | ---: | ---: | ---: |
| 0 | 0.019058323 | 0.050019099 | 1.332980070 | 1.429045841 |
| 2 | 0.004705112 | 0.016966031 | 0.231545403 | 0.268756482 |
| 4 | 0.004710033 | 0.016959218 | 0.201128774 | 0.268756482 |
| 6 | 0.000081866 | 0.000167419 | 0.009414346 | 0.011989462 |

These are comparisons against each cut's maximum G, not independent per-tile/range
certification. Empty-chain branching and cut traversal have a sensitive standalone
oracle control. Member inventory, monotone G and all 156 leaves match the retained
develop-baseline receipt. Archives and decoded inventories are retained by hash.
Prototype root mean/p95/max .013265/.036122/.390762 m used 15,970 triangles versus
8,182 here; those different budgets/ownership are context, not an equal-budget gate.

## Verification and remaining gates

Clippy `--all-targets -- -D warnings`, formatting and tree budget checks pass.
Required test results:

- `cargo test --lib mesh_large`: 342 passed, 6 evidence-only ignores.
- `h26_mesh_core`, `h26_mesh_frontends`, `golden`, `output_digests`: 24 passed,
  1 recipe-exporter ignore.
- Explicit requested jobs/budget determinism: 1 passed.

Analytic plane maxima .25/.50 m and a polygonal cone height `1-.8f32` match within
1e-12. An interior triangular hole exercises source-side area sampling: source
controls see zero, sampled maximum is within 5% of known inradius
`2/(1+sqrt(5))`, and G covers it. Further controls cover slivers, source components,
source/proxy controls, depth refusal, monotone floors, jobs/budget determinism,
encoded-depth seeds/counts, exact decoded bytes, metadata removal, bounded cache
and scratch refusal, and cancellation in both pruned dense partition scans.

The build/test scope peaked at 5,025,546,240 bytes; final checks peaked at
2,039,218,176 bytes. Both recorded zero limit/OOM events. The bake and audit
scratch monitors observed 391614705 and 553003777 logical bytes, respectively,
below the 6 GiB campaign bound. Independent-audit CPU and cgroup events are
pinned separately. Owned campaign scratch is deleted only after preserving both
archives. Tests use the requested Cargo target/jobs/debug settings, test opt-level
2, two test threads and actual two-CPU affinity; the jobs control uses four CPUs.
Full-model, appearance, transfer and integrated S8 performance remain outstanding.
R8 independently reviewed clean receipt HEAD `58063fc4` and found no additional
actionable correctness findings. It verified all 312 production hashes, four
oracle helpers, binary/inputs, all 54 execution artifacts and both archives.
`review-r8.json` pins that read-only review; its addition is the 55th artifact.
This final receipt update changes only documentation and the hash manifest.
