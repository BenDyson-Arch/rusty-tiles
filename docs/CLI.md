# Command reference

This page lists every `rusty-tiles` subcommand and option. It is for users who script conversions and for programs that call the CLI. For a guided first run, start with the [README quick start](../README.md#quick-start).

Every value here comes from `rusty-tiles <command> --help` for version 0.3.0. Run that command to check your installed build.

## Conventions

Options have a camelCase spelling, such as `--sourceCrs`. Each multi-word option also accepts a kebab-case alias, such as `--source-crs`. Both spellings behave the same. The examples in these docs use the kebab-case alias.

Three subcommands also have an alias.

| Command | Alias |
| --- | --- |
| `createTilesetJson` | `create-tileset-json` |
| `glb-to-3tz` | `glbTo3tz` |
| `mesh-to-3tz` | `meshTo3tz` |

Every converter takes the same three file options.

| Option | Alias | Default | Meaning |
| --- | --- | --- | --- |
| `-i`, `--input` | | required | Source file or directory |
| `-o`, `--output` | | required | Output path |
| `-f`, `--force` | | off | Replace an existing output after a successful conversion |

Two global options work before or after the subcommand.

| Option | Default | Meaning |
| --- | --- | --- |
| `--json` | off | Print one machine-readable result on stdout. Diagnostics stay on stderr. |
| `--progress json` | off | Print newline-delimited JSON events on stderr. |

See [machine output](#machine-output) for both formats.

`mesh-to-3tz`, `point-cloud` and `vector` produce 3D Tiles 1.1 implicit tiling by default. Their `--explicit` flag preserves the earlier explicit output and partitioning. Packaging commands keep their existing behaviour.

## Converters

### mesh-to-3tz

Splits a textured GLB or glTF into spatial 3D Tiles with mesh level of detail. Small inputs are wrapped without splitting.

| Option | Alias | Default | Meaning |
| --- | --- | --- | --- |
| `--textureFormat` | `--texture-format` | `jpeg` | `jpeg`, `webp`, `uastc` or `lossless`, which is exact PNG |
| `--explicit` | | off | Keep the earlier explicit hierarchy and median partition |
| `--basisu` | | `basisu` | Basis Universal encoder executable, needed for `uastc` |
| `--maxTriangles` | `--max-triangles` | `20000` | Stop splitting a node at this many triangles |
| `--maxBytes` | `--max-bytes` | `204800` | Source files at or below this size are wrapped, not split |
| `--tileSize` | `--tile-size` | `2048` | Leaf atlas edge in pixels. Parent atlases are capped at 1024. |
| `--maxTexelDensity` | `--max-texel-density` | `0` | Must be 0, so leaf texels keep source resolution |
| `--noMeshopt` | `--no-meshopt` | off | Disable lossless meshopt compression |
| `--sourceCrs` | `--source-crs` | `auto` | Position convention: `auto`, `geographic` or `epsg:3857` |
| `--sourceOffset E N [A]` | `--source-offset` | none | Metashape shift in metres, added in double precision |
| `--sourceOffsetFile` | `--source-offset-file` | none | Metashape `offset.txt` with `E:`, `N:` and `A:` lines |
| `--cartographicPositionDegrees lon lat [height]` | `--cartographic-position-degrees` | none | Place a local model on the globe |
| `--rotationDegrees heading pitch roll` | `--rotation-degrees` | none | Orient a placed model |

Mesh placement rules are in [Coordinates and height](../README.md#coordinates-and-height).

### point-cloud

Tiles LAS or LAZ points into 3D Tiles with sampled parents and full-detail leaves.

| Option | Alias | Default | Meaning |
| --- | --- | --- | --- |
| `--sourceCrs` | `--source-crs` | required | `local`, `header`, or a 2D horizontal CRS such as `EPSG:32632` |
| `--explicit` | | off | Keep the earlier explicit hierarchy and binary partition |
| `--heightOffset` | `--height-offset` | none | Metres added to source Z to give ellipsoidal height. Required for geospatial input. |
| `--maxPoints` | `--max-points` | `50000` | Maximum points per tile, at least 1 |
| `--chunkPoints` | `--chunk-points` | `100000` | Points read per source chunk, at least 1 |

### vector

Converts OGR layers into draft glTF vector content in a `.3tz`. Behaviour and limits are in the [vector guide](VECTOR.md).

| Option | Alias | Default | Meaning |
| --- | --- | --- | --- |
| `--layer` | | automatic for one layer | Select a spatial layer. Repeat for several. |
| `--explicit` | | off | Keep the earlier explicit hierarchy and output bytes |
| `--allLayers` | `--all-layers` | off | Include every spatial layer |
| `--sourceCrs` | `--source-crs` | layer CRS | Override the input CRS, or `local` for metre XYZ |
| `--heightOffset` | `--height-offset` | none | Metres added to source heights to give ellipsoidal height |
| `--where` | | none | OGR attribute filter applied to every selected layer |
| `--fields` | | all | Comma-separated fields to include |
| `--dropFields` | `--drop-fields` | none | Comma-separated fields to exclude |
| `--listFields` | `--list-fields` | `error` | `error` rejects list fields. `json` stores them as JSON text. |
| `--skipInvalid` | `--skip-invalid` | off | Omit unconvertible features and report each one |
| `--maxFeatures` | `--max-features` | `64` | Feature fragments per leaf tile |
| `--maxParentFeatures` | `--max-parent-features` | `4096` | Feature fragments per parent tile |
| `--maxVertices` | `--max-vertices` | `65536` | Encoded POSITION vertices per tile, at least 4 |
| `--maxBytes` | `--max-bytes` | `4194304` | Encoded bytes per tile, at least 4096 |
| `--maxTiles` | `--max-tiles` | `100000` | Tiles in the hierarchy |
| `--maxSourceVertices` | `--max-source-vertices` | `1000000` | Coordinates in one source feature |
| `--lodTolerance` | `--lod-tolerance` | `0.1` | Base simplification tolerance in metres. It doubles at each coarser level. |
| `--lodLevels` | `--lod-levels` | `3` | Coarse levels above each leaf, from 1 to 16 |
| `--repair` | | off | Repair invalid polygon outlines and report each repair |
| `--ambiguousOutlines` | `--ambiguous-outlines` | off | Keep ambiguous filled polygons as their source 3D outlines |
| `--parentRepair` | `--parent-repair` | off | Allow reported outline stand-ins in parent tiles |
| `--aggregatePoints` | `--aggregate-points` | off | Replace dense points in parents with per-layer counts |
| `--quantize` | | off | Lossy 16-bit positions, with the error reported |
| `--meshopt` | | off | Lossless `EXT_meshopt_compression` of accessor buffers |
| `--reuseTileset` | `--reuse-tileset` | none | Reuse unchanged content from a compatible earlier archive |
| `--jobs` | | available cores | Maximum encoding workers |
| `--reproducible` | | off | Omit timing diagnostics so archives are byte-identical |

### raster

Writes a source-preserving COG and a PNG XYZ display pyramid.

| Option | Alias | Default | Meaning |
| --- | --- | --- | --- |
| `--maxZoom` | `--max-zoom` | required | Finest zoom level, from 0 to 24 |
| `--minZoom` | `--min-zoom` | `0` | Coarsest zoom level, from 0 to `--maxZoom` |
| `--display` | | `image` | `image` uses RGB or RGBA bands. `gray` stretches one band. |
| `--band` | | `1` | Band used by `gray` display |
| `--alphaBand` | `--alpha-band` | `0` | Alpha band for `image` or `gray` display, with opacity values 0–255. 0 uses the source mask or NoData. |
| `--displayMin` | `--display-min` | none | Value shown as black in `gray` display |
| `--displayMax` | `--display-max` | none | Value shown as white in `gray` display |

### terrain

Writes a quantized-mesh terrain directory with height sidecars. See the [terrain guide](TERRAIN.md).

| Option | Alias | Default | Meaning |
| --- | --- | --- | --- |
| `--maxZoom` | `--max-zoom` | required | Finest zoom level, from 0 to 24 |
| `--heightOffset` | `--height-offset` | required | Metres added to DEM heights to give ellipsoidal height. Never inferred. |
| `--fillHeight` | `--fill-height` | required | Ellipsoidal height used for NoData and outside coverage |
| `--grid` | | `65` | Samples per tile edge: 17, 33, 65 or 129 |
| `--maxError` | `--max-error` | `1` | Maximum added simplification error in metres. 0 keeps the full grid. |

## Packaging commands

These commands package existing content without building spatial level of detail.

| Command | Input | Output | Extra options |
| --- | --- | --- | --- |
| `glb-to-3tz` | GLB or glTF file, including referenced local resources | `.3tz` archive | `--cartographicPositionDegrees`, `--rotationDegrees` |
| `createTilesetJson` | GLB or glTF file or directory | `tileset.json` | `--cartographicPositionDegrees`, `--rotationDegrees` |
| `convert` | Tileset directory or `tileset.json` | `.3tz` archive | none |

`createTilesetJson` and `convert` match the argument style of `3d-tiles-tools@0.5.4`.

## Checking and viewing

### validate

```sh
rusty-tiles validate output/example.3tz
```

| Argument or option | Default | Meaning |
| --- | --- | --- |
| `INPUT` | required | The `.3tz` archive to check |
| `--external-validator PATH` | none | Also run a locally installed official `3d-tiles-validator` |

`validate` checks `.3tz` archives only. A directory or other file fails with exit code 3 and says so. Raster and terrain directories are not validated yet.

Validation is read-only. It runs these checks in order:

- ZIP CRCs and the complete 3TZ index.
- The bundled 3D Tiles tileset schema.
- Binary implicit subtree headers, aligned buffer views, availability, parent links and native semantic tile metadata.
- Child bounds inside parent bounds, and geometric error that never increases toward the leaves.
- Local content and resource references, including `schemaUri` files.
- Hash-named payload checksums and the build-state checksum.
- Unused entries.
- Recorded budgets for encoded bytes, vertices, points and tiles.

Equal geometric errors are allowed for routing nodes and for parents whose child error is the larger bound. Boxes and spheres use relative tile transforms. Region containment handles the antimeridian. Sphere containment under nonuniform scale uses a conservative bound. An external tileset is checked at each placement, with the referring tile's bounds, error and depth. Only references on the active path count as cycles.

The built-in check covers explicit and native implicit, self-contained archives of GLB, glTF, b3dm and external tileset JSON. Native implicit boundaries are expanded with their actual boxes and errors for the same containment and budget checks. These cases are reported as unsupported rather than passed:

- Mixed region and Cartesian bounds.
- Implicit regions, external subtree buffers, or foreign tile metadata layouts.
- Remote or percent-encoded URIs.
- Other content formats.

Check unsupported cases and content extensions with the official [3d-tiles-validator](https://github.com/CesiumGS/3d-tiles-validator). `--external-validator` runs a local install after the built-in checks pass. It passes `--tilesetFile` and a temporary `--reportFile`, and keeps the tool's log on stderr. Reported errors fail the command even if the tool exits successfully. rusty-tiles never installs the validator, extracts the archive or changes it.

The schema bundle comes from the Cesium GS 3D Tiles specification at commit `4d781014b52294759834018a931223b98ac1ce47`. Its relative references were rewritten to local `$defs`. Attribution and the CC BY 4.0 notice are in [`docs/schema/LICENSE.adoc`](schema/LICENSE.adoc). Schema loading makes no network or file reference lookups.

### preview

```sh
rusty-tiles preview --cesium target/preview-runtime/node_modules/cesium/Build/Cesium \
  --annotations output/example
```

| Option | Default | Serves |
| --- | --- | --- |
| `--cesium` | required | A Cesium 1.143.0 `Build/Cesium` directory containing `Cesium.js` |
| `--host` | `127.0.0.1` | Listen address |
| `--port` | `9227` | Listen port. 0 picks a free port. |
| `--mesh` | none | Extracted mesh directory with `tileset.json` |
| `--point-cloud` | none | Extracted point-cloud directory with `tileset.json` |
| `--annotations` | none | Extracted vector directory with `tileset.json` |
| `--imagery` | none | Raster output directory with `tilejson.json` |
| `--terrain` | none | Terrain output directory with `layer.json` |

Select at least one layer.

### doctor

```sh
rusty-tiles doctor --command vector --command terrain
```

| Option | Default | Meaning |
| --- | --- | --- |
| `--command` | all | Check only these commands. Repeat to select several. Aliases are accepted. |
| `--cesium DIR` | `target/preview-runtime/node_modules/cesium/Build/Cesium` | Where to look for a Cesium runtime. This check is informational only. |

`doctor` reports linked GDAL, PROJ and GEOS versions. It also reports readiness for the selected commands, the PROJ search paths and the local grid count. `--command` limits the command inventory in both human and JSON output. Without a filter it lists every command. It never installs anything or fetches grids. A listed grid does not prove a height operation is available. Conversion checks the exact operation offline.

## Exit codes

Programs should test the exit code or `error.code`, not the message text.

| Exit code | `error.code` | Meaning |
| --- | --- | --- |
| 0 | | Success, including `--help` and `--version` |
| 1 | `io` | Input or output failure |
| 2 | `usage` | Unknown option, missing argument or unimplemented feature |
| 3 | `data` | Invalid input, invalid option value or failed validation |
| 4 | `environment` | Missing native capability, PROJ database or strict CRS operation |
| 5 | `output_conflict` | The output exists and `--force` was not given |

An out-of-range option value names the flag and its range. For example, `--lod-levels 40` prints `--lodLevels must be between 1 and 16, got 40` and exits with code 3.

## Machine output

### Human summary

Without `--json` or `--progress json`, a successful converter prints one to three lines on stderr. They name the output and key counts, any warnings, and the next command.

```text
vector: wrote output/example.3tz (4 features, 3 tiles)
reported: 2 geometry reports in geometry-reports.jsonl
next: rusty-tiles validate output/example.3tz
```

Raster and terrain suggest a `preview` command instead. `createTilesetJson` suggests `convert`.

### JSON result

With `--json`, the command prints exactly one JSON object on stdout. A successful conversion has this shape:

```json
{
  "ok": true,
  "output": "output/example.3tz",
  "counts": {"features": 4, "tiles": 3, "leafTiles": 2, "skippedFeatures": 0, "geometryReportCount": 2},
  "settings": {"lodLevels": 3, "lodToleranceMetres": 0.1},
  "skippedFeatures": 0,
  "reuse": {"previousTileset": false, "reusedContents": 0, "publishedContents": 3},
  "conversionReport": {"archive": "output/example.3tz", "entry": "conversion.json"}
}
```

The example is shortened. `counts` holds only counts, such as `points`, `tiles` and `features`. `settings` holds every other top-level number from `conversion.json`, such as `heightOffset`, `grid` or `lodLevels`. For a directory output, `conversionReport` is `{"path": ".../conversion.json"}`. Commands without a conversion report return empty `counts` and `settings` and null report fields.

A failure has this shape and the matching exit code:

```json
{"ok": false, "error": {"code": "data", "message": "input not found: nope.geojson"}, "exitCode": 3}
```

`validate --json` returns `ok`, `archive`, `tiles`, `contentReferences`, `entries` and the list of `checks` run. `doctor --json` returns the dependency inventory with `ok`. If a selected command is not ready, it also carries `error` and exits with code 4. `preview --json` prints one startup object with `ok`, the bound `url` and the selected `layers`.

### Progress events

`--progress json` writes one JSON object per stderr line. It replaces the human summary, so stderr stays machine-readable.

| `event` | Fields | When |
| --- | --- | --- |
| `progress` | `phase`, `done`, `total` | Work units in a phase. `total` is null when unknown. |
| `warning` | `message`, or the structured feature report | A feature was skipped or degraded |
| `log` | `message` | Informational lines, such as mesh timings |
| `failed` | `phase`, `code` | The conversion failed with this error code |

Every converter emits a `conversion` phase at 0 and at 1. Completion comes only after the output is published. Other phases depend on the command.

| Command | Phases |
| --- | --- |
| `point-cloud` | `ingestion`, `tiling` |
| `vector` | `ingestion`, `encoding` |
| `raster` | `cog`, `display`, `tiling`. `tiling` counts XYZ tiles. |
| `terrain` | `terrain`, counting tiles |

Events report work units, not time remaining.

## Environment variables

| Variable | Used by | Effect |
| --- | --- | --- |
| `RUSTY_TILES_NATIVE_DIAGNOSTICS=1` | vector | Print raw GDAL and GEOS warnings. The deprecated `RUSTY_TILES_PYTHON_TRACEBACK=1` still works when this is unset. |
| `RAYON_NUM_THREADS` | mesh, terrain | Limit worker threads. `1` makes terrain encoding serial. |
| `PROJ_DATA` | geospatial commands | Directories holding `proj.db` and local grids |
| `RUSTY_TILES_DISABLE_NATIVE_JPEG=1` | build | Build with the portable Rust JPEG encoder instead of libjpeg-turbo |

Contributor test variables are listed in [CONTRIBUTING.md](../CONTRIBUTING.md#run-the-tests).
