### Fixed

- The `uniqueItems` unit tests no longer fail about one run in 128 to 256: past 15 items the
  schema library's hashed path finds `0` and `-0.0` as duplicates only when its random hash
  seed makes their hashes collide, so the tests stopped asserting that path on such inputs.
  Validator output is unchanged.
- CI: Playwright's system-library install is bounded to three minutes and no longer fails the job. The runner image already provides Chromium's libraries, and the Azure Ubuntu mirror has stalled on the extra font packages long enough to time out the Python and release-acceptance jobs.
- CI: on pull requests the GDAL job runs only the GDAL-gated modules and integration suites (it was re-running the whole suite and often hit its 25-minute limit at 20–22 minutes). The full tier still runs the complete suite with GDAL.
