# Vector guide

This page explains how the `vector` command turns GIS features into 3D Tiles. **GeoPackage and GeoJSON work in the standard package**, including its Python API, subject to the grid-free CRS limits below. **Shapefile and other OGR formats require native-geospatial**, available through the native container or a CLI/Rust source build. Every option and default is in the [command reference](CLI.md#vector).

The output is experimental. It uses draft glTF vector extensions inside a 3D Tiles 1.1 `.3tz` archive. It is not a finalized 3D Tiles 2.0 format.

Implicit tiling is the default. The existing spatial partition and LOD chains are addressed as a quadtree, with standard `TILE_BOUNDING_BOX` and `TILE_GEOMETRIC_ERROR` metadata preserving padded boxes and refinement errors. These addresses describe the hierarchy; content is not clipped to regular cells. At boundaries that exceed their implicit cell, standard external tileset roots expose actual bounds before Cesium culls or picks the next levels. Add `--explicit` for the earlier explicit manifest and output bytes.

## Requirements

The [installation guide](INSTALL.md#choose-a-build) separates the two builds. Installing GDAL does not enable native-geospatial in a standard binary or Python wheel.

- The standard build reads GeoJSON and GeoPackage using Rust and bundled SQLite. It supports local XYZ and the verified grid-free CRS classes described below, including polygons with holes, repair, LOD, aggregation, metadata, meshopt and reuse.
- Shapefile, other OGR drivers and CRS operations beyond the grid-free tier need `native-geospatial`: GDAL 3.12 or newer built with GEOS 3.10 or newer, PROJ 9.2 or newer, and SQLite. This build keeps its native ingestion, geometry and CRS backend for every vector conversion.
- CesiumJS 1.143.0 to display batched vector content. See [Compatibility](#compatibility).

Conversion runs offline and needs no Python or executable helpers. The standard build needs no system GDAL, GEOS, PROJ database or SQLite installation. `doctor --command vector` reports the enabled reader and geometry backends and CRS limits; readiness does not establish that a particular source's CRS operation is supported.

## Convert a layer

These examples use placeholder file names. Replace them with your own data.

```sh
rusty-tiles vector -i mapping.gpkg -o output/mapping.3tz --layer roads \
  --max-features 64 --max-vertices 65536 --max-bytes 4194304 \
  --lod-tolerance 0.1 --lod-levels 3
```

A file with one spatial layer is selected automatically. A file with several layers needs `--layer` for each layer you want, or `--all-layers`.

## Feature identity and properties

Every feature keeps two reserved properties.

| Property | Contents |
| --- | --- |
| `_source_id` | The original GeoJSON ID or feature-table integer primary key/OGR FID, as JSON text |
| `_source_layer` | The original layer name |

Together they identify a source feature. This holds even when a feature is split into several fragments.

Scalar fields keep their types. Missing numbers and strings use an explicit `noData` value. Nullable 64-bit integers stay integers.

These inputs fail with a clear message:

- Fields whose types differ across selected layers.
- Complex fields that are not excluded.
- Nullable booleans.
- Measured geometries, geometry collections and curves.

### Choose fields

- `--fields name,category` keeps only the listed fields.
- `--drop-fields tags,notes` removes the listed fields.

The two options are mutually exclusive. Unknown names fail. Identity properties are always kept.

List fields fail by default. `--list-fields json` stores each list as JSON text. Element order, nulls and exact integers are kept. An empty list becomes `[]`. A missing list stays `noData`.

### Filter features

`--where "category = 'public'"` applies an attribute filter to every selected layer before geometry is processed. The standard build evaluates expressions using SQLite; the native build uses OGR. The expression must be valid in every selected layer. A filter may use fields that you exclude from the output. A filter that matches nothing publishes an empty tileset. Use a single read-only expression; multiple statements, comments and SQL parameters are refused by the portable reader.

Portable GeoJSON filtering refuses source integers beyond signed 64-bit range instead of converting them to SQLite floating-point values. Encoded integer metadata also requires signed 64-bit values. GeoPackage is opened read-only in immutable mode; checkpoint and close a writer with an active nonempty WAL before conversion so every committed row is available from the main file. Conversion refuses that WAL state rather than silently reading older rows. Recover a hot rollback journal with SQLite and close the writer before conversion; the immutable reader cannot safely perform database recovery.

## Coordinates and height

Each layer uses its declared CRS with traditional X and Y axis order. `--source-crs` overrides the declaration. `--source-crs local` means local metre XYZ.

| Source | Height rule |
| --- | --- |
| 2D geospatial features | Placed at ellipsoidal height zero |
| 3D features with only a horizontal CRS | Need `--height-offset` in metres |
| Compound or 3D CRS | Need the native build to use the declared height reference |
| GeoJSON | Uses its conventional ellipsoidal metre heights |

The standard build accepts verified grid-free WGS84 geographic (`EPSG:4326`), UTM north/south and Mercator operations, plus supported WKT/PROJ projections with explicit WGS84 or three-/seven-parameter Helmert datum parameters. It uses the same [strict CRS classes and projection domains as point clouds](FORMATS.md#point-clouds). Unsupported datums, required grids, compound/geoid heights and coordinate epochs fail with an environment error naming `native-geospatial`; no archive is published. Undefined GeoPackage CRS declarations need an explicit `--source-crs` override, including `local` when those coordinates are known to be local metres.

With the native build, three-axis geographic CRSs such as EPSG:7843 use their declared height. Declared coordinate epochs are kept. A missing epoch stays unspecified.

PROJ networking and ballpark operations are disabled. A missing operation or grid fails the job. An additive height offset is not a geoid transformation.

Each tile has its own local origin to reduce float32 rounding. The reported rounding still depends on the tile's extent.

## Invalid and empty features

By default, the command checks every selected feature first. It reports each failure with its layer, ID and reason, then publishes nothing.

`--skip-invalid` omits those features and publishes the rest. The omitted identities and reasons go to `conversion.json` and `geometry-reports.jsonl`. Schema errors, configuration errors and encoding errors still fail the job. An initial build needs at least one convertible feature.

Features with NULL or empty geometry are omitted automatically, because they have nothing to draw. They do not need `--skip-invalid`. `featuresWithoutGeometry` counts them, and each appears in the report stream with outcome `no-geometry`. A selection of only geometry-free records publishes a valid empty tileset.

### Repair options

| Option | Effect |
| --- | --- |
| `--repair` | Repair invalid polygon outlines. Each repair is reported. |
| `--ambiguous-outlines` | With `--repair`, keep polygons that collapse or self-intersect as their original 3D rings in line content |
| `--parent-repair` | Allow outline stand-ins in parent tiles when a polygon cannot be simplified |

With `--repair --ambiguous-outlines`, a repair that collapses to lines or points keeps the original closed rings as outlines. The report records `outputGeometry: outline` and the reason. No fill is invented. Without the flags, a collapse is a reported failure.

The portable backend validates polygons in Rust, repairs self-intersections using an even-odd fill rule and uses constrained Delaunay triangulation. Repairs that would discard collapsed boundary material or move source boundary vertices beyond the preservation checks are refused or handled by the explicit outline policy. Native builds retain GDAL/GEOS repair and triangulation. Triangle ordering and diagonals can differ between these backends.

Polygons with a horizontal CRS and a constant source height retain their original XY coordinates for validation and triangulation. The resulting triangles use the corresponding vertices on the globe. This prevents globe curvature from introducing false crossings, such as the missing Sudan polygon in earlier country conversions. Longitude coordinates are used as supplied; pole and longitude-seam aliases retain their source boundaries. Local polygons, surfaces with varying heights and native compound or three-axis CRS inputs keep the existing best-fit 3D policy. Simplification carries the matching source vertices through each LOD, and triangle fragmentation retains the original boundary without adding internal edges.

Reconvert existing vector archives to apply this correction. Previous vector build state is incompatible with the corrected encoder, including explicit tilesets.

`--parent-repair` affects parent display only. Leaves are unchanged. A stand-in is used only when its error bound, the source 3D bounding-box diagonal, fits the tolerance. Reports record `substitution: parentOutline`. No feature is dropped.

Oversized polygons that need triangle fragmentation still need unambiguous filled geometry. Outline fallback does not resolve their fragmentation.

Raw GDAL and GEOS warnings in native builds are quiet by default. Set `RUSTY_TILES_NATIVE_DIAGNOSTICS=1` to see them.

## Tile budgets

Every tile is checked against its actual encoded vertices and bytes. The check sums all contents in the tile, including any `b3dm` wrapper.

| Budget | Default |
| --- | --- |
| Feature fragments per leaf, `--max-features` | 64 |
| Feature fragments per parent, `--max-parent-features` | 4,096 |
| POSITION vertices per tile, `--max-vertices` | 65,536 |
| Bytes per tile, `--max-bytes` | 4 MiB |
| Tiles in the hierarchy, `--max-tiles` | 100,000 |
| Coordinates in one source feature, `--max-source-vertices` | 1,000,000 |

Indivisible geometry that exceeds a budget fails. An oversized tile is never published silently. Coincident features with the same source ID still go to separate leaves.

Large features are split to fit:

- Lines split at a shared endpoint. Every original segment is kept.
- Multi-geometries split into smaller parts.
- Polygons split their triangulated fill. Original Z and holes are kept.

A split polygon has two kinds of content. Fills are standard unlit, double-sided glTF triangles inside a `b3dm` wrapper. Source boundaries are separate vector line content. Each original boundary segment appears exactly once, and internal triangle edges are not drawn. Both contents carry feature metadata, so fills and outlines are pickable. The `b3dm` wrapper is a compatibility workaround for Cesium's decoder selection, not a private extension.

Buffered grid clipping and coverage-wide edge reconciliation are not implemented.

### Plain fill GLB limitation

The wrapper remains necessary in CesiumJS 1.146.0. The issue [#75](https://github.com/BenDyson-Arch/rusty-tiles/issues/75) browser regression removes only the wrapper and changes its URI suffix to `.glb`. Cesium selects its vector decoder from the tileset-wide `3DTILES_content_gltf_vector` declaration, including for ordinary triangle fills. That decoder expects polygon loop data and fails with `Cannot read properties of undefined (reading 'loopIndices')`. The corresponding wrapped fills load, pick and style. This also affects fragmented Sudan and Antarctica source rings and compressed fills.

Plain fill output is deferred until a runtime passes this test. There is no plain-fill CLI or Python option, and these archives do not claim compatibility with the proposed 3D Tiles 2.0 removal of `b3dm`. See [the recorded results](../bench/vector_fill_glb_results.json) and [CONTRIBUTING.md](../CONTRIBUTING.md#plain-fill-glb-regression) for reproduction.

### Memory and scratch disk

Features and a shared-coordinate index are spooled to a SQLite store beside the output. Its cache is 32 MiB. The portable GeoJSON reader also spools collection members to disk. Memory depends on one source feature, one candidate tile per worker, the hierarchy and, in native builds, the OGR driver. Leave scratch disk for transformed coordinates and indexes. Staging files are removed on success and on failure.

## Level of detail

Parents are simplified directly from the original geometry and use `REPLACE` refinement.

- `--lod-tolerance` is the base tolerance in metres. It doubles at each coarser level.
- `--lod-levels` sets how many coarse levels sit above each leaf, from 1 to 16.

Levels that remove no vertices are omitted. Parent errors never decrease toward the root.

Lines use 3D Ramer-Douglas-Peucker simplification with a conservative error bound. A polygon ring simplifies only when its planarity deviation leaves room inside the tolerance and the result stays valid. Polygons that are too far from planar stay unsimplified. Shared vertices, including fragment seams, are locked. Triangle fill fragments are not simplified.

Leaves keep every source vertex and segment. Their only change is float32 rounding, which is reported. Leaf `geometricError` is zero unless you quantize.

A parent that cannot fit all its features within the budgets becomes a routing node without content. `extras.routingReason` says why: `parentFeatures`, `vertices`, `bytes` or `estimatedBytes`.

By default, every semantic point is kept. Dense point layers get a routing hierarchy rather than sampling.

### Point aggregation

`--aggregate-points` replaces points in point-only parents with per-layer count aggregates. Leaves always keep the original points and properties.

```sh
rusty-tiles vector -i observations.gpkg -o output/observations.3tz --layer observations \
  --aggregate-points --max-parent-features 64 --lod-tolerance 5
```

Points are grouped by layer and 3D voxel, using the same grid as the point-cloud sampler. Each aggregate sits at its first original point and has these properties:

| Property | Value |
| --- | --- |
| `aggregation` | `"voxel"` |
| `sourceLayer` | The source layer |
| `pointCount` | Number of point coordinates grouped |

Aggregates use the metadata class `pointAggregate` and have no `_source_id`. Styles for distant tiles must use these properties, not the original fields. A MultiPoint counts once per coordinate.

Aggregation falls back to the original content when it gives no reduction, breaks a budget, or exceeds the tolerance. If the original content does not fit either, the tile routes to its children. The parent's geometric error is at least the tolerance, so aggregates always refine to originals up close. `extras.pointAggregation` and `conversion.json.pointAggregation` record what happened.

## Encoding

### Batching

Each content holds at most three primitives: points, lines and polygons. Per-vertex feature IDs link every part to its metadata row. Polygons keep separate exteriors and holes through the draft polygon offsets.

Several line strips in one primitive need the draft `KHR_mesh_primitive_restart` extension. It is declared as required. Engines without it cannot load that content. A single strip does not need it. Batching does not promise one GPU draw call.

### Quantization and compression

Both options are off by default and can be combined.

```sh
rusty-tiles vector -i mapping.gpkg -o output/mapping.3tz --layer roads --quantize --meshopt
```

| Option | Extension | Effect |
| --- | --- | --- |
| `--quantize` | `KHR_mesh_quantization` | Lossy 16-bit positions. The added error is reported and included in bounds and `geometricError`. |
| `--meshopt` | `EXT_meshopt_compression` | Lossless compression of accessor buffers |

Compressed content declares its extensions as required. A decoder without support rejects it rather than reading missing bytes. This encoder uses the ratified EXT encoding. `KHR_meshopt_compression` was still a release candidate when checked on 2026-10-05.

Budgets use the final encoded size. Small contents can grow because of extension JSON. `conversion.json.encoding` records the options, quantization error and byte totals. Each tile's `extras` records encoded vertices, bytes, `primitives`, geometry error, rounding and tolerance.

### Workers and reproducibility

`--jobs N` limits encoding threads. The default is the number of available cores. Each worker can hold one candidate tile up to the budgets, so lower `N` on a small machine. Output does not depend on worker count or completion order.

`conversion.json.performance` records timings. These change between runs. Add `--reproducible` to omit them and get byte-identical archives from the same input, options, binary and libraries. See [byte-identity checking](../CONTRIBUTING.md#check-byte-identity) for the comparison rules.

## Reports

| File in the archive | Contents |
| --- | --- |
| `conversion.json` | Layers, CRS and height decisions, budgets, tile maxima, encoding, reuse, and a sample of geometry reports |
| `geometry-reports.jsonl` | Every geometry report, skipped feature and repair |
| `vector-build.json` | Build state used by `--reuse-tileset` |

`conversion.json.encoding.maximumTilePrimitives` is the largest primitive count in one tile. `primitiveReferences` sums primitive counts across the hierarchy. Neither is a GPU draw-call count.

## Reuse after edits

`--reuse-tileset` rebuilds only what changed since an earlier archive. Apply your edits with an external tool first. This example needs `geodiff` and a changeset you have made:

```sh
geodiff apply updated.gpkg changes.diff
rusty-tiles vector -i updated.gpkg -o output/updated.3tz --layer roads \
  --reuse-tileset output/mapping.3tz
```

### What must match

Reuse needs the same tiling mode (`--explicit` or the implicit default), layer selection, height policy, budgets, LOD, field, list, aggregation and encoding options as the earlier build. It also needs the same encoder revision and backend. Native builds additionally need the same GDAL, GEOS and PROJ versions. Switching between portable and native builds needs a fresh conversion. Archives made by the earlier Python converter also need a fresh build first.

A mismatch fails with a reason. Remove `--reuse-tileset` to build afresh. A changed `--where` filter is the exception: it starts a fresh build and records `reuse.incompatibleReason: attribute filter changed`.

### How reuse works

- Cached source content file names contain their SHA-256 hash. Unchanged source payloads keep their URLs and bytes. Implicit display content uses coordinate templates and includes the original tile translation in its glTF scene nodes. Those display payloads and subtrees are regenerated from the cached originals.
- Changed geometry, properties, inserts, deletes and shared-vertex locks invalidate the affected subtrees.
- `vector-build.json` stores partition decisions and the original local frame. Deleting the first feature does not move unchanged content.
- Every reused payload is checked against its recorded checksum.
- Implicit archives retain the original explicit hierarchy in build state and its immutable payloads for reuse. This increases archive size; the retained sources are listed in manifest extras and validated as references.
- `conversion.json.reuse` reports reused subtrees, tiles and contents, and newly encoded contents.

Reuse saves geometry encoding, not every step. The updated source is still scanned and projected, and the archive is repacked. Source files and the earlier archive are opened read-only, and source triggers are never modified. Geometry reports cover only the new build; earlier reports stay in the earlier archive. A feature that loses its geometry is removed. Repeated edits can unbalance partitions. A fresh build resets them.

The archive is never updated in place. Publish the new archive, and keep the previous one for rollback. Diff creation, application and conflict resolution belong to `geodiff` or `go-geodiff`, not to this command. Tested versions are listed in [CONTRIBUTING.md](../CONTRIBUTING.md#verification-evidence).

## Compatibility

Use CesiumJS 1.146.0 for the audited draft vector contract. Pin your runtime and repeat the browser check when you upgrade, because draft support can change.

| CesiumJS | Result with unbatched fixtures, 2026-10-05 |
| --- | --- |
| 1.139.1 to 1.141.0 | Loads as generic glTF. Styled line widths are missing and point picking is unreliable. Not supported. |
| 1.142.0 | Native vector rendering, styling and picking. The oldest working release. |
| 1.143.0, 1.146.0 | Native vector rendering, styling and picking |

Batched content was checked on 1.143.0 on 2026-10-06. On 2026-10-09, 1.146.0 passed both batched and batched `--quantize --meshopt` fixtures, including point aggregates, holes, fragmentation, LOD, styling and picking after initial load and hard refresh. The combined format was also checked previously on 1.142.0 and 1.143.0. [Recorded results](../bench/vector_draft_compatibility.json) include the validator's unsupported primitive-restart limitation. Other engines are untested. Rendering triangles alone does not prove compatibility.

The encoder follows these draft revisions, audited against CesiumJS 1.146.0 on 2026-10-09. The SHAs remain unchanged: they are still the draft PR heads. The [compatibility audit and schema provenance](vector-schema/README.md) records the fields read by that runtime and the distinction from the 3D Tiles 2.0 RFC. The ratification recheck is tracked in [#87](https://github.com/BenDyson-Arch/rusty-tiles/issues/87).

| Extension | Reference |
| --- | --- |
| `KHR_mesh_primitive_restart` | [Cesium glTF fork revision `9811e84`](https://github.com/CesiumGS/glTF/tree/9811e8407d4533500cfc6b10e3bc408345035a6f/extensions/2.0/Khronos/KHR_mesh_primitive_restart), [PR 2569](https://github.com/KhronosGroup/glTF/pull/2569) |
| `EXT_mesh_polygon` | [Cesium glTF fork revision `c1a0354`](https://github.com/CesiumGS/glTF/tree/c1a035499b70aeb5d8281470101423e5e285dfe3/extensions/2.0/Vendor/EXT_mesh_polygon), [PR 2570](https://github.com/KhronosGroup/glTF/pull/2570) |
| `3DTILES_content_gltf_vector` | [3D Tiles revision `c48ebdc`](https://github.com/CesiumGS/3d-tiles/blob/c48ebdc8db43dc00917b4f200eff5e2131d7e493/extensions/3DTILES_content_gltf_vector/README.md), [PR 838](https://github.com/CesiumGS/3d-tiles/pull/838) |

Feature metadata uses `EXT_mesh_features` and `EXT_structural_metadata`. Cesium added experimental support for these vector extensions in [PR 13478](https://github.com/CesiumGS/cesium/pull/13478).

CesiumJS 1.146.0 selects its vector GLB decoder from the tileset-wide declaration. Fragmented triangle fills therefore retain b3dm wrappers; a plain fill GLB in the same vector tileset fails to load. This runtime limitation is included in the audit.

## Style and pick features

Use the CesiumJS IIFE build at `/cesium/Cesium.js`. Source properties are available on lines, fills, boundaries, repaired outlines and their LOD versions. Excluded fields are not.

### Style by property

This example assumes `category` and `status` string fields. Replace them with your own.

```js
const tileset = await Cesium.Cesium3DTileset.fromUrl('/data/tileset.json');
viewer.scene.primitives.add(tileset);
tileset.style = new Cesium.Cesium3DTileStyle({
  color: {
    conditions: [
      ["${category} === 'survey'", "color('cyan')"],
      ["true", "color('orange')"],
    ],
  },
  show: "${status} === 'active'",
  lineWidth: 12,
  pointSize: 12,
});
```

`show` hides features in the viewer. `--where` removes them from the archive. See the [style API](https://cesium.com/learn/cesiumjs/ref-doc/Cesium3DTileStyle.html).

### Pick a feature

```js
const handler = new Cesium.ScreenSpaceEventHandler(viewer.scene.canvas);
handler.setInputAction((click) => {
  const feature = viewer.scene.pick(click.position);
  if (!feature || typeof feature.getProperty !== 'function') return;

  const properties = Object.fromEntries(feature.getPropertyIds().map((key) => {
    const value = feature.getProperty(key);
    // BigInt needs a string before JSON serialization; undefined displays as null.
    return [key, value === undefined ? null :
      typeof value === 'bigint' ? value.toString() : value];
  }));
  console.log({
    sourceIdJson: properties._source_id,
    sourceLayer: properties._source_layer,
    properties,
  });
}, Cesium.ScreenSpaceEventType.LEFT_CLICK);
// Call handler.destroy() when disposing of this view.
```

`_source_id` is JSON text, not a tile-local number. Keep it as text when you look up source records. Parsing a large numeric ID as a JavaScript Number can lose precision. `_source_layer` separates identical IDs from different layers. See the [feature API](https://cesium.com/learn/cesiumjs/ref-doc/Cesium3DTileFeature.html).

### Missing values in CesiumJS 1.143

| Property type | Missing value returns |
| --- | --- |
| String, FLOAT64 | `undefined` |
| INT64 | The raw BigInt `noData` sentinel, such as `-9007199254740992n` |

Empty strings and zero are real values. INT64 values return `bigint`, so integers above 2^53 stay exact. The sentinel can change if the source uses that value. Compare with the property's `noData` from the content's `EXT_structural_metadata.schema.classes.feature.properties[field]`:

```js
function integerOrNull(value, schemaNoData) {
  if (value === undefined) return null;
  if (typeof value === 'bigint' && schemaNoData !== undefined &&
      value === BigInt(schemaNoData)) return null;
  return value; // Keep exact integers as BigInt, or use toString() for JSON/UI.
}
```

`--list-fields json` values are JSON text strings. Parse them if you need arrays. Repeat this check when you change runtime version, because the INT64 behaviour is specific to 1.143.

Browser probes that check these behaviours are described in [CONTRIBUTING.md](../CONTRIBUTING.md#browser-probes).
