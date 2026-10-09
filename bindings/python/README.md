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
expose `doctor`, `preview` or `createTilesetJson`.

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
Conversions return `ConversionResult` with `output` (a `pathlib.Path`), `archive`
(a boolean) and `report` (a dictionary or `None`). Existing outputs are
preserved unless `force=True`; publication uses the Rust library's private
staging and atomic archive replacement.

| Function | Keyword arguments |
| --- | --- |
| `mesh_to_3tz(input, output, ...)` | `cartographic=None`, `rotation=None`, `force=False`, `max_triangles=20000`, `max_bytes=204800`, `tile_size=2048`, `texture_format="lossless"`, `source_crs="auto"`, `source_offset=None`, `meshopt=True`, `explicit=False`, `node_features=False`, `source_axes=None`, `height_offset=None`, `callback=None` |
| `glb_to_3tz(input, output, ...)` | `cartographic=None`, `rotation=None`, `force=False` |
| `point_cloud_to_3tz(input, output, ...)` | `force=False`, `source_crs="local"`, `height_offset=None`, `max_points=50000`, `chunk_points=100000`, `explicit=False`, `metadata_attributes=False`, `callback=None` |
| `vector_to_3tz(input, output, ...)` | See vector options below; accepts GeoJSON and GeoPackage |
| `convert_to_3tz(input, output, ...)` | `force=False` |
| `convert_to_implicit(input, output, ...)` | `force=False` |
| `validate(input)` | Returns the validation dictionary; always uses the bundled validator |

`cartographic` is `(longitude_degrees, latitude_degrees, height_metres)`;
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

`glb_to_3tz` wraps GLB/glTF without making new levels of detail.
`convert_to_3tz` packs a tileset directory or a `tileset.json` path.
Point-cloud input defaults to local XYZ metres with Z up. For globe placement,
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
dynamic or unknown datum definitions with `EnvironmentError` naming the
native build. See the [CRS limits](../../docs/FORMATS.md#point-clouds).
Text `.xyz` files and in-memory buffers are not supported in this release.

Vector conversion uses the same disk-backed tiling, metadata, LOD and encoding
pipeline as the CLI. GeoJSON defaults to WGS84 longitude/latitude; GeoPackage
uses its layer CRS. Use `source_crs="local"` for metre XYZ coordinates. A 3D
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
See [vector conversion](../../docs/VECTOR.md) for geometry, filtering, repair
and reuse limits. Close or checkpoint a GeoPackage with unmerged WAL edits
before conversion.

Callbacks receive dictionaries using the Rust reporter's event contract:
`{"event": "progress", "phase": ..., "done": ..., "total": ...}`,
`{"event": "warning", ...}` or `{"event": "log", "message": ...}`.
`total` can be `None`. Structured warnings preserve their detail fields.
The other entry points have no reporter events in the Rust API.
Without a callback, converters are silent.

Conversions release the GIL. A callback can run on a Rust worker thread;
Blender callers should queue events for their main thread before changing
Blender state. The first callback exception is saved, later callbacks are
skipped, and the original exception is raised after Rust finishes. A callback
exception does not cancel conversion, so the output may already exist.

Rust failures raise subclasses of `TilesError`: `DataError`,
`EnvironmentError`, `OutputExistsError`, `TilesIOError` or `UnsupportedError`.
Invalid Python arguments raise `TypeError` or `ValueError`. Original callback
exceptions are preserved.

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
