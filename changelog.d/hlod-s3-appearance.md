### Changed

- `mesh --lod hlod`: coarse tiles are textured (slice S3 of the HLOD decision).
  Each material class of a coarse tile gets fresh planar charts, each turned to its
  minimum-area rectangle, with small charts merged into their neighbours. The charts
  are packed into one block-aligned page sized to its content: multiples of four from
  32 to 1024 per side, at `max(G/16, 1e-5 m)` per texel but never finer than the
  source's own texels. Pages that can't fit use the 1024 cap at the finest spacing
  that fits.
- Every present PBR slot is baked from the exact nearest point of the captured source
  surface. Base colour is multiplied by `COLOR_0` in linear light, and normal maps are
  re-expressed in the coarse tangent frame. Pages go through the same delivery encoder
  as leaves, with charts kept apart in every mip level. MASK base colour keeps its
  coverage through the mip chain.
- A coarse page whose mip chain no codec step can deliver within its gates is published
  as its exact level-0 PNG, sampled without mips. These pages are counted.
- Untextured input keeps flat class factors, plus per-vertex `COLOR_0` from the source
  when it has vertex colours. The manifest reports requested and achieved spacing, page
  texels and cap hits per depth.
- `mesh`: leaves with mipmapped samplers now use cropped island pages whose source
  interiors sit on the 8-texel grid, instead of a dedicated page carrying the whole
  source image plus one period of padding. Their first three mip levels are the
  source's own, and islands never mix in any reduced level. Leaves with non-mipmapped
  samplers are byte-identical.

### Fixed

- `mesh`: the leaf stage now charges each leaf's actual largest page rather than a
  profile-wide maximum page. It also releases its decoded source images before the
  HLOD stage, so textured inputs fit the default memory budget.
