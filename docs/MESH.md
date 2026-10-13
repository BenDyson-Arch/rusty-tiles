# Mesh tiling

`rusty-tiles mesh` and Python's `rusty_tiles.mesh_to_tileset` stream one admitted
local static glTF/GLB scene into a 3D Tiles 1.1 implicit forest in one `.3tz`.
Content is glTF 2.0. The encoder is bundled: meshopt and KTX2/Basis need no
external executable or system geospatial library in the standard CLI or wheel.

The current pipeline captures the source, writes full-detail leaf GLBs, and
publishes their hierarchy. Parents currently carry no coarse content, so a far
view can still load every visible leaf. Certified geometry LOD and proxy texture
baking are separate downstream work; this interface will also serve that
pipeline when it lands. A successful conversion does not establish the final
tiler's resource, fidelity, scale or consumer acceptance.

## Convert a local scene

```sh
rusty-tiles doctor --command mesh
rusty-tiles mesh -i local.glb -o local.3tz --jobs auto
rusty-tiles validate local.3tz
```

Input and output are filesystem paths. Output must end in `.3tz`; directory
output and `.3dtiles.zip` are not tiler products. An existing destination is a
conflict unless `--force` (or `-f`) is supplied. Replacement installs a completed
archive after required work and callbacks finish. Failure before installation
preserves the previous destination. The output cannot overwrite or alias the
source or its dependencies. Installation does not promise power-loss durability.

## Source and placement

The source is **local metres with glTF Y up**. Node transforms are evaluated as
part of capture. Each source POSITION component and each component after the
accumulated node transform must lie within ±1,000,000 metres, before the final
f32 rounding. This finite source domain is separate from the placement precision
domain. Coordinates are never interpreted as geographic or projected because of
their magnitudes, file names or an offset file.

The admitted source has a default or sole scene, TRIANGLES, bounded local
resources and core PBR materials. It supports authored normals/tangents, UV0/UV1,
COLOR_0, and all five core PBR texture slots. Source PNG/JPEG images remain
supported. Normal maps require authored normals and tangents. Tangent-bearing
geometry requires a conformal accumulated transform, including supported
reflections. Animation, skins, morph targets, alpha blending, imported
extensions/extras/metadata, remote/data image resources and unadmitted semantics
are refused; they are not silently discarded. Dependencies must stay confined
to the source directory, without escaping/absolute/network URIs or symlinks.
Source files must remain stable through capture; change detection is
observational, not an atomic filesystem snapshot.

Without an anchor the tileset stays in its local frame. To place it on Earth:

```sh
rusty-tiles mesh -i local.glb -o placed.3tz \
  --anchor 153.02 -27.47 25 \
  --orientation-xyzw 0 0 0 1 --scene-offset 10 2 -5
```

`--anchor` takes longitude and latitude in WGS84 degrees and **ellipsoidal height
in metres**, including an explicit zero when intended. Longitude must be in
[-180,180], latitude in [-90,90], and all placement numbers must be finite.
`--orientation-xyzw` is an active right-handed ENU quaternion with scalar W last;
its norm must be within 1e-12 of one and is normalized once. `--scene-offset` is
a metre translation in source glTF Y-up coordinates after node transforms and
before orientation. Both require an anchor. Their defaults with an anchor are
identity rotation and zero translation. `--orientationXyzw` and `--sceneOffset`
are equivalent camelCase spellings.

Placement applies one rigid transform at the tileset root. No projected CRS,
source-axis inference, Metashape E/N/A shift or geoid-height conversion is
performed. A large finite anchor/offset outside the shared precision budget is
`unsupported`; an invalid number, range or quaternion is `invalid_request`.

## Profiles and resources

`--profile` accepts `fast`, `balanced` (the default), `quality` or `lossless`.
These are **provisional policies**: leaf sizing, texture resolution, encoder
effort and other constants will be calibrated together after coarse geometry
and baking exist. Do not build scripts around today's numerical constants.
Every present PBR slot follows its colour/data class and the chosen policy.
Lossy output uses in-process KTX2/Basis; lossless leaf textures use PNG.
`lossless` describes the agreed leaf texel policy, not exact source GLB bytes,
arbitrary viewer filtering, or lossless coarse geometry. Leaf coordinate
encoding and provenance still follow the tiler contract.

`--jobs auto` is memory-aware. A positive `--jobs N` is a **maximum**, also capped
by available cores and each stage's memory plan. The diagnostics report the
requested choice and actual workers independently for capture and leaf
delivery. When available memory cannot be queried, the conservative policy
uses one worker. A one-worker floor does not authorize an item that cannot fit;
required memory or scratch exhaustion returns `resource_limit`.

`--scratch-dir PATH` (`--scratchDir`) selects the parent for owned working data.
Without it, capture uses the platform scratch default and private leaf/forest
intermediates use the output parent. Final archive staging remains on the output
filesystem beside the destination. Disk-backed scratch is usually
preferable for large sources; memory-backed scratch consumes memory in the
worker plan and is reported. Capacity is checked by stage, but a preflight is
not a reservation against unrelated writers. Owned intermediate files are
cleaned on success and failure. Any cleanup failure carries actionable paths
and does not replace the original cause.

There is no total source-byte or triangle-size refusal. JSON, hierarchy,
individual-item and format limits still apply. The 3TZ member ceiling is
178,956,970, including the root document, reports and index; exceeding it returns
`unsupported`. Per-tile budgets are derived from policy. There is no
`--leaf-triangles`, proxy triangle/error flag, explicit/full-detail selector,
texture-format option, external encoder fallback or compression bypass on
`mesh`.

## Python

```python
from pathlib import Path
import rusty_tiles

result = rusty_tiles.mesh_to_tileset(
    Path("local.glb"), Path("placed.3tz"),
    profile="balanced", jobs=None, scratch_dir=None,
    anchor=(153.02, -27.47, 25.0),
    orientation_xyzw=(0.0, 0.0, 0.0, 1.0),
    scene_offset=(10.0, 2.0, -5.0),
    force=False, callback=None,
)
print(result.report["triangles"], result.diagnostics["leaf_delivery"]["jobs"])
```

`mesh_to_tileset(input, output, *, profile="balanced", jobs=None,
scratch_dir=None, anchor=None, orientation_xyzw=None, scene_offset=None,
force=False, callback=None)` accepts `str` or `os.PathLike` paths. `jobs=None`
means auto; a positive integer is a maximum. Booleans and noninteger worker
counts are rejected. Unknown legacy keywords raise `TypeError`. Python calls
the same core operation without launching the CLI or an encoder, and releases
the GIL during native work.

The frozen `TilerResult` has resolved absolute `output: pathlib.Path`,
`archive=True`, required `report` and `diagnostics` dictionaries, and
`cleanup_diagnostics`. Frozen attributes do not make the dictionaries deeply
immutable. Paths are bound before callbacks, so callback changes to the current
directory cannot redirect publication. Required callbacks are precommit and
fallible; a winning callback exception is preserved as the original Python
exception. Python result materialization or unrelated asynchronous exceptions
after installation are outside the core publication guarantee.

## Reports and errors

`--json` returns one stdout object with `ok`, resolved `output`, `tilerReport`,
`diagnostics` and `cleanupDiagnostics`. `counts` and `conversionReport` locate
the same archive-resident `conversion.json`. The versioned `tilerReport` uses
snake_case and equals Python's `result.report` and the published report.

The semantic report includes profile, source/resource counts, leaf/triangle/GLB
counts and maximum leaf geometry error, hierarchy/implicit/document counts,
depth and packed/synthetic leaf counts, member limits, separate root/tileset
errors, and normalized placement. `source_coordinates` is `local-gltf`;
`coordinates` is `local-gltf` or `wgs84-ecef`. `root_transform` is null for local
output, whose root has no transform, or the exact emitted column-major matrix
for placed output. Current parent traversal errors are not proxy certificates.
No LOD or appearance certificate is fabricated for stages that have not landed.

Runtime diagnostics stay **outside the archive** so worker/scratch choices do
not alter its semantic bytes. They include `requested_jobs`, `capture`,
`leaf_delivery`, `archive_bytes` and `peak_held_records`. Stage objects describe
worker admission and owned memory/scratch resources; their declared accounting
is not an automatically measured RSS guarantee. Absolute scratch paths and
resource observations are diagnostics, not semantic profile constants.

`--progress json` emits NDJSON work events on stderr with named phases and
nullable totals. Required events finish before publication; the returned
successful result establishes installation. A post-install cleanup problem
remains success with cleanup diagnostics.

| Core failure | CLI exit / JSON code | Python exception |
| --- | --- | --- |
| Invalid request | 2 / `invalid_request` | `InvalidRequestError` |
| Malformed or changed source | 3 / `invalid_input` | `DataError` |
| Unsupported semantics/domain/container size | 2 / `unsupported` | `UnsupportedError` |
| Resource refusal | 1 / `resource_limit` | `ResourceLimitError` |
| I/O | 1 / `io` | `TilesIOError` |
| Existing output | 5 / `output_conflict` | `OutputExistsError` |
| Cancellation | 1 / `cancelled` | `CancelledError` |
| Required observer failure | 1 / `observer_failure` | `ObserverError` or original winning callback exception |
| Internal invariant failure | 1 / `invalid_state` | `TilesError` |

CLI parser/usage errors happen before the core and use exit 2 and
`error.code="usage"` when `--json` is present. Core errors carry their domain
`kind`, secondary diagnostics and retained paths. Request/placement/output
intent is validated before source I/O; source admission, resource work and
publication then retain causal failures. Cleanup does not replace the primary
failure. `doctor --command mesh` checks build capability read-only; it does not
approve a particular source, resource plan, filesystem or viewer.

## Migration from older mesh converters

The new public workflow replaces the preparatory `mesh-leaf-payloads`,
`mesh-leaves-to-3tz` and `mesh-large-to-3tz` commands. Leaf directories and their
manifests are internal stage artifacts; there is one public source-mesh
operation.

`mesh-to-3tz` / `mesh_to_3tz` and `mesh-local-to-3tz` / `mesh_local_to_3tz`
remain available temporarily while full tiler acceptance is completed. They
are deprecated and will retire together with their old writers. Existing
legacy calls continue to select those old products; they are not aliases of
`mesh`.

Prepare geographic/projected meshes as admitted local metre/Y-up glTF before
using the new tiler. Supply an ellipsoidal anchor, ENU quaternion and local scene
translation as needed. Mesh `auto` CRS, EPSG/WKT/PROJ, source-axis/height
conversion, Metashape shifts/offset files and old Euler placement do not migrate
to new flags. Vector/point CRS support remains a separate retained capability.

Old leaf/proxy triangle and error knobs, atlas sizes and JPEG/WebP/general-PNG
output selectors have no one-to-one replacements: choose a profile. JPEG source
decoding remains distinct from retired JPEG output encoding. Both original
primitive and triangle source identities are always carried for leaf picking;
leaf-local table row IDs are not global business IDs. The implicit forest can
contain an explicit skeleton and link documents without offering an explicit
output mode. `glb-to-3tz` / `glb_to_3tz` and `createTilesetJson` /
`model_to_manifest` remain the separate exact-byte wrapping/sibling-manifest
products.

## Known Cesium colour limitation

CesiumJS 1.146.0 has a known KTX2 colour-filtering limitation. In the tested
Chromium WebGL2 setup with an NVIDIA RTX 4080 SUPER, high-contrast base-colour
and emissive textures filtered incorrectly with ASTC, BC7 and ETC2 transcode
targets despite correct authored sRGB mip levels and transfer metadata. This
can change their appearance, especially between texels or mip levels. The
linear material-data control was unaffected apart from codec error. This is a
bounded viewer finding, not a claim about all devices or other viewers. The
tiler keeps its class-correct textures; it does not bake a viewer-specific
colour correction.

The retained investigation is
`~/.cache/rusty-tiles-evidence/h19-cesium-srgb-scope-20261013/report.md`, SHA-256
`4ca7085ed6317de75709283d0eb9b9ea96640df4353b3705d1734ce4595d89c0`, with
`target-report.json`, SHA-256
`cad4371f2a93a432438945cff15c67dcbdec64e002bfb0eccf8dc23553d0be7d`.
Its generic uncompressed KTX2 diagnostic is not a legal Basis glTF binding.
This documentation uses retained evidence; it adds no new browser execution or
upstream issue.

Worker diagnostics describe capture and active leaf workspace admission. Capture
scratch bounds cover capture files only; accumulated leaf/forest intermediates
and the final archive are outside those bounds. Prefer disk-backed scratch for
large inputs, especially when the output directory is memory-backed. Complete
pipeline scale and memory evidence remain separate acceptance gates.
