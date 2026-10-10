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

Writes a bounded 3D Tiles 1.1 GLB mesh directory. See the [terrain guide](TERRAIN.md).

| Option | Default | Meaning |
| --- | --- | --- |
| `--cells-per-leaf` | `64` | Source-pixel cells per patch edge: 16, 32, 64 or 128 |
| `--height-offset` | required | Metres added to raw DEM samples to obtain ellipsoidal height |
| `--fill-height` | required | Ellipsoidal height for missing lattice samples inside the footprint |

There is no zoom or simplification option. Source, height and precision limits
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

`INPUT` is a self-contained `.3tz` archive. Inspection is read-only and uses the
fixed [C1 validation profile](VALIDATION.md). Directories, including T1 terrain
and raster outputs, are rejected as invalid input; package supported content before inspection.

The typed JSON report lists `checks` and `notInspected`, per-payload accessor,
primitive and vertex counts, archive counts, and fixed admission `limits`.
Passing inspection means the listed checks completed within that profile.
`hierarchyBounds` checks declared child volumes; decoded world-space content
containment, source fidelity, geometric-error accuracy and metadata/material
semantics are not implied. Optional extensions retain explicit inspection gaps;
unknown required extensions fail as unsupported. The `archiveStoredRecordLayout`
check admits one disjoint stored-record layout and one directory interpretation.
Coherent nested records, alternate name encodings and unsupported ZIP64 descriptor
declarations receive `unsupported`; duplicate names and malformed records receive
`invalid_input`. The [validation profile](VALIDATION.md) gives the finite domain.

Failures use `invalid_input` (exit 3), `unsupported` (exit 2), `resource_limit`
(exit 3), or `io` (exit 1). Oversized inputs receive a resource-limit outcome
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
| `--host` | `127.0.0.1` | Listen address |
| `--port` | `9227` | Listen port. 0 picks a free port. |
| `--mesh` | none | Extracted mesh directory with `tileset.json` |
| `--point-cloud` | none | Extracted point-cloud directory with `tileset.json` |
| `--annotations` | none | Extracted vector directory with `tileset.json` |
| `--imagery` | none | Raster output directory with `tilejson.json` |
| `--terrain` | none | Terrain mesh directory with `tileset.json` |

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

Vector and point-cloud conversion use the foundation error categories: `invalid_request` and `unsupported` exit 2, `invalid_input` exits 3, `output_conflict` exits 5, and `io`, `cancelled`, `observer_failure` or `invalid_state` exit 1. Their JSON errors include `kind`, secondary diagnostics and retained paths. For example, vector `--lod-levels 40` names the permitted range and exits 2. Unsupported CRS capability is `unsupported`; malformed source records are `invalid_input`. The legacy categories in the table still apply to converters that have not migrated.

## mesh-local-to-3tz

F1c1 explicit placement is a development implementation candidate pending
independent review and final consumer/platform acceptance. The bounded source
profile remains [F1b3](architecture/f1b3-contract.md).

`mesh-local-to-3tz -i local.glb -o local.3tz --leaf-triangles 1000` converts the
bounded static GLB/glTF profile, explicitly interpreted as local
metres with Y up. `--leaf-triangles` is required and positive; it limits each
leaf's triangle count, not archive bytes, memory or geometric error. `--force`
uses completed-file replacement. This operation emits an explicit hierarchy
with full-detail leaf geometry and source-faithful PNG/JPEG core PBR
textures. Each used image is delivered once in the archive; UV0/UV1 bindings,
authored tangent frames, linear vertex colors, material factors, alpha masking
and sampler settings are preserved. Source CRS is explicit local metres/Y-up.

| Placement option | Meaning |
| --- | --- |
| `--anchor LON LAT HEIGHT` | WGS84 longitude/latitude degrees and explicit ellipsoidal height metres. Omission keeps local output. |
| `--orientation-xyzw X Y Z W` | Right-handed active ENU quaternion, scalar W last; requires anchor. Omission means identity. |
| `--scene-offset X Y Z` | Post-node glTF Y-up metre translation before orientation; requires anchor. Omission means zero. |

```sh
rusty-tiles mesh-local-to-3tz -i local.glb -o placed.3tz --leaf-triangles 1000 \
  --anchor 153.02 -27.47 25 --orientation-xyzw 0 0 0 1 --scene-offset 10 2 -5
```

The cartographic ENU basis uses declared latitude/longitude at the anchor;
supplied longitude fixes the meridian at a pole. Quaternion norm must differ
from one by at most 1e-12 and is normalized once. Longitude/latitude are bounded
to [-180,180]/[-90,90]. Invalid/nonfinite parameters fail before source I/O;
the [finite forward-magnitude limit](architecture/f1c1-contract.md#precision-and-limits)
is Unsupported before source I/O. Placement uses one f64 root transform; leaves
retain local Y-up geometry and every companion/resource association. Offset is
not a projected E/N/A shift, source CRS conversion or geoid correction. The F1c2 candidate emits primitive identity and exact triangle provenance through standard labeled feature sets; see the [identity/picking contract](architecture/f1c2-contract.md).

Relative local buffers and images are captured before callbacks. Network, data,
absolute and escaping resource URIs and dependency symlinks are refused.
Normal maps require authored normals and tangents. Tangent-bearing instances
require a uniform absolute scale with an orthogonal accumulated transform,
including reflections; arbitrary nonuniform tangent baking is refused.
Unsupported source semantics, including alpha blending, animation, extensions
and extras, fail consistently before output staging. See the [finite source,
numerical and admission contract](architecture/f1b3-contract.md). This command
is separate from the broader `mesh-to-3tz`; rejecting a local-profile source
does not silently route it through that operation.

`--json` returns `meshReport` with the same snake_case fields published in
`conversion.json`, plus `cleanupDiagnostics`. Schema 7/profile
`f1d2-adaptive-root-proxy-gltf-v1` records `source_coordinates="local-gltf"`, output
`coordinates` (`local-gltf` or `wgs84-ecef`), tagged `placement` with normalized
parameters, and the exact emitted `root_transform`. It counts the root document
as `source_bytes`; `external_files` and `external_bytes` count unique captured
dependencies, including unused ones. The output is its resolved
absolute installation path. `--progress json` uses fallible precommit domain
events; required observer/finalization failures preserve the previous destination.
Full-detail mode uses an extent-derived omission/selection root error. Leaves
retain all accepted triangles. An explicit root proxy is available with paired
`--root-proxy-triangles N --max-proxy-error-metres E` arguments. N must be positive;
E must be finite and positive. The first proxy profile accepts opaque, untextured,
positions-only geometry. It requires actual reduction, preserves authored
components and material factors, and certifies a complete surface-distance bound
before staging. Complete dyadic proof patches tighten the bound without changing
the emitted geometry. The emitted root error is E after the bound is proved at
most E; `meshReport.approximation.certificate` records `error_metres`, performed
`patch_face_tests`, `accepted_patches` and `max_depth`. The fixed proof profile
allows depth 24 and 16,777,216 global patch/target-face evaluations. Exhausting
these limits means the requested bound could not be certified, even when the
true distance is zero.
Coarse picks expose `proxy_region` membership arrays; fine leaves retain exact
`source_triangle` picking. Unsupported profile, target, error or work requests
fail before staging. The [F1d2 contract](architecture/f1d2-certificate-contract.md)
records proof limits, including a potentially loose certificate and no general
appearance guarantee. The
[bounded evidence](../bench/architecture_audit/mesh_approximation_f1d2/README.md)
records independent decoding, public consumer queries and separate review.

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
| `point-cloud` | `ingestion`, `tiling`, `point_archive`, `ready_to_publish`; domain events finish before installation |
| `vector` | `ingestion`, `encoding` |
| `raster` | `cog`, `display`, `tiling`. `tiling` counts XYZ tiles. |
| `terrain` | `terrain`, counting tiles |
| `convert` | `encoding`, `ready_to_publish`; these package events finish before installation |
| `mesh-local-to-3tz` | `mesh_approximation` when requested, `mesh_leaves`, `mesh_archive`, `ready_to_publish`; domain events finish before installation |

Events report work units, not time remaining.

## Environment variables

| Variable | Used by | Effect |
| --- | --- | --- |
| `RUSTY_TILES_NATIVE_DIAGNOSTICS=1` | vector | Print raw GDAL and GEOS warnings. The deprecated `RUSTY_TILES_PYTHON_TRACEBACK=1` still works when this is unset. |
| `RAYON_NUM_THREADS` | mesh | Limit worker threads. Terrain encoding is sequential. |
| `PROJ_DATA` | geospatial commands | Directories holding `proj.db` and local grids |
| `RUSTY_TILES_DISABLE_NATIVE_JPEG=1` | build | Force the portable Rust JPEG encoder even when `native-jpeg` is enabled; default builds already use portable JPEG |

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
