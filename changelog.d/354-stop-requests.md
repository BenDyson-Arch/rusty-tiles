### Fixed

- A stop request now ends every long phase within a few seconds. This covers
  SIGINT/SIGTERM, Ctrl+C/Ctrl+Break in a Windows console, and
  `KeyboardInterrupt` at a Python progress event. Before, a run could keep
  working for minutes and a service manager would kill it, leaving its scratch
  on disk. The phases are: external sorts and merges (validate, mesh), archive
  admission and member hashing, the validator's HLOD index builds, vector
  SQLite work (GeoJSON ingestion, partition, fingerprint and encoding
  statements), vector reuse, mesh payload staging and image processing. The
  run still removes its scratch, exits 130/143 and publishes nothing.
- If a stopped CLI run has not finished 20 seconds after the signal, for
  example inside one KTX2 page encode that cannot be interrupted, the CLI
  removes the run's spill, work and staging paths itself and exits with the
  same status. A systemd stop no longer reaches the 90-second SIGKILL.

### Added

- Ctrl+C and Ctrl+Break in a Windows console cancel the running command and
  clean up as SIGINT does on Unix (exit 130). A second press ends the process
  at once.
