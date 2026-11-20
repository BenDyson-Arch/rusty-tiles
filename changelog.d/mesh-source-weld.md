### Added

- `mesh --weld-epsilon METRES` and `mesh --join-primitives same-material`
  (Python: `weld_epsilon=`, `join_primitives=`; also `mesh-inspect` and
  `inspect_mesh`, to preview the effect) weld the source's vertex positions
  before capture (#349). Vertices of one primitive (or, with `same-material`, of
  primitives sharing a material) within ε pair; the classes are the connected
  components of the pairs, and every member takes the position of the class's
  lowest-index vertex, so the result does not depend on `--jobs` or the budget.
  At 0 only exact repeats merge, which the bake already treats as one position.
  Only positions move: normals, UVs, colours and provenance (`source_primitive`)
  stay each vertex's own, so UV seams are kept. `tilerReport.weld` states the
  merged count, the true largest displacement (a chain can exceed ε) and the
  faces the weld collapsed or made coincident (CLI warning when non-zero).
  Geometric error is certified against the original pre-weld source. A weld that collapses a triangle onto no surviving
  face is refused. The exact-seam rule between output tiles is unchanged. The
  weld is progress steps `capture_weld` and `capture_weld_check`, charged in capture's memory bound
  (`capture_weld_bytes`) and declared scratch. Without the options, output is
  unchanged.
