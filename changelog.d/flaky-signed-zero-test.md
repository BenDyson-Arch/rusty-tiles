### Fixed

- The `uniqueItems` unit tests no longer fail about one run in 128 to 256: past 15 items the
  schema library's hashed path finds `0` and `-0.0` as duplicates only when its random hash
  seed makes their hashes collide, so the tests stopped asserting that path on such inputs.
  Validator output is unchanged.
