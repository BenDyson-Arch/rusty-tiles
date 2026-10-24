### Changed

- `mesh --lod hlod`: coarse tiles are textured (slice S3 of the HLOD decision).
  Each material class of a coarse tile gets fresh planar charts, each turned to its
  minimum-area rectangle, with small charts merged into their neighbours. The charts
  are packed into one block-aligned page sized to its content: multiples of four from
  32 to 1024 per side, at `max(G/16, 1e-5 m)` per texel but never finer than the
  source's own texels. Pages that can't fit use the 1024 cap at the finest spacing
  that fits. Chart rectangles sit on a 2-texel grid with 2-texel gutters. That keeps
  charts isolated through mip level 1 of each complete authored chain; deeper levels
  average the whole page.
- Every present PBR slot is baked from the exact nearest point of the captured source
  surface. Base colour is multiplied by `COLOR_0` in linear light, and normal maps are
  re-expressed in the coarse tangent frame. Pages go through the same delivery encoder
  as leaves. MASK base colour keeps its coverage. A coarse page that no codec step can
  deliver within its gates is published as its exact level-0 PNG, sampled without
  mips. These pages are counted.
- Untextured input keeps flat class factors, plus per-vertex `COLOR_0` from the source
  when it has vertex colours. The manifest reports requested and achieved spacing, page
  texels and cap hits per depth.
- `mesh`: lossy leaves with mipmapped (or unspecified) samplers now use cropped island
  pages instead of a dedicated page carrying the whole source image plus one period of
  padding. Island interiors sit on the 8-texel source grid with 8-texel gutters. Each
  page keeps a complete authored chain whose first three levels are the source's own
  and never mix islands; deeper levels average the whole page. Lossless leaves and
  leaves with non-mipmapped samplers are byte-identical.
- `mesh`: every level of an authored mip chain is now the area-weighted average of
  level 0 over that texel's footprint, computed directly rather than from the previous
  level. Pages whose sides are not powers of two no longer blur through earlier levels.

### Fixed

- `mesh`: the leaf stage now charges each leaf's actual largest page rather than a
  profile-wide maximum page. It also releases its decoded source images before the
  HLOD stage, so textured inputs fit the default memory budget.
