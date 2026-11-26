### Changed

- Breaking: `mesh-inspect` (Python `inspect_mesh`) no longer captures the
  source or forecasts coarse omissions by default. It reads the source once
  and reports its topology: open edges and loops, primitive boundaries,
  repeated positions, components, the weld preview and the optional GeoJSON of
  loops and seams. Used vertices and edges are written to scratch in ranges of
  position and counted in parallel on the `--jobs` workers, within the memory
  budget. On Walem (10 M triangles) and an 80 M-triangle copy of it, with
  `--jobs 4`, it takes about 1 % of a bake's wall time: 7 s and 54 s, where it
  took 65 s and 931 s with the forecast. The topology counts are unchanged.
- `mesh-inspect --forecast` (Python `forecast=True`) keeps the coarse-omission
  estimate, which captures the source as `mesh` does and costs most of a bake.
  Without it, `inspectReport` omits `hlod_view`, `tree` and `forecast` (its
  `schema_version` is now 2), `geojson.cells` is 0, and the summary's
  last line says the forecast was not run. `diagnostics` reports `jobs`, `jobs_limited_by` and,
  without the forecast, its `partitions`; the capture and forecast fields are
  present only with `--forecast`.
- `inspectReport.source.surface_area_m2` is now the correctly rounded sum of
  the triangles' areas, independent of their order, so it may differ from
  earlier versions in the last bits (by 2.5e-11 m² of 20,598 m² on Walem).
- New `mesh-inspect` progress steps, each with a total: `inspect_scan`,
  `inspect_validate`, `inspect_sample`, `inspect_stream` and
  `inspect_components`.
