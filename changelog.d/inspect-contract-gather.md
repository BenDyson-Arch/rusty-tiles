### Changed

- `mesh-inspect`: the coarse-omission forecast reads the shared-border contracts
  of every leaf and cell in one pass over the border store, sorted by cell,
  instead of following each contract's linked list through the store. Each list
  jumped across the whole store, so every item cost a separate small random
  read and every store page was read again for each list crossing it. On Walem
  ×8 (80 M triangles) the forecast walk read about 113 GB to cover a few hundred
  MB of contracts; on a private model of about 390 M triangles it ran for hours
  in I/O wait. The report is unchanged.
- `mesh` and `mesh-inspect` report progress, with totals, for the shared-border
  contracts built after the incidence pass: `hlod_borders_rows`,
  `hlod_borders_graph`, `hlod_borders_faces`, `hlod_borders_levels`,
  `hlod_borders_publish` and, when the row sort merges, `hlod_borders_merge`.
  `mesh-inspect` also reports `inspect_contracts` and `inspect_forecast`. These
  stages were silent before.
