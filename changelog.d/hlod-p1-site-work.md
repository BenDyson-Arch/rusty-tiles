### Changed

- HLOD: each P1 record in the stage report (`p1_records`) now breaks the main loop's work
  down by site (`siteWork`: initial queueing, target scans, guard checks and the refused
  share, applying collapses, requeueing), with pop and check counts and the largest single
  check and scan, so a cell that spends much of its budget shows where.
- HLOD: P1 records also count refused collapse checks by reason (`refusedBy`), and a P1
  collapse check no longer rescans a high-valence vertex's star for every target it is tried
  against; the decisions and archives are unchanged.
