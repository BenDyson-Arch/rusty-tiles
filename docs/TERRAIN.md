# Terrain / raster (later)

CLI stub:

```bash
tinyowl-tiles terrain -i dem.tif -o terrain.3tz
```

Exit code 2. Not scheduled in v0.

- DEM → quantized-mesh or 3D Tiles terrain
- COG / PMTiles stay in `tinyowl-server` + GDAL until this crate is clearly better
- COLMAP is out of scope (hub `CONTEXT.md`)

See `src/terrain.rs`.
