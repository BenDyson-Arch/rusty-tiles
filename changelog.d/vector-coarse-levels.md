### Fixed

- Vector coarse tiles omit or reduce whole features no larger than their error
  instead of keeping near-full copies; a ring within tolerance of its chord
  reduces to a triangle. Parent tolerances scale with their extent and errors
  shrink at every level. On the walem rock-art layer, 2560×1600 views at 0.75 radii load
  3.1 MB instead of 11.7 MB, and the opening view 0.4 MB instead of 3.6 MB.
- Point aggregation no longer refuses aggregates farther than the tolerance: the
  tile's tolerance and error rise to the measured distance instead.
- A parent over budget retries at its own diagonal before routing. Routing tiles
  without content get a geometric error of 16 times their box diagonal, as empty
  mesh parents do, so CesiumJS no longer stops at an empty root and draws
  nothing; `validate` refuses rusty-tiles vector archives below this. Archives
  from earlier releases with content-less parents now fail `validate`; rebuild them.

### Changed

- Point-only parents whose points exceed their budgets publish voxel aggregates
  by default instead of routing, coarsening the grid until every layer fits;
  `--aggregate-points` aggregates every point-only parent.
