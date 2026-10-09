# Command reference

This page lists every `rusty-tiles` subcommand and option. It is for users who script conversions and for programs that call the CLI. For a guided first run, start with the [README quick start](../README.md#quick-start).

Every value here comes from `rusty-tiles <command> --help` for version 0.4.0. Run that command to check your installed build.

## Build availability

The **standard CLI** is the default Cargo build and the binary supplied by release downloads. It supports meshes, local/grid-free LAS/LAZ, GeoJSON/GeoPackage, packaging, validation and preview without system geospatial libraries. **Native-geospatial** is a separate build from the native container or Cargo with `--features native-geospatial`; it adds raster/terrain, OGR-only inputs and eligible native CRS operations. Mesh placement can need native CRS support too. See [installation](INSTALL.md#choose-a-build).

Both builds expose the command help, including commands whose native capability is unavailable. Check `rusty-tiles doctor --command COMMAND` before using a converter. Adding GDAL to a standard installation does not enable the feature. The Python wheel exposes only the [standard Python API](../bindings/python/README.md), not this CLI.

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

`mesh-to-3tz`, `point-cloud` and `vector` produce 3D Tiles 1.1 implicit tiling by default. Their `--explicit` flag preserves the earlier explicit output and partitioning. Packaging preserves the selected bytes; its inventory and publication rules are documented under `convert`.

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
| `--nodeFeatures` | `--node-features` | off | Pick and style individual source nodes by `name` and `node_index`; also authors small inputs as tiles |
| `--noMeshopt` | `--no-meshopt` | off | Disable lossless meshopt compression |
| `--sourceCrs` | `--source-crs` | `auto` | Legacy `auto`, `geographic`, `epsg:3857`, or a general 2D horizontal EPSG/WKT/PROJ definition |
| `--sourceAxes` | `--source-axes` | none | Required for general CRS: `xyz` is E/N/height; `y-up` is E/height/−N, after glTF node transforms |
| `--heightOffset` | `--height-offset` | none | General CRS only: metres added to source height plus A to obtain ellipsoidal height; required, including explicit zero |
| `--sourceOffset E N [A]` | `--source-offset` | none | E/N in declared horizontal CRS units, A metres; added in double precision. Legacy adapters use EPSG:3857 metre shifts. |
| `--sourceOffsetFile` | `--source-offset-file` | none | Read E/N and optional A from labelled lines; existing `offset.txt` files remain supported |
| `--cartographicPositionDegrees lon lat [height]` | `--cartographic-position-degrees` | none | Place a local model on the globe |
| `--rotationDegrees heading pitch roll` | `--rotation-degrees` | none | Orient a placed model |

Mesh placement rules are in [Coordinates and height](../README.md#coordinates-and-height).

General CRS placement uses the shared [conservative CRS resolver](FORMATS.md#point-clouds). Supply both axes and a height decision, for example `--source-crs EPSG:32632 --source-axes xyz --source-offset 500000 0 100 --height-offset 0` for a model whose transformed POSITION values are local easting/northing/height offsets. Horizontal feet never rescale source height or A. The CRS bake rewrites positions into a local metre ENU frame and transforms authored normals by the projection's inverse transpose, preserving triangle membership/winding, UVs and feature identity before the usual atlas/LOD pipeline. Source files remain unchanged; output positions/normals are float32. Manual cartographic placement or rotation cannot be combined with the general path. Existing commands without axes/height retain their original placement semantics and representative textured output digests. Retiling materials without an authored PBR object now preserves its omission instead of inserting invalid JSON null.

### point-cloud

Tiles LAS or LAZ points into 3D Tiles with sampled parents and full-detail leaves.

Grid-free globe placement works in the default build. CRS selection automatically uses pure Rust for verified definitions and strict native GDAL/PROJ for other operations when enabled. Unsupported grid/datum operations name `--features native-geospatial` in the error. See the [supported CRS classes and height rules](FORMATS.md#point-clouds).

| Option | Alias | Default | Meaning |
| --- | --- | --- | --- |
| `--sourceCrs` | `--source-crs` | required | `local`, `header`, or a 2D horizontal CRS such as `EPSG:32632` |
| `--explicit` | | off | Keep the earlier explicit hierarchy and binary partition |
| `--heightOffset` | `--height-offset` | none | Metres added to source Z to give ellipsoidal height. Required for geospatial input. |
| `--maxPoints` | `--max-points` | `50000` | Maximum points per tile, at least 1 |
| `--metadataAttributes` | `--metadata-attributes` | off | Also expose `vertex_classification`, `vertex_intensity` and `vertex_return_number` as property attributes |
| `--chunkPoints` | `--chunk-points` | `100000` | Points read per source chunk, at least 1 |

### vector

Converts GeoJSON or GeoPackage layers into draft glTF vector content in a `.3tz` with the default build. Shapefile, other OGR drivers and CRS operations beyond the verified grid-free tier need `native-geospatial`. Behaviour and limits are in the [vector guide](VECTOR.md).

| Option | Alias | Default | Meaning |
| --- | --- | --- | --- |
| `--layer` | | automatic for one layer | Select a spatial layer. Repeat for several. |
| `--explicit` | | off | Keep the earlier explicit hierarchy and output bytes |
| `--allLayers` | `--all-layers` | off | Include every spatial layer |
| `--sourceCrs` | `--source-crs` | layer CRS | Override the input CRS, or `local` for metre XYZ |
| `--heightOffset` | `--height-offset` | none | Metres added to source heights to give ellipsoidal height |
| `--where` | | none | Attribute expression applied to every selected layer; SQLite in the default build, OGR in native builds |
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

`convert` selects every regular file beneath its directory input, or beneath
the parent of an exact `tileset.json` input. It requires that root member,
rejects symlinks/special files and unsafe/duplicate names, and rejects the
reserved generated `@3dtilesIndex1@` instead of silently omitting it. Remove that
entry explicitly before repacking an extracted archive. Sources must stay
stable during the call. The output must be outside the source tree and must
not alias a selected source file. Output names must end in `.3tz` or
`.3dtiles.zip`; member names may not contain those extensions. Each member must
be smaller than `2**32 - 1` bytes, though the complete archive may be larger.

Packaging preserves selected bytes, including an existing `conversion.json`,
and does not interpret that source report or certify scene/resource semantics.
Its `--json` result adds `packageReceipt` with `memberCount` (excluding the
generated index), `sourceBytes` and `archiveBytes`, plus `cleanupDiagnostics`.
Its `output` is the resolved absolute destination used for validation and
installation, including when `--output` is relative.
`conversionReport` is null for this operation: the package receipt is separate
from source content. Cleanup diagnostics contain `path`, `kind` and `message`
for retained temporary work beside a committed output.
JSON path fields use lossy UTF-8 display: non-UTF8 operating-system path bytes
appear as replacement characters and may not round-trip to the original path.
The operation uses the original native path.

Without `--force`, package installation refuses an existing or competing
destination. `--force` permits completed-file replacement; encoding, observer
or finalization failure before installation preserves the old destination.
Namespace installation and temporary-name cleanup are separate guarantees;
neither policy promises power-loss durability. On Unix, packaged output is
created with private mode `0600` (subject to umask), including when replacing a previous file;
change its permissions explicitly if other users need access.

Package failures expose `error.kind` alongside `error.code`,
`secondaryDiagnostics` and `retainedPaths`. The CLI maps these domain kinds to
process statuses: `invalid_request` and `unsupported` exit 2, `invalid_input`
exits 3, `output_conflict` exits 5, and `io`, `cancelled`, `observer_failure` or
`invalid_state` exit 1.

### convert-to-implicit

Rewrites an eligible rusty-tiles explicit point-cloud or vector `.3tz` archive into 3D Tiles 1.1 implicit tiling. Takes `-i/--input`, `-o/--output` and `-f/--force`; both paths must end in `.3tz` and must identify different files.

```sh
rusty-tiles convert-to-implicit -i explicit.3tz -o implicit.3tz
rusty-tiles --json convert-to-implicit -i explicit.3tz -o implicit.3tz --force
```

Eligibility is deliberately narrower than the source converters. Earlier explicit clouds use binary midpoint splits; explicit vectors use binary median splits and may include successive LOD nodes in the same cell. They are not automatically octrees or quadtrees. This command accepts a tree only when every child fits one distinct midpoint cell derived from the root box at its depth. Clouds use octants and vectors use quadrants in local XY. Bounds may exceed a cell only by documented minimum thickness and recorded float32/quantization padding. Same-cell LOD chains, cells crossed by content bounds, non-translation child transforms, request volumes, depths beyond 31, external tileset `schemaUri` declarations, explicit tile metadata, content bounding volumes, foreign provenance, already implicit archives and mesh archives are refused. Meshes remain outside this command's scope even when their current source converter produces octrees. Errors identify the offending eligibility rule and the source command to re-run without `--explicit`.

Older explicit vector archives can contain b3dm payloads whose total byte length is not a multiple of eight. These malformed payloads are refused: repairing their alignment would change the bytes. Re-run `vector` from source without `--explicit`; its implicit emitter supplies valid alignment.

The command verifies the existing 3TZ index, archive paths, content checksums, resource references and bounds. Each original content tile becomes an implicit tileset root that owns its replacement descendants. A bounded subtree routes child cells to standard external tileset roots, preserving each original child translation, content declaration, feature metadata and `extras`, while retaining every original GLB/b3dm payload byte-for-byte. Payload URI aliases and subtree URIs include every required implicit coordinate; aliases remain beside their source payloads so relative resources keep resolving. The report records original retained payload URIs. The shared subtree writer preserves actual padded bounds and geometric errors as semantic metadata. Keeping content and descendants under the same root ensures parent proxies retire during replacement refinement. The additional tileset roots add requests; their layout differs from a fresh implicit source conversion. The archive retains original payload members and adds an unchanged copy for each coordinate-template content slot. This typically doubles payload storage, and can cost more when multiple tiles share one source payload. Conversion never re-partitions content or recomputes source LODs.

The [pinned 3D Tiles 1.1 implicit-root constraints](https://github.com/CesiumGS/3d-tiles/blob/4d781014b52294759834018a931223b98ac1ce47/specification/ImplicitTiling/README.adoc#implicit-root-tile) prohibit actual external content at an implicit root. Here the root's external-link availability is false; actual external links occur only in terminal level-one child cells. Each linked document owns the next content tile and its descendants. Raw subtree tests check these availability constraints and the absence of child-subtree links.

`conversion.json` records the operation, scheme, source tile count, external tileset root count and byte preservation, with the original report under `sourceReport`. The candidate archive is validated before atomic publication. A failure leaves an existing output intact, including with `--force`. Vector build state is preserved as provenance; converted archives are not a compatible `--reuse-tileset` baseline, so re-run from source to establish a new reusable implicit baseline.

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
| `--cesium` | required | A Cesium 1.146.0 `Build/Cesium` directory containing `Cesium.js` |
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

`doctor` reports linked GDAL, PROJ and GEOS versions when enabled. It also reports readiness for the selected commands, the PROJ search paths and the local grid count. Point-cloud readiness lists the pure-Rust CRS classes and native fallback readiness; grid-free placement needs no PROJ database. Default-build vector readiness lists GeoJSON/GeoPackage readers, Rust geometry support and grid-free CRS limits independently of native dependencies. Native vector readiness retains GDAL/GEOS/PROJ checks. `--command` limits the command inventory in both human and JSON output. Without a filter it lists every command. It never installs anything or fetches grids. A listed grid does not prove a height operation is available. Conversion checks the exact operation offline.

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

## mesh-local-to-3tz

`mesh-local-to-3tz -i local.glb -o local.3tz --leaf-triangles 1000` converts the
F1a static embedded untextured GLB profile, explicitly interpreted as local
metres with Y up. `--leaf-triangles` is required and positive; it limits each
leaf's triangle count, not archive bytes, memory or geometric error. `--force`
uses completed-file replacement. This operation emits an explicit hierarchy
with exact leaf geometry and no coarse LOD, texture processing or guessed CRS.

Unsupported source semantics, including textures, animation, extensions and
extras, fail consistently before output staging. See the [finite source,
numerical and admission contract](architecture/f1a-contract.md). This command
is separate from the broader `mesh-to-3tz`; rejecting a local-profile source
does not silently route it through that operation.

`--json` returns `meshReport` with the same snake_case fields published in
`conversion.json`, plus `cleanupDiagnostics`. The output is its resolved
absolute installation path. `--progress json` uses fallible precommit domain
events; required observer/finalization failures preserve the previous destination.
The root geometric error is an extent-derived omission/selection metric,
not a measured simplification bound. Leaves retain all accepted triangles.

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
| `convert` | `encoding`, `ready_to_publish`; these package events finish before installation |
| `mesh-local-to-3tz` | `mesh_leaves`, `mesh_archive`, `ready_to_publish`; domain events finish before installation |

Events report work units, not time remaining.

## Environment variables

| Variable | Used by | Effect |
| --- | --- | --- |
| `RUSTY_TILES_NATIVE_DIAGNOSTICS=1` | vector | Print raw GDAL and GEOS warnings. The deprecated `RUSTY_TILES_PYTHON_TRACEBACK=1` still works when this is unset. |
| `RAYON_NUM_THREADS` | mesh, terrain | Limit worker threads. `1` makes terrain encoding serial. |
| `PROJ_DATA` | geospatial commands | Directories holding `proj.db` and local grids |
| `RUSTY_TILES_DISABLE_NATIVE_JPEG=1` | build | Force the portable Rust JPEG encoder even when `native-jpeg` is enabled; default builds already use portable JPEG |

Contributor test variables are listed in [CONTRIBUTING.md](../CONTRIBUTING.md#run-the-tests).
