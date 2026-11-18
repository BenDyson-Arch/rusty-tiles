### Changed

- The large-mesh HLOD source index stores lossless triangle blocks and reconstructs
  fine search nodes on demand. It reserves about 77.6 scratch bytes per triangle
  at scale and reports its actual stored bytes after compression.
