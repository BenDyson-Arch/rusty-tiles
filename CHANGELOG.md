# Changelog

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
