### Added

- Mesh: `--report-path FILE` (Python `report_path=`) writes a versioned JSON run report
  (`rusty_tiles_mesh_run_report`, schema version 1), rewritten atomically after each stage:
  wall and CPU seconds, scratch peaks, `omitted_by_depth`, last-resort counts and records,
  and the final status, so a stopped run leaves a valid partial report. The archive is
  unchanged. A report another program holds open is retried, then reported stale in a
  warning; a hard kill mid-write can leave a `.rusty-tiles-report-*.tmp` beside it.
- Mesh diagnostics: `hlod.walk_utilisation` measures the coarse walk's busy and idle worker
  seconds by cell depth and the slowest cell's share of each depth's wall time.
  `stage_timing` holds the whole run's stage timing, only when asked for
  (`TilerRequest::with_stage_timing`; the CLI asks when `RUSTY_TILES_STAGE_TIMING` is set).

### Changed

- Mesh progress: every phase now reports a total. `capture` and `hlod_incidence` count
  triangles and `archive` its tiles, instead of a null or unit total.
- `RUSTY_TILES_STAGE_TIMING` is read by the CLI instead of the library, which changes when
  the file is written: only after a successful `mesh` run (a failed run, including one
  that fails during archive writing, writes none), where the library wrote it as the
  coarse stage ended. Its counters cover the run from its start instead of the whole
  process; for one CLI run that is the same work. The document's format is unchanged.
- CLI: on failure, secondary diagnostics (cleanup problems, a run report that could not
  record the failure) are printed on stderr too, as `warning:` lines or, with
  `--progress json`, `warning` events with `code` `secondary_diagnostic`.
- Rust API (breaking): `TilerDiagnostics` is now `#[non_exhaustive]`, so code outside the
  crate can no longer build it with a struct literal (it gained `stage_timing`). Read it;
  the tiler builds it.
