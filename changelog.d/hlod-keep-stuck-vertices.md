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

- HLOD: D keeps no vertex on a chord that has an owner under another parent cell earlier in
  the walk (`keep_refused_earlier` counts them). A built cell then holds its inputs only
  until its siblings sharing its chords have built, which its parent waits for anyway, and
  a cell builds once its earlier co-owners under other parents are final, reading their
  splits. No wait outlasts the walk's own order, so the walk keeps its parallelism and the
  held tiles stay within the child window's spill bound. Diagnostics report the hold
  peaks (`keep_peak_waiting`, `keep_peak_held_triangles`) and every cell's keep rounds and
  how they ended (`keep_records`, `keep_outcomes`).
- HLOD: finished coarse rows waiting for an earlier tile no longer stop the walk from
  reading at 1,024 rows. A cell waiting for its siblings holds back the rows of their
  whole subtrees, so up to every internal cell's row can wait: the stage now charges that
  bound (a few kilobytes per internal cell, P1's stuck list cleared from each row) in its
  fixed memory instead.
