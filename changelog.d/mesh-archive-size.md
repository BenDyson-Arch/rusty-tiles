### Changed

- Mesh tiler (`rusty-tiles mesh` / `mesh_to_tileset`): archives are about 2.5x
  smaller (pohnpei, 24.4M triangles: 2.90 GB to 1.14 GB; 119 to 47 bytes per
  source triangle). The per-triangle provenance tables of leaves and proxies are
  losslessly meshopt-compressed (`EXT_meshopt_compression`, ATTRIBUTES/NONE); their
  schema, column types and decoded values are unchanged. Leaf normals in `fast`,
  `balanced` and `quality` are 12-bit octahedral codes behind the meshopt
  OCTAHEDRAL filter, decoded to normalized SHORT `NORMAL` (at most about 0.06
  degrees from the source normal); `lossless` keeps f32 normals. Profile constants
  remain provisional.
- Validation decodes the `EXT_meshopt_compression` OCTAHEDRAL filter: invalid
  octahedral codes, non-ATTRIBUTES modes and strides other than 4 or 8 are
  invalid input; declared bounds on filtered accessors may differ by one unit (the
  filter's specified precision); QUATERNION and EXPONENTIAL remain unsupported.
