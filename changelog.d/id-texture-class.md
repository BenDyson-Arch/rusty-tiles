### Added

- Large-mesh leaves (H8 texture pipeline): an ID / class texture class for
  `EXT_mesh_features` feature ID textures and categorical
  `EXT_structural_metadata` property textures, kept for a
  future annotate pass. `mesh` does not publish ID textures (coarse tiles carry feature IDs
  as per-face vertex attributes only; see the tile batching entry). Any number of feature-ID sets (1..=4 bytes each) are
  packed into RGBA8 PNG pages. Each set gets its own `channels`. Gutters and
  padding hold the no-feature value, so each set's `nullFeatureId` is
  2^(8*bytes)-1. Mips use a support-weighted mode and never average IDs. Each
  mip level is a separate exact PNG for coarse proxies. The sampler uses
  NEAREST/NEAREST, so viewers generate no mipmaps. Basis KTX2 is refused for
  this class. Leaves may carry extra `featureIds` entries after the two source
  sets. The writer refuses any ID texture whose sampler is not NEAREST.
