### Changed

- `mesh --lod hlod`: coarse tiles are textured (slice S3 of the HLOD decision).
  Each material class of a coarse tile gets fresh planar charts, each turned to its
  minimum-area rectangle, with small charts merged into their neighbours. The charts
  are packed into one block-aligned page sized to its content: multiples of four from
  32 to 1024 per side, at `max(G/16, 1e-5 m)` per texel but never finer than the
  source's own texels. Pages that can't fit use the 1024 cap at the finest spacing
  that fits. Coarse pages are published at level 0 with LINEAR minification: a
  complete mip chain cannot keep several charts apart.
- Every present PBR slot is baked from the exact nearest point of the captured source
  surface. Base colour is multiplied by `COLOR_0` in linear light, and normal maps are
  re-expressed in the coarse tangent frame. Pages go through the same delivery encoder
  as leaves, with each chart's gutters filled from that chart. MASK base colour keeps
  its coverage. A coarse page that no codec step can deliver within its gates is
  published as its exact PNG. These pages are counted.
- Untextured input keeps flat class factors, plus per-vertex `COLOR_0` from the source
  when it has vertex colours. The manifest reports requested and achieved spacing, page
  texels and cap hits per depth.
- `mesh`: lossy leaves with mipmapped (or unspecified) samplers now use cropped island
  pages, published at level 0 with LINEAR minification, instead of a dedicated page
  carrying the whole source image plus one period of padding. A complete chain can't
  keep a page's islands apart or source-exact at bounded padding, and Cesium needs
  complete chains for mipmapped sampling. The source filter is recorded in the
  receipts. Lossless leaves and leaves with non-mipmapped samplers are byte-identical.

### Fixed

- `mesh`: the leaf stage now charges each leaf's actual largest page rather than a
  profile-wide maximum page. It also releases its decoded source images before the
  HLOD stage, so textured inputs fit the default memory budget.
