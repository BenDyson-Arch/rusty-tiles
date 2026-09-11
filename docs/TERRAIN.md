# Terrain and raster lab paths

Implemented as GDAL-backed standalone commands; see [README](../README.md#raster-terrain-and-gltf-vector-lab-commands) for invocation and verification.

- `raster`: source-preserving COG plus PNG XYZ display pyramid. No bundled PMTiles writer.
- `terrain`: DEM to quantized-mesh directory and `layer.json`, with explicit height offset and NoData fill height. Regular shared grids; no adaptive simplification or certified maximum error bound yet.

These are lab paths. They do not replace the server raster worker or introduce product integration. Terrain height datum must be supplied; a constant offset cannot replace a spatial geoid transformation.
