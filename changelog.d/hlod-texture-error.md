### Changed

- Mesh HLOD: coarse tiles in `balanced`, `quality` and `lossless` publish a
  texture-aware geometric error. Where a coarse page is much coarser than its
  children's (the 1,024-texel page cap on dense inputs), the tile's error rises to
  `16 · texel spacing / t` (`t` = 2 balanced, 1.25 quality and lossless), so CesiumJS
  refines to the sharper children instead of drawing a blurry coarse tile beside
  sharp leaves. `fast` is unchanged. The rule is declared in
  `asset.extras.rustyTiles.textureError` and `validate` recomputes it from the
  archive's pages, UVs and hierarchy.
