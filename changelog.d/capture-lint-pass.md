### Changed

- Mesh capture's corner-depth join runs its leaf chunks on capture's own worker tasks
  instead of a separate thread pool, sized by the same resolved `--jobs`. When a chunk
  fails, the first failing chunk in leaf order is reported, whatever order the workers
  met it in. Output is unchanged.
