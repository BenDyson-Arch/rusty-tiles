# Changelog

## Unreleased

- Every multi-word camelCase option now also accepts a kebab-case alias
  (`--sourceCrs`/`--source-crs`, `--maxPoints`/`--max-points`, …); camelCase stays
  the primary spelling. The `create-tileset-json`, `glbTo3tz` and `meshTo3tz`
  subcommand aliases are now shown in help. Converter help lists `-i`, `-o` and
  `-f` first.
- `doctor --command` accepts aliased subcommand spellings and takes its list from
  the same table as the readiness report. `doctor` reports whether a Cesium
  runtime is present at the README location or `doctor --cesium DIR`
  (informational only).
- `--json` conversion results add a `settings` object; `counts` now holds only
  genuine counts (for example `points`, `tiles`, `features`), and settings such as
  `heightOffset`, `grid` or `lodLevels` moved from `counts` to `settings`.
- Without `--json`, every converter prints a one-to-three-line stderr summary
  (output, counts, warnings, next command). The previous ad hoc point-cloud
  summary and per-level terrain lines are replaced by it.
- Option errors name the offending `--flag` and its valid range instead of
  grouping several rules in one message.
- `validate` explains that it checks `.3tz` archives only when given a directory
  or non-archive file, instead of reporting an OS error.
- `RUSTY_TILES_NATIVE_DIAGNOSTICS=1` enables raw GDAL/GEOS warnings. The old
  `RUSTY_TILES_PYTHON_TRACEBACK` name is deprecated but still honoured.
- Every converter runs as one job: a private `.tiles-work-*` directory beside
  the output, removed on success or failure, and no-clobber publication unless
  `--force`. `glb-to-3tz` (and small `mesh-to-3tz` inputs) no longer stage in a
  fixed `<output>.tileset-work` folder, which deleted any existing folder of
  that name. An output created by another process during a conversion is
  reported as an output conflict (exit 5) instead of an I/O error.
- `raster --progress json` reports `cog`, `display` and `tiling` phases
  (`tiling` counts XYZ tiles). `mesh-to-3tz --progress json` emits its timing
  lines as `{"event":"log","message":...}` so stderr stays NDJSON. Other events,
  phases and exit codes are unchanged.
- Library: new `rusty_tiles::report` module with `Reporter` (silent, human
  stderr, NDJSON stderr or a custom `EventSink`), `Event` and
  `ConversionResult { output, archive, report }`, where `report` is the
  published `conversion.json` value. New entry points return it:
  `point_cloud::point_cloud_to_3tz_reported`, `vector::vector_to_3tz_reported`,
  `terrain::dem_to_terrain_reported`, `raster::raster_reported`,
  `tile::mesh_to_3tz_reported`, `tileset::glb_to_3tz_reported` and
  `pack::convert_to_3tz_reported`. Existing entry points are unchanged wrappers
  using `Reporter::default()` (warnings and notes on stderr, no progress).
- Library: converters no longer read `RUSTY_TILES_PROGRESS_JSON`, and the CLI no
  longer sets it or `RUSTY_TILES_JSON_STDOUT`; library callers that relied on
  the variable for progress should pass `Reporter::ndjson_stderr()` to a
  `*_reported` entry point.
- Breaking library API (CLI unaffected): removed the unused
  `rusty_tiles::split` and `rusty_tiles::compress` modules, including
  `compress::write_glb_compressed`, `compress::write_glb_compressed_with_scale`
  and the `rusty_tiles::write_glb_compressed` re-export. Mesh tiling and its
  meshopt output are unchanged.

## 0.3.0 — unreleased

- Readiness and local preview now run entirely in Rust. Preview embeds its HTML
  and serves only explicitly selected outputs and a locally installed Cesium IIFE.
- `preview --host`/`--port` choose the listen address (default `127.0.0.1:9227`);
  `preview --json` emits one startup object with the bound URL and layers.
- `doctor --command` also accepts `validate` and `preview`.
- A Dockerfile builds a Python-free runtime image linked against GDAL 3.12.4 and
  PROJ 9.8.1 built from source.
- Native geospatial builds require GDAL >= 3.12, PROJ >= 9.2 and GEOS >= 3.10
  for vectors. Default builds retain mesh/archive and local LAS/LAZ conversion.
- Python remains for independent development fixtures/audits. CI runs the native
  acceptance binaries in a matching-library container with no Python installed.
- The converter version changes derivative/cache identity. Prior Python vector
  archives and different native encoder identities require a fresh reuse baseline.

- Native Rust vector ingestion, disk-backed SQLite LOD/reuse and in-process meshopt compression under `native-geospatial`; Python archives require a fresh native baseline.
- Opt-in `vector --aggregatePoints` uses the Rust point-cloud voxel grid for per-layer count aggregates in point-only parents, with metre-bounded placement, explicit aggregate metadata and original full-detail leaves.
- Native GDAL raster COG, image/grayscale display and XYZ tiling APIs under `native-geospatial`, with no Python or GDAL executable dependency; source-resolution mask/alpha intersection and safe publication retained. Compression and tiling use up to four workers, with uncompressed temporary TIFFs to avoid repeated compression.
- Native GDAL terrain sampling and Rust quantized-mesh encoding, with border-preserving mesh simplification. `--maxError` defaults to 1 metre of added elevation and 3D surface error relative to the quantized regular grid; `0` retains the full grid.

## 0.2.0

### Added

- LAS/LAZ point-cloud conversion to 3D Tiles 1.1, with streamed disk-backed spatial partitioning, voxel-sampled parent LOD, full-detail leaves and per-point numeric metadata. Local metre XYZ and explicit horizontal CRS/height-offset placement are supported.
- GeoPackage, GeoJSON and Shapefile vector input through OGR, explicit layer selection, source layer/feature identity and typed scalar metadata.
- Vector parent LOD with metre-based simplification error, disk-backed indexing, configurable feature/vertex/byte budgets and full-detail leaves. Oversized polygons retain filled surfaces and original outlines without internal fragment edges.
- `vector --reuseTileset previous.3tz` reuses unchanged subtrees and immutable content after an externally applied GeoPackage diff. Spatial moves, insertions, deletions and shared boundaries invalidate affected branches; incompatible settings rebuild from source.
- Explicit raster image/grayscale recipes that preserve source COG pixels and masks, with transparent display derivatives for missing coverage.
- Terrain height sidecars and a custom `heightOverlay` manifest entry that retain missing coverage separately from filled terrain heights.
- Opt-in public dataset download/audit helpers and compatibility tests for upstream geodiff 2.3.0 and go-geodiff v0.4.3, including indexed GeoPackage moves, inserts and deletes. Diff creation and application remain external to rusty-tiles.

### Compatibility and limits

- Vector content uses draft glTF vector extensions tested with Cesium 1.143.0. Fragmented polygon fills use standard glTF inside b3dm wrappers alongside vector outlines. This is experimental content, not a finalized 3D Tiles 2.0 format.
- Point-cloud conversion requires Python, NumPy, `laspy[lazrs]` and pyproj. Raster/terrain/vector conversion requires Python, NumPy and GDAL; vector also needs GEOS. Helpers are embedded in the executable, but these Python dependencies must be installed separately.
- Point-cloud compound vertical CRS, waveform payloads and array extra dimensions are unsupported. Height offsets are constant conversions; no vertical datum is guessed.
- Vector reuse scans the updated source and repacks the output archive. It avoids encoding unchanged content; it does not avoid every source-processing step. Buffered grid clipping and implicit tiling remain unimplemented.

## 0.1.0 — 2026-09-11

Initial standalone release: textured mesh conversion and `.3tz` packaging, raster imagery, regular-grid terrain, experimental vector content and a local Cesium preview.
