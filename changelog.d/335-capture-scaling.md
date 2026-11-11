### Changed

- `mesh` capture sizes its external sorts from the memory budget: runs grow until
  every capture sort merges once (fan-in 64, 128 or 256), and full runs are sorted
  and written by background writers while the next fills. The plan is charged in
  capture's declared memory bound and reported as `diagnostics.capture.sort`;
  the fixed configuration stays the floor, so the minimum budget and refusals are
  unchanged. Archives are byte-identical for every budget and `--jobs`.

### Added

- `mesh` capture progress: `capture_scan`, `capture_validate`, `capture_corners`,
  `capture_positions`, `capture_triangles`, `capture_sequence`, `capture_images`,
  `capture_tree`, `capture_corner_depths`, `capture_leaf_plans` and
  `capture_merge` phases with totals, repeated at least every 250 ms. They are
  cancellation checkpoints, so a cancel or Python callback exception during
  capture lands within about a second.
