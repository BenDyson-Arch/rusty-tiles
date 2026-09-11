# glTF vector prototype

`vector -i features.geojson -o features.3tz --maxFeatures 64` generates glTF vector content under `3DTILES_content_gltf_vector` (3D Tiles 1.1 draft). It was rendered and picked in an isolated Cesium 1.143.0 IIFE preview.

The encoder preserves point/line geometry, polygon rings and holes, stable source IDs, and typed scalar properties. Whole features are partitioned spatially; no clipping, simplification, coarse vector LOD or implicit quadtree is implemented. Line strips are currently separate primitives, so primitive restart batching is not needed yet.

Polygon encoding follows `EXT_mesh_polygon` proposal revision `c1a035499b70aeb5d8281470101423e5e285dfe3` from [Khronos glTF PR 2570](https://github.com/KhronosGroup/glTF/pull/2570). Feature metadata uses `EXT_mesh_features` and `EXT_structural_metadata`. The [3D Tiles extension](https://github.com/CesiumGS/3d-tiles/pull/838) remains a draft; this is not a finalized 3D Tiles 2.0 implementation.

Input is WGS84 GeoJSON. Original 3D positions are retained for nonplanar polygons; the best-fit plane is used only for triangulation. Missing string/numeric fields use explicit noData values. Complex/mixed values, nullable booleans, geometry collections and unsupported schemas fail explicitly. `--repair` reports invalid-outline repairs; `--ambiguousOutlines` preserves irreconcilable crossings as original closed 3D outlines. The bundled fixture contains only invented test geometries. GDAL/GEOS and NumPy are required. See [README](../README.md#raster-terrain-and-gltf-vector-lab-commands) for the tested fixture, limits and preview.
