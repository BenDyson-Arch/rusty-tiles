# Mesh tiling

`rusty-tiles mesh` and Python's `rusty_tiles.mesh_to_tileset` stream one admitted
static glTF/GLB scene, in local metres or a declared source CRS, into a 3D Tiles
1.1 implicit forest in one `.3tz`.
Content is glTF 2.0. The encoder is bundled: meshopt and KTX2/Basis need no
external executable or system geospatial library in the standard CLI or wheel.

The current pipeline captures the source, writes full-detail leaf GLBs, certifies
coarse geometry for every parent tile (H17: simplified proxies whose geometric
error bounds their two-way distance to the source), bakes proxy textures for
those coarse tiles from the source appearance (H27), and publishes the hierarchy.
A successful conversion does not establish the final tiler's resource, fidelity,
scale or consumer acceptance.

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

By default the source is **local metres with glTF Y up**. Node transforms are
evaluated as part of capture. Each source POSITION component and each component
after the accumulated node transform must lie within ±1,000,000 metres (source
units with a [source CRS](#source-crs)), before the final f32 rounding. This
finite source domain is separate from the placement precision domain.
Coordinates are never interpreted as geographic or projected because of their
magnitudes, file names or an offset file; a source CRS is always declared.

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

Placement applies one rigid transform at the tileset root. It never converts a
projected CRS, infers source axes, applies an E/N/A shift or converts geoid
heights; use a [source CRS](#source-crs) for projected or geographic sources. A
large finite anchor/offset outside the shared precision budget is
`unsupported`; an invalid number, range or quaternion is `invalid_request`.

## Source CRS

Projected or geographic sources, such as Metashape GLB exports in EPSG:3857
with an `offset.txt` shift, declare their CRS instead of a rigid placement:

```sh
rusty-tiles mesh -i site.glb -o site.3tz \
  --source-crs EPSG:3857 --source-offset-file site_offset.txt
# the same request, spelled out:
rusty-tiles mesh -i site.glb -o site.3tz \
  --source-crs EPSG:3857 --source-axes y-up \
  --source-offset 14812000 -1384000 100 --height-offset 0
```

| Option | Meaning |
| --- | --- |
| `--source-crs` (`--sourceCrs`) | `local` (the default) or a 2D horizontal EPSG, WKT or PROJ definition, resolved by the shared [conservative CRS resolver](FORMATS.md#point-clouds) |
| `--source-axes` (`--sourceAxes`) | Default `y-up` (glTF is Y up): X=E, Y=height, Z=−N. `xyz` is X=E, Y=N, Z=height. Axes are read after node transforms and never inferred from data |
| `--source-offset E N [A]` (`--sourceOffset`) | Added in double precision: E/N in horizontal CRS units, A metres (default 0). Default `0 0 0` |
| `--source-offset-file` (`--sourceOffsetFile`) | A Metashape offset file holding exactly the lines `E: <number>`, `N: <number>`, `A: <number>` in that order (optional final LF or CRLF, at most 4 KiB). Anything else is `invalid_request`; excludes `--source-offset` |
| `--height-offset` (`--heightOffset`) | Default `0`: metres added to source height plus A to give WGS84 **ellipsoidal** height. The default is right only when source height plus A is already ellipsoidal |

A source CRS cannot be combined with `--anchor`, `--orientation-xyzw` or
`--scene-offset` (`invalid_request`), and `local` admits none of the source-CRS
options. Legacy `auto`, `geographic` and Web Mercator keywords and bare codes are
not accepted: name the CRS, for example `EPSG:4326` or `EPSG:3857`. Horizontal
feet never rescale source height or A, and no geoid model is applied: Metashape
heights that are orthometric need an explicit `--height-offset` to the
ellipsoid.

Capture maps each post-node source position plus the offset through the CRS to
WGS84 ECEF and into one local east-north-up frame in f64, then rounds once to
f32; positions are mapped exactly for every vertex. The frame's origin is the
mapped centre of the used post-node source bounds; the root transform carries
ENU to ECEF. Authored normals are transformed by the inverse transpose of the
Jacobian at the vertex and renormalized. Tangents are transformed by the
Jacobian, then re-orthonormalized against the transformed normal (Gram–Schmidt)
because a CRS map is not conformal in 3D (horizontal scale differs from
vertical); the W handedness sign is kept (it would flip only for an
orientation-reversing Jacobian). Jacobians use central differences through the
same CRS operation (one-sided at a projection-domain edge), evaluated exactly at
the nodes of a lattice over the source bounds (at most 32 cells per axis, each
nominally 1 km horizontally or 0.01° geographic and 100 m vertically) and
interpolated trilinearly. A cell whose interpolated Jacobian at its centre
differs from the exact one by more than 1e-7 (relative), or that touches the
CRS domain edge, uses per-vertex Jacobians instead. The resulting normal error
is about 1e-7 times the Jacobian's condition number (for EPSG:3857 about
1/cos(latitude), so about 2e-7 rad at 60°); the centre check estimates the
quadratic interpolation term, not higher-order terms. Measured errors in the
tests are below 1e-7 rad, including a 150 km site at 60° whose cells sit at the
check's edge. The report states the lattice and its
per-vertex cells. Sorting, leaves, coarse levels and delivery then work in local
metres. Every source coordinate must lie in the CRS's verified domain; a vertex
outside it is `invalid_input`, and a mapped position beyond ±1,000,000 local
metres is `unsupported`.

EPSG:4326, EPSG:3857, EPSG:3395, UTM (EPSG:326xx/327xx) and the grid-free PROJ
methods listed for point clouds work in the standard CLI and wheel. Other EPSG
codes, coordinate epochs and Lambert azimuthal equal area need the
`native-geospatial` build and its PROJ database; without it they are
`unsupported` before source I/O. One operation serves the whole capture: a
source that would need native PROJ for only some vertices is refused.

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

In a lossy profile, a texture page that misses any decoded-image gate at the
profile's own encoder setting (per-level channel and normal floors, MASK
coverage, the source-sampler check) is re-encoded one step stronger instead of
failing the run: UASTC pack level 2, then UASTC level 3. Every step is checked
again by the same gates; gates are never loosened. If level 3 also misses and
the page's sampler reads level 0 only (no mipmap minification), the page is
delivered as an exact level-0 PNG, which keeps the sampler. A mip-sampled page
that still misses at level 3 fails the run, because glTF PNG cannot carry its
authored coarse levels. Escalation costs output size: an ETC1S page re-encoded
as UASTC is several times larger, and a PNG fallback page is typically the
largest form of the page (up to 4 bytes per texel). The ladder depends only on
the page, so output is identical for any worker count. Cancellation is polled
before every re-encode step, so a run can stop between steps of one page.

### Coarse tile textures

Every coarse (proxy) tile is baked after its certified geometry is fixed. The
bake keeps the proxy's exact POSITION words, node transform, face order, source
and material identity and every feature-ID set, adds new chart UVs, and fills
each present PBR slot (base colour, metallic-roughness, normal, occlusion,
emissive) by sampling the captured source images.
Lossy profiles deliver proxy pages through the same KTX2/Basis gates and
escalation ladder as leaves; `lossless` delivers a PNG base level. The complete
proxy GLB is read back before it replaces the untextured one, and a tile that
would exceed the per-tile byte budget is rebaked at a lower texel density.

A proxy starts at its children's texel density: the coarsest child's
world-area-weighted median texel spacing (a few near-degenerate source UV
triangles do not coarsen it), or a profile fraction of its geometric error if
that is coarser (`balanced`: one sixteenth; `fast` twice that; `quality` and
`lossless` half).

Appearance rules:

- **Sub-texel slivers have no texture.** A face whose exact page footprint is
  strictly under one square texel, and whose quantized UVs collapse or invert,
  is published without a texture sampling obligation (a no-raster face). It
  keeps its geometry, corner normals/colours and feature IDs; it covers no texel
  centre, so it would have sampled the gutter or a neighbouring chart anyway. A
  collapse at or above one square texel is a chart defect: the chart is
  repacked at a higher density, and the run refuses after four attempts.
  Zero-area faces are always no-raster.
- **Feature IDs above 2^24 are refused.** glTF says UNSIGNED_INT vertex
  attributes SHOULD NOT be used; that guidance is advisory, but rusty-tiles
  chooses not to write them. A feature-ID attribute is therefore UNSIGNED_BYTE,
  UNSIGNED_SHORT or FLOAT, and FLOAT holds every integer only up to 2^24. A
  larger ID is `unsupported` rather than rounded.

- **A face turned away from its source samples the nearest source surface.**
  A proxy texel samples the nearest point of a source triangle of the face's own
  primitive that faces the proxy face. Simplification can fold or flip a face so
  that no triangle of its primitive in the tile faces it (a flipped sliver); that
  face then samples the nearest same-primitive surface point regardless of
  orientation, with the same material, texture and tie-break order, and its
  normal-map texels keep the source's world-space direction. A donor farther
  than two longest edges of the face is refused as `unsupported`, with the
  distance in the message. The run diagnostics count these texels, faces and
  tiles (`proxy_bake`).

Known limitations (0.4):

- A proxy directly above leaf tiles samples the original source images, not
  the leaves' delivered pages, because leaf pages keep no per-texel terminal
  ledger. This is correct but slower than resampling the children; keeping
  that ledger is a planned 0.4.x follow-up.
- Baking dominates conversion time. Proxies of one depth bake in parallel
  (up to `--jobs`), but one proxy bakes on one thread, so the narrow top of the
  tree runs serially. On a 0.79M-triangle textured crop (eight hardware
  threads, `--jobs 8`) the whole conversion took about two minutes, about
  100 s of it baking 63 coarse tiles.
- A proxy whose retained children do not fit its worker's share of the bake
  memory is baked again after its depth, alone with the whole allowance, so
  `--jobs` does not change which inputs convert (or their output) but can add
  serial time.

Leaf normals are a profile property. `lossless` keeps the f32 source normal
(after the node transform, renormalized at capture). `fast`, `balanced` and
`quality` store 12-bit octahedral codes through the `EXT_meshopt_compression`
OCTAHEDRAL filter; viewers decode them to normalized 16-bit `NORMAL`
(`KHR_mesh_quantization`). The angle to the source normal is at most about 0.06°
(0.0596° maximum and 0.020° mean over the 73 million leaf corners of a
24-million-triangle scene). The bit count is provisional, like the other
profile constants. The per-triangle provenance tables (`source_primitive`,
`source_triangle`) are losslessly meshopt-compressed in every profile. Their
schema, columns and decoded values are unchanged.

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
worker plan and is reported. Admission requires free scratch for the largest
declared stage working set (files retained from earlier stages, H17's pre-pass,
and each external sort's runs). Sort runs are 8 MiB chunk files removed as soon as
they are read, so a sorted stream and the output it feeds share space, with at
most `F` partly read chunks of slack. A 10% margin and per-file rounding are
added: about 0.5 KB per source triangle at scale with every vertex attribute present.
Capture stores only the vertex attributes some selected primitive carries, so sources
with fewer need less: about 0.3 KB with positions and one texture coordinate set.
It also reserves output-side space for leaf staging, archive
staging and a replaced destination: three copies of an estimate of 119 bytes per
triangle (measured before the archive-size reduction, which brought it to about 47; kept as a conservative estimate) plus the encoded image bytes at admission, raised once image dimensions
are known to 4 bytes per source texel for `lossless` PNG pages or 1 byte per
texel for KTX2 profiles, each with a 4/3 mip allowance (a texture-stage
recheck). These are estimates, not bounds. Output demands are added to the
scratch figure when they share a filesystem, told apart by device id: btrfs
subvolumes of one pool report different ids, so their demands are checked
separately against the same free space. Memory-backed (tmpfs) scratch is charged
to the worker memory plan, but a work directory or output on tmpfs is not.
Each capture stage rechecks free space for its own working set on entry and
fails closed if its measured peak exceeds its declared bound; the diagnostics
report both (`scratch_stage_bytes`, `scratch_stage_peak_bytes`). The H17
pre-pass is measured the same way, and the whole-run spill peak
(`run_scratch_peak_bytes`) must stay within the declared need. LOD proxy
sidecars and bake handoff files are budgeted by their own stages and are not in
these spill figures. A preflight is
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

### Primitives, atlases and editability

Each tile publishes as few glTF primitives as glTF allows
([H28 contract](architecture/h28-tile-batching-contract.md)). Source primitives
share one node, mesh, primitive, material and texture page set when their
materials belong to one class and they carry the same vertex attributes. A class
means equal factors, `alphaMode`, MASK `alphaCutoff`, `doubleSided`, the same bound
slots with equal normal scale and occlusion strength, and equal source min/mag
filters. An omitted value equals its glTF default; texture identity and wrap
modes do not count. A leaf's
merged positions are rebased onto one shared quantization offset without being
quantized again, so every decoded vertex is bit-identical to a per-primitive
encoding. Primitives split only where those properties differ, where a mipmapped
source sampler keeps a dedicated leaf page in a lossy profile, or where a class
overflows one page. Picking is unchanged: every triangle keeps its
`source_primitive` and `source_triangle` (leaf) or `proxy_source` (proxy)
feature ID, with one property table per feature-ID set.

Proxies carry no `baked_feature_id_*` textures. Each proxy feature-ID set is
already an exact per-face vertex attribute against the same property table,
which those textures only repeated.

Editability:

- **Leaves** are exact in vertex space: every triangle keeps its source
  provenance. They are also exact in texture space when no two triangles'
  decoded UVs share interior area on a page, so each texel has one owner. The
  tiler checks this exactly after atlasing. Overlapping, mirrored or repeated
  source UVs make a leaf `shared`, which is editable in vertex space only; such a
  leaf is never refused.
- **Proxies** are exact in texture space, because charts never overlap. In
  vertex space they are exact only to the source primitive, or to the majority
  source: a proxy triangle comes from simplified geometry, and its
  `proxy_source` row names one source triangle (or a fill).

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
projected = rusty_tiles.mesh_to_tileset(
    Path("site.glb"), Path("site.3tz"),
    source_crs="EPSG:3857", source_offset_file=Path("site_offset.txt"),
)
print(result.report["triangles"], result.diagnostics["leaf_delivery"]["jobs"])
```

`mesh_to_tileset(input, output, *, profile="balanced", jobs=None,
scratch_dir=None, anchor=None, orientation_xyzw=None, scene_offset=None,
source_crs=None, source_axes=None, source_offset=None, source_offset_file=None,
height_offset=None, force=False, callback=None)` accepts `str` or `os.PathLike`
paths. `source_crs=None` or `"local"` keeps local input; `source_axes` is
`"y-up"` (default with a CRS) or `"xyz"`, `source_offset` a two- or three-number
sequence, `source_offset_file` a path, and `height_offset` defaults to 0, with
the CLI's rules. `jobs=None`
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
errors, and normalized placement. Escalation is reported as `escalated_pages`
(`uastc_level2` and `uastc_level3` counts, by the step that delivered the page),
`lossless_fallback_pages` (pages delivered as exact PNG), and
`escalation_diagnostics`: the first 32 escalated pages in leaf order, each with
its `leaf`, `first` source triangle rank and `depth`, `page`, `slot`, `sources`
(`image`, `material`), delivering `step`, and every gate it missed on the way
(`misses`: `step`, `mip` or null for the all-level sampler check, `gate`).
`escalation_diagnostics_omitted` counts escalated pages beyond that bounded
list. `texel_ownership` reports texture-space editability:
`unique_leaves`, `shared_leaves`, the `shared_triangles` of shared leaves,
`unique_proxies` (always every proxy), and `shared`. That list holds the first
64 shared leaves (`leaf`, `first`, `depth`, `texel_ownership: "shared"`,
`triangles`); `shared_omitted` counts the rest. Every leaf that is not listed
or omitted is `unique`. Ownership is all-or-nothing per leaf: one pair of
overlapping footprints marks the whole leaf `shared`, so use `shared_triangles` (and
a leaf row's `sharedTriangles`) over the leaf's `triangles` for the fraction of the
leaf that is affected. `unique_proxies` is vacuous when no baker runs: untextured
proxies have no pages to share. `source_coordinates` is `local-gltf` or
`source-crs`; `coordinates` is `local-gltf` or `wgs84-ecef`. A source CRS adds
`source_crs` (definition, axes, source offset, height offset, `portable` or
`native` operation, the chosen origin, `jacobian_lattice_cells` and
`jacobian_exact_cells`) and reports its derived ENU frame as a
`wgs84` placement at that origin. `root_transform` is null for local
output, whose root has no transform, or the exact emitted column-major matrix
for placed output. A parent tile's geometric error is its proxy's certified
error ([H17 contract](architecture/h17-lod-contract.md)). Proxy textures are an
appearance approximation (see [Coarse tile textures](#coarse-tile-textures)); no
appearance certificate is fabricated, and baking never changes a proxy's
geometry or geometric error.

Runtime diagnostics stay **outside the archive** so worker/scratch choices do
not alter its semantic bytes. They include `requested_jobs`, `capture`,
`leaf_delivery`, `archive_bytes`, `peak_held_records`, `phase_seconds` and
`proxy_bake`. `phase_seconds` holds the wall seconds of `capture`, `leaf_build`,
`lod_prepass`, `lod_simplify`, `seam_resolution`, `lod_finish`, `proxy_bake` and
`archive` (the three middle LOD phases summed over depths). `proxy_bake` holds
the baked `tiles`, `retried_tiles` (tiles that refused their worker's share of
the bake memory and were baked again alone, serially), worker seconds per bake step (charts, raster, encode,
assemble, commit; summed over workers, so they overlap) and the appearance
fallback counts (`facing_fallback_samples`, `_faces`, `_tiles`;
`fill_fallback_samples`). Stage objects describe
worker admission and owned memory/scratch resources; their declared accounting
is not an automatically measured RSS guarantee. Absolute scratch paths and
resource observations are diagnostics, not semantic profile constants.

`--progress json` emits NDJSON work events on stderr with named phases and
nullable totals: `conversion`, `capture`, then inside the leaf stage
`leaf_build` (leaves), `lod_prepass`, `lod_simplify`, `seam_resolution` and
`lod_finish` (each counting finished depths), `proxy_bake` (tiles), then
`leaf_payloads`, `archive` and `ready_to_publish`. Each finished phase, and each
LOD and bake depth, also logs its wall seconds and the run's elapsed seconds as
a `log` event; the first 16 proxy tiles that take the facing fallback log their
id, then one line says the rest are only counted.
Required events finish before publication; the returned
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

Geographic/projected meshes use `--source-crs`; the legacy Web Mercator adapter
call `--source-crs epsg:3857 --source-offset E N A` becomes
`--source-crs EPSG:3857 --source-offset E N A` (default `y-up` axes and zero
height offset), and `--source-offset-file` accepts only the strict
`E:`/`N:`/`A:` format. Mesh `auto` CRS, the `geographic`/Web Mercator keywords
and old Euler placement do not migrate to new flags. Vector/point CRS support remains a separate retained capability.

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

The measured cause is that CesiumJS 1.146.0 uploads KTX2 sRGB colour textures
as linear (UNORM) formats on the ASTC, BC7, ETC2 and uncompressed paths, so
filtering happens on encoded values. The worst case measured was 72/255 on
high-contrast test textures. Linear data textures are unaffected. The limitation
is documented rather than worked around: it is not filed upstream, and the tiler
adds no viewer-specific compensation.

### Colour space and lighting in CesiumJS

What the tiler writes. Base colour and emissive pages of leaves and of baked
coarse (proxy) tiles are sRGB-encoded: lossy profiles carry the sRGB transfer in
the KTX2 container, and proxy pages are averaged in linear light and re-encoded
to sRGB when baked. Metallic-roughness, occlusion and normal pages are linear
data. The tiler's tests pin that a proxy's base colour uses the same sRGB
transfer as its leaves and reproduces a flat source colour.

Lit and unlit tiles. Whether a tile is lit depends on its vertex attributes, not
on a setting of the tiler. A source without NORMAL (typical photogrammetry
exports) gives leaves with POSITION and TEXCOORD_0 only, and CesiumJS 1.146
draws such tiles unlit, so the texture colour is shown as sampled. Before
commit `bc6b3710` (#237) the proxies of such a source also published NORMAL,
TANGENT and COLOR_0. Cesium lit the proxies (darker, with a blue cast) and drew
the leaves unlit, so a view that mixed the two showed hard seams and a visible
tint step. Proxy primitives now publish NORMAL and COLOR_0 only where their
source primitive's leaves carry them, and TANGENT only with a baked normal map
(which brings NORMAL). Leaves and proxies of one source therefore shade the same
way. A source that does carry normals is lit in Cesium for leaves and proxies
alike, so its apparent brightness follows Cesium's lighting.

What is verified and what is not.

- Verified: the 72/255 and 61/255 worst-case errors above (base colour and
  emissive) in the H19 probe, with numeric shader samples of KTX2 pages; the
  lit-versus-unlit mismatch and its removal on one 0.79M-triangle photogrammetry
  crop in CesiumJS 1.146 (a visual check by the maintainer, not a pixel measurement);
  and the attribute set of proxies against leaves, which the tests pin.
- Not verified: how much of the filtering error shows on the unlit path as
  against the lit one (H19 measured sampled values, not a full rendered
  appearance); other CesiumJS versions, GPUs, browsers and viewers; whether PNG
  pages (`lossless` profile) avoid the error (PNG loading was outside the H19
  measurement); and any lighting setting that hides a tint step on a source that
  carries normals.

What to configure. Nothing in the tiler or in the viewer is required for valid
output, and the tiler has no option for a tile's colour space or lighting. The
filtering limitation affects how high-contrast base colour and emissive
textures blend between texels and mip levels. The authored sRGB mip levels and
transfer metadata are correct, so do not edit the pages to compensate. The H19
probe found that uploading the same KTX2 payload with a matching sRGB format
removes the error. This is the loader's behaviour, and no CesiumJS release or
setting that does it has been tested here. If colour accuracy matters, check
the result in your own viewer.

Worker diagnostics describe capture and active leaf workspace admission. Capture
scratch bounds cover capture files only; accumulated leaf/forest intermediates
and the final archive are outside those bounds. Prefer disk-backed scratch for
large inputs, especially when the output directory is memory-backed. Complete
pipeline scale and memory evidence remain separate acceptance gates.
