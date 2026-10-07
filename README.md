# rusty-tiles

**Turn geospatial files into tiles you can view, share, and update.**

A local command-line tool for textured meshes, point clouds, vector features, imagery, and terrain. Generate 3D Tiles archives and map-ready raster outputs from your own files, then inspect them in the included Cesium preview.

[Get started](#get-started) · [Commands](#choose-a-command) · [Examples](#convert-your-data) · [Preview](#preview-your-results) · [Formats and limits](#formats-and-limits)

![Meshes, point clouds and vectors become 3D Tiles archives; imagery becomes COG and XYZ tiles; elevation rasters become quantized-mesh terrain.](docs/assets/readme-overview.svg)

**MIT licensed** · **Runs locally** · **Version 0.2.0**

[Release notes](CHANGELOG.md) · [Build status](https://github.com/BenDyson-Arch/rusty-tiles/actions/workflows/ci.yml) · [Report an issue](https://github.com/BenDyson-Arch/rusty-tiles/issues)

## Choose a command

| Your data | Command | Output |
| --- | --- | --- |
| Textured GLB / glTF meshes | `mesh-to-3tz` | Spatial 3D Tiles with mesh LOD, packaged as `.3tz` |
| LAS / LAZ point clouds | `point-cloud` | 3D Tiles with sampled parent LOD, full-detail leaves, and point properties |
| GeoPackage, GeoJSON, or Shapefile features | `vector` | Experimental glTF vector tiles with feature properties and geometry LOD, in `.3tz` |
| GeoTIFF or other GDAL-readable imagery | `raster` | Source-preserving COG + PNG XYZ tiles + TileJSON |
| Georeferenced elevation rasters | `terrain` | Prototype quantized-mesh terrain + coverage-aware height sidecars |

**Level of detail (LOD)** uses less detail at a distance and refines as you move closer. Mesh, point-cloud, and vector commands build their own hierarchies; their fidelity and format limits are explained [below](#formats-and-limits).

Already have a model or tileset you only need to package? See [archive utilities](#archive-utilities).

## Get started

### 1. Build the CLI

Install Rust with Cargo and a C++ compiler. `pkg-config` enables detection of optional native JPEG acceleration.

```sh
git clone --branch develop https://github.com/BenDyson-Arch/rusty-tiles.git
cd rusty-tiles
cargo install --path . --locked
rusty-tiles --help
```

`develop` contains the current development version. For a stable version, download a source archive from [Releases](https://github.com/BenDyson-Arch/rusty-tiles/releases) and run the same install command from its extracted directory. Check its release notes for available commands. Releases currently provide source archives rather than prebuilt binaries.

To build without installing, use `cargo build --release` and run `target/release/rusty-tiles` in place of `rusty-tiles` throughout these examples.

If `pkg-config` finds libjpeg-turbo, the build uses native JPEG acceleration and requires that shared library at runtime. Otherwise it uses the portable Rust encoder. Prefix the install command with `RUSTY_TILES_DISABLE_NATIVE_JPEG=1` to force a portable build.

#### Optional native geospatial build

The Python-to-Rust migration is tracked in [issues #56–63](https://github.com/BenDyson-Arch/rusty-tiles/issues/56). Its foundation is the optional `native-geospatial` feature:

```sh
cargo install --path . --locked --features native-geospatial
```

This feature requires GDAL >= 3.11 and PROJ >= 9.2 headers and libraries, their `pkg-config` files, and libclang for generating bindings against the installed GDAL headers. The same compatible native libraries must be available at runtime. Local PROJ database/grid data remains necessary; CRS operations disable networking and require non-ballpark, only-best transformations. The default build does not require or link GDAL.

The point-cloud converter uses native Rust LAS/LAZ decoding and tiling. The feature enables its geospatial CRS placement; local XYZ point clouds work in the default build. Terrain sampling and quantized-mesh encoding also run natively with this feature. Raster, vector and preview workflows still use the Python dependencies below until their individual migration issues are completed. CI exercises native builds with GDAL 3.11 and 3.13.

### 2. Install the dependencies for your data

| Commands | Runtime dependencies |
| --- | --- |
| Mesh conversion and archive utilities | No Python required |
| `point-cloud --sourceCrs local` | No Python or GDAL required |
| Geospatial `point-cloud` | Build with `native-geospatial`; native GDAL/PROJ and local CRS data |
| `terrain` | Build with `native-geospatial`; native GDAL/PROJ and local CRS data |
| `raster` | Python 3, NumPy, GDAL |
| `vector` | Python 3, NumPy, GDAL with GEOS |
| Local preview | Python 3; Node/npm to install the Cesium runtime |

The CLI embeds the remaining raster/vector conversion scripts; their Python libraries must be installed in the environment used by `python3`.

For the Python raster/vector commands, an existing GDAL Python environment is sufficient. If you use Conda, the following matches the Python/GDAL/NumPy versions used in CI:

```sh
conda create -n rusty-tiles -c conda-forge python=3.12 gdal=3.12 numpy=2 pip
conda activate rusty-tiles
python3 -c "from osgeo import gdal; import numpy; print(gdal.VersionInfo('--version'))"
```

The point-cloud Python requirements are for development fixtures and independent audits, rather than conversion.

### 3. Try the included example

This small, invented GeoJSON fixture contains points, a line, a polygon with a hole, and a vertical polygon. It requires the vector dependencies above.

```sh
mkdir -p output
rusty-tiles vector -i tests/fixtures/vector.geojson \
  -o output/example.3tz --maxFeatures 2
python3 -m zipfile -e output/example.3tz output/example
```

You now have an archive and an extracted tileset. Continue to [preview your results](#preview-your-results) to view it.

Check the installed environment with `rusty-tiles doctor --command vector` (or
select `point-cloud`, `raster`, or `terrain`). See the [dependency profiles](CONTRIBUTING.md#check-and-reproduce-the-python-environment)
for exact tested versions and `doctor --json` output.

## Convert your data

Every command uses `-i` for input and `-o` for output. Paths below are examples; replace them with your files. Run `rusty-tiles COMMAND --help` for all options.

### Textured meshes

```sh
rusty-tiles mesh-to-3tz -i model.glb -o output/model.3tz \
  --textureFormat jpeg
```

Large supported meshes are partitioned into full-detail leaves with simplified, textured parents. Small inputs can be wrapped without splitting. The default JPEG delivery retains full chroma resolution at quality 95; materials needing transparency use PNG.

| Texture option | Choose it for |
| --- | --- |
| `jpeg` — default | Fast delivery with high-quality, lossy colour |
| `webp` | Smaller downloads, with lossless alpha |
| `lossless` | Exact decoded leaf RGBA texels in PNG |
| `uastc` | GPU-compressed KTX2 textures; requires Basis Universal |

For georeferenced exports, check [mesh placement](#coordinates-and-height) before converting. Spatial splitting supports a limited set of static glTF content; use `glb-to-3tz` to wrap richer models unchanged.

<details>
<summary>Advanced mesh options</summary>

- `--maxTriangles` sets the leaf split threshold; the default is 20,000 triangles.
- `--tileSize` sets the ordinary leaf atlas edge, default 2048. Individual oversized charts retain their texels in larger atlases.
- `--noMeshopt` disables lossless meshopt compression. Neither mode quantizes leaf float32 geometry.
- `--maxTexelDensity` must remain `0` to preserve source-resolution leaf texels.
- `RAYON_NUM_THREADS` controls conversion parallelism. Allow scratch disk beside the output for staging geometry and textures.
- For KTX2, run `./scripts/build-basisu.sh`, then add `--textureFormat uastc --basisu target/tools/basisu`. The helper explicitly downloads and builds a pinned encoder; conversion itself does not download it. CMake, curl, and tar are needed for this build.

The CLI defaults to JPEG; the Rust library's `MeshTo3tzOptions::default()` uses lossless PNG. Coarse levels may retain child surfaces when safe simplification is unavailable. Fidelity limits are described [below](#formats-and-limits).

</details>

### Point clouds

Convert local, metre-based XYZ data without globe placement:

```sh
rusty-tiles point-cloud -i cloud.laz -o output/cloud.3tz \
  --sourceCrs local --maxPoints 50000 --chunkPoints 100000
```

For geospatial LAS/LAZ, build with `--features native-geospatial`, use `--sourceCrs header` or an explicit horizontal CRS such as `EPSG:32632`, and supply `--heightOffset`. Header CRS declarations may be WKT or GeoTIFF EPSG keys; custom GeoTIFF definitions require an explicit CRS override. A zero offset is appropriate only when source Z is already ellipsoidal metres. [Coordinate requirements](#coordinates-and-height) apply.

The reader streams points through disk-backed partitions. Coarse tiles use voxel samples; detailed leaves retain every point, including coincident points. Metadata keeps source coordinates, source record indices, original RGB, and supported numeric LAS fields for picking and inspection.

### Vector features

Select a GeoPackage layer and set tile budgets:

```sh
rusty-tiles vector -i mapping.gpkg -o output/mapping.3tz \
  --layer roads --maxFeatures 64 --maxVertices 65536 --maxBytes 4194304 \
  --lodTolerance 0.1 --lodLevels 3
```

Repeat `--layer` to include several layers, or use `--allLayers`. Single-layer inputs are selected automatically. Points, lines, polygons, and their Multi forms retain source feature/layer identity and supported scalar properties.

Parent geometry uses a metre-based simplification tolerance. Full-detail leaves preserve source geometry subject to reported float32 rounding. Oversized features may be fragmented; polygon fills and original outlines are retained without internal fragment outlines. Dense point-only layers provide a routing hierarchy rather than point sampling.

Use `--where "<OGR expression>"` to filter features before conversion. Select metadata with `--fields` or `--dropFields`; `--listFields json` preserves list properties as JSON text. NULL/empty geometries are omitted and reported. Invalid drawable features fail by default; `--skipInvalid` explicitly omits them with source identities and reasons in the reports.

See the [vector guide](docs/VECTOR.md) for CRS handling, metadata, budgets, polygon repair, and draft-format compatibility.

### Update vectors without re-encoding everything

Apply your GeoPackage changes with an external tool, then provide the previous archive:

```sh
rusty-tiles vector -i updated.gpkg -o output/updated.3tz \
  --layer roads --maxFeatures 64 --maxVertices 65536 --maxBytes 4194304 \
  --lodTolerance 0.1 --lodLevels 3 --reuseTileset output/mapping.3tz
```

Keep layer selection, CRS, budgets, and LOD options consistent with the previous build. Unchanged subtrees and content are reused; affected branches are rebuilt. Content filenames contain their SHA-256 hash, so unchanged payloads keep their URLs and bytes. Changing `--where` starts a fresh conversion and records why reuse was invalidated. Other incompatible settings, schemas, or CRS fail explicitly; remove `--reuseTileset` to build afresh.

The updated source is still scanned and the archive is repacked. Diff creation, application, and conflict resolution stay outside rusty-tiles. Compatibility tests cover upstream geodiff 2.3.0 and go-geodiff v0.4.3, including indexed GeoPackages. See [replacement behaviour](docs/VECTOR.md#replace-affected-content-after-edits) and [compatibility testing](CONTRIBUTING.md#geopackage-diff-compatibility).

### Raster imagery

Generate display tiles from byte imagery while retaining source pixels in a COG:

```sh
rusty-tiles raster -i orthophoto.tif -o output/imagery \
  --minZoom 10 --maxZoom 18 --display image
```

For numeric data, choose a band and an explicit display range:

```sh
rusty-tiles raster -i measurements.tif -o output/measurements \
  --minZoom 10 --maxZoom 18 --display gray --band 1 \
  --displayMin -10 --displayMax 10
```

Output includes `source.cog.tif`, `tilejson.json`, and `tiles/{z}/{x}/{y}.png`. Display tiles are a reprojected derivative; original values, bands, masks, and NoData are retained in the COG. Use `--alphaBand` only for an image band containing transparency; display alpha is combined with source coverage so masked pixels remain transparent. Jobs exceeding 100,000 imagery tiles fail with guidance to reduce the zoom range.

### Terrain

```sh
rusty-tiles terrain -i elevation.tif -o output/terrain \
  --maxZoom 14 --heightOffset 0 --fillHeight 0 --maxError 1
```

This example assumes source heights are already ellipsoidal metres and explicitly fills missing/outside coverage at zero metres. Supply values appropriate to your height reference and use case.

Terrain requires a build with `native-geospatial` and runs without Python. DEM sampling uses 64 MiB GDAL warp budgets and a 64 MiB process-wide raster block cache; each Rust encoder grid contains at most 129 × 129 samples, and conversion rejects pyramids exceeding 100,000 tiles before writing tiles. Rust encoders process batches of up to four grids while GDAL sampling stays on one thread. `RAYON_NUM_THREADS=1` selects serial encoding; full-grid conversion (`--maxError 0`) stays serial. Heights use the supplied offset; automatic GDAL vertical datum shifts are disabled.

Output includes `layer.json`, TMS `.terrain` tiles, and `.heights.json` sidecars. The sidecars preserve missing coverage as `null`, before fill, through a custom `heightOverlay` manifest entry. The encoder simplifies sampled grids using the Rust mesh machinery while retaining every original tile-edge vertex. `--maxError` defaults to 1 metre; `0` retains the full grid. The limit bounds both added elevation error and 3D surface displacement, including Earth’s curvature, relative to the decoded, quantized regular-grid mesh. Candidates that fail validation retain the full grid. `conversion.json` reports triangle/vertex counts and the measured added errors. DEM sampling and height quantization errors are separate; this remains a prototype.

## Preview your results

The included viewer runs locally using Cesium 1.143.0. It needs no ion token or external basemap and provides layer toggles, extent controls, and feature picking.

Install its runtime from the repository directory:

```sh
npm install --prefix target/preview-runtime --no-save --package-lock=false cesium@1.143.0
python3 scripts/preview.py \
  --cesium target/preview-runtime/node_modules/cesium/Build/Cesium \
  --annotations output/example
```

Open **[127.0.0.1:9227](http://127.0.0.1:9227/)** to view the example converted above.

For your own scene, extract mesh, point-cloud, and vector archives first:

```sh
python3 -m zipfile -e output/model.3tz output/model
python3 -m zipfile -e output/cloud.3tz output/cloud
python3 -m zipfile -e output/mapping.3tz output/mapping
python3 scripts/preview.py \
  --cesium target/preview-runtime/node_modules/cesium/Build/Cesium \
  --mesh output/model --point-cloud output/cloud --annotations output/mapping \
  --imagery output/imagery --terrain output/terrain
```

Use only the flags for datasets you converted; select at least one. Layers must share compatible placement and height references to align. The preview serves the selected directories and their contents, so select generated output directories.

**Using another viewer?** Serve an extracted `tileset.json` and its resources, or use a server that can expose archive members. Cesium does not open a `.3tz` file directly. Raster and terrain outputs use their own manifests rather than `tileset.json`.

## Coordinates and height

Check coordinates before combining layers. A file's CRS and height reference determine where its content belongs.

| Input | Placement rules |
| --- | --- |
| Meshes | Local model coordinates can be placed with `--cartographicPositionDegrees lon lat height`. The mesh CRS adapters support Metashape-style Y-up geographic and EPSG:3857 exports; they are not a general CRS reader. |
| Point clouds | `--sourceCrs local` means metre XYZ, Z up, without globe placement. Geospatial input requires a 2D horizontal CRS and an explicit offset from source Z metres to ellipsoidal height. |
| Vectors | Read the layer's declared CRS, override it with `--sourceCrs`, or use `local` metre XYZ. A 3D GeoPackage with only a horizontal CRS requires `--heightOffset`; 2D geospatial features use ellipsoidal height zero. |
| Terrain | Source heights must be metres. `--heightOffset` and `--fillHeight` are required; establish the height reference yourself. |

A constant height offset is not a spatial geoid transformation. Point-cloud/vector transformations use locally available PROJ operations; missing required grids fail rather than being downloaded automatically or replaced with ballpark operations.

<details>
<summary>Mesh export conventions and placement options</summary>

- `--sourceCrs geographic` expects positions in longitude / height / −latitude order.
- `--sourceCrs epsg:3857` expects easting / height / −northing. Restore a shifted export with `--sourceOffsetFile offset.txt` or `--sourceOffset E N A`.
- The default `auto` uses coordinate heuristics. Choose explicit settings when the export's reference is known; it does not read arbitrary CRS metadata.
- `--cartographicPositionDegrees lon lat [height]` places a model on the globe; `--rotationDegrees heading pitch roll` additionally sets orientation.
- Float32 degree coordinates may already have lost precision that conversion cannot recover.

</details>

## Formats and limits

| Area | What to expect |
| --- | --- |
| Mesh fidelity | Leaves retain original triangle membership and source-resolution texture charts. JPEG/WebP delivery and parent simplification are lossy; choose `lossless` for exact decoded leaf texels. Refinement estimates are not a universal visual-error guarantee. |
| Mesh input | Spatial splitting supports static triangle meshes, optional normals and UVs in `[0,1]`, and supported base-colour images. Animation, skinning, morph targets, extra vertex attributes, additional PBR texture channels, and source extensions are rejected when unsupported. Wrapping with `glb-to-3tz` preserves the original model. |
| Point-cloud fidelity | Every source point is retained in detailed leaves. Rendered positions use float32 with reported rounding; scalar numeric metadata retains source values. Scaled extra dimensions are stored as decoded float64. Waveforms, array extra dimensions, unknown VLR preservation, and compound vertical CRS are unsupported. |
| Vector compatibility | Uses draft glTF vector extensions requiring Cesium 1.142.0 or a checked newer release; see the [tested runtime matrix](docs/VECTOR.md#compatibility-and-validation). Fragmented polygon fills use standard glTF inside b3dm wrappers alongside vector outlines. This is experimental content, not a finalized 3D Tiles 2.0 format. |
| Vector limits | Unsupported complex fields must be excluded; list fields can be represented with `--listFields json`. Mixed property types and nullable booleans are unsupported. Geometry collections, curves, and measured geometries are unsupported. Buffered grid clipping and implicit tiling are not implemented. Detailed rules are in the [vector guide](docs/VECTOR.md). |
| Terrain | Sampled-grid prototype with border-preserving simplification; its added-error limit is relative to the quantized mesh, not a certified accuracy bound against the source DEM. No encoded normals. The coverage sidecars are a custom extension, not part of the quantized-mesh standard. |
| Outputs | `.3tz` uses indexed ZIP/ZIP64. Conversion reports record settings and relevant error/rounding information. Raster output does not include a bundled PMTiles writer. |

Existing output paths are rejected by default. Conversion and archive commands support `--force` for replacement after successful conversion. Archive replacement is atomic; directory replacement uses a backup/swap with rollback and a brief rename gap. Conversion publishes only successful outputs and removes staging files on ordinary success or error. Large jobs need scratch disk beside the output.

## Archive utilities

These commands package content without generating spatial LOD:

```sh
# Wrap an original model without rewriting its content.
rusty-tiles glb-to-3tz -i model.glb -o output/wrapped.3tz

# Create a tileset manifest for a model or model directory.
rusty-tiles createTilesetJson -i model.glb -o output/tileset.json

# Package an existing tileset directory or manifest.
rusty-tiles convert -i tileset-directory -o output/packed.3tz
```

## Contributing and validation

Start with [CONTRIBUTING.md](CONTRIBUTING.md) for bug reports, dependencies, checks, public dataset audits, and external diff compatibility tests. Contributions target `develop`; stable releases are promoted to `main`.

```sh
cargo fmt --check
cargo test --locked
python3 -m unittest discover -s tests -p 'test_*.py'
```

For CLI acceptance tests, build the executable and set `RUSTY_TILES_BIN` to its path. Public dataset and diff compatibility audits are documented in the contributor guide. Core tests use invented fixtures; private source data and credentials are not bundled.

<details>
<summary>Optional mesh fidelity and wrapper audits</summary>

Compare wrapper conventions with the pinned `3d-tiles-tools@0.5.4` oracle (requires npx and network access or a cached installation):

```sh
cargo test --test golden -- --ignored
```

Audit all detailed leaf triangles against a source model:

```sh
RUSTY_TILES_FIDELITY_SOURCE=/path/to/model.glb \
RUSTY_TILES_FIDELITY_ARCHIVE=/path/to/model.3tz \
RUSTY_TILES_FIDELITY_OFFSET=/path/to/offset.txt \
cargo test --release --test fidelity full_model_leaf_triangle_audit -- --ignored --nocapture
```

Omit `RUSTY_TILES_FIDELITY_OFFSET` for an unprojected local source. This checks complete leaf triangle position/winding membership; it is not an independent CRS oracle or a bound on parent error. Independent projection regressions and texture/metadata checks are part of the regular suite.

</details>

Licensed under [MIT](LICENSE).
