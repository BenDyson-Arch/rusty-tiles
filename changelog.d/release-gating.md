### Changed

- Release workflow: a tag publishes only when CI succeeded for the tagged commit.
  Release-candidate tags (`v*-*`) no longer upload to PyPI, whose files cannot be
  replaced; they still publish GitHub prerelease binaries and the container image.
  The release test run skips the load-sensitive
  `spill::tests::spill_memory_is_flat_in_input_size`, which CI runs for the same
  commit.
