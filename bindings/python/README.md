# rusty-tiles for Python

Convert meshes, local LAS/LAZ point clouds and existing tilesets directly from
Python, including Blender's bundled Python. The wheels contain the portable
default Rust build: no CLI subprocess, GDAL, Rust compiler or extra Python
packages are needed at runtime.

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
| `mesh_to_3tz(input, output, ...)` | `cartographic=None`, `rotation=None`, `force=False`, `max_triangles=20000`, `max_bytes=204800`, `tile_size=2048`, `texture_format="lossless"`, `source_crs="auto"`, `source_offset=None`, `meshopt=True`, `explicit=False`, `node_features=False`, `callback=None` |
| `glb_to_3tz(input, output, ...)` | `cartographic=None`, `rotation=None`, `force=False` |
| `point_cloud_to_3tz(input, output, ...)` | `force=False`, `max_points=50000`, `chunk_points=100000`, `explicit=False`, `metadata_attributes=False`, `callback=None` |
| `convert_to_3tz(input, output, ...)` | `force=False` |
| `validate(input)` | Returns the validation dictionary; always uses the bundled validator |

`cartographic` is `(longitude_degrees, latitude_degrees, height_metres)`;
`rotation` is `(heading_degrees, pitch_degrees, roll_degrees)`. Mesh
`source_crs` accepts `auto`, `geographic` or `epsg:3857`;
`source_offset` is `(easting, northing, height)` in metres. Mesh textures use
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
Point-cloud input is LAS/LAZ in local XYZ metres with Z up; this API does
not apply CRS transformations or height offsets. Text `.xyz` files and
in-memory buffers are not supported in this release.

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
