### Fixed

- HLOD: a coarse cell whose whole build ladder fails because P1 cannot remove some child
  border vertices onto its shared chords now keeps them instead (D). Each kept vertex splits
  the one stored chord it lies on; every cell sharing that chord publishes the same pieces,
  rebuilding when its own build used the chord unsplit, so seams stay exact. A piece is
  certified over its own source subarc and admitted only within the chord's ladder-step
  tolerance; every piece floors G as relieved chords do, and coarser relieved chords over a
  split arc floor G with its larger finer displacement. Kept vertices and rebuilt cells are
  reported in the diagnostics (`kept_vertices`, `rebuilt_cells`); where no cell fails, the
  archive is unchanged.

### Changed

- HLOD: a built cell holds its inputs until the other owners of its chords have built (D),
  within a budget planned before the walk: cells are admitted in postorder while the holds
  that can overlap stay within the budget, and an admitted cell starts only after every
  hold ending before it is released, so the bound holds for any number of workers. A cell
  the plan cannot admit seals its chords (no vertex is kept on them). The held-spill bound,
  and with it the stage's scratch forecast, now covers the budget beside the walk's child
  window. Diagnostics report the budget, the least budget that seals nothing, sealed cells
  and refused vertices (`keep_hold_budget`, `keep_plan_peak`, `keep_sealed_cells`,
  `keep_sealed`) and the hold peaks (`keep_peak_waiting`, `keep_peak_held_triangles`).
