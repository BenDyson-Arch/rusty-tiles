### Fixed

- `mesh`: leaf textures whose UVs repeat a source image (REPEAT or MIRRORED_REPEAT, one or more whole periods on an axis) keep that axis whole on a page of the source's own size, published with the source wrap, instead of unrolling one copy per period into the atlas. Such leaves no longer lose resolution to the leaf density reduction: texture-1e5-1024 publishes 147.8 M leaf texels at full source resolution instead of 579 M at 1/2 to 1/8.
