# glTF vector prototype

The standalone `vector` command reads OGR spatial layers, including GeoPackage,
GeoJSON and Shapefile, into draft `3DTILES_content_gltf_vector` content in a
3D Tiles 1.1 `.3tz` archive. GDAL/GEOS and NumPy are required. No source datasets,
credentials or hosted services are bundled.

```sh
rusty-tiles vector -i mapping.gpkg -o mapping.3tz --layer roads \
  --maxFeatures 64 --maxVertices 65536 --maxBytes 4194304 \
  --lodTolerance 0.1 --lodLevels 3
```

`--layer NAME` is repeatable. Multiple spatial layers require explicit selection
or `--allLayers`; a single spatial layer is selected automatically. `_source_id`
is the JSON-encoded native GeoJSON ID or OGR FID, and `_source_layer` identifies
the original layer. Together they identify a source feature, including when it
appears in several fragments. These property names are reserved. Scalar fields
retain their types; missing numeric/string fields use explicit `noData`. Nullable
64-bit integers remain integers without a float conversion. Incompatible types
across selected layers, complex fields, nullable booleans, measured geometries,
geometry collections and curves fail explicitly.

Layers use their declared CRS and traditional X/Y axis order. `--sourceCrs`
overrides the declaration; `--sourceCrs local` means local metre XYZ. Two-dimensional
geospatial sources are placed at ellipsoidal height zero. A 3D GeoPackage with
only a horizontal CRS needs an explicit `--heightOffset`, in metres, to establish
ellipsoidal heights. Native compound/3D CRS operations use their declared height
reference instead, including three-axis geographic CRSs such as EPSG:7843
(GDA2020). GeoJSON follows its conventional ellipsoidal metre heights.
Declared coordinate epochs are retained; a missing epoch stays unspecified.
PROJ networking and ballpark operations are disabled; missing required operations
or grids fail. An additive height offset is not a spatial geoid transformation.
Each tile has its own local origin to reduce float32 position rounding; the
reported rounding still depends on the spatial extent of its contents.

## Budgets and memory

The input features and shared-coordinate index are spooled to a temporary SQLite
store beside the output. Median spatial partitioning uses disk-backed SQL sorts.
The converter does not collect the entire dataset in a Python list. Memory still
depends on one source feature, bounded by `--maxSourceVertices` (default 1,000,000),
one candidate tile, the hierarchy and the OGR driver's own buffering. Parent
candidates are read and simplified one feature at a time; workers stop retaining
geometry as soon as a vertex or estimated-byte guard fails. Coincident features
with duplicate source IDs still partition into separate full-detail leaves.
The SQLite cache is 32 MiB. Leave enough scratch disk space for transformed coordinates and
indexes. Staging files are removed on success and failure; Rust publishes an
archive only after conversion and packing succeed.

Every content tile is checked against **actual encoded** vertices and bytes,
summed across all contents including any `b3dm` wrapper.
Defaults are 64 feature fragments per leaf (`--maxFeatures`), 4,096 per parent
(`--maxParentFeatures`), 65,536 POSITION vertices and 4 MiB across a tile’s contents.
`--maxTiles` caps hierarchy nodes at 100,000. Indivisible geometry or metadata
that exceeds a budget fails; it does not silently publish an oversized tile.

Oversized lines split with a shared endpoint, retaining every original segment.
Multi-geometries split into smaller parts. Oversized polygons partition their
triangulated filled surface, preserving original Z and holes. Filled fragments use
standard unlit, double-sided glTF triangles in a `b3dm` compatibility wrapper;
source boundaries use separate draft
vector line content. A tile can contain both through the 3D Tiles 1.1 `contents`
array. Cesium 1.143 chooses its vector GLB decoder at tileset scope; the standard
`b3dm` container selects its model decoder for fills. This is a legacy container
workaround, with modern glTF feature metadata inside, rather than a private
extension. Original exterior and hole boundary segments occur exactly once; internal
triangle edges are not emitted as outlines. Feature metadata is present in both
contents, so filled surfaces and outlines remain pickable. Geometry reports
explicitly record this policy.
Shared vertices, including fragment seams, are locked during simplification;
triangle surface fragments are not simplified. Buffered spatial clipping, coverage-wide
edge reconciliation, implicit tiling and primitive-restart line batching remain
unimplemented.

## Optional position quantization and compression

`--quantize` writes normalized unsigned 16-bit positions with the standard
[KHR_mesh_quantization](https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Khronos/KHR_mesh_quantization)
extension. It is lossy: full-detail leaves retain feature topology/properties but
positions have an additional conservative quantization error bound. Tile extras
record `quantizationErrorMetres` separately from float32 rounding; it contributes
to bounds padding and `geometricError`, including on leaves.

`--meshopt` losslessly compresses accessor streams with
[EXT_meshopt_compression](https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Vendor/EXT_meshopt_compression).
Feature IDs, polygon loop/triangle order and metadata remain intact. Required
extensions and a standard placeholder buffer make unsupported decoders reject
compressed content rather than read missing bytes. [KHR_meshopt_compression](https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Khronos/KHR_meshopt_compression)
remains a release candidate as checked on 2026-10-05; this encoder uses the
ratified EXT encoding. Both options are off by default and can be combined:

```sh
rusty-tiles vector -i mapping.gpkg -o mapping.3tz --layer roads --quantize --meshopt
```

Budgets use the final encoded payload, including wrappers. Small contents can grow
because of extension JSON overhead. `conversion.json.encoding` records options,
maximum quantization error, and uncompressed/encoded byte totals across tile
content references; deduplicated archive size can differ. Per-tile extras record
both sizes. Reuse requires matching encoding settings and, for compression, the
same native encoder binary. Library callers enabling meshopt supply its executable
through `VectorOptions.meshopt_encoder`.

The combined format is checked with the invented browser cases on CesiumJS
1.142.0, 1.143.0 and 1.146.0. Repeat the fixture generator with `--quantize` and
`--meshopt-helper /path/to/rusty-tiles`, then run the same native browser probe.

## Vector LOD

`--lodTolerance` is the base simplification tolerance in metres, doubling at each
coarser level. `--lodLevels` (1–16, default 3) adds real simplification even for
one detailed feature. Redundant levels with no vertex reduction are omitted.
Parents simplify directly from original full-detail geometry with `REPLACE`
refinement. If a parent cannot retain every feature within the budgets, it is a
routing node without content; `extras.routingReason` identifies `parentFeatures`,
`vertices`, `bytes`, or the conservative `estimatedBytes` memory guard. Parent
feature counts are independent of the leaf partition budget. Semantic points and feature identities are never
silently sampled away. Dense point-only collections therefore provide routing,
not geometry reduction.

Lines use iterative 3D Ramer–Douglas–Peucker with a conservative continuous path
error bound. Polygon rings can simplify when twice their best-fit-plane deviation fits inside
the requested tolerance and the candidate retains validity and holes. The
remaining tolerance is used for 3D path simplification; reported geometry error
includes twice the planarity deviation and stays within the requested tolerance.
Float32 rounding is added separately. Parent errors are monotonic. Full-detail leaves
retain source vertices/segments subject to separately reported float32 rounding;
leaf geometricError is zero with the default unquantized encoding. Polygons whose nonplanarity exceeds that budget and invalid candidates remain
unsimplified. `--repair` explicitly permits invalid-outline repairs;
`--ambiguousOutlines` retains irreconcilable crossings as source 3D outlines.
Oversized polygons requiring triangle fragmentation still need unambiguous filled
geometry; outline fallback does not resolve their fragmentation.

`--parentRepair` optionally substitutes source-chord outlines when invalid topology
or topology/minimum-ring constraints would otherwise retain a polygon at full cost.
This affects parent display only; filled full-detail leaves keep their existing
repair policy. The source 3D AABB diagonal bounds the entire filled-surface/outline
substitution, including removed fill and holes. A stand-in is emitted only when
that conservative bound fits the requested tolerance; shared vertices remain
locked. Reports record `substitution: parentOutline`, source identity, error and
tolerance. This can reduce distant display fidelity; it never silently drops a
feature or changes a leaf. If the bound does not fit, the original fallback remains.

`conversion.json` records layers, CRS/height semantics, budgets, observed tile
maxima, fragmentation and a bounded sample of geometry reports. The complete
report stream is `geometry-reports.jsonl`. Tile `extras` records encoded vertices,
bytes, geometry error, position rounding and requested tolerance.

## Compatibility and validation

**Use CesiumJS 1.142.0 or a checked newer release for native vector content.**
1.142.0 is the oldest release tested with the native vector decoder; it introduced
experimental support for these extensions in
[Cesium PR 13478](https://github.com/CesiumGS/cesium/pull/13478).
The repository preview baseline is 1.143.0. Pin your application runtime and repeat
the probe when upgrading: draft support can change without a stable compatibility
promise.

The encoder follows these draft revisions:

| Extension | Encoding reference |
| --- | --- |
| `EXT_mesh_polygon` | [glTF proposal revision `c1a0354`](https://github.com/KhronosGroup/glTF/tree/c1a035499b70aeb5d8281470101423e5e285dfe3/extensions/2.0/Vendor/EXT_mesh_polygon), from [PR 2570](https://github.com/KhronosGroup/glTF/pull/2570): polygon count, triangle offsets and ring-loop indices/offsets. |
| `3DTILES_content_gltf_vector` | [3D Tiles proposal revision `c48ebdc`](https://github.com/CesiumGS/3d-tiles/blob/c48ebdc8db43dc00917b4f200eff5e2131d7e493/extensions/3DTILES_content_gltf_vector/README.md), from [PR 838](https://github.com/CesiumGS/3d-tiles/pull/838): optional tileset extension with `content.extensions.3DTILES_content_gltf_vector.vector = true`. |

Feature metadata uses `EXT_mesh_features` and `EXT_structural_metadata`.
Output declares 3D Tiles 1.1; this prototype does not claim a finalized 3D Tiles
2.0 format. Fragmented surface fills use standard `b3dm`-wrapped glTF triangles,
while their original boundaries use separate vector line content.

The following matrix was checked on 2026-10-05 using the repository IIFE preview,
headless Chromium with SwiftShader, GDAL 3.13.3 and NumPy 2.5.3. Each row used the
same invented fixtures: a detailed 3D line, a polygon with a hole, a fragmented
polygon with separate boundaries, an outline-only repaired polygon, and a point.

| CesiumJS | Load | Lines / outlines | Polygon and fragmented fills | Line / polygon LOD | Property picking | Native vector decoder |
| --- | --- | --- | --- | --- | --- | --- |
| 1.139.1 | Pass | Thin fallback; styled width absent | Pass; holes empty | Pass | Lines/fills/outlines pass; point unreliable | No |
| 1.140.0 | Pass | Thin fallback; styled width absent | Pass; holes empty | Pass | Lines/fills/outlines pass; point unreliable | No |
| 1.141.0 | Pass | Thin fallback; styled width absent | Pass; holes empty | Pass | Lines/fills/outlines pass; point unreliable | No |
| **1.142.0** | Pass | Pass, including styled widths | Pass; holes empty | Pass | Pass, including points and boundaries | Yes |
| 1.143.0 | Pass | Pass, including styled widths | Pass; holes empty | Pass | Pass, including points and boundaries | Yes |
| 1.146.0 | Pass | Pass, including styled widths | Pass; holes empty | Pass | Pass, including points and boundaries | Yes |

LOD checks observed 2 → 201 line vertices and 24 → 140 polygon vertices while
moving from coarse to full detail. Fragmented fills and semantic points are not
simplified; their hierarchy routes to the full-detail contents. Picking checks
`_source_id`, `_source_layer` and `name` from rendered features, including fill
and outline fallback. Styled line checks pick five pixels off the centerline;
fragmented boundary checks pick outside the filled polygon. These distinguish
native vector rendering from an ordinary thin glTF line. Polygon ring topology
does not itself promise visible styled outlines in every runtime; the fragmented
boundary test uses the separately emitted line content.

The three older releases did not report tile-loading errors in these fixtures.
They ignored the optional draft extensions and displayed generic glTF geometry;
that partial display is **not supported native vector behavior**. Earlier releases
and other engines have not been tested. A runtime that accepts ordinary 3D Tiles
or `b3dm` may still ignore polygon topology, vector styling or feature metadata.
Successful triangle rendering alone is insufficient to establish compatibility.

### Style and pick features by their properties

Use the CesiumJS IIFE (`/cesium/Cesium.js`) and a tested runtime from the matrix
above. Retained source properties are available on lines, polygon fills, source
boundaries, repaired outlines and their LOD representations. Fragments retain
`_source_id` and `_source_layer`, so several picked pieces can identify the same
source feature. Fields excluded with `--fields`/`--dropFields` are unavailable.

For a dataset with `category` and `status` string fields, change its display
without rebuilding the archive:

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

Replace the field names and values with your dataset's properties. The fallback
colour also covers missing categories; the status condition displays only active
features. `show` controls presentation; `--where` excludes source records from the
archive itself. See the [style API](https://cesium.com/learn/cesiumjs/ref-doc/Cesium3DTileStyle.html).

Pick a visible feature and read its source identity and retained properties:

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

`_source_id` is JSON text for the original ID, not a tile-local feature number.
Keep that text when looking up source records: blindly parsing a large numeric ID
as a JavaScript Number can lose precision. `_source_layer` distinguishes identical
IDs from different layers. The public [feature API](https://cesium.com/learn/cesiumjs/ref-doc/Cesium3DTileFeature.html)
is shared by the tested native vector and model/fill picking paths; checking the
method also avoids assuming every scene pick is a metadata feature.

**Missing values in the tested CesiumJS 1.143 runtime:** strings and FLOAT64
properties with `noData` return `undefined` from `getProperty`. Empty strings and
zero remain valid source values. INT64 values return `bigint`, preserving integers
larger than 2^53. In this runtime, missing INT64 values expose the raw BigInt sentinel
instead of `undefined` (the fixture returns `-9007199254740992n`). The encoded schema
still declares `noData`; the runtime compares its numeric JSON sentinel with a
BigInt. A sentinel can move if the source contains that value, so compare with the
property's `noData` from its encoded glTF metadata schema rather than hard-coding it:

```js
function integerOrNull(value, schemaNoData) {
  if (value === undefined) return null;
  if (typeof value === 'bigint' && schemaNoData !== undefined &&
      value === BigInt(schemaNoData)) return null;
  return value; // Keep exact integers as BigInt, or use toString() for JSON/UI.
}
```

The helper takes `noData` from the relevant content's
`EXT_structural_metadata.schema.classes.feature.properties[field]`; it does not
use private Cesium internals. The picking example above serializes values but
cannot infer a missing integer without that schema value. Nullable booleans are
rejected during ingestion. `--listFields json` properties are JSON text strings;
parse them separately if the application needs arrays.

The optional acceptance probe uses two source features per case and checks
property-based cyan/orange colours, visibility filtering, source identity, exact
64-bit values and the missing-value behavior on lines, polygon fills, fragmented
fills/boundaries, repaired outlines and points:

```sh
python3 tests/fixtures/vector_metadata.py target/vector-metadata-cases
python3 scripts/preview.py --port 9257 \
  --cesium target/vector-runtime/node_modules/cesium/Build/Cesium \
  --annotations target/vector-metadata-cases
# In another terminal, with Playwright available to Node:
NODE_PATH=target/browser-probe/node_modules \
  node tests/fixtures/vector_metadata.cjs http://127.0.0.1:9257
```

The probe downloads nothing and exits unsuccessfully if any assertion fails.
Repeat it when changing the runtime version; the nullable INT64 observation above
is specific to the checked 1.143 release.

### Repeat the browser check

This check is optional and separate from the default test suite. It uses invented
geometry and downloads no datasets. Install the normal vector Python dependencies,
Node.js, a Chromium executable, Playwright and one explicitly pinned Cesium release:

```sh
python3 tests/fixtures/vector_compat.py /tmp/rusty-tiles-vector-compat
npm install --prefix target/vector-browser --no-save --package-lock=false playwright
npm install --prefix target/vector-runtime --no-save --package-lock=false cesium@1.142.0
python3 scripts/preview.py \
  --cesium target/vector-runtime/node_modules/cesium/Build/Cesium \
  --annotations /tmp/rusty-tiles-vector-compat --port 9250
```

The fixture output directory must be new. In a second terminal, from the same
repository checkout:

```sh
NODE_PATH="$PWD/target/vector-browser/node_modules" CHROMIUM=/usr/bin/chromium \
  node tests/fixtures/vector_compat.cjs http://127.0.0.1:9250 --require-native
```

The probe prints JSON containing the runtime version, selected vertex counts,
rendered picks/properties, native collection types and browser/tile errors. It
exits unsuccessfully if a native rendering, LOD or picking check fails. To inspect
an older release's fallback behavior, change the pinned Cesium installation and
omit `--require-native`. Private traversal fields are used only in this diagnostic;
applications should use public CesiumJS APIs. These checks establish fixture
compatibility, not a GPU/performance guarantee or support for every source geometry.

### Public dataset validation

On 2026-10-05, the public-domain Natural Earth roads collection, converted to a
GeoPackage, completed in 84.0 seconds with 124.4 MiB peak subprocess RSS on the
validation machine (debug Rust build, GDAL 3.13.3). All 56,600 features and 652,521
source segments survived in 1,024 full-detail leaves and 2,465 hierarchy nodes.
An archive audit checked every scalar property and every leaf position against
the projected input. Maximum observed position rounding was 0.104 m across this
global dataset. Every GLB respected the requested 8,192-vertex/1-MiB limits;
observed maxima were 2,414 vertices and 118,972 bytes. These are measurements of
one run, not throughput or accuracy guarantees.

Reproduce the input and conversion explicitly (downloads are opt-in):

```sh
python3 scripts/public_data.py roads /path/to/cache
rusty-tiles vector -i /path/to/cache/natural-earth-roads.gpkg \
  -o /path/to/cache/natural-earth-roads.3tz --layer roads \
  --maxVertices 8192 --maxBytes 1048576
```

The download helper writes license, attribution, preparation and SHA-256
provenance beside the downloaded files. See the
[Natural Earth roads source](https://www.naturalearthdata.com/downloads/10m-cultural-vectors/roads/)
and [public-domain terms](https://www.naturalearthdata.com/about/terms-of-use/).
No public datasets are committed to this repository.

## Replace affected content after edits

Changesets stay outside the tiler. Apply a GeoPackage diff with `geodiff` or a
compatible tool, then supply the resulting GeoPackage and the previous archive:

```sh
geodiff apply updated.gpkg changes.diff
rusty-tiles vector -i updated.gpkg -o updated.3tz --layer roads \
  --reuseTileset previous.3tz
```

Use the same selection, height policy, budgets and LOD options as the baseline.
The baseline must have been created by the same encoder revision and GDAL/NumPy
versions. Incompatible settings/schema/CRS or missing build state fail explicitly;
run a fresh conversion without `--reuseTileset` in that case. Initial archives
include `vector-build.json`; it stores partition decisions, signatures and the
original local frame. Deleting the original anchor feature therefore does not
move unchanged content into a new coordinate frame.

Partition cuts and feature/fragment tie keys remain stable across revisions.
Changed geometry/properties, inserts, deletes and shared-coordinate locks affect
subtree signatures. The encoder reuses matching subtrees and rebuilds changed
branches and their bounds/errors. A shared-vertex edit also invalidates the
neighbour whose simplification constraints changed. Repeated edits can unbalance
retained partitions; a fresh conversion resets them. There is no incremental
update to an existing archive in place: publish a new archive or manifest after
validation, retaining the previous one for rollback.

Content filenames contain their SHA-256 hash. Unchanged payloads retain identical
URLs and bytes; changed payloads get new URLs. The manifest references only the
current contents. Reuse validates previous state/manifest checksums and every
reused content checksum. `conversion.json.reuse` reports reused subtrees, tiles
and unique contents, and newly encoded published contents.

This saves geometry encoding, not every step of ingestion. The updated source is
still scanned/projected, shared vertices are indexed, and initial oversized
geometry fragmentation can still run. The `.3tz` archive is repacked, including
unchanged payloads. Existing source files and archives are opened read-only.
Geometry reports cover current ingestion/new encoding; historical reports remain
in the previous archive. Diff parsing, conflict resolution and sync belong to
`go-geodiff`/`geodiff`, not this command.

Compatibility tests created byte-identical diffs using upstream geodiff 2.3.0
(`e71dfe1`) and go-geodiff v0.4.3 (`ec6a8d3`), cross-applied them, and compared incremental
world geometry/properties with a fresh conversion. For a 32-feature fixture with
attribute, geometry, insert and delete changes, both paths reused 54 of 62
published contents and encoded 8. Unchanged-input tests prohibit any encoder call.

The same producer/cross-apply checks pass with GDAL-generated spatial indexes.
After geometry moves, inserts and deletes, both applied databases retain the
same R-tree rows and spatial-filter results as the fresh source, and their
incremental tiles match fresh world geometry/properties. go-geodiff v0.4.3
provides the spatial-index functions needed by those triggers and fixes
[go-geodiff issue #3](https://github.com/tinyowl-labs/go-geodiff/issues/3).
The opt-in test requires successful indexed application; older Go versions
without these functions fail rather than being skipped. The tiler does not
modify or drop source triggers.

## Invalid features

By default, ingestion checks all selected features and reports every feature-local
geometry/coordinate failure with its source layer, ID and reason before failing.
No archive is published. Use `--skipInvalid` to explicitly omit those features
and publish the convertible remainder. `conversion.json` records the setting,
`skippedFeatures`, per-layer `invalidFeatures` and a bounded report sample;
`geometry-reports.jsonl` contains every skipped identity and reason. Unsupported
layer schemas and configuration errors still fail the job. At least one
convertible feature is required for an initial tileset. Tile/hierarchy limits and
errors encountered during encoding remain fatal; they are not silently bypassed.

## Field selection and lists

Use `--fields name,category` to include source fields, or `--dropFields tags,notes`
to exclude them before schema validation. These modes are mutually exclusive;
unknown names fail explicitly. Source identity/layer metadata remains present.
List-valued fields are rejected by default. `--listFields json` stores arrays as
JSON text in string properties, retaining element order, nulls and exact JSON
integers rather than flattening or joining values. Empty arrays become `[]`;
missing values remain metadata NoData. Per-layer `jsonFields` and the top-level
`metadata` section in `conversion.json` record the representation and selection.
Other complex property values still fail unless excluded. Field/list settings
participate in prior-tileset compatibility checks.

## Records without geometry

NULL and empty geometries are omitted automatically: they have nothing to draw.
They are distinct from invalid drawable geometry and do not need `--skipInvalid`.
`featuresWithoutGeometry` counts them globally and per layer, and the full report
stream records each source identity with outcome `no-geometry`. Drawable feature
counts and tile metadata exclude these records. A selection consisting entirely
of geometry-free records publishes a valid empty tileset with its counts and
reports. Reuse can remove formerly drawable features that become geometry-free.

## Collapsed repairs

With `--repair --ambiguousOutlines`, a polygon whose repair collapses to lines,
points or no filled area retains its original closed 3D rings as line content.
The report records `outputGeometry: outline`, identity and the collapse reason;
no fill is fabricated. Ambiguous 3D intersections use the same explicit fallback.
Normalization happens before budgeting, so large outlines use line fragmentation
without dropping source segments. A MultiPolygon requiring this fallback retains
all its rings as outlines. Without the flag, collapse remains a reported failure.

## Attribute filtering

`--where "category = 'public'"` applies an OGR attribute filter to every selected
layer, before feature reading/validation. The expression must be valid in each
selected layer; invalid filters fail before publication. Filtered-out features
are absent from geometry, metadata and feature counts. Filters may use source
fields excluded from tile metadata. `attributeFilter` records the expression in
conversion and layer reports, and the build configuration includes it. Reusing
with the same filter retains unchanged content; a changed filter triggers a
fresh rebuild with `reuse.incompatibleReason: attribute filter changed` and no
reused contents. A filter matching no records publishes an empty tileset.

Native GDAL/GEOS validity and repair warnings are quiet by default. Feature
identities and reasons remain in the diagnostics and geometry reports; actual
GDAL failures still propagate. Set `RUSTY_TILES_PYTHON_TRACEBACK=1` to retain raw
native warnings as well as Python tracebacks for debugging.

### Parallel encoding

`vector --jobs N` limits the number of encoding processes; the CLI defaults to
available cores. Use `--jobs 1` for a small-memory machine or embedding without
worker processes. Workers read bounded candidates and shared-vertex masks from
the SQLite spool through independent read-only connections. Leaf candidates and
LOD candidates can run concurrently; source reading, partition decisions and
manifest assembly remain ordered in the coordinator. Unchanged reusable
subtrees launch no encoding jobs. Worker errors prevent archive publication.

Each worker can hold a tile candidate up to the configured vertex/feature budgets,
so choose `N` with available memory in mind. Hash-named payloads, hierarchy order,
geometry reports and build-state signatures do not depend on completion order or
worker count. `conversion.json.performance` records requested jobs, the number of
workers that produced consumed candidates, and wall times for ingestion,
partitioning, encoding and publication. Partitioning includes spool preparation;
encoding includes coordinator overhead and process startup, and publication stops
before Rust archive packing. Timings are diagnostic, not part of content identity.

For a cacheable, byte-identical archive, add `--reproducible`; performance diagnostics
are omitted from `conversion.json`, while content and reuse behavior are retained.
See [reproducible builds](../CONTRIBUTING.md#reproducible-builds) for the precise
comparison rules and tested scope.
