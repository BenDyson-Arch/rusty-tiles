### Changed

- Mesh HLOD: coarse tiles in `balanced`, `quality` and `lossless` publish a
  texture-aware geometric error. Where a coarse page is much coarser than its
  children's (the 1,024-texel page cap on dense inputs), the tile's error rises to
  `16 · texel spacing / t` (`t` = 2 balanced, 1.25 quality and lossless), so CesiumJS
  refines to the sharper children instead of drawing a blurry coarse tile beside
  sharp leaves. `fast` keeps the measured error. Every HLOD archive declares the rule
  in `asset.extras.rustyTiles.textureError` (`pxPerTexel` null in `fast`); `validate`
  refuses an HLOD tileset without it and recomputes the term from the archive's
  pages, UVs and hierarchy. In `quality` and `lossless` the root's error can rise.
