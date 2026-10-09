# Terrain guide

`terrain` converts a bounded elevation GeoTIFF to a **3D Tiles 1.1 directory**
containing glTF 2.0 GLB meshes. This replaces the prototype quantized-mesh format;
there is no compatibility mode. Conversion requires the `native-geospatial`
CLI or Rust build. Standard binaries can preview existing output.

```sh
rusty-tiles terrain -i elevation.tif -o output/terrain \
  --cells-per-leaf 64 --height-offset 0 --fill-height -1000
rusty-tiles preview --cesium <Build/Cesium> --terrain output/terrain \
  --imagery output/imagery
```

## Source and height meaning

The first profile accepts a standalone, one-band Float32 or Float64 GeoTIFF with
internal EPSG:4326/WGS84 north-up PixelIsArea georeferencing. Samples are raw
metres; the caller explicitly supplies an offset to obtain ellipsoidal heights.
Empty or metre band units are allowed. Other units, nonidentity scale/offset,
explicit masks, overviews, sidecar dependencies and alternate georeferencing
are refused. Scalar NoData is supported, including NaN NoData. Valid samples
must be finite and at least one sample must be valid.

The sampling lattice spans the closed source footprint, with one cell per source
pixel. Bilinear interpolation uses pixel centres and clamps within outer pixel
edges. Only contributors with nonzero weight must be valid. Missing lattice
samples use the supplied **ellipsoidal** fill height directly; the offset is
not added to fill. Geometry is never generated outside the footprint.

Sampled and filled vertex heights must be −10,000…8,000 metres. The finite
profile supports public Cesium surface queries whose downward rays start at
9,000 metres. These bounds are checked before staging output. They do not
imply that triangle interiors equal the original raster surface: triangles
are Cartesian chords between samples.

## Output and limits

| Member | Meaning |
| --- | --- |
| `tileset.json` | 3D Tiles 1.1 entrypoint, shared local frame and Cartesian bounds |
| `tiles/{row}/{column}.glb` | Full-detail, opaque triangle mesh patches |
| `conversion.json` | Typed source, sampling, height, placement and output receipt report |

`--cells-per-leaf` accepts 16, 32, 64 or 128, with smaller boundary patches.
Adjacent patches store identical boundary positions. A shared local ENU frame
places the mesh in WGS84 ECEF. Actual Float32 position storage error must be at
most **0.05 metre**; larger extents that cannot meet this are refused. Collapsed
or reversed decoded triangles are refused. There is no simplification, zoom
pyramid, parent proxy, encoded normal or compression.

Source file size is limited to 128 MiB, each source dimension to 32,768, decoded
Float64 values to 64 MiB and native raster blocks to 64 MiB. Validity storage
and the native cache are additional allocations. Output is limited to 10,000
patches and a conservative 1 GiB plan; encoding uses one bounded patch at a time.
These limits are not a whole-process RSS guarantee.

Leaf geometric error is zero relative to the discrete sampled mesh. The empty
routing root uses its decoded bound diameter as a positive selection metric;
this is not a certified source approximation error. Bounds contain decoded
positions and their triangle chords.

Conversion finishes native decoding and validates the full plan before staging.
Members are flushed and synced; an exact inventory and report finish before
exclusive publication. `--force` uses the foundation replacement protocol.
Source identity and containing-directory overlaps are rejected; replacing a
final output symlink replaces that entry and preserves its referent.

## Queries, clamping and imagery

The tested consumer is **CesiumJS 1.146.0**. Load the output as a
`Cesium3DTileset`, with `enableCollision: true`. `CesiumTerrainProvider` and
`sampleTerrain` do not load this representation.

```js
const terrain = await Cesium.Cesium3DTileset.fromUrl("terrain/tileset.json", {
  enableCollision: true,
});
viewer.scene.primitives.add(terrain);
terrain.imageryLayers.addImageryProvider(imageryProvider);
const heights = await viewer.scene.sampleHeightMostDetailed(cartographicPositions);
const clamped = await viewer.scene.clampToHeightMostDetailed(cartesianPositions);
```

Queries intersect the decoded triangle mesh, including fill geometry. They
require 3D mode and depth texture support. Public scene methods can query other
scene geometry too: isolate the terrain by excluding unrelated primitives and
hiding the globe. Normalize query positions to ellipsoid height zero before
calling them so supplied input heights do not alter the pinned implementation's
ray origin. Outside the terrain footprint, isolated queries/clamps return
undefined. `HeightReference.CLAMP_TO_3D_TILE` is available for entity placement.

The included preview does this isolation and normalization, exposes
`window.terrainSurface.sampleHeights` and `clampPositions`, and clamps a marker
when the user clicks the terrain. It hides the global ellipsoid so negative
heights remain visible. Imagery selected with `--imagery` is draped onto the
terrain tileset using its georeferenced provider.

Cesium's `tileset.imageryLayers` API is **experimental** in the pinned version.
Draping acceptance applies to this consumer; other 3D Tiles viewers do not
necessarily implement that API. The mesh files remain ordinary standard GLBs.
Archive packaging is a separate `convert` operation. `validate` currently
checks `.3tz` archives rather than terrain directories.
