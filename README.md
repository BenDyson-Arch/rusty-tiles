# rusty-tiles

**Turn geospatial files into tiles you can view, share and update.**

rusty-tiles is a local command-line tool and Rust library. It converts textured meshes, point clouds, vector features, imagery and elevation rasters into 3D Tiles, XYZ imagery and quantized-mesh terrain. It runs offline, checks its own readiness, and includes a local Cesium preview. This page is for anyone installing it for the first time.

![Meshes, point clouds and vectors become 3D Tiles archives. Imagery becomes COG and XYZ tiles. Elevation rasters become terrain. A first run is doctor, convert, validate, preview.](docs/assets/readme-overview.svg)

**MIT licensed** · **Runs locally** · **Version 0.3.0, in development** · [Release notes](CHANGELOG.md) · [Build status](https://github.com/BenDyson-Arch/rusty-tiles/actions/workflows/ci.yml) · [Report an issue](https://github.com/BenDyson-Arch/rusty-tiles/issues)

## Choose a command

| Your data | Command | Output | Needs `native-geospatial` |
| --- | --- | --- | --- |
| Textured GLB or glTF meshes | `mesh-to-3tz` | 3D Tiles with mesh level of detail, as `.3tz` | No |
| LAS or LAZ point clouds | `point-cloud` | 3D Tiles with sampled parents and full-detail leaves, as `.3tz` | Only for globe placement |
| GeoPackage, GeoJSON or Shapefile | `vector` | Experimental glTF vector tiles, as `.3tz` | Yes |
| GeoTIFF or other GDAL imagery | `raster` | Source COG, PNG XYZ tiles and TileJSON | Yes |
| Elevation rasters | `terrain` | Prototype quantized-mesh terrain with height sidecars | Yes |
| An existing model or tileset | `glb-to-3tz`, `createTilesetJson`, `convert` | `.3tz` or `tileset.json`, without new level of detail | No |

Every option for every command is in the [command reference](docs/CLI.md).

## Install

Releases provide source archives, not prebuilt binaries. Build from source with Cargo, or build the container image.

### Build with Cargo

You need Rust with Cargo and a C++ compiler. `pkg-config` lets the build find libjpeg-turbo for faster JPEG output.

```sh
git clone --branch develop https://github.com/BenDyson-Arch/rusty-tiles.git
cd rusty-tiles
cargo install --path . --locked --features native-geospatial
rusty-tiles --help
```

`develop` holds the current development version. For a stable version, download a source archive from [Releases](https://github.com/BenDyson-Arch/rusty-tiles/releases) and run the same install command in it. To build without installing, run `cargo build --release --features native-geospatial`. Then use `target/release/rusty-tiles` wherever these docs say `rusty-tiles`.

### Choose a build

| Build | Command flag | Commands available | System libraries |
| --- | --- | --- | --- |
| Default | none | Mesh, packaging, validate, preview, doctor, and `point-cloud --source-crs local` | None required |
| Native geospatial | `--features native-geospatial` | Everything, including georeferenced point clouds, vector, raster and terrain | GDAL 3.12+, PROJ 9.2+, GEOS 3.10+ for vector, SQLite |

The native build also needs GDAL and PROJ headers with their `pkg-config` files, SQLite development files, and libclang. The same library versions must be present at run time. CI tests GDAL 3.12 and 3.13. The PROJ database and any datum grids must be installed locally. Conversion never downloads them.

If libjpeg-turbo is found at build time, it is also needed at run time. Set `RUSTY_TILES_DISABLE_NATIVE_JPEG=1` while building to use the portable Rust encoder instead.

### Build the container

The `Dockerfile` builds a Python-free image with GDAL 3.12.4 and PROJ 9.8.1.

```sh
docker build --target runtime -t rusty-tiles:native .
docker run --rm --network none rusty-tiles:native doctor --json
```

The image has no Cesium runtime, datasets, Basis Universal encoder or datum grids. Mount grids yourself and set `PROJ_DATA` to include them and `proj.db`.

## Quick start

This converts the small invented GeoJSON file in the repository. It has a point, a line, a polygon with a hole and a vertical polygon. Run these commands from the repository directory with the native build installed.

### 1. Check readiness

```sh
rusty-tiles doctor --command vector
```

The report lists the linked libraries and each command's readiness. Versions vary by machine:

```text
GDAL: 3.13.3
PROJ: [9,8,1]
GEOS: [3,15,0]
...
vector: ready
```

A missing dependency makes `doctor` exit with code 4.

### 2. Convert

```sh
mkdir -p output
rusty-tiles vector -i tests/fixtures/vector.geojson -o output/example.3tz --max-features 2
```

Every converter ends with a short summary and the next command:

```text
vector: wrote output/example.3tz (4 features, 3 tiles)
reported: 2 geometry reports in geometry-reports.jsonl
next: rusty-tiles validate output/example.3tz
```

### 3. Validate

```sh
rusty-tiles validate output/example.3tz
```

```text
Validated output/example.3tz: 3 tiles, 3 content references
```

### 4. Preview

Install the Cesium runtime once, extract the archive, then start the viewer:

```sh
npm install --prefix target/preview-runtime --no-save --package-lock=false cesium@1.143.0
unzip output/example.3tz -d output/example
rusty-tiles preview --cesium target/preview-runtime/node_modules/cesium/Build/Cesium \
  --annotations output/example
```

Open [http://127.0.0.1:9227/](http://127.0.0.1:9227/) to see the features. Press Ctrl+C to stop.

## Convert your data

Every converter takes `-i` for input and `-o` for output. The file names below are placeholders for your own data.

| Data | Example | Details |
| --- | --- | --- |
| Mesh | `rusty-tiles mesh-to-3tz -i model.glb -o output/model.3tz` | [Mesh guide](docs/FORMATS.md#meshes) |
| Local point cloud | `rusty-tiles point-cloud -i cloud.laz -o output/cloud.3tz --source-crs local` | [Point-cloud guide](docs/FORMATS.md#point-clouds) |
| Georeferenced point cloud | `rusty-tiles point-cloud -i cloud.laz -o output/cloud.3tz --source-crs header --height-offset 0` | [Point-cloud guide](docs/FORMATS.md#point-clouds) |
| Vector layer | `rusty-tiles vector -i mapping.gpkg -o output/mapping.3tz --layer roads` | [Vector guide](docs/VECTOR.md) |
| Imagery | `rusty-tiles raster -i orthophoto.tif -o output/imagery --min-zoom 10 --max-zoom 18` | [Imagery guide](docs/FORMATS.md#imagery) |
| Terrain | `rusty-tiles terrain -i elevation.tif -o output/terrain --max-zoom 14 --height-offset 0 --fill-height 0` | [Terrain guide](docs/TERRAIN.md) |

A height offset of 0 is correct only when source heights are already ellipsoidal metres. Read [Coordinates and height](#coordinates-and-height) before you choose one.

Outputs are never overwritten by default. Add `--force` to replace an existing output after a successful conversion.

To update a vector archive after editing its source, add `--reuse-tileset` with the earlier archive. Unchanged content keeps its bytes and URLs. See [Reuse after edits](docs/VECTOR.md#reuse-after-edits).

## Preview

`preview` serves your outputs and a Cesium runtime on `127.0.0.1:9227`. It needs no Cesium ion token and no external basemap. It offers layer toggles, extent buttons and feature picking.

```sh
unzip output/model.3tz -d output/model
unzip output/cloud.3tz -d output/cloud
unzip output/mapping.3tz -d output/mapping
rusty-tiles preview --cesium target/preview-runtime/node_modules/cesium/Build/Cesium \
  --mesh output/model --point-cloud output/cloud --annotations output/mapping \
  --imagery output/imagery --terrain output/terrain
```

- Extract `.3tz` archives first. Cesium cannot open a `.3tz` directly.
- Select only the layers you have, and at least one.
- Layers align only if they share compatible placement and height references.
- `--host` and `--port` change the address. `--port 0 --json` prints the bound URL as JSON.

The server never lists directories and never downloads a runtime or data. It serves only the directories you select, so point it at generated output.

To use another viewer, serve the extracted `tileset.json` and its files. Raster output uses `tilejson.json`, and terrain output uses `layer.json`.

## Validate and publish

Run `validate` on every `.3tz` before you publish it:

```sh
rusty-tiles validate output/example.3tz
```

It is read-only. It checks the ZIP and 3TZ index, the tileset schema, bounds, geometric error, references, content hashes and recorded budgets. Add `--external-validator` to also run a locally installed official `3d-tiles-validator`. Details are in the [command reference](docs/CLI.md#validate).

`validate` checks `.3tz` archives only. Raster and terrain directories are not validated yet.

Publication is safe by design.

- A failed conversion publishes nothing and removes its work directory.
- Each job works in a private `.tiles-work-*` directory beside the output. Allow scratch disk there.
- Archive replacement with `--force` is atomic.
- Directory replacement swaps in the new output and rolls back on failure. There is a brief rename gap. If the rollback itself fails, the message names the kept backup.

## Coordinates and height

A file's CRS and height reference decide where its content lands. rusty-tiles never guesses a vertical datum.

| Input | Rule |
| --- | --- |
| Mesh | Local model coordinates are placed with `--cartographic-position-degrees lon lat height`. Metashape-style geographic and EPSG:3857 exports are also read. See [mesh placement](docs/FORMATS.md#mesh-placement). |
| Point cloud | `--source-crs local` means metre XYZ with Z up and no globe placement. Geospatial input needs a 2D horizontal CRS and `--height-offset`. |
| Vector | The layer's CRS is used unless `--source-crs` overrides it. 3D data with only a horizontal CRS needs `--height-offset`. 2D data sits at ellipsoidal height zero. |
| Terrain | Heights must be metres. `--height-offset` and `--fill-height` are required. |

A constant height offset is not a geoid transformation. Transformations use only local PROJ operations. A missing grid fails the job, and ballpark operations are refused.

## Library use

Each converter has a `*_reported` function. It takes a `Reporter` for events and returns a `ConversionResult`.

```rust
use rusty_tiles::{tile::mesh_to_3tz_reported, MeshTo3tzOptions, Reporter};

let result = mesh_to_3tz_reported("model.glb".as_ref(), "model.3tz".as_ref(), &MeshTo3tzOptions::default(), &Reporter::silent())?;
println!("wrote {} (archive: {})", result.output.display(), result.archive);
```

`ConversionResult` carries `output`, `archive` and the published `conversion.json` as `report`. `Reporter` can be silent, human text on stderr, NDJSON on stderr, or a custom `EventSink`. The library's `MeshTo3tzOptions::default()` writes lossless PNG textures, while the CLI defaults to JPEG.

## Where to go next

- [Command reference](docs/CLI.md): every option, exit codes, JSON output and progress events.
- [Mesh, point cloud and imagery guide](docs/FORMATS.md): fidelity, limits and advanced options.
- [Vector guide](docs/VECTOR.md): layers, budgets, reuse, styling and picking.
- [Terrain guide](docs/TERRAIN.md): height decisions, sidecars and viewer setup.
- [Contributing](CONTRIBUTING.md): build, test and release.

Licensed under [MIT](LICENSE).
