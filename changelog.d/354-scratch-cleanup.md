### Added

- `rusty-tiles scratch list [--dir DIR] [--json]` lists rusty-tiles spill and
  work folders in the default scratch locations and in `--dir`, with size,
  time since last change, owner PID and command, and whether the owner still
  runs (its PID exists and its start time matches).
- `rusty-tiles scratch clean [--dir DIR] [--older-than DURATION] [--dry-run]
  [--json]` removes only folders whose owner is gone and reports the bytes
  freed. Folders without an owner record (from 0.4.0) are removed only after
  24 hours without change, or after `--older-than`. It recognises only
  rusty-tiles folder names, never follows symlinks, and checks each folder
  again just before removing it.
- Every spill and work folder now holds `rusty-tiles-owner.json`: PID, process
  start time, host, command, rusty-tiles version and creation time.
- `doctor` reports the number and total size of stale scratch folders, with
  the clean command, as an informational `scratch` entry (`scratch-folders`
  probe). It never changes the doctor status or exit code.
- In a Windows console, Ctrl+C and Ctrl+Break cancel the running command and
  remove its scratch, as SIGINT does on Unix (exit 130). A second press ends
  the process at once.

