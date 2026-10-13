### Added

- Mesh tiler (`rusty-tiles mesh` / `mesh_to_tileset`): `--source-crs` (`local` by default, or a
  2D horizontal EPSG/WKT/PROJ definition through the shared conservative resolver) with
  `--source-axes y-up|xyz` (default `y-up`), `--height-offset` (default 0), and either
  `--source-offset E N [A]` added in double precision or `--source-offset-file` holding exactly
  the Metashape `E:`/`N:`/`A:` lines; Python takes `source_crs`, `source_axes`, `source_offset`,
  `source_offset_file` and `height_offset`. Capture maps every position exactly in f64 into one
  ENU frame at the mapped source-bounds centre; normals use the Jacobian inverse transpose and
  tangents the Jacobian followed by Gram-Schmidt against the normal, with Jacobians interpolated
  from an adaptively checked lattice. The root transform carries ENU to ECEF; later stages are
  unchanged. A source CRS excludes `--anchor`, `--orientation-xyzw` and `--scene-offset`. Legacy
  `auto`/`geographic`/Web Mercator keywords are not accepted; EPSG:4326/3857/3395 and UTM work
  in the standard build. The report gains `source_coordinates: "source-crs"` and a `source_crs`
  object.
