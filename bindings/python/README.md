# rusty-tiles for Python

Convert meshes, GeoJSON and GeoPackage vectors, local or grid-free georeferenced LAS/LAZ point clouds and existing tilesets directly from
Python, including Blender's bundled Python. The wheels contain the portable
standard (default) Rust build: no CLI subprocess, GDAL, Rust compiler or extra Python
packages are needed at runtime.

## Package scope

The wheel supports mesh conversion, local/grid-free georeferenced LAS/LAZ,
GeoJSON/GeoPackage vectors, tileset packaging and archive validation. The
function table below is the complete Python API.

**Native-geospatial is a separate CLI/Rust build, not a pip extra or wheel.**
The Python package does not provide imagery or terrain conversion, Shapefile,
PostGIS or other OGR-only inputs, or native GDAL/PROJ fallback. Installing GDAL
or its Python bindings does not add those capabilities to this wheel. Use the
[native CLI/container or Rust library](../../docs/INSTALL.md#native-geospatial-cli)
for those operations. The wheel also does not install the `rusty-tiles` CLI or
expose `doctor` or `preview`. Use `model_to_manifest` for the single-model sibling manifest product.

## Install and convert

After the first PyPI publication, install a released standard wheel with:

```sh
python -m pip install rusty-tiles
```

Use CPython 3.10 or newer. Release wheels cover Linux x86_64 and ARM64
(glibc 2.28+), macOS Intel and Apple Silicon (macOS 15+), and Windows x64.
The CPython stable ABI lets each wheel work across Python minor versions.

Copy `tests/fixtures/example.gltf` from the repository to your working directory,
then run this example:

```python
from pathlib import Path
import rusty_tiles

result = rusty_tiles.mesh_to_3tz(
    Path("example.gltf"),
    Path("example.3tz"),
    cartographic=(153.02, -27.47, 0.0),
)
assert result.archive
assert rusty_tiles.validate(result.output)["ok"]
```

All inputs and outputs are filesystem paths (`str` or `os.PathLike`).
Legacy mesh and implicit conversions return `ConversionResult` with `output` (a `pathlib.Path`), `archive`
(a boolean) and `report` (a dictionary or `None`). Existing outputs are
preserved unless `force=True`; publication uses the Rust library's private
staging and atomic archive replacement.
The bounded mesh, point-cloud and vector entry points return their own typed
results with required reports and cleanup diagnostics, as described below.

| Function | Keyword arguments |
| --- | --- |
| `mesh_local_to_3tz(input, output, ...)` | `leaf_triangles` required; `root_proxy_triangles=None`, `max_proxy_error_metres=None`, `anchor=None`, `orientation_xyzw=None`, `scene_offset=None`, `force=False`, `callback=None`; returns `MeshResult` |
| `mesh_to_3tz(input, output, ...)` | `cartographic=None`, `rotation=None`, `force=False`, `max_triangles=20000`, `max_bytes=204800`, `tile_size=2048`, `texture_format="lossless"`, `source_crs="auto"`, `source_offset=None`, `meshopt=True`, `explicit=False`, `node_features=False`, `source_axes=None`, `height_offset=None`, `callback=None` |
| `glb_to_3tz(input, output, ...)` | `anchor=None`, `orientation_xyzw=None`, `scene_offset=None`, `force=False`, `callback=None`; returns `ModelWrapResult` |
| `model_to_manifest(input, ...)` | Same rigid placement/policy/callback keywords; returns `ModelManifestResult` with fixed sibling output |
| `point_cloud_to_3tz(input, output, ...)` | `force=False`, required `source_crs`, `height_offset=None`, `max_points=50000`, `chunk_points=100000`, `explicit=False`, `metadata_attributes=False`, `callback=None` |
| `vector_to_3tz(input, output, ...)` | See vector options below; accepts GeoJSON and GeoPackage; returns `VectorResult` |
| `convert_to_3tz(input, output, ...)` | `force=False`, `callback=None`; returns `PackageResult` |
| `convert_to_implicit(input, output, ...)` | `force=False` |
| `validate(input)` | Returns the bounded [validation report](../../docs/VALIDATION.md), including completed checks, inspection gaps and limits |

For legacy `mesh_to_3tz`, `cartographic` is `(longitude_degrees, latitude_degrees, height_metres)`;
`rotation` is `(heading_degrees, pitch_degrees, roll_degrees)`. Mesh
`source_crs` retains `auto`, `geographic` and `epsg:3857` adapters. General
horizontal CRS definitions require `source_axes="xyz"` (easting/northing/height)
or `"y-up"` (easting/height/negative northing), after glTF node transforms, and
an explicit `height_offset` in metres to ellipsoidal height. `source_offset`
is `(easting, northing, height)`: E/N in horizontal CRS units and height in
metres; the height offset is added after that source shift. The wheel shares
the [verified grid-free CRS tier](../../docs/FORMATS.md#point-clouds); other
operations require the native CLI/library. General placement refuses
compound/vertical/geocentric input and manual cartographic placement/rotation.
Omitting axes/height preserves legacy adapter behaviour. Mesh textures use
lossless PNG by default; `jpeg` and `webp` use bundled Rust encoders. UASTC
is outside this wheel's API because it requires an external encoder.
`explicit=True` selects the legacy hierarchy instead of implicit tiling.
`node_features=True` preserves mesh nodes as pickable features with `name` and
`node_index` properties, including for small models and across parent LODs.
`metadata_attributes=True` adds point property attributes named
`vertex_classification`, `vertex_intensity` and `vertex_return_number`, retaining
the original LAS property tables for picking and table styling.

### Local static mesh foundation and explicit placement

```python
result = rusty_tiles.mesh_local_to_3tz(
    "local.glb", "local.3tz", leaf_triangles=1000,
)
assert result.report["coordinates"] == "local-gltf"

placed = rusty_tiles.mesh_local_to_3tz(
    "local.glb", "placed.3tz", leaf_triangles=1000,
    anchor=(153.02, -27.47, 25.0),
    orientation_xyzw=(0.0, 0.0, 0.0, 1.0),
    scene_offset=(10.0, 2.0, -5.0),
)
assert placed.report["coordinates"] == "wgs84-ecef"
assert placed.report["schema_version"] == 6
```

`mesh_local_to_3tz(input, output, *, leaf_triangles, root_proxy_triangles=None, max_proxy_error_metres=None,
force=False, anchor=None,
orientation_xyzw=None, scene_offset=None, callback=None)`
requires a positive explicit per-leaf triangle limit and interprets the input
as local metre/Y-up geometry. The bounded [core PBR profile](../../docs/architecture/f1b3-contract.md)
accepts static GLB/glTF, confined local buffers and PNG/JPEG images, all five core
texture bindings, UV0/UV1, vertex colors and authored normals/tangents. It preserves
selected image bytes and material bindings across spatial leaves. Normal textures
require authored normals and tangents; tangent-bearing geometry permits accumulated
rotation, uniform scale and reflection. Extras, extensions, animation and other
excluded semantics are refused before staging. Full detail is the default. Source CRS remains explicit and no generated tangent
basis or atlas is introduced. Source resource limits and
capture rules follow the [F1b2 contract](../../docs/architecture/f1b2-contract.md).

The [F1c1 placement contract](../../docs/architecture/f1c1-contract.md) defines the independently checked rigid frame and numerical domain. `anchor` is `(longitude_degrees, latitude_degrees,
ellipsoidal_height_metres)` on WGS84. Omission keeps local output. An anchor
permits `orientation_xyzw`, a near-unit right-handed active ENU quaternion
(scalar W last), and `scene_offset`, a glTF Y-up metre translation after the
source node chain and before orientation. Their omissions mean identity and zero;
either without an anchor raises `InvalidRequestError`. The quaternion norm must
be within 1e-12 of one and is normalized once. Longitude/latitude must be in
[-180,180]/[-90,90]; nonfinite values are invalid. The finite magnitude ceiling
is specified in the contract and exceeding it raises `UnsupportedError` before
source I/O. These domain decisions are shared with Rust/CLI.

The frame uses cartographic latitude/longitude at the ellipsoidal anchor,
including elevated anchors and poles; supplied longitude defines the pole
meridian. Placement is an f64 rigid root transform. Source GLB coordinates,
normal/tangent frames, UVs, colors and resource associations stay local. Offset
does not restore a projected E/N/A shift, infer source CRS, or correct a geoid.
Schema 6/profile `f1d1-root-proxy-gltf-v1` records `source_coordinates="local-gltf"`,
output `coordinates`, tagged `placement` with normalized parameters, and exact
`root_transform`, along with existing counters. The [F1c2 candidate](../../docs/architecture/f1c2-contract.md) carries two labeled feature sets: `source_primitive` exposes source node/mesh/primitive indices and optional authored name with an explicit presence flag; `source_triangle` adds the original triangle ordinal. Table row IDs are leaf-local; source keys belong to the unchanged source document, not a persistent business namespace. Imported metadata/extras remain excluded.

For one coarse root, supply both `root_proxy_triangles=N` and
`max_proxy_error_metres=E`. Positive finite E is the emitted root error budget,
validated against a separately certified complete surface bound. N bounds the
achieved proxy count; actual reduction is required. This first profile accepts
opaque, untextured, positions-only selected geometry. Unsupported requests fail
before staging with `UnsupportedError`; malformed policy raises
`InvalidRequestError`. `report["approximation"]` distinguishes `full_detail` and
`root_proxy`, reports actual bound/counts/work, and names the admitted appearance
profile. Coarse picking exposes complete `proxy_region` membership arrays while
unchanged leaves retain exact original triangle identities. See the
[F1d1 contract](../../docs/architecture/mesh-approximation-contract.md) for
certificate looseness and appearance limits, and the
[bounded evidence](../../bench/architecture_audit/mesh_approximation/README.md).

It returns frozen `MeshResult` with a resolved absolute `output` Path, `report`
dictionary identical to published `conversion.json`, and `cleanup_diagnostics`.
The limit bounds triangles per leaf, not process memory or output bytes. This
entry point is separate from `mesh_to_3tz`; unsupported sources do not fall back.
Broader legacy CRS/LOD/node-feature APIs remain pending their separate migrations;
this bounded identity candidate does not authorize their deletion.
It shares packaging's fallible precommit callback and typed domain-error contract,
including preservation of the original callback exception when that cause wins.
Python result materialization and external asynchronous exceptions remain outside
the core publication guarantee. `force=True` permits completed-file replacement.

`glb_to_3tz` now returns frozen `ModelWrapResult` with a required report and cleanup diagnostics, and captures exact admitted source/resource bytes beneath `model/` without new LOD. It accepts the bounded static local metre/Y-up/core PBR profile, explicit rigid placement and fallible callbacks. Animation, skins, morph targets, imported extensions/extras, data URIs and larger models are excluded; the former broader wrapping escape hatch and HPR keywords are retired.

`model_to_manifest(input, ...)` returns `ModelManifestResult` and writes fixed sibling `tileset.json` through completed-file F0 publication. The content URI is the escaped original basename; all source resources remain caller owned and must stay unchanged during publication and continued use. Directory discovery and arbitrary manifest output bases are retired. Both products report schema 1/profile `w1-static-model-v1`, original-node conservative bounds, root detail error zero and a separate positive top-level visibility error, captured and emitted byte counts, and actual placement. See the [wrapping contract](../../docs/architecture/model-wrapping-contract.md).
`convert_to_3tz` packs the regular files beneath a tileset directory or the
directory containing its exact `tileset.json` path. It returns `PackageResult`
with `output`, `archive=True`, a `PackageReceipt`, and `cleanup_diagnostics`.
Its `output` is the resolved absolute destination used for validation and
installation; later callback changes to the working directory cannot redirect it.
The receipt has `member_count` (excluding the generated index), `source_bytes`
and `archive_bytes`. A cleanup diagnostic has `path`, `kind` and `message`;
it identifies a temporary-name path needing inspection beside a successfully
installed output. Its message distinguishes cleanup failure from uncertain
post-installation ownership; do not assume every reported path is disposable.
Selected source bytes, including `conversion.json`, pass through unchanged;
the returned receipt is separate from those files. Packaging does not check
scene semantics or discover resources outside the selected directory.

Package outputs must end in `.3tz` or `.3dtiles.zip`. Member names may not contain
either extension, and each member must be smaller than `2**32 - 1` bytes;
the archive as a whole may exceed that size.
Package sources must remain stable during the call. Symlinks, special files,
unsafe/duplicate member names and the reserved `@3dtilesIndex1@` are rejected.
Remove a generated index explicitly before repacking an extracted archive.
The destination must be outside the source tree and must not alias a selected
file. `force=False` refuses a competing destination; `force=True` permits
completed-file replacement. A failure before installation preserves an
existing output. Namespace installation and temporary-name cleanup are
separate; neither policy promises power-loss durability. On Unix, packaged output is
created with private mode `0600` (subject to umask), including when replacing a previous file;
change its permissions explicitly if other users need access.
Point-cloud calls require coordinate intent: pass `source_crs="local"` for
local XYZ metres with Z up. For globe placement,
pass `source_crs="header"` or an explicit CRS such as `"EPSG:32656"`, and
`height_offset=0` only when source Z is already ellipsoidal metres:

```python
rusty_tiles.point_cloud_to_3tz(
    "cloud.laz", "cloud.3tz", source_crs="header", height_offset=0,
)
```

The wheel supports verified grid-free WGS84 geographic, UTM and Mercator
transforms, plus supported WKT/PROJ local projections with explicit WGS84
or Helmert datum parameters. It refuses grid-dependent, compound/geoid,
dynamic or unknown datum definitions with `UnsupportedError` naming the
native build. See the [CRS limits](../../docs/FORMATS.md#point-clouds).
Text `.xyz` files and in-memory buffers are not supported in this release.

Vector conversion uses the same disk-backed tiling, metadata, LOD and encoding
pipeline as the CLI. GeoJSON defaults to WGS84 longitude/latitude; GeoPackage
uses its layer CRS. Use required `source_crs` for metre XYZ coordinates. A 3D
GeoPackage with a horizontal CRS requires an explicit `height_offset`; GeoJSON
Z is ellipsoidal metres. The same grid-free CRS restrictions apply to vectors.

```python
result = rusty_tiles.vector_to_3tz(
    "buildings.gpkg", "buildings.3tz", layers=["buildings"],
    height_offset=0, fields=["name", "height"], reproducible=True,
)
```

Vector keyword defaults are `force=False`, `source_crs=None`, `height_offset=None`,
`layers=None`, `all_layers=False`, `fields=None`, `drop_fields=None`,
`list_fields="error"`, `where_clause=None`, `repair=False`,
`ambiguous_outlines=False`, `skip_invalid=False`, `max_features=64`,
`max_vertices=65536`, `max_bytes=4194304`, `max_tiles=100000`,
`max_source_vertices=1000000`, `lod_tolerance=0.1`, `lod_levels=3`, `jobs=None`
(available cores), `explicit=False`, `reproducible=False`, `quantize=False`,
`meshopt=False`, `parent_repair=False`, `aggregate_points=False`,
`max_parent_features=4096`, `reuse_tileset=None`, and `callback=None`.
Layer and field selections are lists of strings. `where_clause` is a read-only
SQLite expression, `list_fields="json"` preserves list/object fields as JSON
strings, and `reuse_tileset` names an earlier compatible portable archive.
The returned `VectorResult` exposes `output`, `archive=True`, the finalized
`report`, and `cleanup_diagnostics`. Only explicitly typed feature rejections
are skippable. I/O, source reads, SQLite, required report writes, cancellation
and callback failures abort even with `skip_invalid=True`.
See [vector conversion](../../docs/VECTOR.md) for geometry, filtering, repair
and reuse limits. Close or checkpoint a GeoPackage with unmerged WAL edits
before conversion.

Callbacks receive dictionaries using the Rust reporter's event contract:
`{"event": "progress", "phase": ..., "done": ..., "total": ...}`,
`{"event": "warning", ...}` or `{"event": "log", "message": ...}`.
`total` can be `None`. Structured warnings preserve their detail fields.
`convert_to_3tz` also accepts a callback, with `encoding` and
`ready_to_publish` progress phases. Model wrapping/manifest publication also accepts callbacks with `model_capture`, `model_archive` (archive only) and `ready_to_publish` phases.
Without a callback, converters are silent.

Conversions release the GIL. A callback can run on a Rust worker thread;
Blender callers should queue events for their main thread before changing
Blender state. The first callback exception is saved, later callbacks are
skipped, and the original exception is raised after Rust finishes. A callback
exception does not cancel the legacy mesh converters, so their output may already exist.

Packaging, bounded mesh, model wrapping/manifest, point-cloud, raster directory and vector conversion use a fallible
synchronous callback. Every callback for these operations finishes
before sealing and installation, including `ready_to_publish`. A callback
exception aborts before publication and preserves the existing destination,
including with `force=True`. The original exception is raised when observer
failure is the job's selected primary cause; the first fatal cause accepted by
the job gate is primary and later failures are secondary. Nested calls and
concurrent calls use independent state. These operations check pending Python
signals during preparation and events, and does not deliberately check them
after installation. Python asynchronous interruption or allocation failure
after commitment can still interrupt caller-side execution.

Rust failures raise subclasses of `TilesError`: `DataError`, `ResourceLimitError`,
`EnvironmentError`, `OutputExistsError`, `TilesIOError` or `UnsupportedError`.
Invalid Python arguments raise `TypeError` or `ValueError`. Original callback
exceptions are preserved.

Foundation operations additionally raise `InvalidRequestError` for invalid choices or
reserved names, `CancelledError` for cancellation and `ObserverError` when no
original Python exception is available. These are `TilesError` subclasses.
Their domain errors carry `kind`, `secondary_diagnostics` and `retained_paths`.
Stable kinds are `invalid_request`, `invalid_input`, `unsupported`, `io`,
`output_conflict`, `cancelled`, `observer_failure` and `invalid_state`.
Invalid source content maps to `DataError`, source/storage I/O to `TilesIOError`
and competing outputs to `OutputExistsError`. Original callback exceptions are
returned unchanged and need not have these domain metadata attributes.

Build and test from a source checkout with stable Rust and a C++ compiler:

```sh
python -m pip install 'maturin==1.15.0'
RUSTY_TILES_DISABLE_NATIVE_JPEG=1 maturin build --release --locked --out target/wheels
python scripts/test_python_wheel.py target/wheels/*.whl
```

The smoke test installs the wheel into a fresh virtual environment, then runs
this README example and the API tests with an empty executable `PATH`.

To test installation and conversion inside Blender's Python without changing
your Blender installation or user preferences:

```sh
python scripts/test_blender_wheel.py target/wheels/*.whl --blender /path/to/blender --report-json target/blender-acceptance.json
```

The runner supplies an isolated pip installer, and Blender's Python checks the
wheel compatibility and installs into a temporary directory. It then runs the
same API suite, including vectors, georeferenced point clouds and callbacks, with an
empty `PATH`. `--blender` defaults to `blender` on your `PATH`. The JSON report
records the actual Blender/Python versions, platform, wheel SHA-256 and test
counts. The clean-venv runner also accepts `--report-json`.
Both runners require fresh suite evidence matching the candidate wheel and
replace any requested prior report; current failure reports are preserved.

The **Wheel acceptance** workflow runs on relevant pull requests into `develop`,
can be dispatched manually once present on the default branch, and is reused by
the release workflow. It builds wheels for all five release platforms and tests
each artifact with CPython 3.10 and 3.14. Its mandatory official Blender matrix
installs the same wheels inside Blender 4.5.14 LTS on Linux x86_64, Windows x64,
Intel macOS and Apple Silicon macOS. Archives and SHA-256 checksums are pinned in
[blender_official.json](../../scripts/blender_official.json) from
[Blender's published checksums](https://download.blender.org/release/Blender4.5/blender-4.5.14.sha256).
Blender 4.5 LTS is supported until July 2027 and is the last official Intel macOS
release series. See [Blender's compatibility notes](https://developer.blender.org/docs/release_notes/compatibility/).

The downloader verifies each archive before extraction. The runner checks the
extracted executable's hash, actual Blender version and runtime architecture,
and records the archive identity alongside the wheel SHA-256 and API results.
CI uploads wheels and JSON evidence without publishing packages or a release.
The optional Linux distribution Blender check remains separate. Only completed
runs establish platform acceptance; configuring the matrix alone is not test
evidence. The Linux ARM64 wheel has CPython acceptance but no official Blender
4.5 bundle to test. Official Windows ARM64 Blender is outside the project's
current Windows x64 wheel targets.
These checks install local candidate wheels. Publishing to PyPI and verifying
`pip install rusty-tiles` from PyPI inside bundled Blender remain separate
[release acceptance requirements](https://github.com/BenDyson-Arch/rusty-tiles/issues/77).

To repeat an official check locally with host Python 3.12 or newer, select
`linux-x64`, `windows-x64`, `macos-x64` or `macos-arm64` for your host:

```sh
python scripts/download_blender.py --platform linux-x64 --directory /tmp/official-blender --report-json target/blender-distribution.json
python scripts/test_blender_wheel.py target/wheels/*.whl --blender /tmp/official-blender/blender-4.5.14-linux-x64/blender --distribution-json target/blender-distribution.json --report-json target/blender-acceptance.json
```

The downloader requires a new temporary directory and prints the executable
path, which is also recorded in its JSON. It copies the macOS application from
a read-only DMG mount and detaches it; it does not modify a system installation.
Updating the Blender pin requires reviewing the official checksum file,
updating the manifest and rerunning all four platform jobs.

`convert_to_implicit(input, output, *, force=False)` rewrites an eligible rusty-tiles explicit point-cloud/vector `.3tz` as implicit tiling, returning `ConversionResult`. It preserves original GLB/b3dm bytes through implicit tileset roots that own their replacement descendants. Retained originals and coordinate-template aliases typically double payload storage. Eligibility requires distinct regular midpoint cells, with only recorded rounding/minimum-thickness padding; many older binary trees and vector LOD chains need a fresh source conversion. Meshes, foreign and already implicit archives are refused. See [command eligibility and publication semantics](../../docs/CLI.md#convert-to-implicit).

`raster_tile_to_directory(input, output, *, zoom, x, y, force=False, callback=None)` exposes
the bounded raster directory request/error boundary. The standard wheel has no GDAL raster capability
and raises `UnsupportedError` without creating output or invoking callbacks;
invalid XYZ requests raise `InvalidRequestError` first. No native wheel or pip
extra is introduced. The native Rust/CLI profile and filesystem guarantees are
specified in `docs/architecture/d1-raster-contract.md` and
`docs/architecture/d1-directory-contract.md`.

`force=True` selects the same Replace policy as the Rust request and CLI. The
current final-path entry is held at publication, so replacement has an absent-path
interval and no snapshot or crash durability promise. Directory failures carry
`exception.recovery`, either `None` or a dictionary with `output` and
`previous_output` filesystem paths. If restoration fails, the original entry
remains at `previous_output`; resolve the current output path before restoration.
`retained_paths` names scratch separately, and `secondary_diagnostics` includes
the restoration failure. Native filesystem paths preserve non-UTF-8 Unix names.
Successful installation remains success if backup cleanup fails, with the backup
path in the result's existing `cleanup_diagnostics`. Standard wheels still report
`UnsupportedError` for either policy without changing existing output.

Point-cloud conversion requires an explicit `source_crs` keyword. Choose `"local"`
for XYZ metres, `"header"` for the LAS horizontal CRS, or an explicit horizontal
CRS definition. Geospatial input requires finite `height_offset` in metres to
ellipsoidal height; local input forbids it. The result is `PointCloudResult`,
with `output`, `archive`, required `report`, and `cleanup_diagnostics`. Callback
failures abort before publication and preserve an existing output.

Rust callers migrate to `PointCloudRequest::new(input, output, coordinates,
options).with_policy(policy)` and `point_cloud_to_archive(request, &run)`.
`PointCloudOptions` contains only tiling options; coordinate selection is typed,
and publication uses `OutputPolicy` rather than an options `force` field.
The old Rust `point_cloud_to_3tz` and reported overload are removed.
