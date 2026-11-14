### Changed

- Mesh capture builds its cell tree as parallel subtrees on the capture
  workers instead of in one serial pass. Output is identical for every worker
  count; a stop request is honoured between units of 4,096 triangles.
