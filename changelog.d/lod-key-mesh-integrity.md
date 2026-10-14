### Fixed

- Mesh tiler: an impossible lookup miss while welding a coarse tile's vertices (seen once
  on a host that also crashed the compiler, i.e. memory corruption) is now a typed
  `InvalidState` error with its context instead of a worker panic; the LOD proxy row's
  equality now agrees with its sort order.
