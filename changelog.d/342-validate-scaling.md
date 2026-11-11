### Added

- `validate --memory-budget BYTES` and `--jobs N|auto` (Python
  `validate(..., memory_budget_bytes=, jobs=)`, Rust `ValidationRequest::memory_budget` and
  `::jobs`), with `mesh`'s default policy: the lesser of 4 GiB and half the detected
  headroom. The report's new `resources` member states the resolved budget and its
  source, the workers, the store sizes derived from the budget and the scratch-disk bound.
  An explicit budget below 512 MiB is refused as `resource_limit` before the archive is
  opened.
- `validate` progress for the HLOD delivery stage: `validate.delivery` (passes done of
  the total) and `validate.delivery.leaves`, `.edges` and `.coarse` (placements replayed
  of the total). Every event, every charged delivery row and every 10,000 SQLite
  operations is a cancellation checkpoint.
- A start-of-run scratch-disk preflight for `validate`: before traversal, the estimated
  scratch peak (the catalog's terms plus 7 bytes per stored byte for an HLOD archive) is
  checked against free space and refused as typed insufficient space (`io`) when short.

### Changed

- `validate` sizes its external sort runs, merge fan-in and buffers, catalog cache,
  resident-store thresholds and the HLOD delivery store's SQLite page cache from the
  memory budget (SQLite's temporary store stays on disk unless its row bound fits), and
  replays HLOD payload geometry on `--jobs` workers in a bounded lookahead. Verdicts,
  work counts, refusals and every report member except `resources` and `scratch` are
  unchanged and identical for every budget and worker count.
