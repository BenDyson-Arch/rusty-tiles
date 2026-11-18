### Added

- `mesh --bbox MINX MINY MAXX MAXY` and `mesh --polygon FILE` (Python: `bbox=`,
  `polygon=`) bake only the source triangles whose centroid lies in a box or in
  a GeoJSON Polygon/MultiPolygon (holes allowed, even-odd rule). Boundaries are
  half-open (west and south edges inside), so abutting regions partition the
  source. The region is in post-node glTF x and -z for a local source, and in
  WGS84 longitude and latitude with `--source-crs`. Triangles are not split.
  Placement, the source CRS frame, the root cube and the position lattice's
  origin stay the whole source's, so a subset lines up with a later full bake:
  kept triangles' leaf positions are bit-identical in `quality` and `lossless`
  and within the coarser per-depth position step in `fast` and `balanced`. `tilerReport.subset` states the kept count and extent; an empty
  selection is `invalid_request`. The selection runs before any scratch exists
  (progress step `capture_select`), and excluded triangles cost no scratch.
  Without a region, output is unchanged.
