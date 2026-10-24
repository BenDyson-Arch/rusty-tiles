# Command reference

This page lists every `rusty-tiles` subcommand and option. It is for users who script conversions and for programs that call the CLI. For a guided first run, start with the [README quick start](../README.md#quick-start).

Every value here comes from `rusty-tiles <command> --help` for version 0.4.0. Run that command to check your installed build.

## Build availability

The **standard CLI** is the default Cargo build and the binary supplied by release downloads. It supports meshes, local/grid-free LAS/LAZ, GeoJSON/GeoPackage, packaging, validation and preview without system geospatial libraries. **Native-geospatial** is a separate build from the native container or Cargo with `--features native-geospatial`; it adds raster/terrain, OGR-only inputs and eligible native CRS operations. Mesh source-CRS conversion (`mesh --source-crs`) can need native support for CRS outside the verified grid-free tier; the new `mesh` command, its grid-free source CRS and rigid model placement work in the standard build. See [installation](INSTALL.md#choose-a-build).

Both builds expose the command help, including commands whose native capability is unavailable. Check `rusty-tiles doctor --command COMMAND` before using a converter. Adding GDAL to a standard installation does not enable the feature. The Python wheel exposes only the [standard Python API](../bindings/python/README.md), not this CLI.

## Conventions

Options have a camelCase spelling, such as `--sourceCrs`. Each multi-word option also accepts a kebab-case alias, such as `--source-crs`. Both spellings behave the same. The examples in these docs use the kebab-case alias.

Two subcommands also have an alias.

| Command | Alias |
| --- | --- |
| `createTilesetJson` | `create-tileset-json` |
| `glb-to-3tz` | `glbTo3tz` |

Archive/directory converters share these file options. `createTilesetJson`
takes `--input` and `--force` and derives a fixed sibling output; it has no
`--output` option.

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

`mesh`, `point-cloud` and `vector` produce 3D Tiles 1.1 implicit tiling and write only this layout. The `--explicit` flag of `point-cloud` and `vector` was removed in 0.4.0 and is now an unknown argument; the legacy `mesh-to-3tz`, `meshTo3tz` and `mesh-local-to-3tz` commands were removed in 0.4.0 and are unknown subcommands (exit 2). To move an existing explicit point-cloud or vector archive to implicit tiling, re-run its converter from the original source; there is no archive-to-archive rewrite command. `validate` still reads explicit archives, including those written before 0.4.0. Packaging preserves the selected bytes; its inventory and publication rules are documented under `convert`.

The shared layout is a bounded implicit forest. Producer naming/partition details vary; the following member naming describes vector and point-cloud output. A tile with at least four levels of descendants stays an explicit tile, as do its leaf children; each other child, with one to three levels of descendants, becomes an implicit root with exactly one binary subtree, `subtrees/r{n}-{level}-{x}-{y}[-{z}].subtree`, which records every tile's actual bounding box and geometric error and has no child subtrees. When the whole hierarchy has fewer than four levels of descendants, the root itself is the only implicit root. Implicit content is published under `implicit-content/r{n}-{level}-{x}-{y}[-{z}]-{slot}.{suffix}`; no implicit content template names a JSON tileset. A large explicit skeleton is split into explicit external tileset documents, `tileset-d{m}.json`, each of at most 16,384 JSON values and 2 MiB, referenced by explicit link tiles. Hierarchies with more than 32 levels are refused. Archives keep each payload once: explicit tiles publish their payloads in place, and implicit tiles publish copies whose glTF scene root carries the tile's translation.

## Converters

### mesh

Streams one static glTF/GLB scene, local metre/Y-up or in a declared source CRS,
through capture, leaf encoding, HLOD coarse levels and Tiles 1.1 implicit `.3tz` delivery. Leaves are exact;
every parent tile is built from its children with locked outer interfaces (crack-free by construction), a sampled,
measured geometric error rather than a certificate, a representative source-primitive identity and freshly
baked textures. A legacy certified-proxy path still builds the coarse tiles until it is removed. See the [mesh guide](MESH.md)
for admitted sources, placement, resources, migration and the known Cesium
KTX2 colour-filtering limitation. The preparatory leaf/delivery commands and
Rust exports are now internal.

| Option | Alias | Default | Meaning |
| --- | --- | --- | --- |
| `--profile` | | `balanced` | `fast`, `balanced`, `quality` or `lossless`; policies/constants are provisional |
| `--jobs` | | `auto` | Positive maximum workers or memory-aware auto; actual workers are capped per stage by cores and memory; unknown memory uses one worker |
| `--memory-budget BYTES` | `--memoryBudget` | lesser of 4 GiB and half the detected headroom (2 GiB if undetected) | Declared memory ceiling, never raised: a positive byte count, or with a `KiB`/`MiB`/`GiB`/`TiB` suffix (`2GiB`); zero, signs, fractions and overflow are usage errors before any source I/O. The current default coarse-tile method does not enforce one and refuses it |
| `--scratchDir` | `--scratch-dir` | platform scratch parent | Owned working-data parent; final archive staging stays beside output |
| `--anchor LON LAT HEIGHT` | | none | WGS84 longitude/latitude degrees and ellipsoidal height metres; omission keeps local output |
| `--orientationXyzw X Y Z W` | `--orientation-xyzw` | identity with anchor | Active ENU near-unit quaternion, scalar W last; requires anchor |
| `--sceneOffset X Y Z` | `--scene-offset` | zero with anchor | Post-node source Y-up metre translation before orientation; requires anchor |
| `--sourceCrs` | `--source-crs` | `local` | `local` or a 2D horizontal EPSG/WKT/PROJ definition, mapped at capture; excludes the anchor options |
| `--sourceAxes` | `--source-axes` | `y-up` with a source CRS | `y-up` (E, height, −N) or `xyz` (E, N, height) after node transforms |
| `--sourceOffset E N [A]` | `--source-offset` | `0 0 0` | Added in double precision: E/N in horizontal CRS units, A metres |
| `--sourceOffsetFile` | `--source-offset-file` | none | Metashape offset file of exactly `E: <n>`, `N: <n>`, `A: <n>` lines; excludes `--sourceOffset` |
| `--heightOffset` | `--height-offset` | `0` with a source CRS | Metres added to source height plus A to give ellipsoidal height |

Shared `-i`/`-o`/`--force`, `--json` and `--progress json` apply. Output is `.3tz`
only. `--json` returns `tilerReport` (snake_case, identical to `conversion.json`),
`diagnostics` (requested/actual workers and resources, outside the archive),
`cleanupDiagnostics`, and the resolved output. Input is local with optional
rigid placement, or in a declared [source CRS](MESH.md#source-crs) whose
per-vertex map (positions, normals and tangents) is applied at capture and whose
ENU frame becomes the root transform. There is no CRS inference, geoid
conversion, coarse-tile tuning, explicit/full-detail mode or texture-format flag. Profiles do not freeze numerical settings and
`lossless` is not a source-GLB byte-preservation promise.

### point-cloud

Tiles LAS or LAZ points into 3D Tiles with sampled parents and full-detail leaves.
The typed point-cloud facade validates resolvable source/coordinate requirements
before output work and uses one run for observation, cancellation and publication.
`--force` selects replacement policy; it cannot authorize overwriting the source or
an alias. All required reports and producer work finish before publication.

[The P1 contract](architecture/point-acceptance-contract.md) defines admitted LAS
formats, scalar Extra Bytes, duplicate multiplicity, coordinate precision and
remaining proof limits. Scaled 64-bit integer Extra Bytes are unsupported; unscaled
64-bit integers remain exact. No-data/min/max declarations are preserved in schema.
The point and chunk counts below are not whole-job memory bounds.

Grid-free globe placement works in the default build. CRS selection automatically uses pure Rust for verified definitions and strict native GDAL/PROJ for other operations when enabled. Unsupported grid/datum operations name `--features native-geospatial` in the error. See the [supported CRS classes and height rules](FORMATS.md#point-clouds).

| Option | Alias | Default | Meaning |
| --- | --- | --- | --- |
| `--sourceCrs` | `--source-crs` | required | `local`, `header`, or a 2D horizontal CRS such as `EPSG:32632` |
| `--heightOffset` | `--height-offset` | none | Metres added to source Z to give ellipsoidal height. Required for geospatial input. |
| `--maxPoints` | `--max-points` | `8192` | Maximum points per tile, from 1 to 8192. Each point is one row of the tile's property table, and CesiumJS 1.146 cannot build the GPU metadata texture of a table wider than the WebGL maximum texture size (8192 on the pinned target) |
| `--metadataAttributes` | `--metadata-attributes` | off | Also expose `vertex_classification`, `vertex_intensity` and `vertex_return_number` as property attributes |
| `--chunkPoints` | `--chunk-points` | `100000` | Points read per source chunk, at least 1 |

### vector

Converts GeoJSON or GeoPackage layers into draft glTF vector content in a `.3tz` with the default build. Shapefile, other OGR drivers and CRS operations beyond the verified grid-free tier need `native-geospatial`. Behaviour and limits are in the [vector guide](VECTOR.md).

| Option | Alias | Default | Meaning |
| --- | --- | --- | --- |
| `--layer` | | automatic for one layer | Select a spatial layer. Repeat for several. |
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

Writes a standalone source COG, directly sampled nearest-neighbour PNG XYZ pyramid,
TileJSON and typed `report.json`. See the [imagery profile](FORMATS.md#imagery).

| Option | Alias | Default | Meaning |
| --- | --- | --- | --- |
| `--maxZoom` | `--max-zoom` | required | Finest zoom, 0 through 24 |
| `--minZoom` | `--min-zoom` | `0` | Coarsest zoom, at most `--maxZoom` |
| `--display` | | `image` | Byte RGB, gray or opaque palette imagery; `gray` styles raw numeric samples |
| `--band` | | `1` for gray | Positive gray band index; an explicit band is invalid for image display |
| `--alphaBand` | `--alpha-band` | `0` | Explicit opacity band; 0 selects the declared alpha, or opaque when absent |
| `--displayMin` | `--display-min` | required for gray | Raw value shown as black |
| `--displayMax` | `--display-max` | required for gray | Raw value shown as white; finite increasing range with finite width |
| `--jobs` | | `2` | Worker cap, 1 through 4; resolves to min(requested, available CPUs) |
| `--max-source-bytes` | | `536870912` | Aggregate admitted source and companion bytes |
| `--max-source-pixels` | | `268435456` | Source pixel count |
| `--max-decoded-bytes` | | `2147483648` | Decoded source samples and independent mask |
| `--max-tiles` | | `100000` | Total tiles across requested zooms; see the finest ceiling below |
| `--max-output-bytes` | | `8589934592` | Closed output members including both JSON files |
| `--max-working-bytes` | | `17179869184` | Logical source, derivative and concrete owner allowance; also caps closed staged files, including intermediates |

All six limits must be positive. The finest zoom also has a fixed 2 GiB RGBA
ceiling: at most 8192 finest 256×256 tiles, which bounds a pyramid over zooms 0
through 24 to at most 16,451 tiles. The default `--max-tiles` therefore never
refuses a request the ceiling would admit; set a lower value to cap tile count. JSON success includes `rasterReport`, equal to
published `report.json`; the precommit `raster_complete` progress reports the actual tile count.
Replacement occurs only after the complete directory is closed and sealed.
On Linux the output parent must be on ext, btrfs, tmpfs or xfs (kernel 4.9 or
newer); other filesystems, such as overlayfs or network mounts, are refused as `unsupported`.

### terrain

Writes a bounded 3D Tiles 1.1 GLB mesh directory: an implicit level-of-detail
pyramid over full-detail leaves. See the [terrain guide](TERRAIN.md).

| Option | Default | Meaning |
| --- | --- | --- |
| `--cells-per-leaf` | `64` | Source-pixel cells per patch edge: 16, 32, 64 or 128 |
| `--height-offset` | required | Metres added to raw DEM samples to obtain ellipsoidal height |
| `--fill-height` | required | Ellipsoidal height for missing lattice samples inside the footprint |

Level of detail is always on: coarser levels decimate the leaf lattice, carry
certified geometric errors and have skirts on shared edges. There is no flat
mode, simplification or zoom option. Source, height and precision limits
are checked before private staging. Queries, clamping and live imagery draping
are included in the pinned Cesium preview.

## Packaging commands

These commands package existing content without building spatial level of detail.

| Command | Input | Output | Extra options |
| --- | --- | --- | --- |
| `glb-to-3tz` | Admitted static local metre/Y-up GLB/glTF and confined resources | Exact-byte `.3tz` archive | `--anchor`, `--orientation-xyzw`, `--scene-offset` |
| `createTilesetJson` | One admitted static local metre/Y-up GLB/glTF | Fixed sibling `tileset.json` | `--anchor`, `--orientation-xyzw`, `--scene-offset`, `--force`; no `-o` |
| `convert` | Tileset directory or `tileset.json` | `.3tz` archive | none |

`createTilesetJson -i model.glb` writes beside the source, with an escaped basename URI and full-detail root error zero. Its separate positive top-level visibility error permits ordinary tileset traversal. Keep the source tree unchanged during publication and continued manifest use. Directory discovery and arbitrary output bases are retired. Both model operations use required typed reports (`modelReport` in JSON), fallible precommit progress, and completed-file F0 publication. Wrapping packages captured bytes beneath `model/`; it preserves URI spellings and resource aliases. Legacy HPR flags are replaced with the rigid placement grammar documented below. See the [source limits and capability reductions](architecture/model-wrapping-contract.md). `convert` retains its directory packaging grammar.

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

## Checking and viewing

### validate

```sh
rusty-tiles validate output/example.3tz
```

`INPUT` is a self-contained `.3tz` archive. Inspection is read-only and uses the
fixed [C1 validation profile](VALIDATION.md). Directories, including T1 terrain
and raster outputs, are rejected as invalid input; package supported content before inspection.

| Option | Alias | Default | Meaning |
| --- | --- | --- | --- |
| `--maxVolume KEY=N` | `--max-volume` | profile defaults | Set an opt-in volume cap or replace an amplification budget; repeatable, once per key. `KEY` is one of `sourceArchiveBytes`, `archiveEntries`, `centralDirectoryBytes`, `archiveStoredBytes`, `hierarchyVisits`, `references`, `totalBytesRead`, `totalPayloadElements`, `containmentWorkUnits`, `containmentBytesRead` or `containmentDecodedBytes`; `N` is a decimal integer. Values may be below the defaults (stricter); per-item format limits are fixed. See [validation limits](VALIDATION.md) |
| `--scratchDir DIR` | `--scratch-dir` | `TMPDIR`, then the user cache directory, then the system temporary directory | Put validation scratch files inside this existing directory |

The typed JSON report lists `checks` and `notInspected`, per-payload accessor,
primitive and vertex counts, archive counts, and fixed admission `limits`.
Passing inspection means the listed checks completed within that profile.
`hierarchyBounds` checks declared child volumes. `decodedContentContainment`
appears only when every drawn vertex of an explicit, in-domain archive is exactly inside
its tile, content, external-content and ancestor volumes; otherwise `notInspected` names the
`decodedContentBounds.<reason>`, and a proven escape is `invalid_input`. See the
[C2 contract](architecture/c2-decoded-containment-contract.md). Source fidelity,
geometric-error accuracy, rendered extent and metadata/material semantics are not implied. Optional extensions retain explicit inspection gaps;
unknown required extensions fail as unsupported. The `archiveStoredRecordLayout`
check admits one disjoint stored-record layout and one directory interpretation.
Coherent nested records, alternate name encodings and unsupported ZIP64 descriptor
declarations receive `unsupported`; duplicate names and malformed records receive
`invalid_input`. The [validation profile](VALIDATION.md) gives the finite domain.

Failures use `invalid_input` (exit 3), `unsupported` (exit 2), `resource_limit`
(exit 3), `io` (exit 1), or `invalid_state` (exit 1, an internal invariant breach). Oversized inputs receive a resource-limit outcome
before the work covered by that ceiling. Parser usage errors remain exit 2.
The Rust and Python APIs share the same core inspection and fixed limits.

`--external-validator` has been removed. Run an independently installed official
validator separately when its additional checks are needed; its result cannot
silently upgrade unsupported built-in inspection. The library launches no
validator process, fetches no remote resources, and writes no output files.

The schema bundle comes from the Cesium GS 3D Tiles specification at commit `4d781014b52294759834018a931223b98ac1ce47`. Its relative references were rewritten to local `$defs`. Attribution and the CC BY 4.0 notice are in [`docs/schema/LICENSE.adoc`](schema/LICENSE.adoc). Schema loading makes no network or file reference lookups.

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

Preview is read-only. Before binding, each selected directory and the Cesium directory are resolved to their real paths. Each must contain its manifest (`tileset.json`, `tilejson.json` or `Cesium.js`) as a regular file inside that directory; otherwise the command exits 3 with `data`. The server answers only `GET` and `HEAD` for `/`, `/config.json` and regular files under a selected layer's route. A path below a route is decoded once; invalid UTF-8 returns 400.

Every platform then applies the same two rules, and any path that fails either returns 404:

- **Lexical rule:** the path consists of non-empty `/`-separated segments. No segment may be `.` or `..` or contain a backslash, `:` or a control character, so a trailing slash or `//` also fails.
- **Exact-name rule:** each segment must name the stored file or directory exactly. On case-sensitive filesystems a case variant names a different, usually missing, file; where the filesystem folds case or normalizes Unicode and reports stored names, the variant fails the exact-name comparison. A Windows short name or trailing dot, or any symlink below the selected directory, fails, as do directories and unselected layers. A file reached by a variant is always inside the selected directory. The selected directory itself may be a symlink.

On the default loopback address, only requests addressed to `localhost`, `127.0.0.1`, `[::1]` or the bound address, with the bound port, are answered. Any other `Host` returns 421, which blocks DNS-rebinding pages. A request without `Host`, such as a bare HTTP/1.0 request, is refused too. The IPv4-mapped `::ffff:127.0.0.1` counts as loopback. A non-loopback `--host`, such as `0.0.0.0`, is an explicit opt-in to serve every host that can reach it: it accepts any `Host` and prints a warning on stderr.

Imagery uses the TileJSON `bounds`, `minzoom` and `maxzoom`. The rectangle is inset by 1/1024 of a finest-zoom pixel in longitude (under 1/88 of a pixel in latitude) to work around Cesium 1.146, which stops rendering on tile-aligned bounds.

`--json` prints one startup object, `{"ok":true,"url":…,"layers":{…},"loopback":true}`, after the listener is bound. The server runs until it is signalled (for example, SIGTERM or Ctrl-C). It writes nothing, so stopping it needs no cleanup. See the [T1 tooling contract](architecture/t1-tooling-contract.md).

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
- `unavailable`: the build includes the capability, but a runtime dependency failed. Examples are the GDAL or PROJ version, the PROJ database, a GDAL driver and GEOS. The `reason` names the failure.
- `unsupported`: this build does not include the capability. For example, the standard CLI does not include `raster`, `terrain` or `raster-tile-to-directory`; rebuild with `--features native-geospatial`.

`ready` is `true` exactly when `status` is `ready`. Each command that is not ready has a `refusal` with the `code` and `exitCode` that the command itself returns for the same cause; readiness uses the command's own capability check where it has one. The first such command in name order sets the `error.code` and exit status of `doctor`. All are currently `unsupported` and exit 2.

One known exception: with no GTiff driver, `terrain` and `raster-tile-to-directory` fail to open their source and report `invalid_input` (exit 3), while `doctor` reports `unsupported`. The JSON result keeps the full inventory alongside `error` and `exitCode`.

Terrain and `raster-tile-to-directory` require the GDAL GTiff driver. `raster` requires GTiff and COG, and these three also need the PROJ database for their source CRS. Native vector needs GDAL with GEOS and its drivers but no PROJ database; each source's CRS operation is checked during conversion.

`probes` names every check that ran. In a native-geospatial build, the version, PROJ offline-policy and PROJ database checks always run. Configured PROJ directories (`PROJ_DATA`, `PROJ_LIB` and, natively, GDAL's search paths) are always listed. GDAL driver, GEOS and Cesium checks run only for the commands that need them. PROJ directory listings are top-level only and capped at 4096 entries per directory. `doctor` starts no processes, keeps PROJ networking off and writes no files.

`doctor` reports linked GDAL, PROJ and GEOS versions when enabled. It also reports readiness for the selected commands, the PROJ search paths and the local grid count. Point-cloud readiness lists the pure-Rust CRS classes and native fallback readiness; grid-free placement needs no PROJ database. Default-build vector readiness lists GeoJSON/GeoPackage readers, Rust geometry support and grid-free CRS limits independently of native dependencies. Native vector readiness uses the vector converter's own GDAL/GEOS driver check. `--command` limits the command inventory in both human and JSON output. Without a filter it lists every command. It never installs anything or fetches grids. A listed grid does not prove a height operation is available. Conversion checks the exact operation offline.

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

Mesh, vector, point-cloud and raster conversion use the foundation error categories: `invalid_request` and `unsupported` exit 2, `invalid_input` exits 3, `output_conflict` exits 5, and `io`, `resource_limit`, `cancelled`, `observer_failure` or `invalid_state` exit 1. A build without native raster support refuses `raster` as `unsupported`. Their JSON errors include `kind`, secondary diagnostics and retained paths. For example, vector `--lod-levels 40` names the permitted range and exits 2. Unsupported CRS capability is `unsupported`; malformed source records are `invalid_input`. The categories in the table apply to the other commands.

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

The example is shortened. `counts` holds only counts, such as `points`, `tiles` and `features`. `settings` holds every other top-level number from `conversion.json`, such as `heightOffset`, `grid` or `lodLevels`. For legacy directory reports, `conversionReport` is `{"path": ".../conversion.json"}`. Commands without a conversion report return empty `counts` and `settings` and null report fields.

A failure has this shape and the matching exit code:

```json
{"ok": false, "error": {"code": "data", "message": "input not found: nope.geojson"}, "exitCode": 3}
```

`validate --json` returns `ok`, `archive`, `tiles`, `contentReferences`, `entries` and the list of `checks` run. `doctor --json` returns the dependency inventory with `ok`. If a selected command is not ready, it also carries `error` and exits with that capability's refusal status (see [doctor](#doctor)). `preview --json` prints one startup object with `ok`, the bound `url` and the selected `layers`.

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
| `point-cloud` | `ingestion`, `tiling`, `point_archive`, `ready_to_publish`; domain events finish before installation |
| `vector` | `ingestion`, `encoding` |
| `raster` | `source_cog` and `aligned_warp` use native normalized progress out of 1,000,000; `tiling` reports completed tiles on the same scale and resets for each zoom. `raster_complete` reports actual tile count before sealing and installation. |
| `terrain` | `terrain`, counting tiles |
| `convert` | `encoding`, `ready_to_publish`; these package events finish before installation |
| `mesh` | Named capture/leaf/hierarchy/archive work; required events finish before installation; see the mesh guide |

Events report work units, not time remaining.

## Environment variables

| Variable | Used by | Effect |
| --- | --- | --- |
| `RUSTY_TILES_NATIVE_DIAGNOSTICS=1` | vector | Print raw GDAL and GEOS warnings. The former alias `RUSTY_TILES_PYTHON_TRACEBACK` was removed in 0.4.0 and is ignored. |
| `PROJ_DATA` | geospatial commands | Directories holding `proj.db` and local grids |

Contributor test variables are listed in [CONTRIBUTING.md](../CONTRIBUTING.md#run-the-tests).

### One RGB raster tile into a directory

With a `native-geospatial` build, `raster-tile-to-directory` accepts the bounded
[D1 RGB profile](architecture/d1-raster-contract.md): one 256×256 RGB GeoTIFF
already aligned to an explicit Web Mercator XYZ tile.

```sh
rusty-tiles raster-tile-to-directory -i aligned.tif -o ./new-tile --zoom 3 --x 5 --y 2
```

The parent directory must exist on a supported local filesystem. Existing
outputs conflict by default. `--force` (or `-f`) replaces the final-path entry
present when publication holds the old output, including a file or symlink itself.
Replacement has an interval when the output path is absent; it does not promise
snapshot isolation, protection from unrelated writers, or crash durability.
It publishes the
PNG tile, `tilejson.json` and `report.json` together. `--json` returns the exact
`rasterReport` and cleanup diagnostics. Failed restoration returns
`error.recovery: {"output": ..., "previousOutput": ...}`; the previous output is
retained at that backup path. Resolve any current destination entry before
restoring it. Other failures have `error.recovery: null`. Readable path strings use the CLI
UTF-8 display convention; exact recovery filename units are also included. Backup cleanup failure after installation
remains success with actionable `cleanupDiagnostics`. Default builds return
`unsupported`.
See the [CreateNew contract](architecture/d1-directory-contract.md) and
[Replace contract](architecture/d2-directory-contract.md) for platform
support and guarantees. Broader imagery conversion remains the separate
`raster` command.

For failed directory restoration, `error.recovery.nativePaths` preserves exact
filename units alongside the readable recovery paths: `encoding` is
`unix-bytes` or `windows-utf16`, with `output` and `previousOutput` arrays.
Use these native values when a filename is not representable as a Unicode
string. Postcommit cleanup diagnostics do not change a successful exit status.

Raster `--force` rejects source/output overlap before work: the output cannot
be the source file, a hard-link alias, or a directory containing the source.
Parent aliases are checked by filesystem identity. Replacing a final output
symlink remains allowed because its referent is preserved.
