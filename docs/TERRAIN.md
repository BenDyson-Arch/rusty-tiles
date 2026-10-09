# Terrain guide

This page explains what the `terrain` command produces and how to read it. It is for users who build terrain from elevation rasters and for developers who load that terrain in a viewer. Option defaults are in the [command reference](CLI.md#terrain).

**Terrain conversion is available only in the native-geospatial CLI/Rust build.** Use the [native container or build against system libraries](INSTALL.md#native-geospatial-cli). The standard CLI downloads and Python wheels cannot convert DEMs; installing GDAL alongside them does not enable this feature. Both CLI builds can preview existing terrain output.

`terrain` is a prototype. Native conversion runs offline and needs no Python.

## Convert an elevation raster

```sh
rusty-tiles terrain -i elevation.tif -o output/terrain \
  --max-zoom 14 --height-offset 0 --fill-height 0 --max-error 1
```

This example assumes the source heights are already ellipsoidal metres. It fills missing coverage at zero metres. Choose values that suit your own height reference.

For a small test DEM, the command prints a short summary and suggests a preview:

```text
terrain: wrote output/terrain (19 tiles)
next: rusty-tiles preview --cesium <Build/Cesium> --terrain output/terrain
```

## Input requirements

| Requirement | Why |
| --- | --- |
| One band with a declared CRS | Placement is never guessed |
| Heights in metres | The band unit must be empty or a metre spelling |
| No band scale or offset | Convert scaled values to real metres first |
| A finite height range | Empty or all-NoData rasters fail |
| A longitude span under 180 degrees, inside -180 to 180 | Split antimeridian or global DEMs first |
| At most 100,000 tiles in the pyramid | Reduce `--max-zoom` if the job is larger |

The command counts every tile before writing any. A job over the limit fails without output.

## Height decisions

You supply both height values. The command never infers a vertical datum.

- `--height-offset` is added to every source height to give ellipsoidal metres.
- `--fill-height` is the ellipsoidal height used for NoData and for areas outside the source.

A constant offset is not a geoid transformation. If your heights use a geoid or local datum, transform the DEM first. Automatic GDAL vertical shifts are disabled. Horizontal reprojection uses only the best local PROJ operation. Ballpark operations are refused, and missing grids fail the job.

## What the output contains

| Path | Contents |
| --- | --- |
| `layer.json` | Quantized-mesh 1.0 manifest in the TMS scheme and EPSG:4326 tiling |
| `{z}/{x}/{y}.terrain` | Quantized-mesh tiles |
| `{z}/{x}/{y}.heights.json` | Coverage sidecar for each tile |
| `conversion.json` | Settings, height range, simplification results and limitations |

All tiles share one height range. Adjacent tile edges therefore decode to identical heights. The range includes the fill height. `conversion.json` records the range and the height quantization step.

### Coverage sidecars

Each sidecar is a JSON object with `width`, `height` and `heights`. Rows run south to north. A sample is the source height plus the offset. Missing coverage is `null`, recorded before the fill height is applied. Use the sidecar when you need to tell real coverage from filled terrain.

`layer.json` points to the sidecars through a custom `heightOverlay` entry:

```json
"heightOverlay": {"version": 1, "grid": 65, "rowOrder": "south-to-north", "tiles": ["{z}/{x}/{y}.heights.json"]}
```

This entry is a rusty-tiles extension. It is not part of the quantized-mesh standard, and other viewers ignore it.

## Sampling grid

`--grid` sets samples per tile edge. It accepts 17, 33, 65 or 129, and the default is 65. Samples are taken with bilinear resampling in EPSG:4326. A larger grid keeps more detail and writes larger tiles.

## Simplification and `--max-error`

The encoder simplifies each sampled grid and keeps every original tile-edge vertex. `--max-error` is the largest added error allowed, in metres. The default is 1. A value of 0 keeps the full grid.

The limit covers two measurements. One is added elevation error. The other is 3D surface displacement, including the Earth's curvature. Both are measured against the decoded, quantized regular-grid mesh. A candidate that fails validation keeps its full grid instead.

`conversion.json.simplification` records the outcome:

| Field | Meaning |
| --- | --- |
| `maxErrorMetres` | The requested limit |
| `maxAddedHeightErrorMetres` | Measured added elevation error |
| `maxAddedSurfaceErrorMetres` | Measured added 3D surface error |
| `maxMeshoptEstimateMetres` | The simplifier's own estimate, not a measurement |
| `inputTriangles`, `outputTriangles` | Triangle counts before and after |
| `inputVertices`, `outputVertices` | Vertex counts before and after |

These limits exclude DEM sampling error and height quantization error. They are not a certified accuracy bound against the source surface. Use `--max-error 0` when the original grid matters more than smaller tiles.

## Memory and threads

GDAL sampling runs on one thread with a 64 MiB warp budget and a 64 MiB block cache. Encoding runs in batches of up to four grids. `RAYON_NUM_THREADS=1` makes encoding serial. `--max-error 0` is always serial.

## View the terrain

The output is a directory, not an archive. Load it in the included preview:

```sh
rusty-tiles preview --cesium target/preview-runtime/node_modules/cesium/Build/Cesium \
  --terrain output/terrain
```

In your own CesiumJS application, serve the directory and load it with `CesiumTerrainProvider.fromUrl`. Request no vertex normals, because the tiles contain none. Set `requestVertexNormals: false`.

`validate` does not check terrain directories yet.

## Limits

- Regular-grid sampling with border-preserving simplification only.
- No encoded normals and no water mask.
- No antimeridian or polar coverage without preprocessing.
- The coverage sidecar format is a rusty-tiles extension.
