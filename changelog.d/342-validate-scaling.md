### Added

- `validate --memory-budget BYTES` and `--jobs N|auto` (Python
  `validate(..., memory_budget_bytes=, jobs=)`, Rust `ValidationRequest::memory_budget` and
  `::jobs`), with `mesh`'s default policy: the lesser of 4 GiB and half the detected
  headroom. When either is given, the report's new `resources` member states the resolved
  budget and its source, the workers, the store sizes derived from the budget and the
  scratch-disk bound.
  An explicit budget below 512 MiB is refused as `resource_limit` before the archive is
  opened.
- `validate` progress for the HLOD delivery stage: `validate.delivery` (passes done of
  the total) and `validate.delivery.leaves`, `.edges` and `.coarse` (placements replayed
  of the total). Every event, every charged delivery row and every 10,000 SQLite
  operations is a cancellation checkpoint.
- A start-of-run scratch-disk check for `validate`: before traversal, free space is
  compared with the catalog's terms plus, for an HLOD archive, a per-triangle term from the
  triangles its GLB payloads declare. Below 16 bytes per triangle the volume is refused as
  typed insufficient space (`io`); below 256 a `scratch_space` warning event is emitted.

### Changed

- `validate` sizes its external sort runs, merge fan-in and buffers, catalog cache,
  resident-store thresholds and the HLOD delivery store's SQLite page cache from the
  memory budget (SQLite's temporary store stays on disk unless its row bound fits), and
  replays HLOD payload geometry on `--jobs` workers in a bounded lookahead. Verdicts,
  work counts, refusals and every report member except `resources` and `scratch` are
  unchanged and identical for every budget and worker count.

### Fixed

- `validate` creates SQLite's temporary files for the HLOD delivery store (sorter runs,
  temporary tables, statement journals) in its scratch directory instead of SQLite's own
  temporary directory (`/var/tmp` or `TMPDIR`), so `--scratch-dir` holds them.
