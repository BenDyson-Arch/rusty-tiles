### Changed

- Large-mesh tiler (`rusty-tiles mesh`, `mesh_to_tileset`): coarse (proxy) tiles are
  now textured. After each proxy's certified geometry is fixed, the tiler bakes its
  PBR pages from the captured source images, keeps its exact
  POSITION words, face order, source/material identity and feature IDs, reads the
  complete GLB back, and rebakes at a lower texel density when it would exceed the
  per-tile byte budget. A proxy starts at its children's texel density (their
  world-area-weighted median spacing, so stray near-degenerate source UV triangles
  do not coarsen it). Geometric errors are unchanged. Depths are baked deepest
  first, each on up to `--jobs` workers; a proxy that does not fit its worker's
  memory share is retried alone with the whole allowance, so output and admission
  do not depend on `--jobs`. Archives are larger and conversions slower than
  before: baking dominates the run time.
- Proxy faces whose exact texture footprint is strictly under one square texel and
  whose quantized UVs collapse are published as no-raster faces (geometry, corner
  attributes and feature IDs kept, no texture sampling). Larger collapses force a
  chart repack and then a refusal.
- Feature IDs above 2^24 in a proxy are `unsupported`: rusty-tiles does not write
  UNSIGNED_INT vertex attributes (glTF says they SHOULD NOT be used), and FLOAT holds
  every integer only up to 2^24.

