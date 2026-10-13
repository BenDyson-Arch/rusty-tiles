### Added

- Mesh tiler (`rusty-tiles mesh` / `mesh_to_tileset`): `--source-crs` (`local` by default, or a
  2D horizontal EPSG/WKT/PROJ definition through the shared conservative resolver) with required
  `--source-axes xyz|y-up` and `--height-offset`, and `--source-offset E N [A]` added in double
  precision; Python takes `source_crs`, `source_axes`, `source_offset` and `height_offset`. Capture
  maps positions in f64 into one ENU frame at the mapped source-bounds centre, transforms normals
  by each vertex's Jacobian inverse transpose and tangents by the Jacobian, and the root transform
  carries ENU to ECEF; later stages are unchanged. A source CRS excludes `--anchor`,
  `--orientation-xyzw` and `--scene-offset`. Legacy `auto`/`geographic`/Web Mercator keywords and
  offset files are not accepted; EPSG:4326/3857/3395 and UTM work in the standard build. The
  report gains `source_coordinates: "source-crs"` and a `source_crs` object.
