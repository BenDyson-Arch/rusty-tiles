# Terrain guide

`terrain` converts a bounded elevation GeoTIFF to a **3D Tiles 1.1 directory**
containing glTF 2.0 GLB meshes. This replaces the prototype quantized-mesh format;
there is no compatibility mode. Conversion requires the `native-geospatial`
CLI or Rust build. The Python package exposes no terrain conversion API.
Standard binaries can preview existing output. If the native GTiff reader is
unavailable, both `doctor --command terrain` and conversion report `Unsupported`
before opening the DEM; malformed DEM data with the reader available remains
`InvalidInput`.

```sh
rusty-tiles terrain -i elevation.tif -o output/terrain --height-offset 0
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
edges. A lattice node is valid when every contributor with nonzero weight is.
**NoData is a hole:** only triangles of three valid nodes are generated, so
there is no fill geometry, and tiles without any triangle at or below them
are not published. There are no additional geographic tiles or global roots.
Cartesian boundary chords can project slightly beyond the geographic rectangle.

Sampled vertex heights must be −10,000…8,000 metres. The finite
profile supports public Cesium surface queries whose downward rays start at
9,000 metres. These bounds are checked before staging output. They do not
imply that triangle interiors equal the original raster surface: triangles
are Cartesian chords between samples.

## Output and limits

| Member | Meaning |
| --- | --- |
| `tileset.json` | 3D Tiles 1.1 entrypoint, shared local frame, Cartesian bounds and implicit tiling |
| `subtrees/*.subtree` | Quadtree availability with each tile's box and geometric error |
| `implicit-content/*.glb`, `tiles/{level}/{row}/{column}.glb` | Opaque triangle mesh tiles, one primitive each: implicit tiles, and the explicit coarse levels of large pyramids |
| `conversion.json` | Typed source, sampling, height, level, error, placement and output receipt report |

`--cells-per-leaf` accepts 16, 32, 64 or 128 (the default), with smaller
boundary patches.
The full-detail leaves are these patches. A shared local ENU frame places the
mesh in WGS84 ECEF. There are no encoded normals.

### Position storage and compression

Every tile GLB requires `EXT_meshopt_compression`: its vertex and index views
are meshopt attribute (`0xa0`) and triangle (`0xe1`) streams. Positions are
unnormalized uint16 `KHR_mesh_quantization` integers, dequantized by the tile
node's `translation` and `scale`. Each level has one power-of-two step for all
three axes, chosen (with a little headroom) so that every tile of the level,
surface and skirt, spans at most 65,535 steps, and every node translation is
a multiple of it. Decoding is therefore exact, and tiles of one level store
identical positions at shared nodes. Coarser levels round the same lattice
nodes to their coarser grid.

The leaves are the reference surface. If the leaf grid could move a vertex
more than **0.05 metre**, leaf positions stay Float32 instead, and their actual
Float32 storage error must then be at most 0.05 metre: larger extents that
cannot meet this are refused. `positionErrorMetres` is the largest distance
from a decoded leaf vertex, surface or skirt, to the geodetic position it
stores. Collapsed or reversed decoded surface triangles are refused.

### Level of detail

The output is always a level-of-detail pyramid; there is no flat mode. With
`N` leaves across the larger dimension there are `D = ⌈log2 N⌉` coarser levels.
Level `L` keeps every `2^(D−L)`-th node of the leaf lattice, plus the clipped
far edge of the footprint, in tiles of at most `cells-per-leaf` cells. Coarse
vertices are the leaf lattice nodes on the level's grid: there is no
resampling, filtering or simplification. Each tile has up to four
children, one per quadrant with a triangle at or below it. Leaves keep exactly the
flat-profile surface triangles, diagonal and order.

Leaf geometric error is zero relative to the decoded leaf mesh. A coarser
tile's error is a **certified bound** on the distance from its decoded surface
(as stored: quantized positions after dequantization) to the decoded leaf
surface at the same lattice parameters, with outward rounding: the exact maximum distance
to the next finer level, including where a clipped edge cell's diagonal crosses
finer triangles, plus the largest child error. Distances are measured where
both levels have a surface; a cell whose coverage may differ adds its
horizontal diagonal, and a tile without triangles takes 16 times the diagonal
of the surfaces below it, so Cesium refines through it while it is visible. Errors never increase toward the leaves. Over smooth terrain the value is close to the vertical error plus
the grid errors of the levels below, a few 65,535ths of the tile size. The
tileset error leaves an interval in which the root renders. These errors do
not bound the distance to the continuous DEM.

Neighbours at different levels meet with T-junctions. Each tile edge shared
with another tile of its level carries a **skirt**: a vertical curtain from
each edge vertex down to its height minus `max(e + e_beside, 1 m)`, where `e`
is the tile's error and `e_beside` the error of the coarsest tile that can be
drawn across that edge: the neighbour-side child of their lowest common
ancestor. Only segments with a triangle on both sides are skirted, so NoData
edges stay open. The skirt shares the surface's
primitive and material, so each tile is one draw call: its triangles follow
the surface triangles, its tops are the surface vertices, and its bottoms
follow the surface vertices. Each skirt is deepened by twice its level's
worst storage error, so the decoded bottoms are at least that far below the
decoded surface. Any gap between two such neighbours is at most the sum of
their errors, so either skirt covers it vertically. Horizontal offsets, from
Cartesian chords and from coarse levels rounding shared nodes to their own
grid (under a step), are not closed by vertical curtains. Skirts face outward
under the opaque single-sided material. Outer footprint edges have no skirts.

Every box contains its tile's decoded surface and skirt vertices and all of its
descendants' boxes. The hierarchy is published as a bounded 3D Tiles implicit
forest: tiles at least four levels above the leaves stay explicit in
`tileset.json`, and each tile three levels above the leaves (or the root of a
shorter pyramid) roots one quadtree subtree. Package the directory with `convert` before running `validate`.

Source file size is limited to 128 MiB, each source dimension to 32,768, decoded
Float64 values to 64 MiB and native raster blocks to 64 MiB. Validity storage
and the native cache are additional allocations. Output is limited to 10,000
leaves and a conservative 1 GiB plan over every level; encoding holds one tile
at a time, and planning at most four child surfaces per level. These limits are
not a whole-process RSS guarantee.

`conversion.json` (schema 3, profile `t1-terrain-lod-mesh-3dtiles-1.1`) reports
`levels`, `tiles` and `tilesPerLevel`, surface `vertices` and `skirtVertices`
(skirt bottoms), the per-level `quantizationStepMetres` (the node scale, the
same step on each axis, or null for Float32 leaves), the largest `geometricErrorMetres` per level
(coarsest first),
`rootGeometricErrorMetres`, `tilesetGeometricErrorMetres` and the smallest and
largest `skirtDepthMetres` (null without skirts).

Conversion finishes native decoding and validates the full plan, including
every level, error and skirt, before staging. Members, including those the
implicit forest writer creates, are flushed and synced; an exact inventory and
report finish before exclusive publication. `--force` uses the foundation
replacement protocol. Source identity and containing-directory overlaps are
rejected; replacing a final output symlink replaces that entry and preserves
its referent.

## Queries, clamping and imagery

The tested consumer is **CesiumJS 1.146.0**. Load the output as a
`Cesium3DTileset`, with `enableCollision: true`. `CesiumTerrainProvider` and
`sampleTerrain` do not load this representation.

```js
const terrain = await Cesium.Cesium3DTileset.fromUrl("terrain/tileset.json", {
  enableCollision: true,
  skipLevelOfDetail: true,
});
viewer.scene.primitives.add(terrain);
terrain.imageryLayers.addImageryProvider(imageryProvider);
const heights = await viewer.scene.sampleHeightMostDetailed(cartographicPositions);
const clamped = await viewer.scene.clampToHeightMostDetailed(cartesianPositions);
```

Set `skipLevelOfDetail: true` for terrain, as the preview does. The levels use
REPLACE refinement, so by default Cesium loads every ancestor of a tile it
draws. With skipping it loads only tiles that can be drawn. Fully refined
(`maximumScreenSpaceError: 0`) on the 1025x1025 perf-lane DEM (289 leaves),
that is 300 requests instead of 419, 6.6 MB instead of 8.8 MB, and 0.73 s
instead of 0.87-1.03 s on a GPU. The final frame is pixel-identical, and the
geometric errors and what each view finally selects are unchanged. While
tiles load, skipping can show transient holes until the first ancestor draws;
the settled frame has none. Leave `immediatelyLoadDesiredLevelOfDetail` off: it
saved only one more request and shows holes until the finest tiles arrive. In a default overview the tileset
loads 2 requests and 13 KB either way.

`sampleHeightMostDetailed` and `clampToHeightMostDetailed` traverse to the
full-detail leaves, so their answers do not depend on the view. Downward rays
hit the surface above any skirt. Other height queries and
`HeightReference.CLAMP_TO_3D_TILE` use the tiles currently rendered, which lie
within their geometric error of the leaves.

Queries intersect the decoded triangle mesh; over a NoData hole they find no
terrain. They
require 3D mode and depth texture support. Public scene methods can query other
scene geometry too: isolate the terrain by excluding unrelated primitives and
hiding the globe. Normalize query positions to ellipsoid height zero before
calling them so supplied input heights do not alter the pinned implementation's
ray origin. Raw Cesium methods intersect the Cartesian mesh, including any boundary-chord
projection beyond the rectangle. Preview helpers separately fence queries to the
closed source footprint; outside that domain queries/clamps return undefined. `HeightReference.CLAMP_TO_3D_TILE` is available for entity placement.

The included preview does this isolation and normalization, exposes
`window.terrainSurface.sampleHeights` and `clampPositions`, and clamps a marker
when the user clicks the terrain. It hides the global ellipsoid so negative
heights remain visible. Imagery selected with `--imagery` is draped onto the
terrain tileset using its georeferenced provider.

Cesium's `tileset.imageryLayers` API is **experimental** in the pinned version.
Draping acceptance applies to this consumer; other 3D Tiles viewers do not
necessarily implement that API. The mesh files remain ordinary standard GLBs.
Archive packaging is a separate `convert` operation. `validate` checks `.3tz`
archives rather than terrain directories, so pack the directory first.
