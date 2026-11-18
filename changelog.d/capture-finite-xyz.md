### Fixed

- `rusty-tiles mesh` with `--source-crs` no longer refuses a valid source with
  "coordinates must be finite XYZ" right after the capture scan. A run of
  65,536 POSITION vertices that no index of a primitive uses (common when
  several primitives share one accessor) no longer widens the scanned source
  box to infinity, which had made the CRS origin NaN. Output for sources that
  already converted is unchanged.
