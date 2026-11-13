# rusty-tiles

Convert meshes, GeoJSON/GeoPackage vectors and LAS/LAZ point clouds to 3D Tiles from Python, including Blender's bundled Python.
Self-contained wheels: no GDAL, CLI, Rust compiler or other Python packages are needed at runtime.

**Status:** pre-1.0 and under active development. Function signatures, report fields, error types and archive bytes can change in any release, including patch releases; each change is listed in the [release notes](https://github.com/BenDyson-Arch/rusty-tiles/blob/main/CHANGELOG.md). Pin the version (for example `rusty-tiles==0.4.1`) in add-ons and scripts you ship. Output archives are standard 3D Tiles 1.1, so viewers are not affected.

```sh
python -m pip install rusty-tiles
```

Wheels need CPython 3.10 or newer and use the stable ABI, so one wheel covers later Python versions. They are built for Linux x86_64 and ARM64 (manylinux_2_28, glibc 2.28+), macOS 15+ on Intel and Apple Silicon, and Windows x64. They are tested inside official Blender 4.5.14 LTS on Linux x64, Windows x64 and both macOS architectures; Linux ARM64 has no official Blender 4.5 bundle.

## Example

Convert a static local `example.glb` (metres, Y up) and place it near Brisbane:

```python
from pathlib import Path
import rusty_tiles

result = rusty_tiles.mesh_to_tileset(
    Path("example.glb"),
    Path("example.3tz"),
    anchor=(153.02, -27.47, 0.0),
)
assert result.archive
assert rusty_tiles.validate(result.output)["ok"]
```

## API

Converter inputs and outputs are filesystem paths (`str` or `os.PathLike`). All keywords are keyword-only. Existing outputs are kept unless `force=True`. Without a callback, functions are silent.

| Function | Keyword arguments and result |
| --- | --- |
| `mesh_to_tileset(input, output, ...)` | `profile="balanced"`, `jobs=None`, `scratch_dir=None`, `memory_budget_bytes=None`, `anchor=None`, `orientation_xyzw=None`, `scene_offset=None`, `source_crs=None`, `source_axes=None`, `source_offset=None`, `source_offset_file=None`, `height_offset=None`, `force=False`, `callback=None`; returns `TilerResult` |
| `glb_to_3tz(input, output, ...)` | `anchor=None`, `orientation_xyzw=None`, `scene_offset=None`, `force=False`, `callback=None`; returns `ModelWrapResult` |
| `model_to_manifest(input, ...)` | `anchor=None`, `orientation_xyzw=None`, `scene_offset=None`, `force=False`, `callback=None`; writes a sibling `tileset.json` and returns `ModelManifestResult` |
| `point_cloud_to_3tz(input, output, ...)` | required `source_crs`, `force=False`, `height_offset=None`, `max_points=8192` (also the maximum), `chunk_points=100000`, `metadata=False`, `metadata_attributes=False`, `callback=None`; returns `PointCloudResult` |
| `vector_to_3tz(input, output, ...)` | GeoJSON or GeoPackage; see [Vectors](#vectors); returns `VectorResult` |
| `convert_to_3tz(input, output, ...)` | `force=False`, `callback=None`; packs an existing tileset directory and returns `PackageResult` |
| `validate(input, ...)` | `lazy_payloads=False`, `max_volume=None`, `scratch_dir=None`, `expect_profile=None`, `memory_budget_bytes=None`, `jobs=None`, `callback=None`; returns the validation report as a `dict` |
| `resolve_source_crs(definition)` | Resolves one CRS definition as the converters do, without converting; returns a `dict`, see [Source CRS checks](#source-crs-checks) |
| `supported_epsg_codes()` | Every EPSG code this package resolves, as `"EPSG:nnnn"` strings in numeric order |
| `crs_table_info()` | The EPSG table's `epsg`, `proj` and `gdal` versions and its `codes`, `builtin` and `allowList` counts, as a `dict` |
| `list_tiles(archive)` | Lists the content tiles of a `.3tz` archive or unpacked tileset directory; returns a `list` of `dict`s; see [Decoded tile views](#decoded-tile-views) |
| `decode_tile(archive, member, ...)` | `format="glb"` (plain GLB `bytes`) or `"arrays"` (a `dict` of NumPy arrays); see [Decoded tile views](#decoded-tile-views) |
| `raster_tile_to_directory(input, output, *, zoom, x, y, force=False, callback=None)` | Always raises `UnsupportedError` in this package; see [Not included](#not-included) |

Every result has `output` (the resolved absolute path) and `cleanup_diagnostics`. `PackageResult` has a `receipt`; the others have a required `report` dictionary, and `TilerResult` also has `diagnostics`. `TilerResult`, `PointCloudResult`, `VectorResult` and `PackageResult` have `archive=True`. Results are frozen, but their dictionaries are not deeply immutable.

### Not included

This package contains the standard (portable) build only. It has no imagery or terrain conversion, no Shapefile, PostGIS or other OGR-only inputs, no native GDAL/PROJ CRS operations, and no CLI, `doctor` or `preview`. Installing GDAL or its Python bindings does not add them, and there is no native wheel or pip extra. Use the [native-geospatial CLI, container or Rust library](https://github.com/BenDyson-Arch/rusty-tiles/blob/main/docs/INSTALL.md#native-geospatial-cli) for those.

`raster_tile_to_directory` and `RasterDirectoryResult` are exported, but this package cannot convert imagery: the function raises `InvalidRequestError` for an invalid XYZ request and otherwise `UnsupportedError`, without creating output or calling the callback. `raster_to_pyramid`, `RasterRequest` and `RasterDisplay` are not exported.

### Streamed local mesh tiler

`mesh_to_tileset(input, output, *, profile="balanced", jobs=None,
scratch_dir=None, memory_budget_bytes=None, anchor=None, orientation_xyzw=None,
scene_offset=None, source_crs=None, source_axes=None, source_offset=None, source_offset_file=None,
height_offset=None, force=False, callback=None)` converts a source mesh to a
`.3tz` archive in one run. The output has exact full-detail leaves under
coarse parent tiles (hierarchical level of detail, HLOD). A coarse tile's
geometric error is a sampled distance to the source, not a guaranteed bound.
Its triangles carry one representative `source_primitive` ID, while leaves keep
exact provenance, and its textures are new charts sampled from the source. See
the [mesh guide](https://github.com/BenDyson-Arch/rusty-tiles/blob/main/docs/MESH.md#coarse-levels-hlod).

Input is the local static metre/Y-up/core PBR domain in the
[mesh guide](https://github.com/BenDyson-Arch/rusty-tiles/blob/main/docs/MESH.md).
Place it either with [rigid placement](#rigid-placement) (`anchor`,
`orientation_xyzw`, `scene_offset`) or with a declared
[source CRS](https://github.com/BenDyson-Arch/rusty-tiles/blob/main/docs/MESH.md#source-crs),
not both. A source CRS uses `source_crs="EPSG:3857"` (for example),
`source_axes="y-up"` (default) or `"xyz"`, `source_offset=(E, N[, A])` added in
double precision or a `source_offset_file` holding `E:`/`N:`/`A:` lines, and `height_offset` in
metres (default 0), with the CLI's rules. `source_crs=None` or `"local"` keeps
local input. There is no CRS inference, geoid adjustment, texture-format
selector or coarse-tile tuning.

Profiles are `fast`, `balanced` (default), `quality` and `lossless`; their
settings are provisional and may change in later releases. Base-colour and
emissive leaf pages are KTX2 ETC1S in `fast`, KTX2 UASTC with Zstd in
`balanced` and `quality`, and PNG in `lossless`. Coarse pages use ETC1S in
`fast` and `balanced`. KTX2/Basis and meshopt encoding run in-process; JPEG
sources are supported. See [profiles and resources](https://github.com/BenDyson-Arch/rusty-tiles/blob/main/docs/MESH.md#profiles-and-resources).

A lossy **leaf** page that misses a decoded-image quality gate is re-encoded
at UASTC level 2, then level 3, then delivered as its exact level-0 PNG. A
mip-sampled page keeps its sampler, with viewer-generated coarser levels outside
the guarantee. A **coarse** page never fails the call: it falls back to its exact
level-0 PNG, sampled without mips, counted in
`result.diagnostics["hlod"]["appearance_by_depth"]`. Escalated pages are larger,
PNG pages most of all. They are described by these `result.report` keys:

- `escalated_pages` (`uastc_level2`, `uastc_level3`)
- `lossless_fallback_pages` (level-0-sampled)
- `viewer_mip_fallback_pages` and `viewer_mip_fallbacks` (every mip-sampled PNG page: first, depth, page, slot)
- `escalation_diagnostics` (the first 32 pages: leaf, first, depth, page, slot, sources, step, misses) and `escalation_diagnostics_omitted`

The callback is polled for cancellation before every re-encode step.

`memory_budget_bytes` is the run's declared memory ceiling: a positive `int`.
A `bool`, float or string raises `TypeError`; zero, a negative value or a value
above 2**64 - 1 raises `InvalidRequestError` before any file is read. `None`
uses the lesser of 4 GiB and half the detected headroom (2 GiB if undetected).
The budget is never raised: a run that cannot fit raises `ResourceLimitError`
before it allocates. The archive does not depend on it.
`result.diagnostics["memory"]` holds the resolved `budget` and each stage's
minimum. Declared memory accounting is not a measured RSS guarantee. See the
[memory budget](https://github.com/BenDyson-Arch/rusty-tiles/blob/main/docs/MESH.md#memory-budget).

`jobs=None` chooses workers automatically from memory. A positive integer is a
maximum, capped by cores and each stage's memory admission; booleans and
non-integers are rejected. With unknown memory, one worker is used. A mandatory
item that cannot fit is still refused. `scratch_dir` holds working data; final
archive staging stays beside the output. The output must be `.3tz`.
`force=True` replaces a completed file and keeps the original if the run fails
before commit.

`TilerResult.report` is the snake_case `conversion.json` stored in the archive.
`diagnostics` holds requested and admitted stage workers, memory and scratch
use, archive bytes and peak held records, which are kept out of the archive so
`jobs` and `scratch_dir` do not change its bytes. Placement labels and the
actual root transform are reported. See the
[error table and Cesium colour limitation](https://github.com/BenDyson-Arch/rusty-tiles/blob/main/docs/MESH.md#reports-and-errors).

Coarse tiles that need last-resort reduction declare it in
`asset.extras.rustyTiles.lastResort`; `result.diagnostics["hlod"]` reports
`last_resort_tiles`, `last_resort_stages`, `fold_faces` and `declared_tiles`.
See [last-resort declarations](https://github.com/BenDyson-Arch/rusty-tiles/blob/main/docs/MESH.md#last-resort-declarations).

### Rigid placement

`mesh_to_tileset`, `glb_to_3tz` and `model_to_manifest` share one rigid
placement. `anchor` is `(longitude_degrees, latitude_degrees,
ellipsoidal_height_metres)` on WGS84; without it, output stays local.
With an anchor you may add `orientation_xyzw`, a near-unit right-handed active
ENU quaternion (scalar W last), and `scene_offset`, a glTF Y-up metre
translation applied after the source node chain and before orientation. They
default to identity and zero. Either one without an anchor raises
`InvalidRequestError`.

The quaternion norm must be within 1e-12 of one; it is normalised once.
Longitude must be in [-180, 180] and latitude in [-90, 90]; non-finite values
are invalid. Values above the finite magnitude ceiling in the
[placement contract](https://github.com/BenDyson-Arch/rusty-tiles/blob/main/docs/architecture/f1c1-contract.md)
raise `UnsupportedError` before the source is read. These rules match the CLI
and Rust library.

The frame uses geodetic latitude and longitude at the ellipsoidal anchor,
including elevated anchors and poles; at a pole, the supplied longitude sets
the meridian. Placement is an f64 rigid root transform. `scene_offset` does
not restore a projected easting/northing shift, infer a source CRS or correct
for a geoid.

### Model wrapping and manifests

`glb_to_3tz` stores the admitted source and resource bytes unchanged under
`model/` in a `.3tz`, without new level of detail. `model_to_manifest` writes a
`tileset.json` beside the source instead; its content URI is the escaped
original file name, and the source files stay yours and must not change while
in use. Both accept the bounded static local metre/Y-up/core PBR profile.
Animation, skins, morph targets, imported extensions and extras, data URIs and
larger models are refused. Both report schema 1, profile
`w1-static-model-v1`, conservative bounds from the original nodes, root detail
error zero with a separate positive top-level visibility error, captured and
emitted byte counts, and the actual placement. See the
[wrapping contract](https://github.com/BenDyson-Arch/rusty-tiles/blob/main/docs/architecture/model-wrapping-contract.md).

### Packaging

`convert_to_3tz` packs the regular files beneath a tileset directory, or the
directory containing a given `tileset.json`. `PackageResult.receipt` is a
`PackageReceipt` with `member_count` (excluding the generated index),
`source_bytes` and `archive_bytes`. Source bytes, including `conversion.json`,
pass through unchanged. Packaging does not check scene semantics or find
resources outside the selected directory.

Outputs must end in `.3tz` or `.3dtiles.zip`. Member names may not contain
either extension, and each member must be smaller than `2**32 - 1` bytes; the
archive itself may be larger. Sources must not change during the call.
Symlinks, special files, unsafe or duplicate member names and the reserved
`@3dtilesIndex1@` are rejected, so remove a generated index before repacking an
extracted archive. The destination must be outside the source tree and must
not alias a selected file. A failure before installation keeps an existing
output; neither installation nor cleanup promises power-loss durability. On
Unix, the archive is created with mode `0600` (subject to umask), including on
replacement; change its permissions if other users need access.

A `CleanupDiagnostic` has `path`, `kind` and `message`. It names a
temporary-name path that needs inspection beside a successfully installed
output. The message distinguishes a cleanup failure from uncertain ownership
after installation, so do not assume every reported path is disposable.

### Point clouds

`source_crs` is required: `"local"` for local XYZ metres with Z up, `"header"`
for the LAS horizontal CRS, or an explicit CRS such as `"EPSG:32656"`.
Georeferenced input also needs a finite `height_offset` in metres to
ellipsoidal height; use `0` when Z is already ellipsoidal. Local input forbids
it.

```python
rusty_tiles.point_cloud_to_3tz(
    "cloud.laz", "cloud.3tz", source_crs="header", height_offset=0,
)
```

The package supports verified grid-free WGS84 geographic, UTM and Mercator
transforms, about 3,100 further EPSG codes from the
[GDAL-verified allow-list](https://github.com/BenDyson-Arch/rusty-tiles/blob/main/docs/FORMATS.md#gdal-verified-epsg-allow-list)
(such as GDA2020/GDA94 MGA, NZTM2000 and ETRS89 UTM), plus supported WKT/PROJ
local projections with explicit WGS84 or Helmert datum parameters. A report for an
allow-listed code names its EPSG datum operation and stated accuracy as
`crsOperation`. Grid-dependent (such as British National Grid and NAD27),
compound/geoid, dynamic or unknown datums and coordinate epochs raise
`UnsupportedError` naming the native build. See the
[CRS limits](https://github.com/BenDyson-Arch/rusty-tiles/blob/main/docs/FORMATS.md#point-clouds).
Text `.xyz` files and in-memory buffers are not supported.

`metadata=True` keeps every source field in a per-point property table (by
default only position and colour are kept). `metadata_attributes=True` adds
point attributes named `vertex_classification`, `vertex_intensity` and
`vertex_return_number`.

### Source CRS checks

`resolve_source_crs(definition)` is the package's form of `doctor --sourceCrs`: it runs
the converters' own resolver on an EPSG code, WKT or PROJ string and reads no data, so
a tool can show the datum operation before a long conversion.

```python
check = rusty_tiles.resolve_source_crs("EPSG:7856")
# {"accepted": True, "kind": "allowlist", "definition": "+proj=utm +zone=56 ...",
#  "crsOperation": {"source": "EPSG:7856", "kind": "null",
#                   "operation": "GDA2020 to WGS 84 (2)", "operationId": "EPSG:8450",
#                   "accuracyMetres": 3.0, "verification": "..."},
#  "summary": "EPSG:7856: GDA2020 to WGS 84 (2) (EPSG:8450), EPSG-stated accuracy 3 m",
#  "refusal": None}
```

`kind` is `"builtin"` (WGS 84 geographic, UTM and Mercator), `"allowlist"` (any other
EPSG code), `"wkt"` or `"proj"`, or `None` for text of none of those forms.
`definition` is the text the transform is built from. `crsOperation` has the same
shape as the reports' `crsOperation` and is `None` except for allow-listed codes;
`summary` is its one-line form, or `"EPSG:nnnn: WGS 84, no datum change"` for a
built-in code. An unsupported definition is returned with `accepted=False` and the
converters' `refusal` message instead of raising. A non-`str` argument raises
`TypeError` and an empty one `InvalidRequestError`.

`supported_epsg_codes()` lists exactly the codes `resolve_source_crs` accepts (about
3,200); `crs_table_info()` gives the EPSG, PROJ and GDAL versions they were verified
with. No CRS names or areas of use are included: take those from your own catalogue,
such as pyproj.

### Vectors

Vector conversion uses the same disk-backed tiling, metadata, LOD and encoding
as the CLI. GeoJSON defaults to WGS84 longitude/latitude with ellipsoidal metre
Z; GeoPackage uses its layer CRS. Pass `source_crs` for local metre XYZ. A 3D
GeoPackage with a horizontal CRS needs an explicit `height_offset`. The same
grid-free CRS limits apply.

```python
result = rusty_tiles.vector_to_3tz(
    "buildings.gpkg", "buildings.3tz", layers=["buildings"],
    height_offset=0, fields=["name", "height"], reproducible=True,
)
```

Keywords and defaults: `force=False`, `source_crs=None`, `height_offset=None`,
`layers=None`, `all_layers=False`, `fields=None`, `drop_fields=None`,
`list_fields="error"`, `where_clause=None`, `repair=False`,
`ambiguous_outlines=False`, `skip_invalid=False`, `max_features=64`,
`max_vertices=65536`, `max_bytes=4194304`, `max_tiles=100000`,
`max_source_vertices=1000000`, `lod_tolerance=0.1`, `lod_levels=3`, `jobs=None`
(available cores), `reproducible=False`, `quantize=False`, `meshopt=False`,
`parent_repair=False`, `aggregate_points=False`, `max_parent_features=4096`,
`reuse_tileset=None` and `callback=None`.

Layer and field selections are lists of strings. `where_clause` is a read-only
SQLite expression. `list_fields="json"` keeps list and object fields as JSON
strings. `reuse_tileset` names an earlier compatible archive. Only typed
feature rejections can be skipped; I/O, source reads, SQLite, required report
writes, cancellation and callback failures abort even with `skip_invalid=True`.
Close or checkpoint a GeoPackage with unmerged WAL edits before conversion. See
the [vector guide](https://github.com/BenDyson-Arch/rusty-tiles/blob/main/docs/VECTOR.md)
for geometry, filtering, repair and reuse limits.

### Validation

`validate` returns the [validation report](https://github.com/BenDyson-Arch/rusty-tiles/blob/main/docs/VALIDATION.md)
as a `dict`, including `ok`, completed checks, inspection gaps, `storedBytes`,
the effective `limits` and their `limitDerivation`. Its `payloads` entry is a
list, or a lazy sequence when `lazy_payloads=True`.

- `max_volume` maps limit keys to ints: opt-in volume caps such as
  `sourceArchiveBytes`, or replacement budgets such as `hierarchyVisits`. A bad
  key or value raises `InvalidRequestError` before any I/O.
- `scratch_dir` places scratch in an existing directory.
- `expect_profile="hlod"` requires the HLOD profile whatever the archive's
  labels say: every payload must carry source identity and every HLOD check
  runs. An archive that declares HLOD is held to it anyway. Any other value
  raises `ValueError` before the archive is opened.
- `memory_budget_bytes` (an `int`) sizes the scratch stores, with the same
  rule and default as `mesh_to_tileset`; below 512 MiB raises
  `ResourceLimitError`. `jobs` (a positive `int`, or `None` for auto) caps the
  workers. Neither changes a verdict or any report entry outside `resources`
  and `scratch`; `resources`, present when either is given, states what was
  resolved.
- `callback` receives progress events with totals, including the HLOD
  `validate.delivery` phases; raising from it, or Ctrl-C, stops validation.

A limit refusal raises `ResourceLimitError` whose `limit` attribute is
`{"limit", "value", "observed", "derivation"}`.

### Decoded tile views

`list_tiles` and `decode_tile` read one tile of a `.3tz` archive or an unpacked
tileset directory for viewers that read neither `EXT_meshopt_compression` nor
`KHR_texture_basisu`, such as Blender's bundled glTF importer. A decoded tile
is a view for display and inspection, not a reversible round trip: nothing is
re-encoded, and an edited view cannot be published back.

```python
tiles = rusty_tiles.list_tiles("site.3tz")
leaf = max((t for t in tiles if t["role"] == "leaf"), key=lambda t: t["depth"])
Path("leaf.glb").write_bytes(rusty_tiles.decode_tile("site.3tz", leaf["uri"]))
arrays = rusty_tiles.decode_tile("site.3tz", leaf["uri"], format="arrays")
```

- `list_tiles(archive)` returns one dict per content URI, parent before
  children, with external documents and implicit subtrees expanded: `uri`,
  `depth` (hops from the root, which is 0; this can differ from the level in a
  member's file name), `role` (`"leaf"` for a tile without children,
  otherwise `"coarse"`), `geometric_error` and `transform`, the accumulated
  tile transform as four row lists (tile frame to the tileset's world, ECEF
  for placed output).
- `decode_tile(archive, member, *, format="glb")` takes a `uri` from
  `list_tiles`; any other member raises `InvalidRequestError`.
- `format="glb"` returns the `bytes` of a plain glTF 2.0 GLB with one embedded
  buffer. Meshopt views are decoded, and KTX2/Basis pages are transcoded to PNG
  at level 0; their other mip levels are dropped. Everything else is kept:
  accessors (so `KHR_mesh_quantization`, which Blender reads), samplers,
  materials, `_FEATURE_ID_n` attributes with `EXT_mesh_features`, and
  `EXT_structural_metadata` property tables.
- `format="arrays"` needs NumPy (Blender bundles it); without it the call
  raises `EnvironmentError`. It returns a dict with the tile's `uri`, `depth`,
  `role` and `geometric_error`, `transform` and `root_transform` as (4, 4)
  float64 arrays, `primitives`, `pages`, `property_tables`, and the document's
  `materials`, `textures` and `samplers`. A texture's `source` indexes `pages`.
- Each primitive has `node`, `mesh`, `primitive`, `material`, `positions`
  ((N, 3) float64), `indices` ((T, 3) uint32), `normals` ((N, 3) float32 or
  `None`), `uvs` (a list of (N, 2) float32 arrays, `TEXCOORD_0` first),
  `colors` ((N, 4) float32 or `None`) and `feature_ids`. Each feature-ID set is
  a dict with `label`, `feature_count`, `null_feature_id`, `property_table` and
  per-vertex `values` ((N,) uint32). Leaves carry `source_primitive` and
  `source_triangle`; coarse tiles carry `source_primitive` only. A face's ID
  is the value at any of its corners. Triangles are counter-clockwise front
  faces in the tile frame: under a mirroring node transform (negative
  determinant) corners 1 and 2 are swapped relative to the GLB.
- `pages` holds one (H, W, 4) uint8 RGBA array per glTF image, top row first.
  Each property table is a dict with `class`, `count`, `properties` (name to a
  1-D array of the stored values in the schema's component type),
  `value_transforms` and `skipped`. Values are returned as stored: where a
  property is `normalized` or has an `offset` or `scale`, `value_transforms`
  holds them, and its value is `offset + scale * v` (normalize `v` first; an
  omitted offset is 0 and scale 1). `noData` and `default` are not applied.
  Properties that are not numeric scalars (strings, enums, booleans, arrays,
  vectors, matrices) are not decoded; `skipped` maps each name to the reason.
- Positions and normals are in the tile frame: node transforms are applied and
  glTF Y-up is turned to Z-up, `(x, -z, y)`, which is the frame Blender's
  importer shows. `transform @ [x, y, z, 1]` is the world position. Mesh tiler
  output has a transform on the root only, so positions are already in its
  local frame (see the
  [mesh guide](https://github.com/BenDyson-Arch/rusty-tiles/blob/main/docs/MESH.md#decoded-tile-views)).

Both functions release the GIL while reading and decoding. Malformed content
raises `DataError`; other codecs, external resources, legacy tile formats and,
in a directory, members that are symlinks or not regular files raise
`UnsupportedError`. Each decode is bounded before it allocates or walks:

- a member of at most 1 GiB, and JSON (a tile's JSON chunk or one tileset
  document) of at most 64 MiB, counted at 16 times its size once parsed;
- at most 1 GiB held at once by one decode: the parsed JSON, decoded views and
  outputs stay held, decode scratch is released as it is freed, and the copy
  into Python is counted as it happens (one primitive, page or table at a time
  for arrays, the whole GLB for `format="glb"`), so a tile with two 8192 x 8192
  pages decodes;
- 65,536 node visits plus primitive instances;
- pages of at most 16384 texels per edge and 8192 x 8192 texels in total, sized
  from their headers.

A refusal raises `ResourceLimitError` whose `limit` is
`{"limit", "value", "observed", "derivation": "fixed"}`, with `limit` one of
`memberBytes`, `jsonBytes`, `decodedBytes`, `sceneWork`, `pageEdge` or
`pageTexels`.

## Callbacks and threads

Callbacks receive dictionaries in the CLI's NDJSON event format:
`{"event": "progress", "phase": ..., "done": ..., "total": ...}` (`total` can
be `None`), `{"event": "warning", ...}` with the warning's detail fields, or
`{"event": "log", "message": ...}`. Packaging adds `encoding` and
`ready_to_publish` phases; model wrapping and manifests add `model_capture`,
`model_archive` (archive only) and `ready_to_publish`.

Conversions release the GIL, and a callback can run on a Rust worker thread.
Blender callers should queue events for the main thread before changing
Blender state.

Every callback finishes before the output is sealed and installed, including
`ready_to_publish`. An exception from a callback aborts the job before
publication and keeps an existing destination, even with `force=True`. Paths
are resolved before the first callback, so changing the working directory in a
callback cannot redirect the output. If the callback failure is the job's first
fatal cause, the original exception is raised. Nested and concurrent calls are
independent. Pending Python signals are checked during preparation and events,
not after installation; an asynchronous interruption or allocation failure
after commit can still interrupt the caller.

## Errors

Rust failures raise subclasses of `TilesError`: `DataError` (invalid source
content), `ResourceLimitError`, `EnvironmentError`, `OutputExistsError`
(competing output), `TilesIOError` (source or storage I/O), `UnsupportedError`,
`InvalidRequestError` (invalid choices or reserved names), `CancelledError` and
`ObserverError` (callback failure with no original Python exception). An
internal invariant failure raises `TilesError` itself. Invalid Python argument
types raise `TypeError` or `ValueError`. Original callback exceptions are
raised unchanged.

Converter errors carry `kind`, `secondary_diagnostics`, `retained_paths` and
`recovery`. Kinds are `invalid_request`, `invalid_input`, `unsupported`,
`resource_limit`, `io`, `output_conflict`, `cancelled`, `observer_failure` and
`invalid_state`. `retained_paths` names kept scratch. `recovery` is `None`, or
for a failed directory replacement a dict with `output` and `previous_output`
paths; if restoration failed, the original entry remains at `previous_output`.
Original callback exceptions do not have these attributes.

## Build and test from source

With stable Rust, a C++ compiler and CPython 3.10+, from a source checkout:

```sh
python -m pip install 'maturin==1.15.0'
maturin build --release --locked --out target/wheels
python scripts/test_python_wheel.py target/wheels/*.whl
```

Leave `LIBSQLITE3_SYS_USE_PKG_CONFIG` unset. The test installs the wheel into a
fresh virtual environment and runs this README's example and the API tests
with an empty executable `PATH`.

To test a wheel inside Blender's Python without changing your Blender
installation or preferences:

```sh
python scripts/test_blender_wheel.py target/wheels/*.whl --blender /path/to/blender --report-json target/blender-acceptance.json
```

Blender's Python checks wheel compatibility and installs it into a temporary
directory using an isolated pip, then runs the same API suite with an empty
`PATH`. `--blender` defaults to `blender` on your `PATH`. The JSON report
records the Blender and Python versions, platform, wheel SHA-256 and test
counts; `test_python_wheel.py` also accepts `--report-json`. Both runners need
fresh results matching the wheel and replace an earlier report.

To repeat the official Blender check with host Python 3.12 or newer, pick
`linux-x64`, `windows-x64`, `macos-x64` or `macos-arm64` for your host:

```sh
python scripts/download_blender.py --platform linux-x64 --directory /tmp/official-blender --report-json target/blender-distribution.json
python scripts/test_blender_wheel.py target/wheels/*.whl --blender /tmp/official-blender/blender-4.5.14-linux-x64/blender --distribution-json target/blender-distribution.json --report-json target/blender-acceptance.json
```

The downloader verifies each archive against the checksums pinned in
`scripts/blender_official.json` (from
[Blender's published checksums](https://download.blender.org/release/Blender4.5/blender-4.5.14.sha256))
before extraction. It needs a new directory, prints the executable path and
records it in its JSON. On macOS it copies the application from a read-only DMG
mount and detaches it; it does not modify a system installation. The runner
checks the extracted executable's hash, Blender version and architecture.
Blender 4.5 LTS is supported until July 2027 and is the last official release
series for Intel macOS; see
[Blender's compatibility notes](https://developer.blender.org/docs/release_notes/compatibility/).
CI acceptance and updating the Blender pin are described in
[CONTRIBUTING.md](https://github.com/BenDyson-Arch/rusty-tiles/blob/main/CONTRIBUTING.md#python-acceptance).
