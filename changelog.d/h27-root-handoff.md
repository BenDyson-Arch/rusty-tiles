### Changed

- Mesh tiler internals (proxy texture bake handoff): frozen tile packets and retained leaf
  appearance are read under explicit memory leases with cancellation, the certified
  untextured proxy can be read back by its frozen hash, the member assembler leases its
  memory and checks transform, material and no-raster semantics, commits are bound to the
  exact verified bytes, and copied feature IDs are written exactly. No output changes
  without a baker.
