### Added

- `validate --coarseCrossings` (Python `validate(..., coarse_crossings=True)`) counts
  coarse self-intersections on HLOD archives in `interfaceCensus.coarseCrossings`: pairs
  of faces of one coarse tile that meet anywhere other than a shared vertex or a shared
  edge, decided exactly on the decoded positions. Each pair is classed as present (both
  faces are faces of the tile's children together, so the children cross there too) or
  new, with
  totals, counts by cell depth and the 16 tiles with the most new pairs. It is a report
  only: crossings within a tile's geometric error are permitted, so no verdict, warning or
  exit code depends on it. It is off by default because it costs about 3–11 µs per
  coarse face (9–15% more validation time on the archives measured, 41% on a dense synthetic
  city).
