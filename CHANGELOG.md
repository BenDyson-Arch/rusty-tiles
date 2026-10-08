# Changelog

All notable changes to rusty-tiles are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## 0.4.0 - Unreleased

### Added

- GeoJSON and GeoPackage vector conversion in the default binary and Python wheel, using streaming Rust readers, bundled SQLite, grid-free CRS transforms and constrained polygon triangulation. Both builds share tiling, metadata, LOD, compression and reuse; `native-geospatial` retains OGR/GEOS/PROJ for all vector inputs. Python adds `vector_to_3tz`, and `doctor` reports supported inputs and CRS limits.
- Grid-free point-cloud CRS transforms in the default binary and Python wheel using `proj4rs`: WGS84 geographic, UTM, Mercator and supported WKT/PROJ local projections with explicit Helmert datum shifts. Unsupported grids, epochs or unverified datums require strict native GDAL/PROJ; shifts are never silently discarded. `doctor` lists CRS classes and native fallback readiness. Python point-cloud conversion adds `source_crs` and `height_offset` keywords.
- Shared public `metadata` types and aligned GLB/property-table authoring for `EXT_mesh_features`, `EXT_structural_metadata` and implicit tile bounds/error semantics. Vector payloads and default converter output bytes are preserved.
- `mesh-to-3tz --node-features` keeps source nodes pickable and styleable by `name` and `node_index` through all LODs, including small inputs and instanced meshes. Unnamed nodes use `node_<index>`.
- `point-cloud --metadata-attributes` also exposes classification, intensity and return number as property attributes, retaining existing lossless property tables.
- Python bindings via PyO3 and maturin: path-based mesh, glTF/GLB, local LAS/LAZ and tileset conversion, archive validation, progress callbacks and typed exceptions. Tag builds smoke-test CPython stable ABI wheels on the five release platforms and publish to PyPI with trusted publishing.
- Opt-in native `demodata-suite` acceptance and benchmarks: pinned public inputs, smoke/core/scale profiles, conversion timing and memory, payload repeatability, and geometry/metadata/tile audits. Normal CI runs only the fast harness checks.
- 3D Tiles 1.1 implicit tiling is the default for point clouds, vectors and meshes. Binary subtrees carry Morton-ordered availability and standard tile bounds/error metadata. Meshes and point clouds use midpoint octrees; vectors preserve padded boxes and reuse. Bounds that exceed regular subtree cells use external implicit tileset roots so Cesium can refine and pick them. `--explicit` preserves the earlier output bytes.
- Library: `implicit::Subtree`, `TileMetadata` and `expand_tileset` support quadtree/octree availability, multiple contents, child-subtree links, semantic metadata and bounded expansion for audits. Built-in archive validation checks native implicit output.
- Tag-triggered release binaries for Linux and macOS (x86_64 and ARM64) and Windows x64, a native geospatial GHCR image, a SHA-256-verifying installer and `cargo-binstall` metadata.
- Fixed converter recipes and committed output digests, cross-commit comparison tooling and a manual release acceptance workflow. CI fails when the Python acceptance binary is missing and checks native Clippy warnings.
- Every multi-word camelCase option now also accepts a kebab-case alias, such as `--sourceCrs`/`--source-crs` and `--maxPoints`/`--max-points`. camelCase stays the primary spelling. The `create-tileset-json`, `glbTo3tz` and `meshTo3tz` subcommand aliases are now shown in help.
- `doctor` reports whether a Cesium runtime is present at the README location or at `doctor --cesium DIR`. This check is informational only.
- `raster --progress json` reports `cog`, `display` and `tiling` phases. `tiling` counts XYZ tiles.
- Library: new `rusty_tiles::report` module with `Reporter`, `Event` and `ConversionResult { output, archive, report }`. `Reporter` can be silent, human stderr, NDJSON stderr or a custom `EventSink`. `report` is the published `conversion.json` value.
- Library: new entry points return a `ConversionResult`: `point_cloud::point_cloud_to_3tz_reported`, `vector::vector_to_3tz_reported`, `terrain::dem_to_terrain_reported`, `raster::raster_reported`, `tile::mesh_to_3tz_reported`, `tileset::glb_to_3tz_reported` and `pack::convert_to_3tz_reported`. Existing entry points are unchanged wrappers using `Reporter::default()`, which writes warnings and notes on stderr and no progress.

### Changed

- Installation and converter guides distinguish development capabilities from published releases, explain build selection, and retain the existing mesh offset options with application-neutral help.
- JPEG uses the portable Rust encoder by default. System libjpeg-turbo now requires the explicit `native-jpeg` feature; `RUSTY_TILES_DISABLE_NATIVE_JPEG=1` still overrides it for portable builds.
- README installation starts with prebuilt downloads. A simpler header figure and an invented mesh example make the quick start work with the default build.
- Doctor, machine protocol, diagnostics, preview, force replacement and archive validation tests now run in Rust. The vector Python oracle loads only in tests that compare it with the native converter.
- Benchmark harnesses, public-data audits and recorded evidence now live in `bench/`.
- Converter help lists `-i`, `-o` and `-f` first.
- `doctor --command` accepts aliased subcommand spellings. It takes its list from the same table as the readiness report.
- `--json` conversion results add a `settings` object. `counts` now holds only genuine counts, such as `points`, `tiles` and `features`. Settings such as `heightOffset`, `grid` or `lodLevels` moved from `counts` to `settings`.
- Without `--json`, every converter prints a one-to-three-line stderr summary: output, counts, warnings and next command. It replaces the previous ad hoc point-cloud summary and the per-level terrain lines.
- Option errors name the offending `--flag` and its valid range, instead of grouping several rules in one message.
- `validate` explains that it checks `.3tz` archives only when given a directory or non-archive file, instead of reporting an OS error.
- `RUSTY_TILES_NATIVE_DIAGNOSTICS=1` enables raw GDAL/GEOS warnings.
- Every converter runs as one job. It uses a private `.tiles-work-*` directory beside the output, removed on success or failure, and no-clobber publication unless `--force`.
- `mesh-to-3tz --progress json` emits its timing lines as `{"event":"log","message":...}`, so stderr stays NDJSON. Other events, phases and exit codes are unchanged.
- Library: converters no longer read `RUSTY_TILES_PROGRESS_JSON`. The CLI no longer sets it or `RUSTY_TILES_JSON_STDOUT`. Library callers that relied on the variable for progress should pass `Reporter::ndjson_stderr()` to a `*_reported` entry point.

### Deprecated

- `RUSTY_TILES_PYTHON_TRACEBACK` is deprecated in favour of `RUSTY_TILES_NATIVE_DIAGNOSTICS`, but still honoured.

### Removed

- Breaking library API, CLI unaffected: removed the unused `rusty_tiles::split` and `rusty_tiles::compress` modules. This includes `compress::write_glb_compressed`, `compress::write_glb_compressed_with_scale` and the `rusty_tiles::write_glb_compressed` re-export. Mesh tiling and its meshopt output are unchanged.

### Fixed

- Release acceptance checks the README route on default binaries as well as native builds. Missing optional raster/terrain support is reported separately from required mesh, validation and preview readiness.
- Georeferenced polygons with constant source height now retain their source XY topology through globe placement, LOD and fragmentation. This restores Sudan and Antarctica fills in country conversions without changing source vertices or relaxing repair safeguards. Polar seam fragments retain original boundaries and conservative bounds. Previous vector archives need a fresh conversion to use the corrected encoder.
- Native placement normalizes projected horizontal units and projection offsets to metres before adding an explicit metre height axis. This preserves feet, US survey feet and kilometre inputs under PROJ 9.9, including Albers and Helmert definitions, without scaling source Z.
- Point-cloud WKT parameters retain their declared projection method: original names and EPSG identities are validated before flattening aliases; incomplete or mismatched sets, unverified spellings and colliding native-exported aliases use native interpretation. Albers (spherical or ellipsoidal) refuses source latitudes at or beyond ±80° in both builds because portable and native inverses can share a polar clamp. Native conic conditioning inspects canonical method/parameter identities and angular units, including Michigan and Belgium variants. Polar-point regressions verify native refusal as well as successful fallback across platforms.
- Point-cloud polar stereographic preserves native hemisphere interpretation for conflicting origins/parallels and rejects opposite-pole scale conflicts. Failed stereographic latitude probes now permit native retry, and nonpolar ordinary stereographic also checks each point’s latitude. Native LCC 1SP variant B conditioning uses its natural origin, including quoted definitions and exported WKT.
- Point-cloud LCC definitions with omitted origins or second parallels retain native CRS interpretation. Polar stereographic rejects conflicting scale/standard-parallel parameters. Ill-conditioned nearly opposite conic parallels are refused in both builds; near-pole conic parallels use native PROJ. Oblique stereographic checks each point’s source latitude and retries the whole batch through native PROJ when needed.
- Point-cloud conic projections with distinct parallels less than one degree apart and ordinary stereographic origins from 80 degrees to below 90 degrees latitude use strict native PROJ for accuracy. Exact polar stereographic remains portable. PROJ `+init`, spaced assignments and additional WKT syntax retain native fallback; spaced EPSG codes are normalized.
- Point-cloud CRS validation rejects invalid latitude parameters and angular projected PROJ units. Oblique stereographic origins at or beyond 80 degrees latitude use strict native PROJ to avoid near-pole numerical instability. EPSG coordinate epochs and quoted PROJ values retain native fallback.
- Point-cloud CRS validation rejects nonpositive projection scales and missing UTM zones before publishing. DMS prime meridians, spherical transverse Mercator and polar oblique stereographic retain strict native fallback. Lambert azimuthal equal area uses native PROJ to meet the 1 mm accuracy threshold.
- Point-cloud geographic `+lon_0` offsets use strict native PROJ instead of being silently ignored by the portable tier. Valid DMS angular parameters retain native fallback; default builds name the required feature.
- `glb-to-3tz` bundles referenced local buffers, images and structural metadata schemas from glTF/GLB inputs without rewriting source bytes. Missing, unsupported or escaping resource URIs fail before publication.
- `raster --display gray --alphaBand` preserves explicit transparency from numeric alpha bands, intersected with the selected band's mask/NoData before resampling.
- Default implicit mesh output accepts uppercase and mixed-case glTF/GLB extensions consistently with the mesh loader.
- Archive validation traverses nested external tilesets with a work queue, retaining cycle/depth checks without recursive stack growth.
- Published `.3tz` files honor the process umask instead of retaining temporary-file mode 0600.
- `doctor --command` limits both human and JSON command inventories to the selected commands.
- Empty triangle grids allocate one cell and return immediately from nearest-surface searches.
- `glb-to-3tz`, and small `mesh-to-3tz` inputs, no longer stage in a fixed `<output>.tileset-work` folder, which deleted any existing folder of that name.
- An output created by another process during a conversion is reported as an output conflict, exit 5, instead of an I/O error.

## 0.3.0 - 2026-10-07

### Added

- `preview --host` and `--port` choose the listen address. The default is `127.0.0.1:9227`. `preview --json` emits one startup object with the bound URL and layers.
- `doctor --command` also accepts `validate` and `preview`.
- A Dockerfile builds a Python-free runtime image linked against GDAL 3.12.4 and PROJ 9.8.1 built from source.
- Opt-in `vector --aggregatePoints` uses the Rust point-cloud voxel grid for per-layer count aggregates in point-only parents. It has metre-bounded placement, explicit aggregate metadata and original full-detail leaves.
- Terrain `--maxError` defaults to 1 metre of added elevation and 3D surface error relative to the quantized regular grid. `0` retains the full grid.

### Changed

- Readiness and local preview now run entirely in Rust. Preview embeds its HTML and serves only explicitly selected outputs and a locally installed Cesium IIFE.
- Native geospatial builds require GDAL >= 3.12, PROJ >= 9.2 and GEOS >= 3.10 for vectors. Default builds retain mesh/archive and local LAS/LAZ conversion.
- Python remains for independent development fixtures and audits. CI runs the native acceptance binaries in a matching-library container with no Python installed.
- The converter version changes derivative and cache identity. Prior Python vector archives and different native encoder identities require a fresh reuse baseline.
- Native Rust vector ingestion, disk-backed SQLite LOD and reuse, and in-process meshopt compression under `native-geospatial`. Python archives require a fresh native baseline.
- Native GDAL raster COG, image/grayscale display and XYZ tiling APIs under `native-geospatial`, with no Python or GDAL executable dependency. Source-resolution mask/alpha intersection and safe publication are retained. Compression and tiling use up to four workers, with uncompressed temporary TIFFs to avoid repeated compression.
- Native GDAL terrain sampling and Rust quantized-mesh encoding, with border-preserving mesh simplification.

## 0.2.0

### Added

- LAS/LAZ point-cloud conversion to 3D Tiles 1.1, with streamed disk-backed spatial partitioning, voxel-sampled parent LOD, full-detail leaves and per-point numeric metadata. Local metre XYZ and explicit horizontal CRS/height-offset placement are supported.
- GeoPackage, GeoJSON and Shapefile vector input through OGR, explicit layer selection, source layer/feature identity and typed scalar metadata.
- Vector parent LOD with metre-based simplification error, disk-backed indexing, configurable feature/vertex/byte budgets and full-detail leaves. Oversized polygons retain filled surfaces and original outlines without internal fragment edges.
- `vector --reuseTileset previous.3tz` reuses unchanged subtrees and immutable content after an externally applied GeoPackage diff. Spatial moves, insertions, deletions and shared boundaries invalidate affected branches. Incompatible settings rebuild from source.
- Explicit raster image/grayscale recipes that preserve source COG pixels and masks, with transparent display derivatives for missing coverage.
- Terrain height sidecars and a custom `heightOverlay` manifest entry that retain missing coverage separately from filled terrain heights.
- Opt-in public dataset download/audit helpers and compatibility tests for upstream geodiff 2.3.0 and go-geodiff v0.4.3, including indexed GeoPackage moves, inserts and deletes. Diff creation and application remain external to rusty-tiles.
- Compatibility: vector content uses draft glTF vector extensions tested with Cesium 1.143.0. Fragmented polygon fills use standard glTF inside b3dm wrappers alongside vector outlines. This is experimental content, not a finalized 3D Tiles 2.0 format.
- Compatibility: point-cloud conversion requires Python, NumPy, `laspy[lazrs]` and pyproj. Raster, terrain and vector conversion require Python, NumPy and GDAL. Vector also needs GEOS. Helpers are embedded in the executable, but these Python dependencies must be installed separately.
- Limit: point-cloud compound vertical CRS, waveform payloads and array extra dimensions are unsupported. Height offsets are constant conversions. No vertical datum is guessed.
- Limit: vector reuse scans the updated source and repacks the output archive. It avoids encoding unchanged content. It does not avoid every source-processing step. Buffered grid clipping and implicit tiling remain unimplemented.

## 0.1.0 - 2026-09-11

### Added

- Initial standalone release: textured mesh conversion and `.3tz` packaging, raster imagery, regular-grid terrain, experimental vector content and a local Cesium preview.
