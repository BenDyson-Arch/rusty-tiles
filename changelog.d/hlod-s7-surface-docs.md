### Added

- `mesh --memory-budget BYTES` (`--memoryBudget`), Rust `TilerRequest::with_memory_budget`
  and Python `mesh_to_tileset(..., memory_budget_bytes=None)` declare the run's memory
  ceiling, with one rule on every surface: a positive byte count (the CLI also takes
  `B`, `KiB`, `MiB`, `GiB` and `TiB` suffixes). Zero, signs, fractions, exponents,
  decimal units, booleans (Python `TypeError`) and overflow are `invalid_request`
  (CLI usage error, exit 2), refused before any file is read. The default is the lesser
  of 4 GiB and half the detected headroom (2 GiB when undetected). A budget is a
  ceiling that is never raised; a run that cannot fit under it is a `resource_limit`
  refusal before the binding stage allocates, and the archive is identical for every
  budget that admits the run.
- Run diagnostics always carry `memory` (the resolved budget, its source and each
  stage's minimum) and `hlod` (tiles, workers, CPU, lease peak, sampled-error
  measurement and per-depth appearance), and `phase_seconds` with the phases
  `capture`, `leaf_build`, `hlod_incidence`, `hlod_geometry` and `archive`.
  `mesh --progress json` emits those phases and logs each phase's wall seconds.

- `validate --expectProfile hlod` (`--expect-profile`), Rust
  `ValidationRequest::expect_profile` and Python `validate(..., expect_profile="hlod")`
  require the HLOD profile whatever an archive's labels say: every payload must carry
  source identity and every HLOD check runs. An archive that declares HLOD itself is
  held to it anyway. Any other profile name is a usage error (`ValueError` in Python).

### Changed

- `mesh` builds its coarse tiles with the 0.4 classic octree HLOD only. Leaves stay
  exact. Coarse tiles are crack-free by construction (locked outer interfaces), carry
  a **sampled, measured** geometric error rather than a certificate, and carry one
  representative `source_primitive` feature ID per triangle (exact provenance stays on
  leaves, so exact annotation maps to leaves). Coarse textures are fresh
  content-sized charts, and a profile picks one colour codec for leaves and coarse
  pages (ETC1S for `fast` and `balanced`, UASTC for `quality`, PNG for `lossless`).
  A parent whose locked interface cannot fit the 8,192-triangle cap is omitted (with
  every ancestor) instead of refusing the run, and the capped descendant frontier is
  published; a coarse page that no codec step can deliver within its gates falls back to
  its exact level-0 PNG, sampled without mips. The mesh guide, CLI reference and Python
  README describe this; the H16 and H28 contracts carry HLOD notes. Where an earlier
  entry in this release (the H16 mesh delivery) says coarse tiles are certified, read
  it as superseded by this one.

### Removed

- Unreleased development builds had certified coarse proxies with a separate texture
  bake, and a hidden `--lod` switch. They are removed outright, with no legacy mode,
  and no release publishes their output: coarse tiles carry no `proxy_source`,
  `lod_vanished` or `lod_filled` tables, the diagnostics carry no `proxy_bake`, and the
  `lod_prepass`, `lod_simplify`, `seam_resolution`, `lod_finish` and `proxy_bake`
  phase seconds are gone. The scratch diagnostics `lod_prepass` and
  `lod_prepass_scratch_peak_bytes` are now `hlod_incidence` and
  `hlod_incidence_scratch_peak_bytes`. The migration table in `docs/MESH.md` maps them.
