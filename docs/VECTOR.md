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
reference instead. GeoJSON follows its conventional ellipsoidal metre heights.
PROJ networking and ballpark operations are disabled; missing required operations
or grids fail. An additive height offset is not a spatial geoid transformation.
Each tile has its own local origin to reduce float32 position rounding; the
reported rounding still depends on the spatial extent of its contents.

## Budgets and memory

The input features and shared-coordinate index are spooled to a temporary SQLite
store beside the output. Median spatial partitioning uses disk-backed SQL sorts.
The converter does not collect the entire dataset in a Python list. Memory still
depends on one source feature, bounded by `--maxSourceVertices` (default 1,000,000),
one candidate tile, the hierarchy and the OGR driver's own buffering. The SQLite
cache is 32 MiB. Leave enough scratch disk space for transformed coordinates and
indexes. Staging files are removed on success and failure; Rust publishes an
archive only after conversion and packing succeed.

Every content tile is checked against **actual encoded** vertices and bytes,
summed across all contents including any `b3dm` wrapper.
Defaults are 64 feature fragments, 65,536 POSITION vertices and 4 MiB across a tile’s contents.
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

## Vector LOD

`--lodTolerance` is the base simplification tolerance in metres, doubling at each
coarser level. `--lodLevels` (1–16, default 3) adds real simplification even for
one detailed feature. Redundant levels with no vertex reduction are omitted.
Parents simplify directly from original full-detail geometry with `REPLACE`
refinement. If a parent cannot retain every feature within the budgets, it is a
routing node without content. Semantic points and feature identities are never
silently sampled away. Dense point-only collections therefore provide routing,
not geometry reduction.

Lines use iterative 3D Ramer–Douglas–Peucker with a conservative continuous path
error bound. Polygon rings simplify only when planar within one micrometre and
when the candidate retains validity and holes. Error includes twice the planarity
deviation and float32 rounding. Parent errors are monotonic. Full-detail leaves
retain source vertices/segments subject to separately reported float32 rounding;
leaf geometricError is zero. Nonplanar polygons and invalid candidates remain
unsimplified. `--repair` explicitly permits invalid-outline repairs;
`--ambiguousOutlines` retains irreconcilable crossings as source 3D outlines.
Oversized polygons requiring triangle fragmentation still need unambiguous filled
geometry; outline fallback does not resolve their fragmentation.

`conversion.json` records layers, CRS/height semantics, budgets, observed tile
maxima, fragmentation and a bounded sample of geometry reports. The complete
report stream is `geometry-reports.jsonl`. Tile `extras` records encoded vertices,
bytes, geometry error, position rounding and requested tolerance.

## Compatibility and validation

Polygon encoding follows `EXT_mesh_polygon` proposal revision
`c1a035499b70aeb5d8281470101423e5e285dfe3` from
[Khronos glTF PR 2570](https://github.com/KhronosGroup/glTF/pull/2570).
Feature metadata uses `EXT_mesh_features` and `EXT_structural_metadata`.
The [3D Tiles extension](https://github.com/CesiumGS/3d-tiles/pull/838) remains a
draft. It is not a finalized 3D Tiles 2.0 format. Rendering, LOD selection and
property picking were checked with the Cesium 1.143.0 IIFE runtime.

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
