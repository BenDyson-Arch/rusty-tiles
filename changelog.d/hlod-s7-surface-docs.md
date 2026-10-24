### Added

- `mesh --memory-budget BYTES` (`--memoryBudget`), Rust `TilerRequest::with_memory_budget`
  and Python `mesh_to_tileset(memory_budget_bytes=...)` declare one memory ceiling
  for the run, with one rule on every surface. The value is a positive byte count
  (the CLI also takes `B`/`KiB`/`MiB`/`GiB`/`TiB` suffixes, Python an `int`).
  Zero, signs, fractions, decimal-unit suffixes, booleans and overflow are
  `invalid_request`, refused before any source I/O. Without one, the budget is the
  lesser of 4 GiB and half the detected host/cgroup headroom, or 2 GiB when it
  cannot be detected; an explicit budget is never raised. `diagnostics.memory`
  reports the requested and resolved budget and each stage's minimum. The
  default coarse-tile method does not enforce a budget, so it refuses one
  (`invalid_request`) instead of ignoring it; the HLOD method does.

### Changed

- Documentation: the mesh guide, CLI reference and Python README describe the 0.4
  HLOD coarse levels. Leaves stay exact. Coarse tiles are crack-free by
  construction (locked outer interfaces), carry a **sampled, measured** geometric
  error rather than a certificate, and carry one representative `source_primitive`
  ID per triangle (exact provenance stays on leaves, so exact annotation maps to
  leaves). Coarse textures are fresh content-sized charts, and the profiles pick
  one colour codec for leaves and coarse pages (ETC1S for `fast`/`balanced`, UASTC
  for `quality`, PNG for `lossless`). The certified-proxy, `proxy_source` and
  bake-ledger descriptions are kept only in a marked legacy section until that
  path is removed. The H16, H17 and H28 contracts carry HLOD amendment notes.
- Migration (effective when HLOD becomes the default): read `source_primitive`
  rather than coarse `proxy_source`; read leaves for exact provenance; stop relying
  on a coarse `geometricError` as a distance bound; expect phases `hlod_incidence`
  and `hlod_geometry` and the `memory` and `hlod` diagnostics in place of the
  `lod_*`/`proxy_bake` ones. See the migration table in `docs/MESH.md`.
