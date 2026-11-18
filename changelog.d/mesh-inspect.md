### Added

- `rusty-tiles mesh-inspect` and Python `rusty_tiles.inspect_mesh` (returning
  `InspectResult`): a pre-bake check of a `mesh` source that writes no archive (#348).
  - Exact counts, on exact positions: open edges and loops by length class,
    primitive-boundary edges and how many coincide with another primitive's edge,
    repeated positions and the edges welding them would close, unwelded seams and
    connected components.
  - An estimated coarse-omission risk per cell, over the cells the bake would build.
    It uses the same capture, cell tree and border contracts, and the bake's
    propagation and root recovery.
  - The estimate is modest. On four measured bakes its omitted-cell precision was
    0.13–1.00 and its recall 0.33–1.00, and it found 3 of 18 actual omission sources.
    Only `certain` cells (a border contract past the triangle cap) follow the bake's
    rule exactly, and that has been checked only on a synthetic case.
  - `--geojson` writes the open loops and seams as ordered polylines, and the cells at
    risk, in WGS84. An unplaced source needs `--geojson-local`, which writes labelled
    local metres.
  - The report does not depend on `--jobs`. Memory and scratch are admitted under
    `--memory-budget`.
