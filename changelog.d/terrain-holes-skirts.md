### Changed

- **Breaking:** `terrain` leaves NoData out of the mesh; `--fill-height`,
  `fill_height_metres` and the report's `fillHeightMetres` are removed.
- `terrain` skirts are sized by the tile's and its neighbours' errors, not
  the root error, so boxes follow the surface. `--cells-per-leaf` defaults
  to 128.
- `validate`: a tile without content may state a larger geometric error than
  its parent; tiles with content are checked against their nearest ancestor
  with content.
