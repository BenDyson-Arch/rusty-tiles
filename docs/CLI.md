# Command reference

This page lists every `rusty-tiles` subcommand and option. It is for users who script conversions and for programs that call the CLI. For a guided first run, start with the [README quick start](../README.md#quick-start).

Every value here comes from `rusty-tiles <command> --help` for version 0.4.0. Run that command to check your installed build.

## Build availability

Release downloads and the default Cargo build are the standard CLI. It runs `mesh`, `point-cloud`, `vector` (GeoJSON and GeoPackage), the packaging commands, `validate`, `preview` and `doctor` without system geospatial libraries, and covers CRS operations in the verified grid-free tier.

These need the [native-geospatial build](INSTALL.md#native-geospatial-cli) (GDAL, PROJ and GEOS):

- `raster`, `terrain` and `raster-tile-to-directory`;
- vector formats read through OGR, such as Shapefile;
- CRS operations outside the grid-free tier, for `mesh`, `point-cloud` and `vector`.

Both builds list every command in `--help`. A command or operation the build lacks is refused as `unsupported` (exit 2). Run `rusty-tiles doctor --command COMMAND` to check before converting. Installing GDAL next to a standard build does not enable these features. The Python wheel exposes the [Python API](../bindings/python/README.md), not this CLI.

## Conventions

Multi-word options have a camelCase spelling, such as `--sourceCrs`, and a kebab-case alias, such as `--source-crs`. Both behave the same. The examples in these docs use the kebab-case spelling. Options listed without an alias, such as the `terrain` options and the `raster` `--max-*` limits, have only the spelling shown. `mesh --memory-budget` is kebab-case first, with the alias `--memoryBudget`.

Two subcommands also have an alias.

| Command | Alias |
| --- | --- |
| `createTilesetJson` | `create-tileset-json` |
| `glb-to-3tz` | `glbTo3tz` |

Converters and `convert` share these file options. `createTilesetJson` takes only `-i`/`--input` and `--force` (no `-f`), and has no `--output`.

| Option | Default | Meaning |
| --- | --- | --- |
| `-i`, `--input` | required | Source file or directory |
| `-o`, `--output` | required | Output path |
| `-f`, `--force` | off | Replace an existing output after a successful conversion |

Two global options work before or after the subcommand.

| Option | Default | Meaning |
| --- | --- | --- |
| `--json` | off | Print one machine-readable result on stdout. Diagnostics stay on stderr. |
| `--progress json` | off | Print newline-delimited JSON events on stderr. `json` is the only value. |

See [machine output](#machine-output) for both formats.

### Tiled output layout

`mesh`, `point-cloud` and `vector` write 3D Tiles 1.1 implicit tiling as a bounded implicit forest:

- A tile with at least four levels of descendants stays an explicit tile, as do its leaf children.
- Each other child, with one to three levels of descendants, becomes an implicit root with exactly one binary subtree. The subtree records every tile's actual bounding box and geometric error and has no child subtrees.
- When the whole hierarchy has fewer than four levels of descendants, the root is the only implicit root.
- A large explicit skeleton is split into external tileset documents, `tileset-d{m}.json`, each of at most 16,384 JSON values and 2 MiB, referenced by explicit link tiles.
- Hierarchies with more than 32 levels are refused.
- Each payload is stored once. Explicit tiles publish their payloads in place. Implicit tiles publish copies whose glTF scene root carries the tile's translation.

In `vector` and `point-cloud` output, subtrees are named `subtrees/r{n}-{level}-{x}-{y}[-{z}].subtree` and implicit content `implicit-content/r{n}-{level}-{x}-{y}[-{z}]-{slot}.{suffix}`. No implicit content template names a JSON tileset. `mesh` naming may differ.

To move an existing explicit archive to implicit tiling, re-run its converter on the original source. There is no archive-to-archive rewrite. `validate` still reads explicit archives.

## Converters

### mesh

Tiles one static glTF/GLB scene into a 3D Tiles 1.1 implicit `.3tz`. The scene is local (metres, Y-up) or in a declared source CRS. Leaves keep full detail. Each parent tile is built from its children: its outer seams match its neighbours exactly, its geometric error is sampled and measured rather than certified, it keeps a representative source-primitive identity, and its textures are baked fresh. The [mesh guide](MESH.md) covers admitted sources, placement, resources and a known Cesium KTX2 colour-filtering limitation.

| Option | Alias | Default | Meaning |
| --- | --- | --- | --- |
| `--profile` | | `balanced` | `fast`, `balanced`, `quality` or `lossless`. Profile settings may change in later releases. |
| `--jobs` | | `auto` | A positive maximum worker count, or `auto`. Each stage caps workers by cores and memory; `auto` uses one worker when memory is unknown. |
| `--scratchDir` | `--scratch-dir` | platform scratch directory | Parent directory for working data. Archive staging stays beside the output. |
| `--memory-budget BYTES` | `--memoryBudget` | lesser of 4 GiB and half the detected headroom (2 GiB if undetected) | Memory ceiling for every stage, never raised. A positive byte count with an optional `B`, `KiB`, `MiB`, `GiB` or `TiB` suffix, such as `2GiB`. A run that cannot fit is refused as `resource_limit` before it allocates. The archive does not depend on the budget. Invalid values are usage errors (exit 2). See [memory budget](MESH.md#memory-budget). |
| `--anchor LON LAT HEIGHT` | | none | Place a local scene on the globe: WGS84 longitude and latitude in degrees, ellipsoidal height in metres. Without it, output stays local. |
| `--orientationXyzw X Y Z W` | `--orientation-xyzw` | identity with `--anchor` | Rotation in the anchor's east-north-up frame, as a near-unit quaternion with W last. Requires `--anchor`. |
| `--sceneOffset X Y Z` | `--scene-offset` | zero with `--anchor` | Translation in source Y-up metres, applied after node transforms and before the orientation. Requires `--anchor`. |
| `--sourceCrs` | `--source-crs` | `local` | `local`, or a 2D horizontal CRS as an EPSG code, WKT or PROJ string, applied at capture. Cannot be combined with the anchor options. |
| `--sourceAxes` | `--source-axes` | `y-up` with a source CRS | Axes after node transforms: `y-up` (east, height, −north) or `xyz` (east, north, height). |
| `--sourceOffset E N [A]` | `--source-offset` | `0 0 0` | Shift added in double precision: E and N in CRS units, A in metres. |
| `--sourceOffsetFile` | `--source-offset-file` | none | Offset file you write, with exactly the lines `E: <n>`, `N: <n>` and `A: <n>` (Metashape does not export one). Cannot be combined with `--sourceOffset`. |
| `--heightOffset` | `--height-offset` | `0` with a source CRS | Metres added to source height plus A to give ellipsoidal height. |

The shared `-i`, `-o` and `--force` options apply. Output is `.3tz` only.

With a [source CRS](MESH.md#source-crs), positions, normals and tangents are mapped per vertex at capture, and the CRS's local east-north-up frame becomes the root transform. There is no CRS inference, geoid conversion, coarse-tile tuning, full-detail-only mode or texture-format option. `lossless` does not preserve the source GLB bytes.

`--json` returns:

- `tilerReport`: the conversion report, in snake_case, identical to the archive's `conversion.json`;
- `diagnostics`: requested and actual workers, the resolved `memory` budget and stage plan, the coarse stage's `hlod` facts and `phase_seconds` (not stored in the archive);
- `cleanupDiagnostics`;
- the resolved `output`.

### point-cloud

Tiles LAS or LAZ points into 3D Tiles with sampled parents and full-detail leaves. Source and coordinate requirements are checked before output work starts. `--force` cannot overwrite the source file or an alias of it. All reports finish before the output is published.

The [point-cloud acceptance contract](architecture/point-acceptance-contract.md) defines admitted LAS formats, scalar Extra Bytes, duplicate points and coordinate precision. Scaled 64-bit integer Extra Bytes are unsupported; unscaled 64-bit integers stay exact. No-data, minimum and maximum declarations are kept in the schema.

Grid-free globe placement works in the standard build. Other CRS operations need the native-geospatial build; the error names `--features native-geospatial`. See the [supported CRS classes and height rules](FORMATS.md#point-clouds).

| Option | Alias | Default | Meaning |
| --- | --- | --- | --- |
| `--sourceCrs` | `--source-crs` | required | `local` (metre XYZ), `header`, or a 2D horizontal CRS such as `EPSG:32632` |
| `--heightOffset` | `--height-offset` | none | Metres added to source Z to give ellipsoidal height. Required for geospatial input. |
| `--maxPoints` | `--max-points` | `8192` | Maximum points per tile, from 1 to 8192. With `--metadata`, each point is one row of the tile's property table; CesiumJS 1.146 cannot build the GPU metadata texture for a table wider than the WebGL maximum texture size (8192 on the supported target). |
| `--metadata` | | off | Keep every source field per point in a property table. Off, tiles hold position and colour only. |
| `--metadataAttributes` | `--metadata-attributes` | off | Also expose `vertex_classification`, `vertex_intensity` and `vertex_return_number` as property attributes |
| `--chunkPoints` | `--chunk-points` | `100000` | Points read per source chunk, at least 1 |

`--maxPoints` and `--chunkPoints` do not bound total memory.

### vector

Converts GeoJSON or GeoPackage layers into draft glTF vector content in a `.3tz`. Other OGR formats, such as Shapefile, need the native-geospatial build. Behaviour and limits are in the [vector guide](VECTOR.md).

| Option | Alias | Default | Meaning |
| --- | --- | --- | --- |
| `--layer` | | automatic for one layer | Select a spatial layer. Repeat for several. Input with several layers needs `--layer` or `--allLayers`. |
| `--allLayers` | `--all-layers` | off | Include every spatial layer. Cannot be combined with `--layer`. |
| `--sourceCrs` | `--source-crs` | layer CRS | Override the input CRS, or `local` for metre XYZ |
| `--heightOffset` | `--height-offset` | none | Metres added to source heights to give ellipsoidal height |
| `--where` | | none | Attribute filter applied to every selected layer: SQLite syntax in the standard build, OGR in the native-geospatial build |
| `--fields` | | all | Comma-separated fields to include. Cannot be combined with `--dropFields`. |
| `--dropFields` | `--drop-fields` | none | Comma-separated fields to exclude |
| `--listFields` | `--list-fields` | `error` | `error` rejects list fields. `json` stores them as JSON text. |
| `--skipInvalid` | `--skip-invalid` | off | Omit unconvertible features and report each one |
| `--maxFeatures` | `--max-features` | `64` | Feature fragments per leaf tile |
| `--maxParentFeatures` | `--max-parent-features` | `4096` | Feature fragments per parent tile |
| `--maxVertices` | `--max-vertices` | `65536` | Encoded POSITION vertices per tile, at least 4 |
| `--maxBytes` | `--max-bytes` | `4194304` | Encoded GLB bytes per tile, at least 4096 |
| `--maxTiles` | `--max-tiles` | `100000` | Tiles in the hierarchy |
| `--maxSourceVertices` | `--max-source-vertices` | `1000000` | Coordinates in one source feature; bounds reader memory |
| `--lodTolerance` | `--lod-tolerance` | `0.1` | Tolerance in metres of the first coarse level above each leaf. It doubles at each level above that; parents also scale with their extent. See [level of detail](VECTOR.md#level-of-detail). |
| `--lodLevels` | `--lod-levels` | `3` | Coarse levels above each leaf, from 1 to 16 |
| `--repair` | | off | Repair invalid polygon outlines and report each repair |
| `--ambiguousOutlines` | `--ambiguous-outlines` | off | Keep ambiguous filled polygons as their source 3D outlines |
| `--parentRepair` | `--parent-repair` | off | Allow reported outline stand-ins in parent tiles |
| `--aggregatePoints` | `--aggregate-points` | off | Aggregate every point-only parent into per-layer voxel counts. Parents whose points exceed their budgets are always aggregated. |
| `--quantize` | | off | Lossy 16-bit positions, with the error reported |
| `--meshopt` | | off | Lossless `EXT_meshopt_compression` of accessor buffers |
| `--reuseTileset` | `--reuse-tileset` | none | Reuse unchanged content from a compatible earlier archive |
| `--jobs` | | available cores | Maximum encoding workers |
| `--reproducible` | | off | Omit timing diagnostics so archives are byte-identical |

### raster

Needs the native-geospatial build. Writes a standalone source COG, a nearest-neighbour PNG XYZ pyramid sampled directly from the source, TileJSON and a typed `report.json` into an output directory. See the [imagery profile](FORMATS.md#imagery).

| Option | Alias | Default | Meaning |
| --- | --- | --- | --- |
| `--maxZoom` | `--max-zoom` | required | Finest zoom, 0 to 24 |
| `--minZoom` | `--min-zoom` | `0` | Coarsest zoom, at most `--maxZoom` |
| `--display` | | `image` | `image` for byte RGB, grey or opaque palette imagery; `gray` styles raw numeric samples from one band |
| `--band` | | `1` for `gray` | Positive band index for `gray`. Not allowed with `image`. |
| `--alphaBand` | `--alpha-band` | `0` | Explicit opacity band. `0` uses the declared alpha band, or opaque when there is none. |
| `--displayMin` | `--display-min` | required for `gray` | Raw value shown as black |
| `--displayMax` | `--display-max` | required for `gray` | Raw value shown as white. The range must be finite and increasing, with a finite width. |
| `--jobs` | | `2` | Worker cap, 1 to 4; resolves to the lesser of the request and available CPUs |
| `--max-source-bytes` | | `536870912` | Total admitted source and companion bytes |
| `--max-source-pixels` | | `268435456` | Source pixel count |
| `--max-decoded-bytes` | | `2147483648` | Decoded source samples and independent mask |
| `--max-tiles` | | `100000` | Total tiles across requested zooms; see the finest-zoom ceiling below |
| `--max-output-bytes` | | `8589934592` | Completed output files, including both JSON files; checked after each file is closed |
| `--max-working-bytes` | | `17179869184` | Logical work and completed working files, including intermediates. Not an RSS quota. |

All six limits must be positive. The finest zoom also has a fixed 2 GiB RGBA ceiling: at most 8192 finest 256×256 tiles. That bounds a pyramid over zooms 0 to 24 to at most 16,451 tiles, so the default `--max-tiles` never refuses a request the ceiling admits. Set a lower value to cap the tile count.

`--json` success includes `rasterReport`, equal to the published `report.json`. The `raster_complete` progress event reports the actual tile count before publication. The old output is replaced only after the new directory is complete and sealed.

On Linux the output parent must be on ext, btrfs, tmpfs or xfs (kernel 4.9 or newer). Other filesystems, such as overlayfs or network mounts, are refused as `unsupported`.

With `--force`, the output cannot be the source file, a hard link to it, or a directory containing it. This is checked before any work; parent aliases are compared by filesystem identity. Replacing an output that is a symlink is allowed, because the symlink's target is kept.

### terrain

Needs the native-geospatial build. Writes a bounded 3D Tiles 1.1 GLB mesh directory: an implicit level-of-detail pyramid over full-detail leaves. See the [terrain guide](TERRAIN.md).

| Option | Default | Meaning |
| --- | --- | --- |
| `--cells-per-leaf` | `128` | Source-pixel cells per leaf patch edge: 16, 32, 64 or 128 |
| `--height-offset` | required | Metres added to raw DEM samples to give ellipsoidal height; never inferred |

The shared `-i`, `-o` and `--force` options apply. Level of detail is always on: coarser levels decimate the leaf lattice, carry certified geometric errors and have skirts on shared edges. There is no flat mode, simplification or zoom option. Source, height and precision limits are checked before any output is staged. `preview` supports queries, clamping and live imagery draping over terrain.

### raster-tile-to-directory

Needs the native-geospatial build. Copies one 256×256 RGB GeoTIFF, already aligned to an explicit Web Mercator XYZ tile, into a new directory as a PNG tile with `tilejson.json` and `report.json`. Broader imagery conversion is the `raster` command.

```sh
rusty-tiles raster-tile-to-directory -i aligned.tif -o ./new-tile --zoom 3 --x 5 --y 2
```

| Option | Default | Meaning |
| --- | --- | --- |
| `-i`, `--input` | required | Aligned RGB GeoTIFF |
| `-o`, `--output` | required | Output directory, in an existing parent on a supported local filesystem |
| `-f`, `--force` | off | Replace the current output entry after conversion finishes |
| `--zoom` | required | XYZ zoom of the tile |
| `--x` | required | XYZ column |
| `--y` | required | XYZ row |

An existing output is a conflict unless `--force` is given. `--force` replaces whatever entry is at the output path when publication starts, including a file or symlink. During replacement there is a moment when the output path is absent. Replacement does not promise snapshot isolation, protection from unrelated writers, or crash durability. The three files are published together.

`--json` returns the `rasterReport` and `cleanupDiagnostics`. If restoring the previous output fails, the error has `error.recovery: {"output": ..., "previousOutput": ...}`, and the previous output is kept at that backup path. Resolve whatever is at the destination before restoring it. Other failures have `error.recovery: null`. `error.recovery.nativePaths` gives exact filename units for when a name is not valid Unicode: `encoding` is `unix-bytes` or `windows-utf16`, with `output` and `previousOutput` arrays. A backup cleanup failure after installation is still a success, reported in `cleanupDiagnostics`.

The [directory creation](architecture/d1-directory-contract.md) and [replacement](architecture/d2-directory-contract.md) contracts give platform support and guarantees.

## Packaging commands

These commands package existing content without building level of detail.

| Command | Input | Output | Extra options |
| --- | --- | --- | --- |
| `glb-to-3tz` | One static local GLB/glTF (metres, Y-up) and its resources | `.3tz` archive with the source bytes unchanged | `--anchor`, `--orientation-xyzw`, `--scene-offset` |
| `createTilesetJson` | One static local GLB/glTF (metres, Y-up) | `tileset.json` beside the source | `--anchor`, `--orientation-xyzw`, `--scene-offset`; `-i` and `--force` only |
| `convert` | Tileset directory or `tileset.json` | `.3tz` archive | none |

The placement options mean the same as for [`mesh`](#mesh).

### glb-to-3tz and createTilesetJson

`createTilesetJson -i model.glb` writes `tileset.json` beside the source. Its content URI is the escaped file name. The root tile has full-detail geometric error zero, and a separate positive top-level geometric error lets viewers traverse it normally. Keep the source files unchanged while the manifest is in use.

`glb-to-3tz` stores the captured source bytes under `model/`, keeping URI spellings and resource aliases. Resources must stay inside the source directory.

Both commands return a typed `modelReport` in JSON, finish progress events before publication, and publish only complete files. See the [model wrapping contract](architecture/model-wrapping-contract.md) for source limits and unsupported features.

### convert

`convert` packages every regular file beneath its input directory, or beneath the parent of an exact `tileset.json` input. It requires that root `tileset.json`. It rejects symlinks, special files, unsafe or duplicate names, and a member named `@3dtilesIndex1@` (the generated index); remove that entry before repacking an extracted archive. Sources must not change during the call.

The output must be outside the source tree and must not be a selected source file. Output names must end in `.3tz` or `.3dtiles.zip`; member names may not contain those extensions. Each member must be smaller than `2**32 - 1` bytes; the archive may be larger.

`convert` keeps the selected bytes unchanged, including any existing `conversion.json`, and does not interpret that report or check scene or resource meaning. Its `--json` result adds:

- `packageReceipt`, with `memberCount` (excluding the generated index), `sourceBytes` and `archiveBytes`;
- `cleanupDiagnostics`, each with `path`, `kind` and `message`, for temporary work left beside a committed output;
- `output`, the resolved absolute destination, even when `--output` is relative.

`conversionReport` is null, because the receipt describes the package, not its content.

### Package output and errors

JSON path fields are shown as lossy UTF-8: operating-system path bytes that are not UTF-8 appear as replacement characters and may not round-trip. The command itself uses the original path.

Without `--force`, packaging refuses an existing or competing destination. With `--force`, the completed file replaces it. A failure before installation, during encoding, in a progress observer or at finalization, leaves the old destination in place. Neither policy promises power-loss durability. On Unix, packaged output is created with mode `0600` (subject to umask), including when it replaces a file; change its permissions if other users need access.

Package failures carry `error.kind`, `error.code`, `secondaryDiagnostics` and `retainedPaths`. See [exit codes](#exit-codes) for how kinds map to exit status.

## Checking and viewing

### validate

```sh
rusty-tiles validate output/example.3tz
```

`INPUT` is a self-contained `.3tz` archive. Validation is read-only and uses the fixed profile described in [validation](VALIDATION.md); `--help` calls it the C1 payload profile. Directories, such as terrain and raster output, are rejected as invalid input; package a tileset directory with `convert` first.

| Option | Alias | Default | Meaning |
| --- | --- | --- | --- |
| `--maxVolume KEY=N` | `--max-volume` | profile defaults | Set an opt-in volume cap or replace an amplification budget. Repeatable, once per key. `KEY` is one of `sourceArchiveBytes`, `archiveEntries`, `centralDirectoryBytes`, `archiveStoredBytes`, `hierarchyVisits`, `references`, `totalBytesRead`, `totalPayloadElements`, `containmentWorkUnits`, `containmentBytesRead` or `containmentDecodedBytes`. `N` is a decimal integer and may be below the default (stricter). Per-item format limits are fixed. See [limits](VALIDATION.md#limits). |
| `--scratchDir DIR` | `--scratch-dir` | `TMPDIR`, then the user cache directory, then the system temporary directory, disk-backed first | Put validation scratch files inside this existing directory |
| `--expectProfile PROFILE` | `--expect-profile` | none | Require a delivery profile whatever the archive's labels say. `hlod` is the only value: every payload must carry source identity and every HLOD check runs. An archive that declares HLOD is held to it anyway. |

The JSON report lists the `checks` that ran and what was `notInspected`, per-payload accessor, primitive and vertex counts, archive counts, and the `limits` used. Passing means the listed checks completed within the profile. It does not imply source fidelity, geometric-error accuracy, rendered extent, or metadata and material meaning. [Validation](VALIDATION.md) describes every check and limit.

Failures use `invalid_input` (exit 3), `unsupported` (exit 2), `resource_limit` (exit 3), `io` (exit 1) or `invalid_state` (exit 1, an internal error). Oversized inputs are refused as `resource_limit` before the work that limit covers. Parser usage errors exit 2. The Rust and Python APIs use the same inspection and limits.

`validate` starts no other process, fetches no remote resources and writes no output files. To run the official 3D Tiles validator as well, run it separately; its result does not change this report.

The bundled JSON schemas come from the Cesium 3D Tiles specification at commit `4d781014b52294759834018a931223b98ac1ce47`, with relative references rewritten to local `$defs`. Attribution and the CC BY 4.0 notice are in [`docs/schema/LICENSE.adoc`](schema/LICENSE.adoc). Schema loading makes no network or file lookups.

### preview

```sh
rusty-tiles preview --cesium target/preview-runtime/node_modules/cesium/Build/Cesium \
  --annotations output/example
```

| Option | Default | Serves |
| --- | --- | --- |
| `--cesium` | required | A Cesium 1.146.0 `Build/Cesium` directory containing `Cesium.js` |
| `--host` | `127.0.0.1` | Listen IP address. Host names are refused, not resolved. |
| `--port` | `9227` | Listen port. 0 picks a free port. |
| `--mesh` | none | Extracted mesh directory with `tileset.json` |
| `--point-cloud` | none | Extracted point-cloud directory with `tileset.json` |
| `--annotations` | none | Extracted vector directory with `tileset.json` |
| `--imagery` | none | Raster output directory with `tilejson.json` |
| `--terrain` | none | Terrain mesh directory with `tileset.json` |

Select at least one layer.

Preview is read-only and writes nothing, so stopping it needs no cleanup. It runs until signalled, for example with SIGTERM or Ctrl-C. Before binding, each selected directory and the Cesium directory are resolved to their real paths. Each must contain its manifest (`tileset.json`, `tilejson.json` or `Cesium.js`) as a regular file; otherwise the command exits 3 with `data`.

The server answers only `GET` and `HEAD` for `/`, `/config.json` and regular files under a selected layer's route. A path below a route is decoded once; invalid UTF-8 returns 400. Any path that fails either of these rules returns 404:

- **Lexical rule:** the path consists of non-empty `/`-separated segments. No segment may be `.` or `..` or contain a backslash, `:` or a control character, so a trailing slash or `//` also fails.
- **Exact-name rule:** each segment must name the stored file or directory exactly. On case-sensitive filesystems a case variant names a different, usually missing, file. Where the filesystem folds case or normalizes Unicode, the variant fails the exact-name comparison. A Windows short name or trailing dot, any symlink below the selected directory, a directory and an unselected layer all fail. The selected directory itself may be a symlink.

On the default loopback address, only requests addressed to `localhost`, `127.0.0.1`, `[::1]` or the bound address, with the bound port, are answered. Any other `Host` returns 421, which blocks DNS-rebinding pages. A request without `Host`, such as a bare HTTP/1.0 request, is refused too. The IPv4-mapped `::ffff:127.0.0.1` counts as loopback. A non-loopback `--host`, such as `0.0.0.0`, serves every host that can reach it: it accepts any `Host` and prints a warning on stderr.

Imagery uses the TileJSON `bounds`, `minzoom` and `maxzoom`. The rectangle is inset by 1/1024 of a finest-zoom pixel in longitude (under 1/88 of a pixel in latitude), because Cesium 1.146 stops rendering on tile-aligned bounds.

`--json` prints one startup object, `{"ok":true,"url":…,"layers":{…},"loopback":true}`, after the listener is bound.

### doctor

```sh
rusty-tiles doctor --command vector --command terrain
```

| Option | Default | Meaning |
| --- | --- | --- |
| `--command` | all | Check only these commands. Repeat to select several. Aliases are accepted. |
| `--cesium DIR` | `target/preview-runtime/node_modules/cesium/Build/Cesium`, relative to the working directory | Where to look for a Cesium runtime. This check is informational only. |

Each selected command has a `status`:

- `ready`: the command can run in this build.
- `unavailable`: the build includes the capability, but a runtime dependency failed, such as the GDAL or PROJ version, the PROJ database, a GDAL driver or GEOS. The `reason` names the failure.
- `unsupported`: this build does not include the capability. For example, the standard CLI does not include `raster`, `terrain` or `raster-tile-to-directory`.

`ready` is `true` exactly when `status` is `ready`. Each command that is not ready has a `refusal` with the `code` and `exitCode` that the command itself returns for the same cause. The first such command in name order sets the `error.code` and exit status of `doctor`. All such refusals are `unsupported` (exit 2). The JSON result keeps the full inventory alongside `error` and `exitCode`.

One exception: with no GTiff driver, `terrain` and `raster-tile-to-directory` fail to open their source and report `invalid_input` (exit 3), while `doctor` reports `unsupported`.

Requirements in the native-geospatial build:

- `terrain` and `raster-tile-to-directory` need the GDAL GTiff driver;
- `raster` needs the GTiff and COG drivers;
- these three also need the PROJ database for their source CRS;
- native vector needs GDAL with GEOS and its drivers, but no PROJ database. Each source's CRS operation is checked during conversion.

`doctor` reports linked GDAL, PROJ and GEOS versions when present, readiness for the selected commands, the PROJ search paths and the local grid count. `probes` names every check that ran. In a native-geospatial build the version, PROJ offline-policy and PROJ database checks always run. Configured PROJ directories (`PROJ_DATA`, `PROJ_LIB` and GDAL's search paths) are always listed, top level only and capped at 4096 entries per directory. GDAL driver, GEOS and Cesium checks run only for the commands that need them.

Point-cloud readiness lists the pure-Rust CRS classes and native fallback readiness; grid-free placement needs no PROJ database. Standard-build vector readiness lists the GeoJSON and GeoPackage readers, Rust geometry support and grid-free CRS limits. Native vector readiness uses the converter's own GDAL and GEOS driver check.

`--command` limits the inventory in both human and JSON output; without it every command is listed. `doctor` starts no processes, keeps PROJ networking off, writes no files, installs nothing and fetches no grids. A listed grid does not prove a height operation is available; conversion checks the exact operation offline.

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

`mesh`, `vector`, `point-cloud`, `raster`, `raster-tile-to-directory` and the packaging commands report error kinds instead:

| Exit code | Error kinds |
| --- | --- |
| 1 | `io`, `resource_limit`, `cancelled`, `observer_failure`, `invalid_state` |
| 2 | `invalid_request`, `unsupported` |
| 3 | `invalid_input` |
| 5 | `output_conflict` |

Their JSON errors include `kind`, secondary diagnostics and retained paths. For example, `vector --lod-levels 40` names the permitted range and exits 2. An unsupported CRS operation, or a native command in the standard build, is `unsupported`; malformed source records are `invalid_input`. `validate` has its own mapping, listed [above](#validate). The first table applies to the other commands.

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

The example is shortened. `counts` holds only counts, such as `points`, `tiles` and `features`. `settings` holds every other top-level number from `conversion.json`, such as `heightOffset`, `grid` or `lodLevels`. For a report in an output directory, `conversionReport` is `{"path": ".../conversion.json"}`. Commands without a conversion report return empty `counts` and `settings` and null report fields.

A failure has this shape and the matching exit code:

```json
{"ok": false, "error": {"code": "data", "message": "input not found: nope.geojson"}, "exitCode": 3}
```

`validate --json` returns `ok`, `archive`, `tiles`, `contentReferences`, `entries` and the list of `checks` run; [validation](VALIDATION.md) lists every field. `doctor --json` returns the dependency inventory with `ok`. If a selected command is not ready, it also carries `error` and exits with that command's refusal status (see [doctor](#doctor)). `preview --json` prints one startup object with `ok`, the bound `url` and the selected `layers`.

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
| `mesh` | `capture`, `leaf_build`, `hlod_incidence`, `hlod_geometry`, `leaf_payloads`, `archive`, `ready_to_publish`; required events finish before installation. See the [mesh guide](MESH.md). |
| `point-cloud` | `ingestion`, `tiling`, `point_archive`, `ready_to_publish`; these finish before installation |
| `vector` | `ingestion`, `encoding` |
| `raster` | `source_cog` and `aligned_warp` report progress out of 1,000,000. `tiling` reports completed tiles on the same scale and resets for each zoom. `raster_complete` reports the actual tile count before sealing and installation. |
| `terrain` | `terrain`, counting tiles |
| `convert` | `encoding`, `ready_to_publish`; these finish before installation |
| `validate` | `validate.hash`, `validate.traverse`, `validate.stream`, `validate.rows`; see [validation](VALIDATION.md#errors-cancellation-and-progress) |

Events report work units, not time remaining.

## Environment variables

| Variable | Used by | Effect |
| --- | --- | --- |
| `RUSTY_TILES_NATIVE_DIAGNOSTICS=1` | vector | Print raw GDAL and GEOS warnings |
| `RUSTY_TILES_STORAGE_DISK=1` | mesh | Treat storage whose kind cannot be detected, such as a container's overlay root, as disk-backed, so it is not charged to the memory budget. Never overrides detected tmpfs. See [memory budget](MESH.md#memory-budget). |
| `PROJ_DATA` | geospatial commands | Directories holding `proj.db` and local grids |
| `TMPDIR` | validate | First candidate for validation scratch files |

Contributor test variables are listed in [CONTRIBUTING.md](../CONTRIBUTING.md#run-the-tests).

## Changes from 0.3

- `mesh-to-3tz` (`meshTo3tz`) is removed; use [`mesh`](#mesh). The old name is an unknown subcommand (exit 2). See [migration from older mesh converters](MESH.md#migration-from-older-mesh-converters).
- `point-cloud` and `vector` write implicit tiling only. Re-run a conversion from the original source to update an archive.
- `vector --reuseTileset` refuses archives from 0.3 and earlier. Convert once without it to make a new baseline.
- `glb-to-3tz` and `createTilesetJson` take one GLB/glTF file, not a directory. `--cartographicPositionDegrees` and `--rotationDegrees` are replaced by `--anchor`, `--orientationXyzw` and `--sceneOffset`.
- `createTilesetJson` always writes `tileset.json` beside the source. It has no `-o` option and no `-f` short form.
- `validate --external-validator` is removed. Run the official validator separately.
- `validate` refuses legacy b3dm, i3dm, pnts and cmpt content as `unsupported` (exit 2). Convert such archives again.
- `doctor` exits 2 (`unsupported`) for a command that is not ready, instead of 4 (`environment`).
- `RUSTY_TILES_PYTHON_TRACEBACK` is ignored; use `RUSTY_TILES_NATIVE_DIAGNOSTICS`.
