# Vector feature acceptance and publication contract

This is the implementation contract for [#119](https://github.com/BenDyson-Arch/rusty-tiles/issues/119), under the [#113 architecture gate](https://github.com/BenDyson-Arch/rusty-tiles/issues/113). It describes the candidate implementation and its proof obligations. It does not claim that the acceptance matrix has passed, approve a release, or validate polygon, CRS, repair or LOD algorithms. Those algorithms remain separately gated, including [#123](https://github.com/BenDyson-Arch/rusty-tiles/issues/123).

## Outcomes and owners

Portable and native readers use the same private `FeatureResult<T> = Result<T, FeatureFailure>` boundary. `Ok(T)` means accepted. `FeatureFailure::Rejected(FeatureRejection)` is an explicitly constructed feature rejection. `FeatureFailure::Fatal(Error)` aborts the job. Conversion from an ordinary error constructs only a fatal failure. There is no exception whitelist and no message-prefix classification. Polygon outline eligibility has a typed `OutlineFallback` rejection kind. Native geometry operations reset thread-local CPL diagnostics before the operation and distinguish an ordinary invalid-topology result from a CPL failure. Allocation, mutation, validity and area-query failures remain fatal infrastructure errors; a stale diagnostic cannot classify a later feature.

Only a rejection may become a skipped feature under `skip_invalid`. Without that option, rejections prevent publication. I/O, SQLite, source resource close, required report writes and finalization, cancellation, observer failures and output storage failures remain fatal under either policy. Null or empty source geometry has a separate reported no-geometry outcome; it does not select a frame or increment accepted geometry counts.

GeoJSON FeatureCollections use the same bounded semantic reader in every build, including with native geometry kernels. Native GDAL identification of GeoJSON under an unrecognized filename or as a sequence is refused before ingestion; it cannot bypass shared validation. Other native formats use the native reader. The reader owns preparation and source traversal. Declared layer schemas and selection metadata are source preparation state. Feature-derived schema changes, JSON-list-field inventory and frame selection are proposed locally and committed only after the acceptance callback succeeds. Accepted schemas and JSON-list inventory participate in cache signatures: changes force rebuilding rather than refusing a source revision, including deletion of all content. Requested conversion settings still require compatibility. A verified reused frame is fixed for the run; otherwise the first accepted geometry selects the frame. A rejected first geometry cannot choose the frame. Source IDs preserve the original JSON ID representation or documented source-FID fallback. Fragment paths belong to the accepted source feature, rather than a process-global identity allocator.

## Feature transaction

`store::accept_feature` owns a feature savepoint, its SQL rows and vertex-lock mutations, provisional fragmentation counters and staged accepted geometry diagnostics. SQLite uses rollback-capable journaling. Recursive splitting and full-detail encoding feasibility checks occur before admission. A rejected fragment rejects the whole feature, including fragments inserted earlier in the same savepoint.

Accepted diagnostics are written only after feature preparation and all fragment insertion have succeeded. Savepoint release must succeed before accepted counters are incremented and before the reader commits proposed state. SQL rollback alone is insufficient: counters and diagnostics are staged independently, and frame/schema state belongs to the reader's corresponding proposal.

A required report-write or release failure aborts the whole private workspace. Partially written required report bytes are never an accepted archive. The job does not recover from such a failure by continuing to the next feature. If rollback or release fails while cleaning up an explicit rejection, the infrastructure failure is the primary fatal cause and the rejection is retained as secondary context. If a fatal failure already occurred, it remains primary and the cleanup failure is secondary. SQLite and I/O causes must remain available through the causal error chain.

The feature boundary is not a promise that every collection of accepted features can produce a hierarchy under every global constraint. Later cross-feature schema promotion, aggregate or hierarchy budgets, `maxTiles`, LOD construction and reuse consistency can fail the entire job. Those failures do not silently remove an already accepted source feature. Full-detail preflight uses the feature's own metadata schema and reserves implicit-content framing; later global schema or hierarchy constraints are fatal outside admission.

## Hierarchy resources and reports

Encoding candidates are provisional. Their geometry diagnostics are admitted only when their node is retained in the hierarchy; a budget-rejected candidate or a discarded coarse level does not contribute accepted diagnostics. Workers can generate immutable content candidates speculatively. The final manifest determines the accepted content URI inventory, and reuse publication removes unreferenced content before inventory admission. Content deduplication uses content hashes; reused content is validated against its named hash before it is admitted.

The producer returns a completed report and explicit archive members. Required geometry reports and `conversion.json` are finalized before completion. Source handles and SQL spool handles close explicitly; successful spool cleanup finishes before inventory enumeration. Scratch directories are beneath the facade-owned private workspace, so failure cleanup has one owner. The archive codec receives the completed member inventory rather than rediscovering files from a public output directory.

## One F0 publication attempt

The public Rust entry point is `vector_to_archive(VectorRequest, &RunControl)`. `VectorRequest` carries source, destination, vector options and output policy; `VectorResult` carries the published output and report. CLI and Python adapters construct this request and use the same job path. The obsolete `vector_to_3tz` overload family and producer-owned publication options are removed; compatibility wrappers are not part of this change. See [the vector migration guide](../VECTOR.md).

One F0 attempt owns event admission, cancellation, producer work, archive serialization, cleanup and publication. Events are fallible. All producer workers finish before completion. The archive is serialized from admitted members, private workspace cleanup completes, the last pre-publication event finishes, and event admission closes before the staged archive is sealed and published. There is no fallible success callback after publication. Infrastructure failure before publication must preserve an existing destination; cleanup failures retain their causes and actionable paths.

## Bounds and ordering

`maxSourceVertices` bounds feature geometry ingestion. Source properties and raw JSON have no separate byte budget; peak memory can grow with the largest individual feature and its metadata. Schema/catalog state grows with distinct fields and layers, and manifest/reuse state grows with the hierarchy. These runs do not establish a bound independent of source size. Feature-local fragment work and diagnostic staging are bounded by the source feature and recursive splitting, not the number of features in the dataset. The implementation still clones geometry and accumulates some feature-local fragment/diagnostic vectors; this is a bounded ownership argument, not a measured constant-memory claim. SQLite stores source and geometry rows on disk. Worker count and LOD levels bound concurrent encoding candidates, subject to geometry, metadata and encoding allocations.

Traversal follows source order for GeoJSON and ordered source FIDs for portable GeoPackage. Indexed parallel candidate collection retains level order, reports follow admission order, and archive inventory is sorted by member name. Reuse state, frame, counters and diagnostics belong to one run. Independent repeated/concurrent-run tests and measured memory/scratch/order evidence are required; historical digest agreement alone does not establish these guarantees. Portable/native topology implementations may produce different triangles, and this contract does not require identical algorithmic output between them.

## Disposition and evidence boundaries

| Component | Candidate disposition | Required justification |
| --- | --- | --- |
| Reader callback error catching | Replace | Explicit typed rejection; infrastructure faults never become skipped features |
| Feature SQL/counter/report ownership | Rework | Savepoint plus staged non-SQL effects; independent row, source-ID, counter and report inventory |
| Frame/schema commit | Rework | Rejected-first-feature control compared with valid-only input and verified reused frame |
| Message-based outline classification | Replace | Typed semantic eligibility; fatal errors cannot select outline fallback |
| Source decoders and geometry kernels | Retain as unproven candidates | Source-fidelity and geometry evidence independently establishes correctness; #119 does not confer it |
| Parallel candidate/reuse construction | Rework ownership; algorithms remain candidates | Retained-node diagnostics, final content inventory, dedup/reuse and ordering inspection |
| Archive publication | Migrate to F0 | One attempt across Rust, CLI and Python; no post-publication fallible event |

The issue records executed baseline evidence of an oversized rejected polygon leaving a fragmentation count and success-like report, and injected acceptance-callback ENOSPC becoming a skipped feature. The earlier orphan-content suspicion was withdrawn for the tested controls; it is not an observed general leak. Source inspection of speculative files and final pruning is not equivalent to a demonstrated publication leak or a cleanup proof.

New implementation and tests meet the same evidence standard as the baseline. Production-helper tests exercise late-fragment SQL rollback and transaction/report faults; their existence is not a passing result. Independent archive inspection must check full-detail feature IDs, report outcomes and exact content membership against explicit fixture expectations. The evidence ledger must distinguish source observations, proposed requirements, executed tests and measured resource results, and record unresolved limits before closing #119.
