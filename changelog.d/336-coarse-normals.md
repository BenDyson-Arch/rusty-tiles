### Added

- `validate` reports coarse HLOD tiles whose corner `NORMAL` points more than
  90° from its face (the face shades against its winding) as the
  `coarseNormalInversion` warning, with counts by cell depth in
  `interfaceCensus.coarseNormals`. Warnings appear in a new report `warnings`
  list, never fail validation (`ok` stays `true`), and the CLI prints them on
  stderr. Archives written before this release usually carry the warning.

### Fixed

- Mesh HLOD coarse tiles no longer shade surfaces with starburst gradients or
  inverted facets when the source carries normals. Coarse `NORMAL` is now
  crease-aware: a vertex splits its normal across edges sharper than 60°, and
  every corner normal lies within 60° of its face, so no face is shaded against
  its own winding. Leaves are unchanged; coarse tiles of sources with normals
  have more vertices (13–15% more coarse bytes on the measured meshes).
