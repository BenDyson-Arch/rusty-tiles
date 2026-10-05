# glTF vector prototype

`vector -i features.geojson -o features.3tz --maxFeatures 64` generates glTF vector content under `3DTILES_content_gltf_vector` (3D Tiles 1.1 draft). It was rendered and picked in an isolated Cesium 1.143.0 IIFE preview.

The encoder preserves point/line geometry, polygon rings and holes, stable source IDs, and typed scalar properties. Whole features are partitioned spatially with full-detail leaves and actual simplified parent content. No clipping or implicit quadtree is implemented. Line strips are currently separate primitives, so primitive restart batching is not needed yet.

Polygon encoding follows `EXT_mesh_polygon` proposal revision `c1a035499b70aeb5d8281470101423e5e285dfe3` from [Khronos glTF PR 2570](https://github.com/KhronosGroup/glTF/pull/2570). Feature metadata uses `EXT_mesh_features` and `EXT_structural_metadata`. The [3D Tiles extension](https://github.com/CesiumGS/3d-tiles/pull/838) remains a draft; this is not a finalized 3D Tiles 2.0 implementation.

Input is WGS84 GeoJSON. Original 3D positions are retained for nonplanar polygons; the best-fit plane is used only for triangulation. Missing string/numeric fields use explicit noData values. Complex/mixed values, nullable booleans, geometry collections and unsupported schemas fail explicitly. `--repair` reports invalid-outline repairs; `--ambiguousOutlines` preserves irreconcilable crossings as original closed 3D outlines. The bundled fixture contains only invented test geometries. GDAL/GEOS and NumPy are required. See [README](../README.md#raster-terrain-and-gltf-vector-lab-commands) for the tested fixture, limits and preview.


## Vector LOD

```sh
rusty-tiles vector -i features.geojson -o features.3tz \
  --maxFeatures 64 --lodTolerance 0.1 --lodLevels 3
```

`--lodTolerance` is the base simplification tolerance in metres. It doubles at
each coarser level. `--lodLevels` (1–16, default 3) adds levels even when the
input is a single detailed feature; redundant levels with no vertex reduction
are omitted. Spatial grouping adds upper parent levels.
Every node contains glTF geometry; full-detail leaves retain the source vertices.
Parents simplify directly from full-detail source data, so errors do not silently
accumulate through repeated simplification. `REPLACE` refinement substitutes
children for parent content. IDs and scalar feature properties remain available
for picking at every level.

Line simplification uses iterative 3D Ramer–Douglas–Peucker, retaining source
vertices. Its maximum accepted point-to-segment distance conservatively bounds
the continuous original path against replacement chords in both directions.
Polygon rings use the same method only when the entire polygon is planar within
one micrometre; the validity check runs in the original best-fit plane. Holes and
ring order are retained. Polygon error includes twice the planarity deviation;
float32 encoding error is included in parent refinement error. Each parent uses
at least its children's error. Full-detail leaves have geometricError zero,
with position-rounding values separately reported in tile `extras`.

Source vertices shared by different features are locked, preserving shared
boundaries and line junctions with matching source coordinates. This does not
snap inconsistent input boundaries or repair a coverage. Nonplanar polygons,
invalid source outlines and candidate polygons that fail topology checks remain
unsimplified; `conversion.json` reports each source/reason fallback. Existing
`--repair` and `--ambiguousOutlines` policies still govern encoding invalid or
irreconcilable source polygons. The simplification error contract is relative to
that source representation; an explicitly repaired source is not an unchanged
original outline.

Semantic point features are retained at every level. No points disappear or
silently acquire aggregate meanings; dense point-only layers therefore get no
geometry reduction from this policy. Tile `extras` records source/output vertex
counts, geometry error, rounding and requested tolerance. `conversion.json`
records LOD settings, locked shared vertices and fallbacks.

This is the first implementation of real vector LOD, not completion of every
vector tiling capability. Input remains WGS84 GeoJSON with ellipsoidal metre
heights. The converter reads the complete feature collection into memory. Whole
features may cross spatial partitions, and `--maxFeatures` is a feature-count
budget, not a byte/vertex budget. Large individual features remain large leaves;
upper parents retain every feature's identity and metadata. Additional readers,
dense-point aggregation, bounded-memory vector ingestion, buffered clipping and
implicit tiling remain follow-up work. Draft extension/runtime compatibility
above still applies.
