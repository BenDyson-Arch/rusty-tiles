### Changed

- Mesh capture assembles its retained triangle sequence directly. Decoded
  vertices are kept in memory within the budget, or spilled and read through
  block caches; triangles are bucketed by Morton prefix and sorted bucket by
  bucket. This replaces the two corner sorts and the triangle merge sort. A
  spilled primitive without index locality sorts its corners instead. Output
  is unchanged.
