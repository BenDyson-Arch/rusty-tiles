### Added

- HLOD: the stage report lists every coarse tile that only a retry published
  (`lastResortRecords` in the stage manifest's `hlod` section and in the run
  diagnostics): its first node, depth, last-resort stage (0 for fix 2 and staged
  reduction, which previously left no per-tile record), the build that published
  it, why the primary build did not, and the primary build's refused proposals by
  check. Payloads, tilesets and their bytes are unchanged.
