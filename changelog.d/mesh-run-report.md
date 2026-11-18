### Added

- Mesh: `--report-path FILE` (Python `report_path=`) writes a versioned JSON run report
  (`rusty_tiles_mesh_run_report`, schema version 1), rewritten atomically after each stage:
  wall and CPU seconds, scratch peaks, `omitted_by_depth`, last-resort counts and records,
  and the final status, so a stopped run leaves a valid partial report. The archive is
  unchanged.
- Mesh diagnostics: `hlod.walk_utilisation` measures the coarse walk's busy and idle worker
  seconds by cell depth and the slowest cell's share of each depth's wall time, and
  `stage_timing` holds the whole run's stage timing.

### Changed

- Mesh progress: every phase now reports a total. `capture` and `hlod_incidence` count
  triangles and `archive` its tiles, instead of a null or unit total.
- `RUSTY_TILES_STAGE_TIMING` is read by the CLI, which writes the run's
  `diagnostics.stage_timing` to it; the library no longer reads the environment for it.
