### Added

- `mesh`: a hidden `--lod hlod` coarse-tile method (Rust `TilerRequest::with_lod`)
  builds 0.4 octree HLOD geometry instead of H17 (slice S1 of the HLOD decision).
  Every occupied cell above the unchanged, byte-identical leaves is its children's
  union, simplified with meshopt to at most 8,192 triangles. Only the outer
  interface with other ownership subtrees is locked, as complete decoded segments
  and points at the leaves' own decoded coordinates. The bytes are decoded and
  checked before they are published, so independently refined neighbours agree at
  every depth mix. Coarse tiles are untextured and carry a provisional error (the
  largest of the children's errors and meshopt's estimate) until the error and
  appearance slices land. H17 remains the default.
- `mesh --lod hlod`: a hidden `--memory-budget BYTES` (`KiB`/`MiB`/`GiB`/`TiB`
  suffixes; Rust `with_memory_budget`) declares the run's memory ceiling.
  Without it the budget is the lesser of 4 GiB and half the detected headroom,
  or 2 GiB when that is unknown. Capture, leaves and the HLOD stage plan against
  it and are refused before they allocate when one job cannot fit. Diagnostics
  report each stage's minimum and the HLOD stage's lease peak.

### Fixed

- `mesh`: image-free input no longer reserves a leaf texture workspace (about
  3 GiB for `balanced`), and the leaf stage refuses a plan whose single job does
  not fit available memory even when it builds no page.
