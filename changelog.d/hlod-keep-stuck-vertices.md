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
