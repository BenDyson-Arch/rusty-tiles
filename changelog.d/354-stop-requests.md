### Fixed

- A stop request now ends the long phases that capture progress did not
  already cover within a few seconds. This covers SIGINT/SIGTERM,
  Ctrl+C/Ctrl+Break in a Windows console, and `KeyboardInterrupt` at a Python
  progress event. Before, such a run could keep working for minutes and a
  service manager would kill it, leaving its scratch on disk. The phases are:
  - `validate` archive admission (before its first progress event), hashing
    of a large member, its catalog, row and delivery external sorts and
    merges, and the HLOD index build and final commit;
  - the mesh archive's index sort, staging of each leaf payload, density mip
    chains and coarse texture halving;
  - vector GeoJSON ingestion, its SQLite partition, fingerprint, grid and
    encoding statements, and restoring a reused tileset.

  The run still removes its scratch, exits 130/143 and publishes nothing.
- A few calls cannot see a stop, such as one KTX2 page encode, which can
  take minutes at the highest settings of the lossy ladder. If a stopped CLI
  run has not finished 20 seconds after the signal, the CLI removes the run's
  spill, work and staging paths itself and exits with the same status. A
  systemd stop then ends after about 20 seconds plus the time to remove the
  run's temporary files, instead of reaching the 90-second SIGKILL.

### Added

- Ctrl+C and Ctrl+Break in a Windows console cancel the running command and
  clean up as SIGINT does on Unix (exit 130). A second press ends the process
  at once.
