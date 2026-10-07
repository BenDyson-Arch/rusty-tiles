# Changelog

## 0.3.0 — unreleased

- Readiness and local preview now run entirely in Rust. Preview embeds its HTML
  and serves only explicitly selected outputs and a locally installed Cesium IIFE.
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
