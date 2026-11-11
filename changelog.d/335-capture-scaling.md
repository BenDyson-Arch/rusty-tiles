### Changed

- `mesh` capture sizes its external sorts from the memory budget: runs grow until
  every capture sort merges once (fan-in 64, 128 or 256), and full runs are sorted
  and written by background writers while the next fills. The plan is charged in
  capture's declared memory bound and reported as `diagnostics.capture.sort`;
  the fixed configuration stays the floor, so the minimum budget and refusals are
  unchanged. Archives are byte-identical for every budget and `--jobs`.
- The coarse stage's `hlod_incidence` pass plans its item and endpoint sorts the same
  way within the coarse stage's admission (floor fan-in, so no new scratch refusals),
  reported as `diagnostics.memory.hlod.incidence_sort`.

### Added

- `mesh` capture progress: `capture_scan`, `capture_validate`, `capture_corners`,
  `capture_positions`, `capture_triangles`, `capture_sequence`, `capture_images`,
  `capture_tree`, `capture_corner_depths`, `capture_leaf_plans` and
  `capture_merge` phases with totals, repeated at least every 250 ms, and
  `hlod_incidence_items`, `hlod_incidence_reduce`, `hlod_incidence_ends` and
  `hlod_incidence_merge` inside `hlod_incidence`. They are cancellation
  checkpoints, so a cancel or Python callback exception during capture or the
  incidence pass lands within about a second.
