### Added

- `rusty-tiles mesh-inspect` and Python `rusty_tiles.inspect_mesh`: a pre-bake check of a
  `mesh` source that writes no archive (#348). On exact positions it reports open edges and
  loops by length class, primitive-boundary edges and how many coincide with another
  primitive's edge, repeated positions and the edges welding them would close, unwelded
  seams and connected components. It also forecasts coarse omission per cell over the cells
  the bake would build, labelled an estimate: the same capture, cell tree, incidence pass and
  border contracts, the bake's omission propagation and root recovery, and a load score from
  the children's border points and border-crossing open boundaries against the triangle cap.
  Cells whose border contract alone exceeds the cap are reported as certain. `--geojson`
  writes the open loops, seams and cells at risk (WGS84 when placed). The report is the same
  for every `--jobs`; memory and scratch are admitted under `--memory-budget`.
