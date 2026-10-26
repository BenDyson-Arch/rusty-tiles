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

For an admitted local metre/Y-up static `example.glb`, run:

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

All inputs and outputs are filesystem paths (`str` or `os.PathLike`).
Existing outputs are preserved unless `force=True`; publication uses the Rust
library's private staging and atomic archive replacement. The mesh,
point-cloud and vector entry points return their own typed results with
required reports and cleanup diagnostics, as described below.

| Function | Keyword arguments |
| --- | --- |
| `mesh_to_tileset(input, output, ...)` | `profile="balanced"`, `jobs=None`, `scratch_dir=None`, `memory_budget_bytes=None`, `anchor=None`, `orientation_xyzw=None`, `scene_offset=None`, `source_crs=None`, `source_axes=None`, `source_offset=None`, `source_offset_file=None`, `height_offset=None`, `force=False`, `callback=None`; returns `TilerResult` |
| `glb_to_3tz(input, output, ...)` | `anchor=None`, `orientation_xyzw=None`, `scene_offset=None`, `force=False`, `callback=None`; returns `ModelWrapResult` |
| `model_to_manifest(input, ...)` | Same rigid placement/policy/callback keywords; returns `ModelManifestResult` with fixed sibling output |
| `point_cloud_to_3tz(input, output, ...)` | `force=False`, required `source_crs`, `height_offset=None`, `max_points=8192` (at most 8192), `chunk_points=100000`, `metadata_attributes=False`, `callback=None` |
| `vector_to_3tz(input, output, ...)` | See vector options below; accepts GeoJSON and GeoPackage; returns `VectorResult` |
| `convert_to_3tz(input, output, ...)` | `force=False`, `callback=None`; returns `PackageResult` |
| `validate(input, *, lazy_payloads=False, max_volume=None, scratch_dir=None, expect_profile=None, callback=None)` | Returns the bounded [validation report](../../docs/VALIDATION.md), including completed checks, inspection gaps, `storedBytes`, the effective `limits` and their `limitDerivation`. `max_volume` maps limit keys to ints (opt-in volume caps such as `sourceArchiveBytes`, or replaced budgets such as `hierarchyVisits`); a bad key or value raises `InvalidRequestError` before any I/O. `scratch_dir` places scratch in an existing directory. `expect_profile="hlod"` requires the HLOD profile whatever the archive's labels say (every payload must carry source identity and every HLOD check runs; an archive that declares HLOD itself is held to it anyway); any other value raises `ValueError` before the archive is opened. `callback` receives progress events; raising from it, or Ctrl-C, stops the validation. A limit refusal's `ResourceLimitError.limit` is `{"limit", "value", "observed", "derivation"}` |

### Streamed local mesh tiler

`mesh_to_tileset(input, output, *, profile="balanced", jobs=None,
scratch_dir=None, memory_budget_bytes=None, anchor=None, orientation_xyzw=None,
scene_offset=None, source_crs=None, source_axes=None, source_offset=None, source_offset_file=None,
height_offset=None, force=False, callback=None)` is the single new public
source-mesh operation.
It runs capture, leaf payloads, coarse levels and implicit hierarchy delivery through
one core run and publication; leaf directories/manifests are internal. Output has
exact full-detail leaves under coarse parent tiles built as a classic octree level
of detail (HLOD). A coarse tile's geometric error is a sampled, measured distance to
the source, not a certificate; its triangles carry one representative
`source_primitive` ID while leaves keep exact provenance; and its textures are fresh
charts sampled from the source, an approximation with no appearance certificate. See
the [mesh guide](../../docs/MESH.md#coarse-levels-hlod).

It accepts the local static metre/Y-up/core PBR domain in the [mesh guide](../../docs/MESH.md),
with optional rigid WGS84 ellipsoidal anchor, active ENU quaternion and source
scene translation. Alternatively a declared [source CRS](../../docs/MESH.md#source-crs)
(`source_crs="EPSG:3857"`, `source_axes="y-up"` by default or `"xyz"`, `source_offset=(E, N[, A])`
added in double precision or a strict Metashape `source_offset_file`, and
`height_offset` metres defaulting to 0) is mapped at capture,
with the CLI's rules; it excludes the rigid placement keywords and `None` or
`"local"` keeps local input. There is no CRS inference, geoid height adjustment,
texture-format selector, explicit mode or coarse-tile tuning.
All four profile names (`fast`, default `balanced`, `quality`, `lossless`) are
provisional; constants will be calibrated together as HLOD acceptance
progresses. Base-colour and emissive pages, leaf and coarse, are KTX2 ETC1S
(quality 50 in `fast`, 90 in `balanced`), UASTC in `quality` and PNG in
`lossless`; KTX2/Basis and meshopt are in-process, and JPEG source decoding
remains supported.

A lossy texture page that misses a decoded-image gate is re-encoded at UASTC
level 2, then level 3, and, for pages sampled at level 0 only, delivered as an
exact PNG; a mip-sampled page still failing at level 3 fails the call. Escalated
pages are larger than the profile's own encoding (PNG fallback pages most of
all). `result.report["escalated_pages"]` (`uastc_level2`, `uastc_level3`),
`["lossless_fallback_pages"]`, `["escalation_diagnostics"]` (the first 32 pages:
leaf, first, depth, page, slot, sources, step, misses) and
`["escalation_diagnostics_omitted"]` describe them. Cancellation through the
callback is polled before every re-encode step. See the
[mesh guide](../../docs/MESH.md#profiles-and-resources).

`memory_budget_bytes` is the run's declared memory ceiling in bytes: a positive `int`
(a `bool`, float or string is a `TypeError`; zero, a negative number or a value above
2**64 - 1 is `InvalidRequestError`, raised before any file is read). `None` resolves
the lesser of 4 GiB and half the detected headroom (2 GiB if undetected). The budget
is never raised, a run that cannot fit under it raises `ResourceLimitError` before it
allocates, and the archive does not depend on it. `result.diagnostics["memory"]` holds
the resolved `budget` and each stage's minimum. See the
[memory budget](../../docs/MESH.md#memory-budget).

`jobs=None` means memory-aware auto. A positive integer is a maximum capped by
cores and the stage's memory admission. Unknown memory uses one worker; a
mandatory item that cannot fit is still refused. Booleans/nonintegers are
rejected. `scratch_dir` selects owned working data, while final archive staging
stays beside output. Output must be `.3tz`. `force=True` allows completed-file
replacement while preserving the original on precommit failure.

Frozen `TilerResult` exposes resolved `output`, `archive=True`, required `report`
and `diagnostics` dictionaries and `cleanup_diagnostics`. `report` is the exact
snake_case semantic `conversion.json` in the archive; `diagnostics` contains
requested/admitted stage workers, memory/scratch ownership, archive bytes and
peak held records outside the archive, so jobs and scratch do not change its
bytes. Placement/coordinate labels and the actual root transform are reported.
Declared memory accounting is not a measured RSS guarantee. The dictionaries
are not deeply immutable.

Required callbacks are precommit and share the existing fallible observer
contract; the original winning Python callback exception is preserved. Normal
request/source/I/O/unsupported/conflict/resource kinds use the existing typed
exceptions. Path binding precedes callbacks, including cwd changes. See the
[guide's error table and Cesium colour limitation](../../docs/MESH.md#reports-and-errors).
The standard wheel needs no external encoder or system geospatial dependency.

The legacy `mesh_to_3tz` and `mesh_local_to_3tz` functions, with their
`MeshResult` and `ConversionResult` classes and the `texture_format`, `explicit`
and `node_features` keywords, were removed in 0.4.0; see the
[migration notes](../../docs/MESH.md#migration-from-older-mesh-converters).
Exact wrapping through `glb_to_3tz` and sibling `model_to_manifest` remains a
separate product. `point_cloud_to_3tz` and `vector_to_3tz` write implicit
tiling only; their `explicit` keyword was removed in 0.4.0 and now raises
`TypeError`.
`metadata_attributes=True` adds point property attributes named
`vertex_classification`, `vertex_intensity` and `vertex_return_number`, retaining
the original LAS property tables for picking and table styling.

### Rigid placement

`mesh_to_tileset`, `glb_to_3tz` and `model_to_manifest` share the [F1c1 placement contract](../../docs/architecture/f1c1-contract.md), which defines the independently checked rigid frame and numerical domain. `anchor` is `(longitude_degrees, latitude_degrees,
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
meridian. Placement is an f64 rigid root transform. Offset does not restore a
projected E/N/A shift, infer source CRS, or correct a geoid.

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
(available cores), `reproducible=False`, `quantize=False`,
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

Callbacks receive dictionaries using the CLI's NDJSON event contract:
`{"event": "progress", "phase": ..., "done": ..., "total": ...}`,
`{"event": "warning", ...}` or `{"event": "log", "message": ...}`.
`total` can be `None`. Structured warnings preserve their detail fields.
`convert_to_3tz` also accepts a callback, with `encoding` and
`ready_to_publish` progress phases. Model wrapping/manifest publication also accepts callbacks with `model_capture`, `model_archive` (archive only) and `ready_to_publish` phases.
Without a callback, converters are silent.

Conversions release the GIL. A callback can run on a Rust worker thread;
Blender callers should queue events for their main thread before changing
Blender state.

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
maturin build --release --locked --out target/wheels
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

`convert_to_implicit` was removed in 0.4.0. To obtain implicit tiling for an
explicit point-cloud or vector archive, re-run `point_cloud_to_3tz` or
`vector_to_3tz` from the original source; implicit tiling is their only
output.

`raster_tile_to_directory(input, output, *, zoom, x, y, force=False, callback=None)` exposes
the bounded raster directory request/error boundary. The standard wheel has no GDAL raster capability
and raises `UnsupportedError` without creating output or invoking callbacks;
invalid XYZ requests raise `InvalidRequestError` first. No native wheel or pip
extra is introduced. R2 imagery pyramid conversion is a native Rust/CLI capability.
Python exports none of `raster_to_pyramid`, `RasterRequest` or `RasterDisplay`.
The native Rust/CLI profile and filesystem guarantees are
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
