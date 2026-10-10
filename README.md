# rusty-tiles

**Turn geospatial files into tiles you can view, share and update.**

rusty-tiles converts textured meshes, LAS/LAZ point clouds and GeoJSON/GeoPackage vectors into 3D Tiles. The **standard package needs no GDAL, PROJ or GEOS installation**. It is available as a command-line tool, Rust library and Python wheel for scripts and Blender addons.

The optional **`native-geospatial` build** adds imagery, terrain, other OGR vector formats and native CRS operations. It needs GDAL/PROJ/GEOS, supplied by the native container or installed alongside a source build. Conversion runs offline in both builds; the CLI includes archive validation and a local Cesium preview.

![Meshes, point clouds and vectors become 3D Tiles archives. Imagery becomes COG and XYZ tiles. Elevation rasters become terrain.](docs/assets/readme-overview.svg)

**MIT licensed** · **Runs locally** · **0.4.0 development documentation** · [Release notes](CHANGELOG.md) · [Build status](https://github.com/BenDyson-Arch/rusty-tiles/actions/workflows/ci.yml) · [Report an issue](https://github.com/BenDyson-Arch/rusty-tiles/issues)

This branch describes the upcoming 0.4.0 release. Build `develop` to use its new capabilities; published 0.3.0 downloads have the feature set documented at that tag.

## Choose a build

Start with the **standard package** unless your input needs one of the native capabilities below. “Default” and “portable” in build instructions refer to this standard build.

| Capability | Standard package | `native-geospatial` build |
| --- | --- | --- |
| Mesh tiling, local LAS/LAZ, GeoJSON and GeoPackage | Included | Included |
| Georeferenced mesh, point-cloud and vector conversion | Verified grid-free CRS operations | Also supports eligible native GDAL/PROJ operations using local resources |
| Shapefile, PostGIS, GeoParquet and other OGR inputs | Not included | Available through installed GDAL drivers |
| Imagery and terrain conversion | Not included | Included |
| CLI packaging, validation and preview | Included | Included |
| How to install | Standard CLI downloads, Python wheels, or default Cargo build | Native container, or Cargo with `--features native-geospatial` and system libraries |

Python wheels contain the standard conversion APIs; there is no native-geospatial wheel or pip extra. Installing GDAL does not add capabilities to an existing standard binary or wheel. Use the native CLI/container or build the Rust library with the feature enabled. See the [installation guide](docs/INSTALL.md).

## Choose a command

| Your data | Command | Output | Needs `native-geospatial` |
| --- | --- | --- | --- |
| Textured GLB or glTF meshes | `mesh-to-3tz` | 3D Tiles with mesh level of detail, as `.3tz` | Only for CRS operations outside the grid-free tier |
| Local metre/Y-up GLB/glTF with core PBR and full-detail leaves | `mesh-local-to-3tz` | Explicit `.3tz`, optionally rigidly placed at a WGS84 anchor | No; bounded source profile |
| LAS or LAZ point clouds | `point-cloud` | 3D Tiles with sampled parents and full-detail leaves, as `.3tz` | Only for CRS operations outside the grid-free tier |
| GeoPackage or GeoJSON | `vector` | Experimental glTF vector tiles, as `.3tz` | Only for CRS operations outside the grid-free tier |
| Shapefile or other OGR vector formats | `vector` | Experimental glTF vector tiles, as `.3tz` | Yes |
| GeoTIFF or other GDAL imagery | `raster` | Source COG, PNG XYZ tiles and TileJSON | Yes |
| Elevation rasters | `terrain` | Bounded 3D Tiles terrain meshes | Yes |
| An existing model or tileset | `glb-to-3tz`, `createTilesetJson`, `convert` | `.3tz` or `tileset.json`, without new level of detail | No |

Use the [converter guide](docs/CONVERTERS.md) to choose an input path and build. Every option for every command is in the [command reference](docs/CLI.md).

`mesh-to-3tz`, `point-cloud` and `vector` write 3D Tiles 1.1 implicit tiling by default. Add `--explicit` for applications that inspect the explicit `children` hierarchy. Vector archives from earlier encoder versions need a fresh conversion after the 0.4 geometry fixes, including in explicit mode.

## Install

The [installation guide](docs/INSTALL.md) compares the released downloads, current source builds and Python wheels, and explains how to check the installed build.

The macOS/Linux installer and Windows ZIP below install the **standard CLI**, not the native-geospatial build. Standard vector conversion includes polygons with holes, repair, level of detail, metadata, compression and archive reuse. See the [vector input limits](docs/VECTOR.md#requirements) and [CRS limits](docs/FORMATS.md#point-clouds). For native capabilities, use the [native container](#run-the-native-geospatial-container) or the native Cargo command below.

### macOS / Linux

Run in a terminal:

```sh
curl -fsSL https://raw.githubusercontent.com/BenDyson-Arch/rusty-tiles/main/scripts/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"
rusty-tiles --version
```

This installs the latest release into `~/.local/bin` and verifies its checksum. No Rust or administrator privileges are needed. Add the `export PATH` line to your shell's startup file to keep it in new terminals.

Supports Intel/AMD and ARM64. Requires macOS 15+, or Linux with glibc 2.35+ and libstdc++.

### Windows

1. Open [the latest release](https://github.com/BenDyson-Arch/rusty-tiles/releases/latest) and download `rusty-tiles-VERSION-x86_64-pc-windows-msvc.zip` under **Assets**.
2. Extract the ZIP, then open PowerShell in the extracted folder.
3. Check the executable:

   ```powershell
   .\rusty-tiles.exe --version
   ```

To run `rusty-tiles` from any folder, add the extracted folder to your user `PATH` and reopen PowerShell. The Windows download is for x64.

### Run the native-geospatial container

The published image contains the **native-geospatial CLI** and its GDAL/PROJ/GEOS/SQLite libraries. No host geospatial libraries are needed. It is a Linux amd64 image; other architectures need Docker's amd64 emulation.

```sh
docker run --rm --platform linux/amd64 --network none \
  ghcr.io/bendyson-arch/rusty-tiles:latest doctor --json

docker run --rm --platform linux/amd64 --network none \
  -v "$PWD:/data" -w /data ghcr.io/bendyson-arch/rusty-tiles:latest \
  raster -i orthophoto.tif -o imagery --max-zoom 14
```

Replace the example input with a file in the mounted directory. Pin the image to `:v0.3.0` for a repeatable setup. The image contains no datasets, Cesium runtime, Basis Universal encoder or extra datum grids. Mount required grids and set `PROJ_DATA` to include them and `proj.db`.

With a rootful Docker daemon, add `--user "$(id -u):$(id -g)"` so new outputs belong to you. Rootless Docker already maps the container's root user to your account.

### Build with Cargo

You need current stable Rust with Cargo and a C++ compiler. For the **standard CLI**, with no system geospatial libraries:

```sh
git clone --branch develop https://github.com/BenDyson-Arch/rusty-tiles.git
cd rusty-tiles
cargo install --path . --locked
```

For the **native-geospatial CLI**, first install GDAL 3.12+, PROJ 9.2+, GEOS 3.10+, SQLite, development headers, `pkg-config` and libclang. Then, from the checkout:

```sh
LIBSQLITE3_SYS_USE_PKG_CONFIG=1 cargo install --path . --locked --features native-geospatial
rusty-tiles doctor --command raster --command terrain --command vector
```

The SQLite override makes Rust, GDAL and PROJ use the same system library. Leave it unset for standard builds and wheels, which bundle SQLite. Both CLI builds install the same `rusty-tiles` executable name; the native command replaces the standard installation at the same Cargo install root. Matching native libraries must also be present at runtime. CI tests GDAL 3.12 and 3.13. PROJ's database and required datum grids must be installed locally; conversion never downloads them. See [native setup](docs/INSTALL.md#native-geospatial-cli) for details.

`develop` holds the development version. For a stable version, use a source archive from [Releases](https://github.com/BenDyson-Arch/rusty-tiles/releases). To build without installing, use `cargo build --release` and run `target/release/rusty-tiles`.

Both builds use the portable Rust JPEG encoder unless `native-jpeg` is selected separately. That optional codec feature uses system libjpeg-turbo; it does not enable geospatial converters. See [optional build features](CONTRIBUTING.md#build).

If you already have `cargo-binstall`, run `cargo binstall --manifest-path Cargo.toml rusty-tiles` from a source checkout to download the prebuilt binary instead.

### Build the container

```sh
docker build --target runtime -t rusty-tiles:native .
docker run --rm --network none rusty-tiles:native doctor --json
```

The `Dockerfile` builds a Python-free image with GDAL 3.12.4 and PROJ 9.8.1.

## Quick start

This example works with the prebuilt binary or either Cargo build. It converts a tiny invented pyramid, places it near Brisbane, checks the archive and opens the local viewer. You need `curl`, `unzip` and Node.js with npm for the viewer setup.

### 1. Check readiness

```sh
rusty-tiles doctor --command mesh-to-3tz
```

Look for `mesh-to-3tz: ready`. The standard build can run this command without GDAL. `doctor` exits with code 4 if a selected command is missing a dependency.

### 2. Make tiles

Download the example (or copy `tests/fixtures/example.gltf` from a source checkout):

```sh
curl -fsSLo example.gltf https://raw.githubusercontent.com/BenDyson-Arch/rusty-tiles/v0.3.0/tests/fixtures/example.gltf
mkdir -p output
rusty-tiles mesh-to-3tz -i example.gltf -o output/example.3tz \
  --cartographic-position-degrees 153.02 -27.47 0
```

The converter prints what it wrote and the next command. Outputs are not overwritten; choose a new path or add `--force` to repeat the conversion.

### 3. Validate

```sh
rusty-tiles validate output/example.3tz
```

A successful check prints `Validated output/example.3tz: 1 tiles, 1 content references`.

### 4. Preview

Install Cesium once, extract the archive and serve the mesh:

```sh
npm install --prefix target/preview-runtime --no-save --package-lock=false cesium@1.146.0
unzip output/example.3tz -d output/example
rusty-tiles preview --cesium target/preview-runtime/node_modules/cesium/Build/Cesium \
  --mesh output/example
```

Open [http://127.0.0.1:9227/](http://127.0.0.1:9227/). Use **3D mesh extent** to frame the pyramid; press Ctrl+C to stop the server. After the one-time Cesium install, the viewer works offline and needs no ion token.

From a 0.4 source checkout, try the [invented vector fixture](tests/fixtures/vector.geojson) with the standard build too. This example needs 0.4; the published 0.3 standard binary does not include vector conversion:

```sh
rusty-tiles vector -i tests/fixtures/vector.geojson -o output/vector.3tz --max-features 2
rusty-tiles validate output/vector.3tz
```

## Convert your data

Every converter takes `-i` for input and `-o` for output. The file names below are placeholders for your own data.

| Data | Example | Build and details |
| --- | --- | --- |
| Mesh | `rusty-tiles mesh-to-3tz -i model.glb -o output/model.3tz` | Standard; [mesh placement](docs/FORMATS.md#meshes) may need native CRS support |
| Bounded core PBR mesh | `rusty-tiles mesh-local-to-3tz -i local.glb -o output/local.3tz --leaf-triangles 1000` | Standard; local source metres/Y-up and full-detail leaves; [explicit placement](docs/CLI.md#mesh-local-to-3tz) |
| Local point cloud | `rusty-tiles point-cloud -i cloud.laz -o output/cloud.3tz --source-crs local` | Standard; [point-cloud guide](docs/FORMATS.md#point-clouds) |
| Georeferenced point cloud | `rusty-tiles point-cloud -i cloud.laz -o output/cloud.3tz --source-crs header --height-offset 0` | Standard for grid-free CRS; otherwise native; [CRS limits](docs/FORMATS.md#point-clouds) |
| Vector layer | `rusty-tiles vector -i mapping.gpkg -o output/mapping.3tz --layer roads` | Standard for grid-free CRS; otherwise native; [vector guide](docs/VECTOR.md) |
| Imagery | `rusty-tiles raster -i orthophoto.tif -o output/imagery --min-zoom 10 --max-zoom 18` | **Native-geospatial only**; [imagery guide](docs/FORMATS.md#imagery) |
| Terrain | `rusty-tiles terrain -i elevation.tif -o output/terrain --cells-per-leaf 64 --height-offset 0 --fill-height 0` | **Native-geospatial only**; [terrain guide](docs/TERRAIN.md) |

A height offset of 0 is correct only when source heights are already ellipsoidal metres. Read [Coordinates and height](#coordinates-and-height) before you choose one.

Outputs are never overwritten by default. Add `--force` to replace an existing output after a successful conversion.

To update a vector archive after editing its source, add `--reuse-tileset` with the earlier archive. Unchanged source payloads keep their bytes; implicit display addresses are regenerated. See [Reuse after edits](docs/VECTOR.md#reuse-after-edits).

## Preview

`preview` serves your outputs and a Cesium runtime on `127.0.0.1:9227`. It needs no Cesium ion token and no external basemap. It offers layer toggles, extent buttons and feature picking.

Preview is included in both CLI builds. The standard CLI can serve imagery and terrain already produced by a native build; only their **conversion** requires native-geospatial. The Python wheel does not expose the preview server.

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

To use another viewer, serve the extracted `tileset.json` and its files. Raster output uses `tilejson.json`, and terrain output uses `tileset.json`.

## Validate and publish

Run `validate` on every `.3tz` before you publish it:

```sh
rusty-tiles validate output/example.3tz
```

It is read-only. The [validation profile](docs/VALIDATION.md) checks archive integrity, actual payload ranges and values, primitive indices, declared hierarchy bounds, references and recorded budgets under fixed limits. The report names completed checks and uninspected semantics; success does not certify decoded world-space bounds or source fidelity. Run external validators separately. Details are in the [command reference](docs/CLI.md#validate).

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
| Bounded mesh foundation | `mesh-local-to-3tz` accepts local source metres/Y-up. Omit `--anchor` for unplaced output, or give `--anchor lon lat ellipsoidal_height`, optional `--orientation-xyzw x y z w` and `--scene-offset x y z`. See the [placement contract](docs/architecture/f1c1-contract.md). |
| Legacy broader mesh | `mesh-to-3tz` uses `--cartographic-position-degrees` or a horizontal CRS with explicit axes/height offset. These distinct source-conversion routes remain advertised pending migration; bounded rigid placement does not certify or replace them. See [mesh placement](docs/FORMATS.md#mesh-placement). |
| Point cloud | `--source-crs local` means metre XYZ with Z up and no globe placement. Geospatial input needs a 2D horizontal CRS and `--height-offset`. |
| Vector | The layer's CRS is used unless `--source-crs` overrides it. 3D data with only a horizontal CRS needs `--height-offset`. 2D data sits at ellipsoidal height zero. |
| Terrain | Heights must be metres. `--height-offset` and `--fill-height` are required. |

A constant height offset is not a geoid transformation. Legacy general mesh, point-cloud and vector CRS routes use the bounded grid-free/native policies described in their guides. Switching builds does not remove converter-specific source, axis or height limits. Native operations use local PROJ resources only; missing required grids and ballpark operations are refused.

The bounded mesh foundation uses the independently checked F1c1 placement contract. Its anchor is WGS84 ellipsoidal height; its quaternion rotates ENU vectors in the cartographic frame, with supplied longitude defining the pole meridian. The scene offset is a post-node glTF Y-up metre translation before orientation. It is not a projected E/N/A shift or vertical datum correction. Placement uses an f64 root transform and keeps local GLB geometry and companions. Schema 6, profile `f1d1-root-proxy-gltf-v1`, reports source/output coordinates, normalized placement and the exact root matrix. The F1c2 candidate carries source node/mesh/primitive identity and exact triangle provenance in two labeled standard feature sets; see the [identity contract](docs/architecture/f1c2-contract.md). The wrapping candidate preserves admitted static source bytes through [typed F0 publication](docs/architecture/model-wrapping-contract.md). Broader legacy mesh APIs still require separate migration; this does not imply release acceptance.

## Library use

Python and Blender addons can call the standard converters directly. After the first PyPI publication, install a released wheel with:

```sh
python -m pip install rusty-tiles
```

For the current development API, [build and install a wheel from `develop`](docs/INSTALL.md#python-and-blender).

```python
import rusty_tiles

result = rusty_tiles.mesh_to_3tz(
    "example.gltf", "example.3tz", cartographic=(153.02, -27.47, 0.0)
)
assert rusty_tiles.validate(result.output)["ok"]
```

The wheels need CPython 3.10+ and contain the **standard conversion APIs**, with no GDAL, CLI subprocess or extra Python packages. They do not provide raster/terrain conversion, OGR-only inputs, native CRS fallback, `doctor` or `preview`. For those native conversion capabilities, use the native CLI/container or Rust library. See the [Python API and source build guide](bindings/python/README.md) for available functions and CRS limits.

Each converter has a `*_reported` function. It takes a `Reporter` for events and returns a `ConversionResult`.

```rust
use rusty_tiles::{tile::mesh_to_3tz_reported, MeshTo3tzOptions, Reporter};

let result = mesh_to_3tz_reported("model.glb".as_ref(), "model.3tz".as_ref(), &MeshTo3tzOptions::default(), &Reporter::silent())?;
println!("wrote {} (archive: {})", result.output.display(), result.archive);
```

The public [`metadata` module](src/metadata.rs) provides serde types for `EXT_mesh_features`, `EXT_structural_metadata` and implicit tile semantics. `MetadataGlb` appends aligned buffers, `encode_property_table` preserves scalar/string/boolean values, and `StructuralMetadata::attach_gltf` attaches tables to a `gltf_json::Root`.

`ConversionResult` carries `output`, `archive` and the published `conversion.json` as `report`. `Reporter` can be silent, human text on stderr, NDJSON on stderr, or a custom `EventSink`. The library's `MeshTo3tzOptions::default()` writes lossless PNG textures, while the CLI defaults to JPEG.

## Where to go next

- [Installation guide](docs/INSTALL.md): released downloads, development builds and Python wheels.
- [Converter guide](docs/CONVERTERS.md): choose a command, build and placement policy.
- [Command reference](docs/CLI.md): every option, exit codes, JSON output and progress events.
- [Mesh, point cloud and imagery guide](docs/FORMATS.md): fidelity, limits and advanced options.
- [Vector guide](docs/VECTOR.md): layers, budgets, reuse, styling and picking.
- [Terrain guide](docs/TERRAIN.md): height decisions, mesh queries and viewer setup.
- [Contributing](CONTRIBUTING.md): build, test and release.
- [Demo-data suite](CONTRIBUTING.md#demo-data-acceptance-and-benchmarks): opt-in native acceptance and benchmarks against the pinned public corpus.

Licensed under [MIT](LICENSE).

The [F1d1 approximation candidate](docs/architecture/mesh-approximation-contract.md) adds one explicitly requested certified root proxy over unchanged leaves. Paired `--root-proxy-triangles` and `--max-proxy-error-metres` arguments admit opaque, untextured, positions-only geometry, with region-level provenance for coarse picks. Full detail remains the default. Final-source evidence and review govern acceptance.
