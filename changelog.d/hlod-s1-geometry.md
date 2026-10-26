### Added

- `mesh`: coarse tiles are built by the 0.4 classic octree HLOD (slice S1 of the HLOD
  decision). Every occupied cell above the exact, byte-identical leaves is its
  children's union, simplified with meshopt to at most 8,192 triangles. Only the outer
  interface with other ownership subtrees is locked, as complete decoded segments
  and points at the leaves' own decoded coordinates. The bytes are decoded and
  checked before they are published, so independently refined neighbours agree at
  every depth mix.

### Fixed

- `mesh`: image-free input no longer reserves a leaf texture workspace (about
  3 GiB for `balanced`), and the leaf stage refuses a plan whose single job does
  not fit available memory even when it builds no page.
