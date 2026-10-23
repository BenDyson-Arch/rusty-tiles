### Fixed

- CLI: a closed or broken stderr (for example `rusty-tiles ... 2>&1 | head -1`) no longer panics with exit 101. Progress, warning and summary writes ignore stderr errors, a broken stdout does not change the exit status, and the exit code reflects the conversion result. With `--json --progress json` the JSON result is no longer lost.
