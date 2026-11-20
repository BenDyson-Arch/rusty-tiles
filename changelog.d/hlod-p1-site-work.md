### Changed

- HLOD: each P1 record in the stage report (`p1_records`) now breaks the main loop's work
  down by site (`siteWork`: initial queueing, target scans, guard checks and the refused
  share, applying collapses, requeueing), with pop and check counts and the largest single
  check and scan, so a cell that spends much of its budget shows where.
