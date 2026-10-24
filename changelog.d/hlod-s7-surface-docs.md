### Added

- Hidden, not yet public: `mesh --memory-budget BYTES` (`--memoryBudget`), Rust
  `TilerRequest::with_memory_budget` (`#[doc(hidden)]`) and the private Python keyword
  `_memory_budget_bytes` share one parser and validator, refuse zero, signs,
  fractions, decimal units, booleans and overflow as `invalid_request` before any
  source I/O, and report the resolved budget (default: the lesser of 4 GiB and half
  the detected headroom, 2 GiB when undetected; an explicit budget is never raised).
  They are hidden until HLOD is the default coarse-tile method, because the
  default method cannot enforce a budget and refuses one. The planned surface is
  described in the mesh guide, marked as not a current option.

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
