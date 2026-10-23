### Changed

- `mesh --lod hlod`: coarse tiles are textured (slice S3 of the HLOD decision).
  Each coarse tile gets fresh planar charts packed into one content-sized page per
  material class: the smallest power of two from 128 to 1024 per side that holds
  the charts at `max(G/16, 1e-5 m)` per texel, or the 1024 cap at the finest
  spacing that fits. Every present PBR slot is baked from the nearest captured
  source surface (base colour × `COLOR_0` in linear light; normal maps re-expressed
  in the coarse tangent frame), and pages go through the same delivery encoder as
  leaves. Untextured input keeps flat class factors, plus per-vertex `COLOR_0` from
  the source when it has vertex colours. The manifest reports requested and achieved
  spacing, page texels and cap hits per depth.
- `mesh`: leaves with mipmapped samplers now use cropped, aligned island pages
  instead of a dedicated page carrying the whole source image plus one period of
  padding, so no leaf depends on a whole source image it does not cover. Leaves
  with non-mipmapped samplers are byte-identical.
- `mesh`: mip levels under 1,024 texels of data textures (metallic-roughness,
  occlusion) use the same relaxed PSNR floor as small colour levels and keep their
  per-texel maximum-error caps. Small levels of packed pages mix islands inside
  codec blocks.

### Fixed

- `mesh`: the leaf stage now charges each leaf's actual largest page rather than a
  profile-wide maximum page. It also releases its decoded source images before the
  HLOD stage, so textured inputs fit the default memory budget.
