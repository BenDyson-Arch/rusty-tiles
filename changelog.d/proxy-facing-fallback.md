### Fixed

- `mesh` no longer refuses real photogrammetry with "proxy has no facing
  same-primitive source appearance donor". When simplification turns a proxy
  face away from every source triangle of its primitive, its texels sample the
  nearest same-primitive source surface regardless of orientation.

### Added

- `mesh --progress json` reports the work inside the leaf stage as new phases
  (`leaf_build`, `lod_prepass`, `lod_simplify`, `seam_resolution`, `lod_finish`,
  `proxy_bake`, `archive`) with the unchanged progress schema, and logs each
  phase's (and each LOD and bake depth's) wall seconds and the run's elapsed
  seconds. The run diagnostics add `phase_seconds` and `proxy_bake` (tiles,
  serial retries, per-step worker seconds and appearance fallback counts).
