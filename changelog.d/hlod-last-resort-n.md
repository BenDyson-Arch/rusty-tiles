### Fixed

- HLOD: a coarse tile that every strategy fails is now rebuilt by a declared last resort instead of being omitted, whenever its shared border fits the triangle cap: new faces on shared border points in the one cell that owns them, then admitted folds, then clustering with the border fixed. Exact seams are unchanged, the geometric error stays measured, and validators recount the declaration (`asset.extras.rustyTiles.lastResort`). Tiles built today are unchanged.
