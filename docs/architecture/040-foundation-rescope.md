# 0.4.0 non-mesh foundation rescope — proposal

| Decision requested | Proposed 0.4.0 bar | Remaining agent-days |
| --- | --- | ---: |
| Point cloud (#122) | Retain P1; check current implicit output, realistic resources and frontend equivalence | 1.5–2.5 |
| Vector (#123) | Retain transactional pipeline; add a small published-output fidelity oracle, check placement/reuse and resources | 3–5 |
| Raster / imagery (#124) | Retain native R2; complete literal display fixtures and realistic resource receipt | 1–2 |
| Terrain (#124) | Retain native T1 + packed LOD; retain its existing numerical guarantees and check current resources/capability reporting | 1–2 |
| Shared non-mesh closeout (#113/#120/#125/#126) | Reconcile supported domains, final evidence and release checks once | 1.5–2.5 |
| **Total, excluding mesh and release promotion** | **Existing implementations plus focused gaps; no replacement programme** | **8–14** |

Proposed on 2026-10-24 against `develop` **cda7ca6110c714f8eaceea04df8f5945eb6443c8**.
This is a design decision for approval, not new acceptance. Only source, tests,
contracts, Git history and GitHub issue/PR metadata were read; no build, test,
bake or browser was run. “Pass” below means recorded bounded acceptance or an
existing substantive regression, never a fresh pass on this checkout. All release
boxes remain unchecked until the final candidate supplies the named evidence.

## 1. What already exists

Checklist references below count bullets in each issue's original **Acceptance**
section, starting at 1. **Met** means substantively covered for the stated finite
domain; **partial** means the broad issue wording still exceeds the evidence.
A test's existence establishes coverage intent; historical execution comes from
its linked acceptance record/merged PR. Issue bodies inspected were #113, #120–#126, #82, #221 and #222. Neither establishes arbitrary inputs.

| Tiler | Current capability and state | Original checklist already substantively met | Still unmet under original scope |
| --- | --- | --- | --- |
| Point | Standard LAS/LAZ formats 0/1/2/3/6/7/8; local metres or declared horizontal CRS + explicit ellipsoidal height offset; all scalar source dimensions, ordinals, flags, RGB/NIR and scalar Extra Bytes. Full-detail leaves, first-record voxel representatives; implicit octree forest only; `.3tz`, one F0 attempt. Max/default 8,192 points/tile. | #122 **1–2:** declared field/omission policy, source-enumerated multiset and fully transformed POSITION, sensitive missing/duplicate/position controls. **3–4 partial:** sparse/outlier/reordered/chunk/deep hierarchy and analytic WGS84/UTM references. **6:** lifecycle, reports, aliases, cancellation and concurrency. **5/7 partial:** measured resources and scoped dispositions. P1 [#134](https://github.com/BenDyson-Arch/rusty-tiles/pull/134), bounds [#183](https://github.com/BenDyson-Arch/rusty-tiles/pull/183), viewer width [#195](https://github.com/BenDyson-Arch/rusty-tiles/pull/195), forest [#161](https://github.com/BenDyson-Arch/rusty-tiles/pull/161)/[#198](https://github.com/BenDyson-Arch/rusty-tiles/pull/198). | Full CRS/backend/source breadth, whole-process resource ceiling over schema/hierarchy extremes, exhaustive sampler dispositions. P1's small resource series and older archive-layout results need current-output reconciliation, not a new source parser. |
| Vector | Standard GeoJSON/GeoPackage, native OGR; points/lines/polygons/holes/multipart, repair/outline policies, typed/null/list metadata, stable source identity, LOD, aggregation, quantization/meshopt and reuse. Transactional accepted-feature spool, F0 publication, implicit quadtree forest; experimental pinned vector extensions. | #123 **3:** typed rejection vs fatal error, atomic accepted counters/frame/schema and exact inventory. **1/2/4 partial:** explicit source policy, analytic hole area/segment/identity/metadata tests, source-chart and repair regressions. **5 partial:** reuse equivalence/corruption, LOD/budget/aggregation, quantized seams and forest traversal. **6/7 partial:** both backends, finite resource runs and lifecycle faults. [#100](https://github.com/BenDyson-Arch/rusty-tiles/pull/100)/[#112](https://github.com/BenDyson-Arch/rusty-tiles/pull/112)/[#131](https://github.com/BenDyson-Arch/rusty-tiles/pull/131)/[#154](https://github.com/BenDyson-Arch/rusty-tiles/pull/154)/[#177](https://github.com/BenDyson-Arch/rusty-tiles/pull/177)/[#185](https://github.com/BenDyson-Arch/rusty-tiles/pull/185)/[#186](https://github.com/BenDyson-Arch/rusty-tiles/pull/186)/[#191](https://github.com/BenDyson-Arch/rusty-tiles/pull/191)/[#196](https://github.com/BenDyson-Arch/rusty-tiles/pull/196)/[#241](https://github.com/BenDyson-Arch/rusty-tiles/pull/241). | No small single independent checker yet establishes topology, source coordinates, metadata and bounds together on the final published compressed forest. Existing admission oracle deliberately excludes topology/CRS/repair/LOD certification. No source-independent byte/RSS bound on arbitrary feature properties, field/layer catalogues or hierarchy. Full per-algorithm ledger remains unfinished. |
| Raster / imagery | Native R2, owned admitted local source closure → standalone sample-preserving COG → Byte RGB/gray or explicit numeric-gray styling → nearest PNG XYZ pyramid, TileJSON and report. EPSG:4326/3857 display domain; masks/NoData/one opacity source explicit. No terrain-height promise. | #124 **1:** band/scale/NoData/mask/alpha/display/grid policies. **2 partial:** literal RGB, masks, numeric values, exact coarse-sampling phase and bounds. **3/5/6:** native close/config ownership, final reports/inventory and D1/D2 publication/faults. **4/7 partial:** logical caps, historical native resources, Rust/CLI/default capability and D1 Python boundary. [#129](https://github.com/BenDyson-Arch/rusty-tiles/pull/129)/[#130](https://github.com/BenDyson-Arch/rusty-tiles/pull/130)/[#155](https://github.com/BenDyson-Arch/rusty-tiles/pull/155)/[#156](https://github.com/BenDyson-Arch/rusty-tiles/pull/156)/[#159](https://github.com/BenDyson-Arch/rusty-tiles/pull/159)/[#178](https://github.com/BenDyson-Arch/rusty-tiles/pull/178). | Full R2 independent display matrix across every admitted codec/driver; realistic final RSS receipt, not merely the logical working ledger; exhaustive native-cache/fault/platform claims. Standard wheel deliberately has no imagery pyramid API. |
| Terrain | Native standalone north-up EPSG:4326 Float32/64 PixelIsArea DEM, raw metre heights + explicit offset/fill. 3D Tiles 1.1 GLB directory; decimated LOD, skirts, meshopt and shared quantization grid (f32 fallback). Footprint queries/clamping and experimental Cesium imagery draping. | #124 **1–2:** independent plane/Float64/NoData heights, lattice, topology, exact seams, bounds and corruption controls. **3/5/6:** checked source close before staging, complete plan/report/inventory, physical write/finish/cancel and D2 safety. **4/7 partial:** measured finite resources, Rust/CLI parity, concurrency and native capability. T1 [#136](https://github.com/BenDyson-Arch/rusty-tiles/pull/136), LOD [#169](https://github.com/BenDyson-Arch/rusty-tiles/pull/169), packed output [#184](https://github.com/BenDyson-Arch/rusty-tiles/pull/184), tooling [#159](https://github.com/BenDyson-Arch/rusty-tiles/pull/159)/[#201](https://github.com/BenDyson-Arch/rusty-tiles/pull/201). | Realistic resource receipt for final packing; projected inputs/COGs/integer DEMs excluded, no Python terrain conversion API. Broad continuum-DEM error and all-CRS proof not provided. Doctor's missing-GTiff category differs from execution. |

### Evidence to retain and reuse

| Tiler | Concrete tests/oracles to credit | What they actually establish; limits |
| --- | --- | --- |
| Point | `tests/p1_point_oracle.py`, `tests/p1_laz_adapter.py`; `tests/test_point_cloud.py::test_las_and_laz_fidelity_bounds_and_parent_error`, `test_all_supported_formats_preserve_original_scalar_fields_and_flags`, `test_projected_tree_positions_match_an_independent_pyproj_reader`; `tests/point_acceptance.rs::cancellation_at_commit_boundary_preserves_old_output`, `observer_failure_before_commit_preserves_previous_output_and_removes_scratch` | P1 oracle uses stdlib source/ZIP/GLB/implicit readers, not product transforms/expander. Independent source fields, multiplicity, rendered POSITION, ancestors and representatives; LAS/LAZ delivery tested separately. [P1 evidence](../../bench/architecture_audit/point_acceptance/README.md): 18 LAS cases, 30 source/reference controls, 14 LAZ cases and seven semantic archive mutants per build; 32,768 rows observed 35.32 MiB portable RSS, native geographic 50.16 MiB. Small finite workloads, not realistic maximum-scale acceptance. |
| Vector | `tests/vector_acceptance_oracle.py` (including published `run_forest`); `tests/portable_vector.rs::polygons_preserve_holes_and_opt_in_repair_reports`, `exact_geojson_metadata_lists_filters_and_empty_selection`; `tests/test_vector_gpkg.py::test_fragmented_line_covers_every_source_segment_and_caps_every_glb`, `test_oversized_polygon_fragments_preserve_hole_area`; `tests/test_vector_batching.py::test_polygon_batch_offsets_holes_multipart_and_feature_ids`; `tests/vector_fill_seams.rs::country_fragments_share_decoded_seam_vertices_across_pole_and_antimeridian`; `tests/vector_m1_serializer.rs::fixture_b_crosses_the_subtree_boundary_under_a_skeleton`, `staged_payload_mismatch_publishes_nothing`; `tests/test_vector_reuse.py` | [Admission record](../../bench/architecture_audit/vector_acceptance/README.md): 27 portable/30 native source-derived cases, five resource runs each. Stdlib oracle independently invents IDs and GeoPackage bytes; checks accepted inventory, counters, identity and reuse. Other tests add analytic area/segments/metadata/seams. Many Python tests inspect reconstructed `vector-build.json` hierarchy through `vector_test_support.py`; this is useful, but published-forest placement also needs direct decoding. Frozen encoder differential tests share lineage; agreement alone does not establish geometry truth. |
| Raster | `tests/d1_raster_oracle.py`, `tests/d2_directory_oracle.py`; `tests/raster_contract.rs::cog_preserves_typed_metadata_point_affine_and_crs`, `cog_preserves_palette_rotated_affine_and_area`, `six_cli_caps_pass_at_need_and_refuse_one_below`; `tests/raster_exact_tiler.rs::coarse_zooms_sample_exact_greater_tie_finest_pixels`; `tests/test_raster_import.py::test_json_stdout_and_explicit_alpha_intersect_source_mask`, `test_numeric_values_and_masks`; `tests/raster_pyramid.rs::interior_native_observer_panic_keeps_cause_and_restores_owners`, `source_mutation_at_reached_cog_completion_refuses_publication` | D1 authors TIFF/decodes PNG with stdlib; covers a finite aligned RGB pilot, not all R2. R2 exact-index test checks every PNG pixel and padding at z16/z24. [R2 evidence](../../bench/architecture_audit/raster_source/README.md) records six independent publications, cap equality/one-below, real native finalizer faults and final local review; some leading status prose is stale. [#155](https://github.com/BenDyson-Arch/rusty-tiles/pull/155)/[#156](https://github.com/BenDyson-Arch/rusty-tiles/pull/156) establish merged implementation/regressions. Python GDAL fixtures/readback share a decoder, so add a small literal PNG display check rather than accepting GDAL agreement as universal truth. |
| Terrain | `tests/test_native_terrain.py::test_complete_analytic_plane_compressed_tiled_and_shared_edges`, `test_lod_pyramid_errors_skirts_and_forest_validation`, `test_native_float64_fraction_earns_its_reported_storage_error`, `test_source_nodata_is_filled_only_in_footprint`; `tests/t1_terrain_oracle.py::{check_plane,check_directory,corruption_controls}`; `tests/native_terrain.rs::packed_lod_forest_passes_validation_and_keeps_flat_leaf_surfaces`, `concurrent_jobs_and_repeated_publication_keep_independent_state`; `src/terrain.rs` tests `physical_partial_leaf_and_report_failures_preserve_old_output`, `ready_observer_failure_and_cancellation_clean_finished_tree` | Independent lattice/ECEF and GLB/forest inspection, including a Python meshopt decoder written from the pinned extension text (`tests/ext_meshopt_spec.py`), sharing no meshoptimizer implementation code. Expected source values are literal; GDAL authors source fixtures. Current tests already include a bump, clipped coarse cells, skirts and error mutations. [T1 evidence](../../bench/architecture_audit/terrain_acceptance/README.md) records 65,060–76,608 KiB RSS on finite sources, native-close/fault/concurrency and Cesium query/drape controls; #169/#184 extend the earlier flat evidence. |

Shared evidence is already substantial: C1 [#137](https://github.com/BenDyson-Arch/rusty-tiles/pull/137)/[#152](https://github.com/BenDyson-Arch/rusty-tiles/pull/152)/[#153](https://github.com/BenDyson-Arch/rusty-tiles/pull/153)/[#176](https://github.com/BenDyson-Arch/rusty-tiles/pull/176)/[#202](https://github.com/BenDyson-Arch/rusty-tiles/pull/202), I1/P
[#161](https://github.com/BenDyson-Arch/rusty-tiles/pull/161), C2 [#157](https://github.com/BenDyson-Arch/rusty-tiles/pull/157)/[#179](https://github.com/BenDyson-Arch/rusty-tiles/pull/179)/[#194](https://github.com/BenDyson-Arch/rusty-tiles/pull/194) and validator scale [#206](https://github.com/BenDyson-Arch/rusty-tiles/pull/206)/[#217](https://github.com/BenDyson-Arch/rusty-tiles/pull/217)/[#214](https://github.com/BenDyson-Arch/rusty-tiles/pull/214). Use the
[C1](c1-payload-integrity-contract.md), [C2](c2-decoded-containment-contract.md)
and [I1](i1-implicit-read-contract.md) **amendments**, not their historical opening
status paragraphs. C1 is not a topology/metadata fidelity oracle; C2 has explicit
unsupported domains and cannot certify every placement by a green exit code.
Terrain directories must first be packaged with `convert` for `validate`.

### Real defects, fixed defects and evidence gaps

| Class | Finding and release consequence |
| --- | --- |
| Fixed, keep regressions | Point f32 bounds could under-enclose payloads (#183); >8,192 point property textures failed in Cesium (#195). Vector rejected-feature state leaked (#131), geographic polygon charts lost topology (#100), partitioned outlines broke runs (#186), independently quantized fills cracked (#196), native dereferencing drivers were admitted (#241). Raster coarse sample phase, RGB PNG inefficiency and native publication/preview defects were corrected (#155/#156/#159). Terrain prototype Float64 precision loss was replaced (#136); packed LOD has subsequent controls (#169/#184). These are not grounds to rewrite today's tilers again. |
| Known current mismatch | `t1-tooling-contract.md` records terrain/D1 missing GTiff: doctor reports Unsupported/exit 2, producer open reports InvalidInput/exit 3. Fix producer capability classification, or explicitly document this narrow exception; proposal below budgets the fix. No claim that conversion succeeds without GTiff. |
| Known resource limitation | Vector's `UniqueValue` materializes a feature, and properties/catalogues/hierarchy have no whole-job byte cap (`src/vector/portable.rs`, `vector-acceptance-contract.md`). Point hierarchy/member state grows with leaves; raster native opaque allocation exceeds logical accounting; terrain holds decoded samples plus validity/cache. These are observed ownership/allocation properties, not reproduced OOM or leak defects. Measure a supported workload; do not advertise constant RAM. |
| Known fidelity/support limits | Terrain skirts cover vertical gaps but not horizontal chord/grid offsets; its errors bound decoded discrete surfaces, not a continuous DEM. Cartesian chords can project outside the source rectangle; preview fences queries. Vector output needs its pinned experimental consumer; aggregation parents intentionally omit exact `_source_id`. Point representative selection depends on source order. Preserve these honest documented limits. |
| Unproven, not a demonstrated defect | Arbitrary CRS/topology failure, universal cancellation race/leak, lost dimensions in all LAS, or broken R2 masks cannot be inferred from unchecked umbrella boxes. No new executable defect was established in this read-only review. Any concrete wrong output, destructive publication or failure of the small release fixtures remains a blocker. |

## 2. Minimal acceptance work

**P = existing scoped pass/coverage; T = small test or current-candidate receipt;
W = real implementation/oracle work.** Estimates are remaining focused agent-days,
including the named checks and routine fixes; no time is charged for completed
work. Existing P rows still run with the final acceptance suite. A substantive
new failure is assessed at its owning boundary, not waived by this estimate.

| Tiler / item | Concrete acceptance condition | Status | Days |
| --- | --- | --- | ---: |
| Point: decoded output | Replay independent source enumeration on tiny LAS/LAZ: duplicates, RGB/NIR, flags, scalar Extra Bytes and outlier; every leaf ordinal exactly once, metadata exact, parent representative policy unchanged. Keep the missing-field/duplicate/wrong-POSITION controls. | T: current forest receipt using P1; no new general decoder | .25–.5 |
| Point: placement/bounds | Local XYZ plus analytic WGS84 equator and UTM32 + explicit height; compose full GLB/tile frame; every drawn point in own/ancestor bounds, finite nonincreasing errors. Keep outward-f32 regression. Source comparison uses reported rounding plus 1e-6 m analytic-reference allowance. | P: P1, #183 and projected Python test | 0 |
| Point: resources | Run the realistic point workload in the resource table below once, final binary pinned. | T | .5–.75 |
| Point: cancellation/publication | Pre-cancel, ingestion/encoding observer abort, ready-to-publish cancellation; old archive/source bytes survive, no orphan staging, finalized report equals returned result. | P: `point_acceptance.rs` and physical spool/leaf/archive/report fault units | 0 |
| Point: CLI/Python | One local + one supported CRS fixture through both adapters; compare decoded points/fields, report settings, explicit height/error kind, callback-abort preservation. | T: existing binding tests need a direct paired comparison | .25–.5 |
| Point: suites | Existing portable/native Rust, P1/LAZ and Python point suites green with required binary/codecs present. | T: final-candidate execution | .25–.5 |
| Point: domain receipt | Link P1 retained domain, 8,192 cap, exclusions and migration in one release record. | T | .25 |
| **Point subtotal** | | | **1.5–2.5** |
| Vector: decoded output | Tiny point + disconnected line + square-with-hole/multipart + repaired/rejected feature. Directly decode published forest (raw and quantized/meshopt): source IDs/field values exact; line segment multiset complete; no cross-feature joins; hole interior uncovered; triangle union covers accepted polygon. Use analytic expected coordinates/area, not historical encoder output. Include missing-feature/hole-fill/wrong-ID controls. | W: consolidate a small independent checker; extend existing readers | 1–1.5 |
| Vector: placement/bounds | Local fixture and WGS84/UTM fixture with asymmetric XYZ/height; full frames and own/ancestor containment; coordinate error ≤ reported rounding + quantization + 1e-5 m fixture allowance. Force forest boundary and fresh/reuse outputs, verify same source identities/world geometry. | T: combine existing placement, forest and seam checks | .5–.75 |
| Vector: resources | One realistic mixed-feature run, fixed two workers, and record largest feature/property/schema size. Explicitly describe input-dependent RAM; no universal byte-bound claim. | T: resource receipt; cap redesign deferred | .5–.75 |
| Vector: cancellation/publication | Under skip-invalid, infrastructure/observer/cancel failure aborts entire archive; no rejected feature changes accepted frame/counters/schema; ready abort preserves old bytes; workers drain before seal/report completion. | P: #131 units/oracle, M1 staged-content/ready faults and binding callback tests | 0 |
| Vector: CLI/Python | Pair GeoJSON and GeoPackage conversions; same resolved defaults/identity/metadata/height and strict/skip outcomes; unchanged reuse retains content, corrupt previous content refuses. Backend triangulation order need not match. | T | .25–.5 |
| Vector: suites | Portable/native vector, direct forest oracle, seams, fills, metadata, repair, aggregation, LOD, parallel/reuse and Python API suites green. | T | .5–.75 |
| Vector: domain receipt | Record experimental consumer pin, source chart/repair policies, aggregation identity exception and source-dependent memory. | T | .25–.75 |
| **Vector subtotal** | | | **3–5** |
| Raster: decoded output | Literal constant/ramp/impulse/NoData-mask-alpha fixtures; decode emitted PNG independently, check selected gray/RGB scaling and valid black/partial alpha, exact finest/coarse phase and zero padding. COG raw samples/mask equal literal input. Reuse existing tests; add only missing impulse/literal display coverage. | T | .25–.5 |
| Raster: placement/bounds | Aligned EPSG:3857 and small EPSG:4326 cover, edge pixels, z16/z24 phase and XYZ/TileJSON bounds. Exact source-index selection; outward coverage contains transformed footprint; existing grid tolerances apply. | P: D1, R2 grid and exact-tiler tests | 0 |
| Raster: resources | Once on realistic imagery; pin GDAL/cache/workers; distinguish RSS, scratch, encoded bytes and logical allowance. | T | .25–.5 |
| Raster: cancellation/publication | COG/native callback abort, finalizer/storage failure and ready abort preserve prior directory or report typed recovery paths; handles/config restored and exact four-role inventory finalized. | P: D1/D2, R2 physical faults and `raster_pyramid.rs` | 0 |
| Raster: CLI/Python | Rust/native CLI agree. Wheel's D1 adapter explicitly raises Unsupported and has no R2 pyramid export; assert package surface/help/docs agree. **Do not add a Python imagery API to satisfy parity.** | T: capability/surface assertion | .25–.5 |
| Raster: suites | Native raster/D1/D2, R2 exact/contract/pyramid and Python CLI tests green; standard capability refusals green. | T | .25–.5 |
| **Raster subtotal** | | | **1–2** |
| Terrain: decoded output | Keep plane, bump/ridge-like nonplanar, Float64 fraction and NoData-hole fixtures. Independent decoded leaf lattice/topology/coverage, LOD/skirt and mutation checks; height/storage error ≤ reported bound, capped at .05 m. No new continuous-surface certificate. | P: current T1 oracle + #169/#184 | 0 |
| Terrain: placement/bounds | Independent ENU/ECEF, negative and positive height, fill without added offset, shared edges identical per level; boxes contain decoded skirts/surfaces and descendants. Queries remain fenced to footprint. | P: `check_plane`, `check_directory`, native/viewer fixtures | 0 |
| Terrain: resources | Once on realistic DEM with final meshopt/quantization path and fixed native cache. | T | .25–.5 |
| Terrain: cancellation/publication | Cancel at native decode and ready boundary, partial leaf/report/forest/finalize faults, CreateNew conflict and Replace preservation; source closed before staging. | P: terrain/raster units and native integration | 0 |
| Terrain: CLI/Python | Rust/native CLI option/report/refusal parity; wheel advertises no terrain API. Classify unavailable GTiff consistently before open; keep portable Unsupported. | W: small capability fix and tests | .25–.5 |
| Terrain: suites | T1 native Rust/Python oracle, packed LOD/forest and existing consumer acceptance green on final candidate. | T | .25–.5 |
| Terrain: domain receipt | Keep source/height/error/skirts/query limits explicit; record no new Python/GDAL-free/projected capability. | T | .25–.5 |
| **Terrain subtotal** | | | **1–2** |

### One realistic resource measurement per tiler

These are **proposed engineering budgets**, not measured passes or universal
allocation bounds. Keep one receipt per final producer binary: source hash,
options, backend/native versions, machine, peak process RSS, scratch/disk,
descriptors, workers, elapsed time and cleanup. Fresh child-process high-water
RSS avoids counting Python fixture creation. Fix two workers where applicable;
point currently has no producer worker pool. A failed budget requires a fix or
an explicit owner-approved supported-input/budget change before shipping.

| Tiler | One workload (prepare outside converter measurement) | Proposed pass budget | Existing useful baseline, not final acceptance |
| --- | --- | --- | --- |
| Point | 10 million LAS format-3 points, RGB/GPS, varied spatial density, default maxPoints/chunkPoints; use corresponding LAZ only if it is the representative user input | ≤1 GiB RSS; ≤10 GiB scratch; no retained staging | P1 small rows/schema series; perf corpus 200k points, 48.6 MiB RSS |
| Vector | 100k mixed features, holes/lines/points, representative scalar/list metadata, at least one 4k-vertex feature, two jobs, three LOD levels | ≤1 GiB RSS; ≤4 GiB scratch; record feature/property/catalogue maxima | Perf corpus 10k mixed features, 93.0 MiB RSS; admission five-run series |
| Raster | 8192² RGBA, internal invalid hole, EPSG:4326 or 3857, z9–14, two requested workers | ≤768 MiB RSS; ≤3 GiB scratch; caps and cleanup respected | Same-size perf imagery, 236.3 MiB RSS |
| Terrain | 2048² Float32 DEM, varied heights/NoData hole, cells-per-leaf=64 | ≤512 MiB RSS; ≤2 GiB scratch; source/output limits and cleanup respected | T1 finite 63.5–74.8 MiB; perf terrain 60.9 MiB before #184 |

[Perf receipts](../../bench/perf_lane/REPORT.md) are pinned to `7cb35f8f826c`,
not cda7ca61. Finite caps and this workload protect ordinary use; hostile-input
OOM resistance and asymptotic proofs move to 0.4.x. No universal hard RAM
limit is claimed. A new giant-property budget/interface for vector is therefore
not smuggled into this minimal release scope.

Shared closeout **1.5–2.5 days** covers one small domain/evidence matrix,
non-mesh C1/I1/C2 claim reconciliation, focused nonauthor review and aggregation
of existing CI/package/consumer receipts. Per-tiler execution above already
includes its suites: do not charge another complete run programme here. Existing
applicable platform, installed-wheel, official-Blender and consumer/release gates
remain; no new browser matrix is proposed. Skipped prerequisites are not passes.

## 3. Defer explicitly

| Work | Destination and rationale |
| --- | --- |
| Exhaustive independent source/CRS/repair/metadata/LOD oracles, independent oracle re-audits/mutation campaigns for every algorithm, per-component retain/rework/replace ledgers | **0.4.x audit backlog.** Preserve existing scripts, provenance and failures. Small analytic source-to-output checks remain 0.4.0 blockers; blanket distrust/reimplementation adds no commensurate user protection. |
| Worst-case RSS/scratch/descriptor attribution per stage, arbitrary giant properties/schema, fuzz and every native-close/cancellation interleaving | **0.4.x hardening.** Keep actual observed failures blocking now. Measure ordinary inputs once and state limits; batch count/logical bytes are not RSS. |
| Broad datum/grid/epoch/pole/antimeridian/numerical-conditioning matrix and all spatial kernels (#120) | **0.4.x within supported capabilities.** Retain existing frozen PROJ and analytic references plus honest unsupported errors. New CRS capabilities require their own version decision. |
| Full validator profile/metadata/scene/topology/hostile-expansion audit and non-mesh-wide VS-D programme (#125) | **0.4.x**, except existing corruption suites, truthful labels and small producer decoded checks now. Do not delete C1/I1/C2 implementations or ignore known false acceptance. Mesh VS-D/scale policy remains the separate mesh rescope. |
| New crate/module layouts, universal converter framework, complete internal export stabilization and old A2 rewrite (#126) | **0.4.x investigation** only when a retained caller needs it. `convert-to-implicit` is already removed (#159); re-run point/vector from source. Do not resurrect it or old explicit/legacy payload modes. |
| GDAL-free raster/terrain #82; pure GeoTIFF/COG DEM reader #221 (issue estimate 18–20 days) | **Out of 0.4.0.** 0.4.x may evaluate dependencies/hostile fixtures and refine scope; actual new portable terrain capability stays **0.5** under current roadmap. Imagery remains native; #82's “terrain warping” description predates T1 and should be corrected. |
| Projected-CRS terrain #222 (issue estimate 4–6 days; depends on #221) | **Out of 0.4.0.** 0.4.x design/spike if useful; delivered capability remains **0.5**. Preserve explicit refusal today. No transfer of mesh source-CRS acceptance or unpriced distortion certificate. |
| 3D Tiles 2.0 / glTF 2.1 | **0.6**, unchanged. Real 1.1/2.0 output, implicit tiling and usable delivery remain 0.4 requirements. |

The request to move #82/#221/#222 “to 0.4.x” conflicts with the roadmap's
accepted “0.4.x = completion/fixes, no new public capability” rule (PR #220).
The proposal above moves their **investigation** outside the release gate,
while retaining the existing 0.5 delivery target. Shipping them in 0.4.x would
require a separate explicit amendment to that rule; it is not a foundation fix.
The owner's mesh H17 → classic HLOD decision is relevant precedent for reducing
proof cost, not authority to silently weaken terrain's already implemented cheap
discrete error/precision guarantees.

## 4. Copy-ready issue and roadmap edits

Apply only after owner approval; **this PR edits none of these issues or the
roadmap**. Archive original checklists under “Deferred audit scope (0.4.x)” with
historical receipts intact. Replace the active acceptance sections with the
following text; do not check boxes merely because this proposal merges. Mesh
#121 replacement is a coordination proposal, subject to the separate mesh design.

### #113 — replace audit premise and active release checklist

> For 0.4.0, retain the current supported non-mesh producers against the finite
> contracts and release checks in `docs/architecture/040-foundation-rescope.md`.
> Existing independently sourced tests and accepted slices count within their
> stated domains. Do not require blanket subsystem replacement, a new oracle
> audit or a per-component disposition ledger to ship. Observed correctness,
> data-loss and publication failures remain blockers. Broader proof obligations
> are tracked as 0.4.x hardening, not marked completed.

```markdown
- [ ] #120's supported-domain placement/bounds checks pass on the release candidate.
- [ ] #122 point, #123 vector and #124 imagery/terrain pass their finite decoded-output, resource, lifecycle, capability and suite gates in 040-foundation-rescope.md.
- [ ] #121 passes the separately approved classic-HLOD mesh acceptance contract; no certified-H17 requirement survives solely through historical umbrella wording.
- [ ] #125 validator claims remain truthful, existing corruption suites pass, and small producer correctness checks use decoded output independently of the product validator.
- [ ] #126 retained Rust/CLI/Python/package/help surfaces agree; exclusions, experimental consumers and removals are documented.
- [ ] Existing applicable platform, installed-wheel, official-Blender and release consumer gates pass on the final candidate; skips are recorded as skips.
- [ ] Deferred audit work is linked with owners/limits; no known destructive publication or analytic-fixture correctness defect remains. Promotion/tagging/publication remain separately authorized release steps.
```

### #120 — replacement Acceptance

```markdown
- [ ] Record the actual point/vector/raster/terrain supported CRS, axes, metre/height and capability domains; keep explicit unsupported behavior and current frozen reference provenance.
- [ ] Point local/WGS84/UTM and vector local/WGS84/UTM analytic fixtures verify fully transformed decoded coordinates, explicit height offsets and own/ancestor bounds within the tolerances in 040-foundation-rescope.md.
- [ ] Imagery EPSG:4326/3857 footprint/XYZ/TileJSON and terrain EPSG:4326 ENU/ECEF/fill/decoded-skirt bounds checks pass; unsupported projected terrain is refused.
- [ ] Rust/CLI/Python placement/default/refusal behavior agrees wherever the API exists; missing native capabilities fail without publication.
- [ ] Retain existing spatial suites; link deeper grid/epoch/datum/conditioning and per-kernel audits as 0.4.x work, not prerequisites for these finite checks. Mesh placement follows #121's separate contract.
```

### #121 — proposed replacement Acceptance for coordination

```markdown
- [ ] Record and pass the owner's separately approved classic-HLOD contract, supported textured-mesh domain, approximation/error meaning and resource budget; replace universal H17 certificate requirements explicitly.
- [ ] Final decoded leaves preserve supported geometry/attributes/materials/source identity and placement/bounds; representative coarse levels preserve useful appearance, coverage and refinement under that contract.
- [ ] Resource closure, picking, cancellation/publication and existing relevant regression/consumer/platform checks pass on final output; concrete fidelity/data-loss failures remain blockers.
- [ ] Record removed APIs and unsupported inputs; move deeper universal geometry/appearance certificates and exhaustive per-component audit obligations to the follow-up backlog with their proof limits intact.
```

This does not accept a mesh implementation, waive unrelated scale gates or add
mesh effort to the non-mesh estimate. The mesh proposal must name the actual
fixtures and resource/scale obligations before this coordination text is applied.

### #122 — replacement Acceptance

```markdown
- [ ] Keep the P1 LAS/LAZ source/field/omission/voxel profile and 8192-point tile ceiling documented; no silent loss of required fields or multiplicity.
- [ ] Independent tiny LAS/LAZ output checks pass for source ordinals, scalar attributes/flags/RGB/NIR, rendered positions, representatives, finite errors and own/ancestor bounds, including sensitive corruption controls and the current implicit forest.
- [ ] One pinned realistic 10-million-point run meets the proposed resource budget in 040-foundation-rescope.md, records actual high-water RSS/scratch and removes staging.
- [ ] Cancellation/observer/storage/report failures preserve source and old output; one completed report/inventory and publication owner; concurrent runs remain isolated.
- [ ] CLI/Python decoded results, resolved settings and refusal/callback behavior agree; existing portable/native Rust, LAS/LAZ oracle and Python suites pass on the final candidate.
- [ ] Link broader source/CRS/sampling/resource and component-ledger audits to 0.4.x with current limits stated; do not claim they passed.
```

### #123 — replacement Acceptance

```markdown
- [ ] Publish the finite geometry/property/repair/aggregation/experimental-consumer policy; source identities and required scalar/list/null/integer fields are explicit.
- [ ] A small analytic published-forest checker verifies point coordinates, complete disconnected lines, polygon/hole/multipart coverage, repaired/rejected feature outcomes, identity and metadata; raw and quantized/meshopt cases and omission/wrong-ID/hole-fill controls pass.
- [ ] Fully transformed local/geographic/projected fixtures fit own/ancestor bounds and declared rounding/quantization tolerance; subtree-boundary, seam and fresh/reuse equivalence checks pass.
- [ ] One pinned 100k-feature mixed workload meets 040-foundation-rescope.md's RSS/scratch budget at two workers; document largest feature/properties/schema and input-dependent memory.
- [ ] Skip-invalid remains feature-atomic; infrastructure/cancel/observer/report failures abort and preserve old output, with worker completion before publication.
- [ ] CLI/Python settings, identity/metadata/height/refusal semantics agree; existing portable/native vector, reuse/LOD/repair/compression/parallel and Python suites stay green.
- [ ] Defer exhaustive topology/repair/CRS oracle audits, arbitrary-property byte bounds and per-algorithm ledgers to 0.4.x, preserving known limits and regressions.
```

### #124 — replacement Acceptance (two producers, independently satisfied)

```markdown
- [ ] Keep native R2 imagery and T1 terrain source/band/NoData/mask/alpha/height/interpolation/refusal domains explicit; no new GDAL-free/projected capability is required.
- [ ] Imagery constant/ramp/impulse/masked-alpha fixtures verify literal decoded display values, raw COG samples, coarse phase, padding and XYZ/TileJSON placement/bounds.
- [ ] Terrain plane/bump/Float64/NoData fixtures verify decoded heights, lattice topology/coverage, seams, existing discrete LOD errors, skirts and ENU/ECEF own/descendant bounds; storage error is at most 0.05 m and no continuous-DEM accuracy claim is made.
- [ ] One pinned 8192-square imagery run and one 2048-square DEM run each meet 040-foundation-rescope.md's separate resource budgets and cleanup conditions; logical accounting is not called an RSS bound.
- [ ] Native source/worker/member/report completion precedes D1/D2 publication; cancel/observer/storage/close failures preserve prior output or report typed recovery paths. Existing lifecycle/fault suites pass.
- [ ] Rust/native CLI capability/default/report behavior agrees, missing GTiff is classified consistently, and the Python wheel's absence of R2/terrain APIs plus D1 Unsupported behavior are tested/documented. Existing native/portable capability, Python CLI and consumer suites stay green.
- [ ] Link deeper native-lifetime/resource/oracle audits to 0.4.x; #82/#221/#222 remain outside 0.4.0 with new capability delivery targeted at 0.5.
```

### #125 — replacement Acceptance

```markdown
- [ ] Retain C1/I1/C2 and validator scale implementations; document actual supported checks/unsupported domains without promising metadata/topology/continuous-surface proof.
- [ ] Existing independent payload/range/truncation/resource/availability/CRC/index/wrong-bounds corruption suites remain green; known false acceptance in the supported profile is fixed.
- [ ] Final tiny point/vector archives and packaged terrain fixtures pass applicable format/availability/bounds checks; imagery uses its PNG/COG/TileJSON checker. Product validator success does not replace source-to-decoded-output fidelity checks.
- [ ] Keep resource limits/cancellation effective and evidence pinned; move exhaustive non-mesh full-profile VS-D, oracle re-audits and component ledgers to 0.4.x. Mesh final-output/scale acceptance follows its separate approved plan.
```

### #126 — replacement Acceptance

```markdown
- [ ] Reconcile the retained public-surface inventory with Rust exports, CLI/help, Python exports and packages; document removed explicit modes, convert-to-implicit and legacy mesh APIs.
- [ ] Point/vector Rust/CLI/Python settings, decoded semantics and errors agree on small fixtures; imagery/terrain remain native Rust/CLI, D1 Python reports Unsupported, and no unavailable Python capability is advertised.
- [ ] Existing migrated operation publication/cancel/failure and read-only doctor/preview tests pass; correct the terrain/D1 missing-GTiff category mismatch and keep experimental consumer limits explicit.
- [ ] Existing applicable installed-package/platform checks pass; evidence points to actual final candidate sources. No generic API framework, module rewrite or new binding is required.
- [ ] Move exhaustive internal export stabilization, per-utility ledgers and deeper operation/oracle audits to 0.4.x; do not restore retired A2/legacy paths as compatibility work.
```

### Roadmap — exact non-mesh replacement text

Replace the opening “The foundation work remains replacement-first…” policy
and add this paragraph to **Scope and migration policy** (also mirror the approved exception
in `architecture/README.md`/`AGENTS.md` so implementation is not governed by
contradictory blanket requirements):

> For point cloud, vector, imagery and terrain, the finite 0.4.0 release bar is
> `040-foundation-rescope.md`: small analytic decoded-output checks, placement
> and bounds, one realistic resource measurement per producer, safe cancellation
> and publication, truthful frontend capabilities and green existing suites.
> Retain accepted foundation slices within their established domains. Deep
> independent-oracle audits, per-component disposition ledgers and worst-case
> resource proofs continue in 0.4.x; they are not blanket rewrite prerequisites.
> Known wrong output or destructive publication remains a release blocker.

Add to **Milestones 1–4**: M1 retains the merged typed serializer/forest and
existing producer integrations; M2 retains real Tiles 1.1/glTF 2.0 semantics;
M3 adds no non-mesh rewrite. M4's non-mesh acceptance list becomes exactly:

```markdown
- [ ] Point P1/current forest: finite decoded fidelity, placement/bounds, lifecycle, paired CLI/Python and realistic resource receipt accepted.
- [ ] Vector: direct analytic published-forest fidelity, metadata/identity, placement/bounds, reuse/seams, lifecycle, paired CLI/Python and realistic resource receipt accepted.
- [ ] Native imagery R2: literal display/COG/phase/bounds, lifecycle, capability surface and realistic resource receipt accepted.
- [ ] Native terrain T1/packed LOD: existing lattice/height/bounds/LOD/skirt guarantees, lifecycle, capability reporting and realistic resource receipt accepted.
- [ ] Existing applicable non-mesh Rust/Python/native/format/platform/package/consumer suites green on the final candidate, with exclusions and deferred audits recorded.
```

Append to **Status at 2026-10-23** as a dated correction: “2026-10-24 non-mesh
rescope: merged foundation/encoding work is credited as enumerated in
`040-foundation-rescope.md`; the five finite M4 checks above replace blanket
non-mesh audit completion. Nothing is newly marked passed by this amendment.”
Preserve mesh status/scale gates for the separate classic-HLOD proposal. In
**Release sequence**, add “0.4.x includes deferred non-mesh foundation audits
and fixes within existing capabilities; #82/#221/#222 investigation may proceed,
while portable/projected terrain delivery remains 0.5.” Keep 0.6 unchanged.

## 5. Branch recovery and effort comparison

| Supposedly unmerged branch | Static Git/GitHub finding at this baseline | Salvage decision |
| --- | --- | --- |
| `feat/132-point-foundation` @ `8c7562b5` | **Already merged**, PR #134; merge `c4912f8c`. `HEAD...branch` left/right count **886/0** | Retain current P1/oracle/evidence; no commits to cherry-pick. |
| `feat/terrain-foundation` @ `2533104d` | **Already merged**, PR #136; merge `00a9d1c3`. Left/right **876/0** | Retain T1 and later #169/#184. Do not revive old quantized terrain. |
| `feat/c1-payload-validation` @ `d0a237a1` | **Already merged**, PR #137; merge `fdc5d186`. Left/right **860/0** | Retain current C1 plus later #152/#153/#176/#202; old head is superseded, not abandoned work. |

These checks used refs from this worktree; no other checkout was entered or
modified. `HEAD..branch` is empty for all three. Historical evidence removed
by #165 is recoverable from the tracked audit READMEs' pinned commits/hash
manifests; recovering bulk records is not equivalent to rerunning acceptance.
The local branch named `develop` is stale (`2edc308f`); comparisons here use
cda7ca61 / `origin/develop`, avoiding a false “unmerged” conclusion.

| Remaining work, same non-mesh baseline | Point | Vector | Imagery | Terrain | Shared | Total |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Proposed finite 0.4.0 scope | 1.5–2.5 | 3–5 | 1–2 | 1–2 | 1.5–2.5 | **8–14 days** |
| Original broad checklist completion | 8–14 | 14–22 | 8–14 | 7–12 | 8–13 | **45–75 days** |

The original-scope estimate is this proposal's bottom-up planning judgement,
**not an issue estimate or measured productivity ratio**: source/CRS/sampler
breadth for point; topology/repair/metadata/LOD oracle work for vector;
all admitted display/driver/native fault/resource attribution for imagery;
terrain numerical/lifetime breadth; then cross-cutting independent-oracle and
per-component ledger work. It credits the same merged work and excludes mesh,
0.5 capabilities and release promotion. It is open-ended if “all inputs” remains
the contract; 45–75 is a bounded audit programme estimate, not a completeness
proof. The owner's mesh bake-time 20× observation is not extrapolated here.

Deferring those obligations removes **37–61 agent-days** from the matched low/high
planning cases (roughly 80%); it does not finish them. #221/#222 add a separately
quoted **22–26 days** if pulled forward; #82's broader imagery work is unpriced
and already deferred, so neither is inflated into the current release comparison.
Allow **2–5 additional days contingency** if final fixtures/resources expose a
real defect; re-estimate larger findings rather than starting an automatic
replacement programme. No general redesign is presently justified by this
read-only evidence.
